use super::instructions::Instruction;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueType {
    /// 32-bit integer.
    ///
    /// `0x7F`
    I32,
    /// 64-bit integer.
    ///
    /// `0x7E`
    I64,
    /// 32-bit float.
    ///
    /// `0x7D`
    F32,
    /// 64-bit float.
    ///
    /// `0x7C`
    F64,
    /// 128-bit vector.
    ///
    /// `0x7B`
    V128,
    /// 64-bit reference to a function.
    ///
    /// `0x70`
    FuncRef,
    /// 64-bit reference to a external function.
    ///
    /// `0x6F`
    ExternRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuncType {
    pub params: Vec<ValueType>,
    pub results: Vec<ValueType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefType {
    FuncRef,
    ExternRef,
}

/// Limits.
///
/// WebAssembly specification: https://webassembly.github.io/spec/core/syntax/types.html#limits
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    pub min: u32,
    pub max: Option<u32>,
}

/// Table type.
///
/// WebAssembly specification: https://webassembly.github.io/spec/core/syntax/types.html#table-types
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableType {
    pub element_type: RefType,
    pub limits: Limits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryType {
    pub limits: Limits,
}

/// Global type.
///
/// WebAssembly specification: https://webassembly.github.io/spec/core/syntax/types.html#global-types
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalType {
    pub value_type: ValueType,
    pub mutable: bool,
}

/// Global section.
///
/// WebAssembly specification: https://webassembly.github.io/spec/core/binary/modules.html#global-section
#[derive(Clone, Debug, PartialEq)]
pub struct Global {
    pub global_type: GlobalType,
    pub init_expr: Instruction,
}

/// Import description.
///
/// WebAssembly specification: https://webassembly.github.io/spec/core/binary/modules.html#binary-importdesc
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportDesc {
    Func(u32),
    Table(TableType),
    Memory(Limits),
    Global(GlobalType),
}

/// Import.
///
/// WebAssembly specification: https://webassembly.github.io/spec/core/binary/modules.html#binary-import
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    pub module: String,
    pub field: String,
    pub desc: ImportDesc,
}

/// Export.
///
/// WebAssembly specification: https://webassembly.github.io/spec/core/binary/modules.html#binary-export
#[derive(Clone, Debug, PartialEq)]
pub struct Export {
    pub name: String,
    pub desc: ExportDesc,
}

/// Export description.
///
/// WebAssembly specification: https://webassembly.github.io/spec/core/binary/modules.html#binary-exportdesc
#[derive(Clone, Debug, PartialEq)]
pub enum ExportDesc {
    Func(u32),
    Table(u32),
    Mem(u32),
    Global(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub mode: ElementMode,
    pub ref_type: RefType,
    pub init: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ElementMode {
    Passive,
    Active {
        table_index: u32,
        offset: Instruction,
    },
    Declarative,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Data {
    pub mode: DataMode,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DataMode {
    Passive,
    Active {
        memory_index: u32,
        offset: Instruction,
    },
}
