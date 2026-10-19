//! Instantiation: import resolution, allocation of memory, table and globals,
//! and segment application.
//!
//! Import resolution is not built into the core; it is delegated to the
//! `Resolver` the port passes in. The exact-match HAL link
//! (docs/abi-spec.md §6.4) is port policy, not the core's job. The `spectest`
//! shim for the spec tests is supplied through the same mechanism.

use crate::arena::Arena;
use crate::config::Config;
use crate::error::{Error, Result, Trap};
use crate::module::{Data, ExportDesc, ImportDesc, Module};
use crate::reader::Reader;
use crate::types::{FuncType, GlobalType, Limits, ValType};
use crate::validate::Validated;

/// Bytes per page.
pub const PAGE_SIZE: usize = 65536;

/// Empty table slot.
pub const NO_FUNC: u32 = u32::MAX;

/// The type an import asks for.
pub enum ExternType<'m> {
    Func(FuncType<'m>),
    Global(GlobalType),
    Memory(Limits),
    Table(Limits),
}

/// What the host returns for an import.
#[derive(Clone, Copy)]
pub enum Extern {
    /// Host function. The `u32` is the identifier passed to `Resolver::call`.
    Func(u32),
    /// Initial value of a global.
    Global(u64),
}

/// Resolves imports and calls host functions.
pub trait Resolver {
    /// Resolves an import. Returns `None` when it is missing or the type does not match.
    fn resolve(&mut self, module: &str, name: &str, ty: &ExternType<'_>) -> Option<Extern>;

    /// Calls a host function.
    ///
    /// `mem` is the guest's linear memory. Range-checking `list<u8>` arguments
    /// and out-pointers is the host function's responsibility
    /// (docs/abi-spec.md §4.2, §4.4).
    fn call(&mut self, host: u32, args: &[u64], results: &mut [u64], mem: &mut [u8]) -> Result<()>;
}

/// Linear memory. Owns the rest of the arena and moves only the current page count.
pub struct Memory<'a> {
    data: &'a mut [u8],
    pages: u32,
    max_pages: u32,
}

impl<'a> Memory<'a> {
    #[must_use]
    pub fn pages(&self) -> u32 {
        self.pages
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.pages as usize * PAGE_SIZE
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pages == 0
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.data[..self.len()]
    }

    #[must_use]
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        let n = self.pages as usize * PAGE_SIZE;
        &mut self.data[..n]
    }

    /// Grows by `delta` pages. Returns the previous page count, or `None` on failure.
    pub fn grow(&mut self, delta: u32) -> Option<u32> {
        let old = self.pages;
        let new = old.checked_add(delta)?;
        if new > self.max_pages || new as usize * PAGE_SIZE > self.data.len() {
            return None;
        }
        // The new pages must be zero.
        self.data[old as usize * PAGE_SIZE..new as usize * PAGE_SIZE].fill(0);
        self.pages = new;
        Some(old)
    }
}

/// Instance.
pub struct Instance<'m, 'a> {
    pub module: Module<'m, 'a>,
    pub validated: Validated<'a>,
    pub memory: Option<Memory<'a>>,
    pub globals: &'a mut [u64],
    pub table: &'a mut [u32],
    /// Host-side identifiers of the imported functions, in import order.
    pub host_funcs: &'a [u32],
    /// Whether each data segment has been dropped with `data.drop`.
    pub dropped_data: &'a mut [bool],
}

impl<'m> Instance<'m, '_> {
    /// Looks up the index of an exported function.
    #[must_use]
    pub fn export_func(&self, name: &str) -> Option<u32> {
        match self.module.export(name)? {
            ExportDesc::Func(i) => Some(i),
            _ => None,
        }
    }

    /// Looks up the value of an exported global.
    #[must_use]
    pub fn export_global(&self, name: &str) -> Option<u64> {
        match self.module.export(name)? {
            ExportDesc::Global(i) => self.globals.get(i as usize).copied(),
            _ => None,
        }
    }

    /// Type of a function.
    #[must_use]
    pub fn func_type(&self, idx: u32) -> Option<FuncType<'m>> {
        self.module.func_type(idx)
    }
}

/// Evaluates a constant expression. Validation has already checked its type.
fn eval_const(expr: &[u8], globals: &[u64]) -> Result<u64> {
    let mut r = Reader::new(expr);
    let op = r.u8()?;
    Ok(match op {
        0x41 => r.i32_leb()? as u32 as u64,
        0x42 => r.i64_leb()? as u64,
        0x43 => u64::from(r.f32_bits()?),
        0x44 => r.f64_bits()?,
        0x23 => {
            let i = r.u32_leb()? as usize;
            *globals.get(i).ok_or(Error::Invalid("unknown global"))?
        }
        _ => return Err(Error::Invalid("constant expression required")),
    })
}

/// Instantiates a module.
///
/// The rest of `arena` becomes linear memory, so the port bounds memory by the
/// size of the arena it hands in.
pub fn instantiate<'m, 'a>(
    module: Module<'m, 'a>,
    validated: Validated<'a>,
    cfg: &Config,
    arena: &mut Arena<'a>,
    resolver: &mut dyn Resolver,
) -> Result<Instance<'m, 'a>> {
    // --- import ---
    let n_func_imports = module.imported_funcs as usize;
    let host_funcs = arena.alloc(n_func_imports, 0u32)?;
    let globals = arena.alloc(module.global_count() as usize, 0u64)?;

    let mut fi = 0usize;
    let mut gi = 0usize;
    for im in module.imports {
        let ty = match im.desc {
            ImportDesc::Func(t) => ExternType::Func(
                *module
                    .types
                    .get(t as usize)
                    .ok_or(Error::Invalid("unknown type"))?,
            ),
            ImportDesc::Global(g) => ExternType::Global(g),
            ImportDesc::Memory(l) => ExternType::Memory(l),
            ImportDesc::Table(l) => ExternType::Table(l),
        };
        if matches!(im.desc, ImportDesc::Memory(_) | ImportDesc::Table(_)) {
            return Err(Error::Unsupported(
                "imported memories and tables are not supported",
            ));
        }
        match resolver.resolve(im.module, im.name, &ty) {
            Some(Extern::Func(h)) => {
                if !matches!(im.desc, ImportDesc::Func(_)) {
                    return Err(Error::Unlinkable("incompatible import type"));
                }
                host_funcs[fi] = h;
                fi += 1;
            }
            Some(Extern::Global(v)) => {
                if !matches!(im.desc, ImportDesc::Global(_)) {
                    return Err(Error::Unlinkable("incompatible import type"));
                }
                globals[gi] = v;
                gi += 1;
            }
            None => return Err(Error::Unlinkable("unknown import")),
        }
    }
    if fi != n_func_imports || gi != module.imported_globals as usize {
        // Imported memories and tables are not accepted in v0.1.
        return Err(Error::Unsupported(
            "imported memories and tables are not supported",
        ));
    }

    // --- globals ---
    for (i, g) in module.globals.iter().enumerate() {
        let idx = module.imported_globals as usize + i;
        globals[idx] = eval_const(g.init, &globals[..idx])?;
    }

    // --- table ---
    let table_len = module.tables.first().map_or(0, |l| l.min) as usize;
    if table_len > cfg.max_table_elems as usize {
        return Err(Error::Invalid("table size exceeds the port limit"));
    }
    let table = arena.alloc(table_len, NO_FUNC)?;

    let dropped_data = arena.alloc(module.datas.len(), false)?;

    // --- linear memory (the rest of the arena) ---
    let memory = if module.mem_count() > 0 {
        let lim = *module
            .mems
            .first()
            .ok_or(Error::Invalid("unknown memory"))?;
        let max_pages = lim
            .max
            .unwrap_or(cfg.max_memory_pages)
            .min(cfg.max_memory_pages);
        let data = arena.alloc_rest();
        let used = lim.min as usize * PAGE_SIZE;
        if used > data.len() {
            return Err(Error::OutOfArena);
        }
        // Zero only the pages in use (the arena is not initialized).
        data[..used].fill(0);
        Some(Memory {
            data,
            pages: lim.min,
            max_pages,
        })
    } else {
        None
    };

    let mut inst = Instance {
        module,
        validated,
        memory,
        globals,
        table,
        host_funcs,
        dropped_data,
    };

    // --- element segments ---
    for e in inst.module.elems {
        let offset = eval_const(e.offset, inst.globals)? as u32 as usize;
        let end = offset
            .checked_add(e.funcs.len())
            .ok_or(Error::Trap(Trap::TableOutOfBounds))?;
        if end > inst.table.len() {
            return Err(Error::Trap(Trap::TableOutOfBounds));
        }
        inst.table[offset..end].copy_from_slice(e.funcs);
    }

    // --- data segments ---
    let datas: &[Data<'m>] = inst.module.datas;
    for d in datas {
        let Some((_, off)) = d.active else { continue };
        let offset = eval_const(off, inst.globals)? as u32 as usize;
        let mem = inst
            .memory
            .as_mut()
            .ok_or(Error::Invalid("unknown memory"))?;
        let end = offset
            .checked_add(d.bytes.len())
            .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?;
        if end > mem.len() {
            return Err(Error::Trap(Trap::MemoryOutOfBounds));
        }
        mem.bytes_mut()[offset..end].copy_from_slice(d.bytes);
    }

    Ok(inst)
}

/// Picks the stack slot interpretation from a value type (for traces).
#[must_use]
pub fn slot_is_float(t: ValType) -> bool {
    matches!(t, ValType::F32 | ValType::F64)
}
