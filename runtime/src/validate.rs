//! Type checking, plus construction of the side table that precomputes branch targets.
//!
//! The algorithm follows the validation algorithm in the appendix of the Wasm
//! spec. Unreachable code is handled as a polymorphic stack that pushes an
//! "unknown" type; a home-grown simplification fails on edge cases such as
//! `unreached-valid.wast`.
//!
//! The side table maps a branch instruction's position to its target, the
//! number of values carried over and the number of values dropped. The
//! interpreter binary-searches it at run time, so it needs no control stack.

use crate::arena::Arena;
use crate::config::Config;
use crate::error::{Error, Result};
use crate::module::{Code, ExportDesc, Module};
use crate::reader::Reader;
use crate::types::{FuncType, ValType};

use ValType::{F32, F64, I32, I64};

/// One side table entry.
#[derive(Clone, Copy, Default)]
pub struct Branch {
    /// Opcode position of the branch instruction (offset within the function body).
    pub src: u32,
    /// Target (offset within the function body).
    pub target: u32,
    /// Number of values carried over to the target.
    pub keep: u32,
    /// Number of values dropped from below the carried values.
    pub drop: u32,
}

/// Validation result for one function.
#[derive(Clone, Copy, Default)]
pub struct FuncInfo {
    pub branch_start: u32,
    pub branch_len: u32,
    /// Maximum operand stack depth needed at run time.
    pub max_stack: u32,
    /// Total slots for parameters plus locals.
    pub frame_slots: u32,
}

/// Validation result for the whole module.
pub struct Validated<'a> {
    pub funcs: &'a [FuncInfo],
    pub branches: &'a [Branch],
    /// Deepest block nesting any function reached, the function's own block
    /// included: the smallest `max_control_depth` that accepts the module.
    pub max_control: u32,
}

impl Validated<'_> {
    /// Looks up the n-th entry for a branch instruction's position (`br_table` uses n > 0).
    #[must_use]
    pub fn branch(&self, info: &FuncInfo, pc: u32, n: usize) -> Option<&Branch> {
        let start = info.branch_start as usize;
        let slice = &self.branches[start..start + info.branch_len as usize];
        let mut i = slice.binary_search_by_key(&pc, |b| b.src).ok()?;
        while i > 0 && slice[i - 1].src == pc {
            i -= 1;
        }
        slice.get(i + n).filter(|b| b.src == pc)
    }
}

/// Value stack element. In unreachable code the type is unknown.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MaybeVal {
    Unknown,
    Val(ValType),
}

impl MaybeVal {
    fn matches(self, t: ValType) -> bool {
        matches!(self, MaybeVal::Unknown) || self == MaybeVal::Val(t)
    }
}

/// Control frame.
#[derive(Clone, Copy)]
struct Ctrl<'m> {
    opcode: u8,
    ty: FuncType<'m>,
    height: u32,
    unreachable: bool,
    /// Target of a `loop`. `None` otherwise; resolved on reaching `end`.
    known_target: Option<u32>,
    /// Head of the chain of unresolved branch entries (`NO_PENDING` when empty).
    /// Until resolution, `Branch::target` holds the index of the next entry in the chain.
    pending: u32,
    /// Entry for the jump taken when the `if` condition is false (resolved at `else` or `end`).
    if_entry: u32,
    saw_else: bool,
}

impl Default for Ctrl<'_> {
    fn default() -> Self {
        Ctrl {
            opcode: 0,
            ty: FuncType::default(),
            height: 0,
            unreachable: false,
            known_target: None,
            pending: NO_PENDING,
            if_entry: NO_PENDING,
            saw_else: false,
        }
    }
}

const NO_PENDING: u32 = u32::MAX;
const TYPE_MISMATCH: Error = Error::Invalid("type mismatch");

/// Where branch entries go. The counting pass and the writing pass use different sinks.
enum Sink<'s> {
    Count(usize),
    Fill(&'s mut [Branch], usize),
}

impl Sink<'_> {
    fn push(&mut self, b: Branch) -> u32 {
        match self {
            Sink::Count(n) => {
                *n += 1;
                NO_PENDING
            }
            Sink::Fill(buf, used) => {
                let i = *used;
                buf[i] = b;
                *used += 1;
                i as u32
            }
        }
    }

    fn entry(&mut self, i: u32) -> Option<&mut Branch> {
        match self {
            Sink::Count(_) => None,
            Sink::Fill(buf, _) => buf.get_mut(i as usize),
        }
    }
}

/// Validator. The value stack and local table come from the scratch arena once and are reused.
struct Validator<'m, 's> {
    vals: &'s mut [MaybeVal],
    ctrls: &'s mut [Ctrl<'m>],
    locals: &'s mut [ValType],
    nvals: usize,
    nctrls: usize,
    nlocals: usize,
    max_stack: u32,
    /// Over the whole module; `max_stack` is per function.
    max_ctrls: u32,
}

/// Validates a module.
///
/// The result (the side table) is allocated from `arena`; the working memory
/// used only during validation comes from `scratch`, which may be discarded
/// afterwards.
pub fn validate<'m, 'a, 's>(
    m: &Module<'m, '_>,
    cfg: &Config,
    arena: &mut Arena<'a>,
    scratch: &mut Arena<'s>,
) -> Result<Validated<'a>>
where
    'm: 's,
{
    check_module(m, cfg)?;

    let mut v = Validator {
        vals: scratch.alloc(cfg.max_value_stack, MaybeVal::Unknown)?,
        ctrls: scratch.alloc(cfg.max_control_depth, Ctrl::default())?,
        locals: scratch.alloc(cfg.max_locals, ValType::I32)?,
        nvals: 0,
        nctrls: 0,
        nlocals: 0,
        max_stack: 0,
        max_ctrls: 0,
    };

    // First pass: count the branch entries.
    let mut total = 0usize;
    for (i, code) in m.code.iter().enumerate() {
        let mut sink = Sink::Count(0);
        v.run(m, cfg, i, code, &mut sink)?;
        if let Sink::Count(n) = sink {
            total += n;
        }
    }

    let funcs = arena.alloc(m.code.len(), FuncInfo::default())?;
    let branches = arena.alloc(total, Branch::default())?;

    // Second pass: write them.
    let mut used = 0usize;
    for (i, code) in m.code.iter().enumerate() {
        let start = used;
        let mut sink = Sink::Fill(branches, used);
        let (max_stack, frame_slots) = v.run(m, cfg, i, code, &mut sink)?;
        if let Sink::Fill(_, u) = sink {
            used = u;
        }
        funcs[i] = FuncInfo {
            branch_start: start as u32,
            branch_len: (used - start) as u32,
            max_stack,
            frame_slots,
        };
        // The run-time binary search needs ascending `src`. The validator walks the
        // body once from the front, so the push order is naturally ascending; this
        // checks it anyway (`br_table` entries share one `src`, ordered by target).
        for w in branches[start..used].windows(2) {
            if w[0].src > w[1].src {
                return Err(Error::Invalid("internal: side table is not ordered"));
            }
        }
    }

    Ok(Validated {
        funcs,
        branches,
        max_control: v.max_ctrls,
    })
}

// ---------------------------------------------------------------- whole module

fn check_module(m: &Module<'_, '_>, cfg: &Config) -> Result<()> {
    if m.mems.len() as u32 + m.imported_mems > 1 {
        return Err(Error::Invalid("multiple memories"));
    }
    if m.tables.len() as u32 + m.imported_tables > 1 {
        return Err(Error::Unsupported("multiple tables are not supported"));
    }
    for lim in m.mems {
        if lim.min > cfg.max_memory_pages {
            return Err(Error::Invalid("memory size exceeds the port limit"));
        }
    }
    for &t in m.funcs {
        if t as usize >= m.types.len() {
            return Err(Error::Invalid("unknown type"));
        }
    }
    for im in m.imports {
        if let crate::module::ImportDesc::Func(t) = im.desc
            && t as usize >= m.types.len()
        {
            return Err(Error::Invalid("unknown type"));
        }
    }
    // Export names must be unique.
    for (i, e) in m.exports.iter().enumerate() {
        if m.exports[..i].iter().any(|o| o.name == e.name) {
            return Err(Error::Invalid("duplicate export name"));
        }
        let ok = match e.desc {
            ExportDesc::Func(i) => i < m.func_count(),
            ExportDesc::Table(i) => i < m.table_count(),
            ExportDesc::Memory(i) => i < m.mem_count(),
            ExportDesc::Global(i) => i < m.global_count(),
        };
        if !ok {
            return Err(Error::Invalid("unknown export target"));
        }
    }
    if let Some(s) = m.start {
        let ty = m.func_type(s).ok_or(Error::Invalid("unknown function"))?;
        if ty.param_count() != 0 || ty.result_count() != 0 {
            return Err(Error::Invalid("start function"));
        }
    }
    // Global initializers.
    for (i, g) in m.globals.iter().enumerate() {
        let declared = m.imported_globals + i as u32;
        let t = const_expr_type(m, g.init, declared)?;
        if t != g.ty.val {
            return Err(TYPE_MISMATCH);
        }
    }
    for e in m.elems {
        if e.table >= m.table_count() {
            return Err(Error::Invalid("unknown table"));
        }
        if const_expr_type(m, e.offset, m.global_count())? != I32 {
            return Err(TYPE_MISMATCH);
        }
        for &f in e.funcs {
            if f >= m.func_count() {
                return Err(Error::Invalid("unknown function"));
            }
        }
    }
    for d in m.datas {
        if let Some((mem, off)) = d.active {
            if mem >= m.mem_count() {
                return Err(Error::Invalid("unknown memory"));
            }
            if const_expr_type(m, off, m.global_count())? != I32 {
                return Err(TYPE_MISMATCH);
            }
        }
    }
    Ok(())
}

/// Computes the type of a constant expression. Only globals with an index below
/// `limit` may be referenced.
fn const_expr_type(m: &Module<'_, '_>, expr: &[u8], limit: u32) -> Result<ValType> {
    let mut r = Reader::new(expr);
    let mut ty = None;
    while !r.is_empty() {
        if ty.is_some() {
            return Err(Error::Invalid("constant expression required"));
        }
        ty = Some(match r.u8()? {
            0x41 => {
                r.i32_leb()?;
                I32
            }
            0x42 => {
                r.i64_leb()?;
                I64
            }
            0x43 => {
                r.f32_bits()?;
                F32
            }
            0x44 => {
                r.f64_bits()?;
                F64
            }
            0x23 => {
                let idx = r.u32_leb()?;
                // The current spec allows references to earlier immutable globals.
                if idx >= limit {
                    return Err(Error::Invalid("unknown global"));
                }
                let g = m.global_type(idx).ok_or(Error::Invalid("unknown global"))?;
                if g.mutable {
                    return Err(Error::Invalid("constant expression required"));
                }
                g.val
            }
            _ => return Err(Error::Invalid("constant expression required")),
        });
    }
    ty.ok_or(TYPE_MISMATCH)
}

// ---------------------------------------------------------------- instructions

/// Types of the values a label carries (parameters for `loop`, results otherwise).
fn label_arity(f: &Ctrl<'_>) -> u32 {
    if f.opcode == OP_LOOP {
        f.ty.param_count() as u32
    } else {
        f.ty.result_count() as u32
    }
}

fn label_type(f: &Ctrl<'_>, i: usize) -> Option<ValType> {
    if f.opcode == OP_LOOP {
        f.ty.param(i)
    } else {
        f.ty.result(i)
    }
}

const OP_BLOCK: u8 = 0x02;
const OP_LOOP: u8 = 0x03;
const OP_IF: u8 = 0x04;
const OP_FUNC: u8 = 0xff;

impl<'m> Validator<'m, '_> {
    fn push(&mut self, t: ValType) -> Result<()> {
        if self.nvals >= self.vals.len() {
            return Err(Error::Exhausted("value stack overflow"));
        }
        self.vals[self.nvals] = MaybeVal::Val(t);
        self.nvals += 1;
        if self.nvals as u32 > self.max_stack {
            self.max_stack = self.nvals as u32;
        }
        Ok(())
    }

    fn push_maybe(&mut self, v: MaybeVal) -> Result<()> {
        if self.nvals >= self.vals.len() {
            return Err(Error::Exhausted("value stack overflow"));
        }
        self.vals[self.nvals] = v;
        self.nvals += 1;
        if self.nvals as u32 > self.max_stack {
            self.max_stack = self.nvals as u32;
        }
        Ok(())
    }

    fn pop(&mut self) -> Result<MaybeVal> {
        let f = self.ctrls[self.nctrls - 1];
        if self.nvals as u32 == f.height {
            if f.unreachable {
                return Ok(MaybeVal::Unknown);
            }
            return Err(TYPE_MISMATCH);
        }
        self.nvals -= 1;
        Ok(self.vals[self.nvals])
    }

    fn pop_t(&mut self, t: ValType) -> Result<()> {
        if self.pop()?.matches(t) {
            Ok(())
        } else {
            Err(TYPE_MISMATCH)
        }
    }

    fn unary(&mut self, from: ValType, to: ValType) -> Result<()> {
        self.pop_t(from)?;
        self.push(to)
    }

    fn binary(&mut self, a: ValType, to: ValType) -> Result<()> {
        self.pop_t(a)?;
        self.pop_t(a)?;
        self.push(to)
    }

    fn set_unreachable(&mut self) {
        let f = &mut self.ctrls[self.nctrls - 1];
        self.nvals = f.height as usize;
        f.unreachable = true;
    }

    /// The spec's `push_ctrl`. The caller (block / loop / if) pops the parameters first.
    fn push_ctrl(&mut self, opcode: u8, ty: FuncType<'m>, known_target: Option<u32>) -> Result<()> {
        if self.nctrls >= self.ctrls.len() {
            return Err(Error::Exhausted("control stack overflow"));
        }
        let height = self.nvals as u32;
        self.ctrls[self.nctrls] = Ctrl {
            opcode,
            ty,
            height,
            unreachable: false,
            known_target,
            pending: NO_PENDING,
            if_entry: NO_PENDING,
            saw_else: false,
        };
        self.nctrls += 1;
        self.max_ctrls = self.max_ctrls.max(self.nctrls as u32);
        for i in 0..ty.param_count() {
            self.push(ty.param(i).ok_or(TYPE_MISMATCH)?)?;
        }
        Ok(())
    }

    /// Shared by block / loop / if. Pops the parameters, then pushes the frame.
    fn enter_block(
        &mut self,
        opcode: u8,
        ty: FuncType<'m>,
        known_target: Option<u32>,
    ) -> Result<()> {
        for i in (0..ty.param_count()).rev() {
            self.pop_t(ty.param(i).ok_or(TYPE_MISMATCH)?)?;
        }
        self.push_ctrl(opcode, ty, known_target)
    }

    /// Number of values carried by the label at depth `depth`.
    fn label_arity_at(&self, depth: u32) -> Result<u32> {
        let idx = self
            .nctrls
            .checked_sub(1 + depth as usize)
            .ok_or(Error::Invalid("unknown label"))?;
        Ok(label_arity(&self.ctrls[idx]))
    }

    fn pop_ctrl(&mut self) -> Result<Ctrl<'m>> {
        let f = self.ctrls[self.nctrls - 1];
        for i in (0..f.ty.result_count()).rev() {
            self.pop_t(f.ty.result(i).ok_or(TYPE_MISMATCH)?)?;
        }
        if self.nvals as u32 != f.height {
            return Err(TYPE_MISMATCH);
        }
        self.nctrls -= 1;
        Ok(f)
    }

    /// Checks that the label's types match the top of the stack (without consuming values).
    ///
    /// `meet` is for `br_table`. As in the spec's `push_vals(pop_vals(...))`, it
    /// pushes back the values actually popped rather than the expected types. In
    /// unreachable code those are the unknown type (bottom), so a later target
    /// with different types still passes (`unreached-valid.wast`, meet-bottom).
    fn check_label(&mut self, frame: &Ctrl<'m>, meet: bool) -> Result<u32> {
        let n = label_arity(frame) as usize;
        let before = self.nvals;
        for i in (0..n).rev() {
            self.pop_t(label_type(frame, i).ok_or(TYPE_MISMATCH)?)?;
        }
        if !meet {
            for i in 0..n {
                self.push(label_type(frame, i).ok_or(TYPE_MISMATCH)?)?;
            }
            return Ok(n as u32);
        }

        // Only what was actually popped is real. The shortfall is inserted below as unknown.
        let after = self.nvals;
        let real = before - after;
        let virt = n - real;
        if virt == 0 {
            self.nvals = before;
        } else {
            if after + n > self.vals.len() {
                return Err(Error::Exhausted("value stack overflow"));
            }
            self.vals.copy_within(after..after + real, after + virt);
            for slot in &mut self.vals[after..after + virt] {
                *slot = MaybeVal::Unknown;
            }
            self.nvals = after + n;
        }
        if self.nvals as u32 > self.max_stack {
            self.max_stack = self.nvals as u32;
        }
        Ok(n as u32)
    }

    /// Pushes a branch to the label at depth `depth` onto the side table.
    fn emit_branch(&mut self, sink: &mut Sink<'_>, depth: u32, src: u32, meet: bool) -> Result<()> {
        let idx = self
            .nctrls
            .checked_sub(1 + depth as usize)
            .ok_or(Error::Invalid("unknown label"))?;
        let frame = self.ctrls[idx];
        let keep = self.check_label(&frame, meet)?;
        let drop = (self.nvals as u32).saturating_sub(frame.height + keep);
        match frame.known_target {
            Some(target) => {
                sink.push(Branch {
                    src,
                    target,
                    keep,
                    drop,
                });
            }
            None => {
                let prev = self.ctrls[idx].pending;
                let e = sink.push(Branch {
                    src,
                    target: prev,
                    keep,
                    drop,
                });
                self.ctrls[idx].pending = e;
            }
        }
        Ok(())
    }

    /// Fills an unresolved branch chain with its target.
    fn resolve(sink: &mut Sink<'_>, mut head: u32, target: u32) {
        while head != NO_PENDING {
            let next = match sink.entry(head) {
                Some(b) => {
                    let n = b.target;
                    b.target = target;
                    n
                }
                None => break,
            };
            head = next;
        }
    }

    fn run(
        &mut self,
        m: &Module<'m, '_>,
        cfg: &Config,
        func_idx: usize,
        code: &Code<'m>,
        sink: &mut Sink<'_>,
    ) -> Result<(u32, u32)> {
        self.nvals = 0;
        self.nctrls = 0;
        self.max_stack = 0;

        let ftype = m
            .func_type(m.imported_funcs + func_idx as u32)
            .ok_or(Error::Invalid("unknown function"))?;

        // Local table (parameters, then the declared locals).
        let total = ftype.param_count() + code.local_count as usize;
        if total > cfg.max_locals || total > self.locals.len() {
            return Err(Error::Invalid("too many locals"));
        }
        self.nlocals = 0;
        for t in ftype.params() {
            self.locals[self.nlocals] = t;
            self.nlocals += 1;
        }
        for t in m.locals_of(code) {
            self.locals[self.nlocals] = t;
            self.nlocals += 1;
        }
        if self.nlocals != total {
            return Err(Error::Malformed("malformed local declarations"));
        }

        // The whole function is one control frame whose label types are the results.
        self.ctrls[0] = Ctrl {
            opcode: OP_FUNC,
            ty: FuncType::new(&[], ftype.results_bytes()),
            height: 0,
            unreachable: false,
            known_target: None,
            pending: NO_PENDING,
            if_entry: NO_PENDING,
            saw_else: false,
        };
        self.nctrls = 1;
        self.max_ctrls = self.max_ctrls.max(1);

        let body = code.body;
        let mut r = Reader::new(body);
        let end_of_body = body.len() as u32;

        while self.nctrls > 0 {
            let pc = r.pos() as u32;
            let op = r.u8()?;
            self.step(m, &mut r, sink, op, pc, end_of_body, ftype)?;
        }
        if !r.is_empty() {
            return Err(Error::Malformed("junk after function body"));
        }
        Ok((self.max_stack, total as u32))
    }
}

/// Reads a `memarg` and checks that the alignment does not exceed the natural width.
fn memarg(r: &mut Reader<'_>, natural: u32) -> Result<()> {
    let align = r.u32_leb()?;
    // memop flags are valid only when they fit in one byte (align.wast).
    if align >= 0x80 {
        return Err(Error::Malformed("malformed memop flags"));
    }
    // Bit 6 means a multi-memory memory index follows.
    if align & 0x40 != 0 {
        return Err(Error::Unsupported("multi-memory is not supported"));
    }
    if align > natural {
        return Err(Error::Invalid("alignment must not be larger than natural"));
    }
    // An offset that does not fit in u32 is invalid per spec (u64 only with memory64).
    if r.u64_leb()? > u64::from(u32::MAX) {
        return Err(Error::Invalid("offset out of range"));
    }
    Ok(())
}

impl<'m> Validator<'m, '_> {
    /// Reads a block type.
    fn block_type(&self, m: &Module<'m, '_>, r: &mut Reader<'m>) -> Result<FuncType<'m>> {
        let at = r.pos();
        match r.peek_u8()? {
            0x40 => {
                r.u8()?;
                Ok(FuncType::new(&[], &[]))
            }
            0x7c..=0x7f => {
                r.u8()?;
                Ok(FuncType::new(&[], r.slice(at, at + 1)))
            }
            0x70 | 0x6f => Err(Error::Unsupported("reference types are not supported")),
            0x7b => Err(Error::Unsupported("SIMD is not supported")),
            _ => {
                let v = r.i64_leb()?;
                if v < 0 || v > u32::MAX as i64 {
                    return Err(Error::Invalid("unknown type"));
                }
                m.types
                    .get(v as usize)
                    .copied()
                    .ok_or(Error::Invalid("unknown type"))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn step(
        &mut self,
        m: &Module<'m, '_>,
        r: &mut Reader<'m>,
        sink: &mut Sink<'_>,
        op: u8,
        pc: u32,
        end_of_body: u32,
        ftype: FuncType<'m>,
    ) -> Result<()> {
        let _ = end_of_body;
        match op {
            // --- control ---
            0x00 => self.set_unreachable(),
            0x01 => {}
            OP_BLOCK => {
                let ty = self.block_type(m, r)?;
                self.enter_block(OP_BLOCK, ty, None)?;
            }
            OP_LOOP => {
                let ty = self.block_type(m, r)?;
                let target = r.pos() as u32;
                self.enter_block(OP_LOOP, ty, Some(target))?;
            }
            OP_IF => {
                let ty = self.block_type(m, r)?;
                self.pop_t(I32)?;
                self.enter_block(OP_IF, ty, None)?;
                // Jump taken when the condition is false. Resolved at else or end.
                let e = sink.push(Branch {
                    src: pc,
                    target: NO_PENDING,
                    keep: 0,
                    drop: 0,
                });
                self.ctrls[self.nctrls - 1].if_entry = e;
            }
            0x05 => {
                let f = self.pop_ctrl()?;
                if f.opcode != OP_IF || f.saw_else {
                    return Err(Error::Invalid("unexpected else"));
                }
                // Jump from the end of the then branch to end.
                let e = sink.push(Branch {
                    src: pc,
                    target: f.pending,
                    keep: 0,
                    drop: 0,
                });
                // A false condition jumps to the start of the else body.
                Self::resolve(sink, f.if_entry, r.pos() as u32);
                self.push_ctrl(OP_IF, f.ty, None)?;
                let top = self.nctrls - 1;
                self.ctrls[top].pending = e;
                self.ctrls[top].saw_else = true;
            }
            0x0b => {
                let f = self.pop_ctrl()?;
                let after = r.pos() as u32;
                if f.opcode == OP_IF && !f.saw_else {
                    // An if without else needs parameter types == result types.
                    if f.ty.params_bytes() != f.ty.results_bytes() {
                        return Err(TYPE_MISMATCH);
                    }
                    Self::resolve(sink, f.if_entry, after);
                }
                Self::resolve(sink, f.pending, after);
                for i in 0..f.ty.result_count() {
                    self.push(f.ty.result(i).ok_or(TYPE_MISMATCH)?)?;
                }
            }
            0x0c => {
                let depth = r.u32_leb()?;
                self.emit_branch(sink, depth, pc, false)?;
                self.set_unreachable();
            }
            0x0d => {
                let depth = r.u32_leb()?;
                self.pop_t(I32)?;
                self.emit_branch(sink, depth, pc, false)?;
            }
            0x0e => {
                let count = r.u32_leb()? as usize;
                self.pop_t(I32)?;
                // The default target is needed first, so skip ahead, then come back and check.
                let targets_start = r.pos();
                for _ in 0..count {
                    r.u32_leb()?;
                }
                let targets_end = r.pos();
                let default = r.u32_leb()?;
                let arity = self.label_arity_at(default)?;

                let mut t = Reader::new(r.slice(targets_start, targets_end));
                for _ in 0..count {
                    let d = t.u32_leb()?;
                    if self.label_arity_at(d)? != arity {
                        return Err(TYPE_MISMATCH);
                    }
                    self.emit_branch(sink, d, pc, true)?;
                }
                self.emit_branch(sink, default, pc, false)?;
                self.set_unreachable();
            }
            0x0f => {
                // return keeps only the function results. No target needed.
                for i in (0..ftype.result_count()).rev() {
                    self.pop_t(ftype.result(i).ok_or(TYPE_MISMATCH)?)?;
                }
                self.set_unreachable();
            }
            0x10 => {
                let f = r.u32_leb()?;
                let ty = m.func_type(f).ok_or(Error::Invalid("unknown function"))?;
                for i in (0..ty.param_count()).rev() {
                    self.pop_t(ty.param(i).ok_or(TYPE_MISMATCH)?)?;
                }
                for i in 0..ty.result_count() {
                    self.push(ty.result(i).ok_or(TYPE_MISMATCH)?)?;
                }
            }
            0x11 => {
                let t = r.u32_leb()?;
                let table = r.u32_leb()?;
                if table != 0 || m.table_count() == 0 {
                    return Err(Error::Invalid("unknown table"));
                }
                let ty = *m
                    .types
                    .get(t as usize)
                    .ok_or(Error::Invalid("unknown type"))?;
                self.pop_t(I32)?;
                for i in (0..ty.param_count()).rev() {
                    self.pop_t(ty.param(i).ok_or(TYPE_MISMATCH)?)?;
                }
                for i in 0..ty.result_count() {
                    self.push(ty.result(i).ok_or(TYPE_MISMATCH)?)?;
                }
            }

            // --- parametric ---
            0x1a => {
                self.pop()?;
            }
            0x1b => {
                self.pop_t(I32)?;
                let t2 = self.pop()?;
                let t1 = self.pop()?;
                let t = match (t1, t2) {
                    (MaybeVal::Val(a), MaybeVal::Val(b)) if a == b => MaybeVal::Val(a),
                    (MaybeVal::Unknown, x) | (x, MaybeVal::Unknown) => x,
                    _ => return Err(TYPE_MISMATCH),
                };
                self.push_maybe(t)?;
            }
            0x1c => return Err(Error::Unsupported("typed select is not supported")),

            // --- variables ---
            0x20..=0x22 => {
                let i = r.u32_leb()? as usize;
                let t = *self
                    .locals
                    .get(i)
                    .filter(|_| i < self.nlocals)
                    .ok_or(Error::Invalid("unknown local"))?;
                match op {
                    0x20 => self.push(t)?,
                    0x21 => self.pop_t(t)?,
                    _ => {
                        self.pop_t(t)?;
                        self.push(t)?;
                    }
                }
            }
            0x23 | 0x24 => {
                let i = r.u32_leb()?;
                let g = m.global_type(i).ok_or(Error::Invalid("unknown global"))?;
                if op == 0x23 {
                    self.push(g.val)?;
                } else {
                    if !g.mutable {
                        return Err(Error::Invalid("global is immutable"));
                    }
                    self.pop_t(g.val)?;
                }
            }

            // --- memory ---
            0x28..=0x35 => {
                if m.mem_count() == 0 {
                    return Err(Error::Invalid("unknown memory"));
                }
                let (natural, ty) = load_shape(op);
                memarg(r, natural)?;
                self.pop_t(I32)?;
                self.push(ty)?;
            }
            0x36..=0x3e => {
                if m.mem_count() == 0 {
                    return Err(Error::Invalid("unknown memory"));
                }
                let (natural, ty) = store_shape(op);
                memarg(r, natural)?;
                self.pop_t(ty)?;
                self.pop_t(I32)?;
            }
            0x3f | 0x40 => {
                if m.mem_count() == 0 {
                    return Err(Error::Invalid("unknown memory"));
                }
                if r.u8()? != 0x00 {
                    return Err(Error::Malformed("zero byte expected"));
                }
                if op == 0x40 {
                    self.pop_t(I32)?;
                }
                self.push(I32)?;
            }

            // --- constants ---
            0x41 => {
                r.i32_leb()?;
                self.push(I32)?;
            }
            0x42 => {
                r.i64_leb()?;
                self.push(I64)?;
            }
            0x43 => {
                r.f32_bits()?;
                self.push(F32)?;
            }
            0x44 => {
                r.f64_bits()?;
                self.push(F64)?;
            }

            // --- comparisons ---
            0x45 => self.unary(I32, I32)?,
            0x46..=0x4f => self.binary(I32, I32)?,
            0x50 => self.unary(I64, I32)?,
            0x51..=0x5a => self.binary(I64, I32)?,
            0x5b..=0x60 => self.binary(F32, I32)?,
            0x61..=0x66 => self.binary(F64, I32)?,

            // --- numeric ---
            0x67..=0x69 => self.unary(I32, I32)?,
            0x6a..=0x78 => self.binary(I32, I32)?,
            0x79..=0x7b => self.unary(I64, I64)?,
            0x7c..=0x8a => self.binary(I64, I64)?,
            0x8b..=0x91 => self.unary(F32, F32)?,
            0x92..=0x98 => self.binary(F32, F32)?,
            0x99..=0x9f => self.unary(F64, F64)?,
            0xa0..=0xa6 => self.binary(F64, F64)?,
            0xa7 => self.unary(I64, I32)?,
            0xa8 | 0xa9 => self.unary(F32, I32)?,
            0xaa | 0xab => self.unary(F64, I32)?,
            0xac | 0xad => self.unary(I32, I64)?,
            0xae | 0xaf => self.unary(F32, I64)?,
            0xb0 | 0xb1 => self.unary(F64, I64)?,
            0xb2 | 0xb3 => self.unary(I32, F32)?,
            0xb4 | 0xb5 => self.unary(I64, F32)?,
            0xb6 => self.unary(F64, F32)?,
            0xb7 | 0xb8 => self.unary(I32, F64)?,
            0xb9 | 0xba => self.unary(I64, F64)?,
            0xbb => self.unary(F32, F64)?,
            0xbc => self.unary(F32, I32)?,
            0xbd => self.unary(F64, I64)?,
            0xbe => self.unary(I32, F32)?,
            0xbf => self.unary(I64, F64)?,
            // sign-extension-ops
            0xc0 | 0xc1 => self.unary(I32, I32)?,
            0xc2..=0xc4 => self.unary(I64, I64)?,

            0xfc => self.step_fc(m, r)?,

            // Instructions outside the supported feature set. Valid Wasm, so kept
            // distinct from malformed.
            0x06..=0x09 | 0x18 | 0x19 | 0x1f => {
                return Err(Error::Unsupported("exception handling is not supported"));
            }
            0x12 | 0x13 => return Err(Error::Unsupported("tail calls are not supported")),
            0x14 | 0x15 => return Err(Error::Unsupported("function references are not supported")),
            0x25 | 0x26 => return Err(Error::Unsupported("table instructions are not supported")),
            0xd0..=0xd6 => return Err(Error::Unsupported("reference types are not supported")),
            0xfb => return Err(Error::Unsupported("GC is not supported")),
            0xfd => return Err(Error::Unsupported("SIMD is not supported")),
            0xfe => return Err(Error::Unsupported("threads are not supported")),

            _ => return Err(Error::Malformed("illegal opcode")),
        }
        Ok(())
    }

    /// The 0xFC prefix (trunc_sat and the bulk-memory `memory.*` instructions).
    fn step_fc(&mut self, m: &Module<'m, '_>, r: &mut Reader<'m>) -> Result<()> {
        let sub = r.u32_leb()?;
        match sub {
            0 | 1 => self.unary(F32, I32)?,
            2 | 3 => self.unary(F64, I32)?,
            4 | 5 => self.unary(F32, I64)?,
            6 | 7 => self.unary(F64, I64)?,
            8 => {
                // memory.init
                let d = r.u32_leb()?;
                if r.u8()? != 0x00 {
                    return Err(Error::Malformed("zero byte expected"));
                }
                self.check_data(m, d, true)?;
                self.pop_t(I32)?;
                self.pop_t(I32)?;
                self.pop_t(I32)?;
            }
            9 => {
                let d = r.u32_leb()?;
                // data.drop does not need a memory (bulk.wast).
                self.check_data(m, d, false)?;
            }
            10 => {
                if r.u8()? != 0x00 || r.u8()? != 0x00 {
                    return Err(Error::Malformed("zero byte expected"));
                }
                if m.mem_count() == 0 {
                    return Err(Error::Invalid("unknown memory"));
                }
                self.pop_t(I32)?;
                self.pop_t(I32)?;
                self.pop_t(I32)?;
            }
            11 => {
                if r.u8()? != 0x00 {
                    return Err(Error::Malformed("zero byte expected"));
                }
                if m.mem_count() == 0 {
                    return Err(Error::Invalid("unknown memory"));
                }
                self.pop_t(I32)?;
                self.pop_t(I32)?;
                self.pop_t(I32)?;
            }
            12..=17 => return Err(Error::Unsupported("table instructions are not supported")),
            _ => return Err(Error::Malformed("illegal opcode")),
        }
        Ok(())
    }

    fn check_data(&self, m: &Module<'m, '_>, idx: u32, needs_memory: bool) -> Result<()> {
        if m.data_count.is_none() {
            return Err(Error::Malformed("data count section required"));
        }
        if idx as usize >= m.datas.len() {
            return Err(Error::Invalid("unknown data segment"));
        }
        if needs_memory && m.mem_count() == 0 {
            return Err(Error::Invalid("unknown memory"));
        }
        Ok(())
    }
}

/// `(log2 of the natural alignment, result type)` of a load instruction.
const fn load_shape(op: u8) -> (u32, ValType) {
    match op {
        0x28 => (2, I32),
        0x29 => (3, I64),
        0x2a => (2, F32),
        0x2b => (3, F64),
        0x2c | 0x2d => (0, I32),
        0x2e | 0x2f => (1, I32),
        0x30 | 0x31 => (0, I64),
        0x32 | 0x33 => (1, I64),
        _ => (2, I64), // 0x34 | 0x35
    }
}

/// `(log2 of the natural alignment, value type)` of a store instruction.
const fn store_shape(op: u8) -> (u32, ValType) {
    match op {
        0x36 => (2, I32),
        0x37 => (3, I64),
        0x38 => (2, F32),
        0x39 => (3, F64),
        0x3a => (0, I32),
        0x3b => (1, I32),
        0x3c => (0, I64),
        0x3d => (1, I64),
        _ => (2, I64), // 0x3e
    }
}
