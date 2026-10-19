//! Representation of a decoded module.
//!
//! Instruction bodies, names and type bytes point straight into the module
//! binary (in flash). Only what needs an index (the type table, the function
//! table and so on) lives in the arena.

use crate::types::{FuncType, GlobalType, Limits, ValType};

/// What an import refers to.
#[derive(Clone, Copy)]
pub enum ImportDesc {
    Func(u32),
    Table(Limits),
    Memory(Limits),
    Global(GlobalType),
}

impl Default for ImportDesc {
    fn default() -> Self {
        ImportDesc::Func(0)
    }
}

/// One import.
#[derive(Clone, Copy, Default)]
pub struct Import<'m> {
    pub module: &'m str,
    pub name: &'m str,
    pub desc: ImportDesc,
}

/// What an export refers to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ExportDesc {
    Func(u32),
    Table(u32),
    Memory(u32),
    Global(u32),
}

impl Default for ExportDesc {
    fn default() -> Self {
        ExportDesc::Func(0)
    }
}

/// One export.
#[derive(Clone, Copy, Default)]
pub struct Export<'m> {
    pub name: &'m str,
    pub desc: ExportDesc,
}

/// Global definition. The initializer is kept as bytes.
#[derive(Clone, Copy, Default)]
pub struct Global<'m> {
    pub ty: GlobalType,
    /// Bytes of the constant expression (without the trailing `end`).
    pub init: &'m [u8],
}

/// Function body.
#[derive(Clone, Copy, Default)]
pub struct Code<'m> {
    /// Bytes of the local declarations (after the group count LEB, up to the body).
    pub locals: &'m [u8],
    /// Total number of locals (parameters excluded).
    pub local_count: u32,
    /// Instruction sequence, including the trailing `end` (0x0B).
    pub body: &'m [u8],
    /// Offset of the body from the start of the module (for traces).
    pub body_offset: usize,
}

/// Element segment. v0.1 has active segments only (the `table.*` instructions are unsupported).
#[derive(Clone, Copy, Default)]
pub struct Elem<'m, 'a> {
    pub table: u32,
    /// Constant expression that gives the offset.
    pub offset: &'m [u8],
    pub funcs: &'a [u32],
}

/// Data segment.
#[derive(Clone, Copy, Default)]
pub struct Data<'m> {
    /// `(memory index, offset constant expression)` when active; `None` when passive.
    pub active: Option<(u32, &'m [u8])>,
    pub bytes: &'m [u8],
}

/// Decoded module.
pub struct Module<'m, 'a> {
    pub types: &'a [FuncType<'m>],
    pub imports: &'a [Import<'m>],
    /// Type indices of the defined functions. Imports are not included.
    pub funcs: &'a [u32],
    pub tables: &'a [Limits],
    pub mems: &'a [Limits],
    pub globals: &'a [Global<'m>],
    pub exports: &'a [Export<'m>],
    pub start: Option<u32>,
    pub elems: &'a [Elem<'m, 'a>],
    pub datas: &'a [Data<'m>],
    pub code: &'a [Code<'m>],
    pub imported_funcs: u32,
    pub imported_tables: u32,
    pub imported_mems: u32,
    pub imported_globals: u32,
    /// Type indices of the imported functions, in import order. Lets `func_type`
    /// skip the import scan, which matters because it runs on every call.
    pub imported_func_types: &'a [u32],
    /// Types of the imported globals (used when validating constant expressions).
    pub imported_global_types: &'a [GlobalType],
    /// Value of the DataCount section. Needed by `memory.init` / `data.drop`.
    pub data_count: Option<u32>,
}

impl<'m> Module<'m, '_> {
    /// Total number of functions, imports included.
    #[must_use]
    pub fn func_count(&self) -> u32 {
        self.imported_funcs + self.funcs.len() as u32
    }

    /// Total number of globals, imports included.
    #[must_use]
    pub fn global_count(&self) -> u32 {
        self.imported_globals + self.globals.len() as u32
    }

    /// Total number of tables, imports included.
    #[must_use]
    pub fn table_count(&self) -> u32 {
        self.imported_tables + self.tables.len() as u32
    }

    /// Total number of memories, imports included.
    #[must_use]
    pub fn mem_count(&self) -> u32 {
        self.imported_mems + self.mems.len() as u32
    }

    /// Looks up the type of a function index.
    #[must_use]
    pub fn func_type(&self, idx: u32) -> Option<FuncType<'m>> {
        let type_idx = if idx < self.imported_funcs {
            *self.imported_func_types.get(idx as usize)?
        } else {
            *self.funcs.get((idx - self.imported_funcs) as usize)?
        };
        self.types.get(type_idx as usize).copied()
    }

    /// Looks up the type of a global index.
    #[must_use]
    pub fn global_type(&self, idx: u32) -> Option<GlobalType> {
        if idx < self.imported_globals {
            self.imported_global_types.get(idx as usize).copied()
        } else {
            self.globals
                .get((idx - self.imported_globals) as usize)
                .map(|g| g.ty)
        }
    }

    /// Looks up an export by name.
    #[must_use]
    pub fn export(&self, name: &str) -> Option<ExportDesc> {
        self.exports.iter().find(|e| e.name == name).map(|e| e.desc)
    }

    /// Returns the local types in order (used by validation).
    pub fn locals_of(&self, code: &Code<'m>) -> LocalIter<'m> {
        LocalIter {
            bytes: code.locals,
            pos: 0,
            remaining: 0,
            ty: ValType::I32,
        }
    }
}

/// Expands the local declarations `(count, type)*` one local at a time.
pub struct LocalIter<'m> {
    bytes: &'m [u8],
    pos: usize,
    remaining: u32,
    ty: ValType,
}

impl Iterator for LocalIter<'_> {
    type Item = ValType;

    fn next(&mut self) -> Option<ValType> {
        while self.remaining == 0 {
            // Next group. `decode` has already checked these bytes, so this cannot fail.
            let mut r = crate::reader::Reader::new(&self.bytes[self.pos..]);
            let n = r.u32_leb().ok()?;
            let t = ValType::from_byte(r.u8().ok()?).ok()?;
            self.pos += r.pos();
            if n == 0 {
                continue;
            }
            self.remaining = n;
            self.ty = t;
        }
        self.remaining -= 1;
        Some(self.ty)
    }
}
