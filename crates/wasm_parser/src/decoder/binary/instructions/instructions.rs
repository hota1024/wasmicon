use crate::decoder::binary::types::{RefType, ValueType};

use super::Block;

#[derive(Debug, Clone, PartialEq)]
pub struct MemArg {
    pub align: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    /* Controls */
    Unreachable,
    Nop,
    Block {
        block: Block,
    },
    Loop {
        block: Block,
    },
    If {
        block: Block,
    },
    Else,
    Br {
        level: u32,
    },
    BrIf {
        level: u32,
    },
    BrTable {
        label_indexes: Vec<u32>,
        default_index: u32,
    },
    Return,
    Call {
        func_index: u32,
    },
    End,
    CallIndirect {
        type_index: u32,
        table_index: u32,
    },
    /* References */
    RefNull {
        ref_type: RefType,
    },
    RefIsNull,
    RefFunc {
        func_index: u32,
    },
    /* Parametric */
    Drop,
    Select {
        result_types: Option<Vec<ValueType>>,
    },
    SelectResult {
        result_type: ValueType,
    },
    /* Variables */
    LocalGet {
        local_index: u32,
    },
    LocalSet {
        local_index: u32,
    },
    LocalTee {
        local_index: u32,
    },
    GlobalGet {
        global_index: u32,
    },
    GlobalSet {
        global_index: u32,
    },
    /* Tables */
    TableGet {
        table_index: u32,
    },
    TableSet {
        table_index: u32,
    },
    TableInit {
        element_index: u32,
        table_index: u32,
    },
    ElemDrop {
        element_index: u32,
    },
    TableCopy {
        dst_table_index: u32,
        src_table_index: u32,
    },
    TableGrow {
        table_index: u32,
    },
    TableSize {
        table_index: u32,
    },
    TableFill {
        table_index: u32,
    },
    /* Memory */
    I32Load {
        mem_arg: MemArg,
    },
    I64Load {
        mem_arg: MemArg,
    },
    F32Load {
        mem_arg: MemArg,
    },
    F64Load {
        mem_arg: MemArg,
    },
    I32Load8S {
        mem_arg: MemArg,
    },
    I32Load8U {
        mem_arg: MemArg,
    },
    I32Load16S {
        mem_arg: MemArg,
    },
    I32Load16U {
        mem_arg: MemArg,
    },
    I64Load8S {
        mem_arg: MemArg,
    },
    I64Load8U {
        mem_arg: MemArg,
    },
    I64Load16S {
        mem_arg: MemArg,
    },
    I64Load16U {
        mem_arg: MemArg,
    },
    I64Load32S {
        mem_arg: MemArg,
    },
    I64Load32U {
        mem_arg: MemArg,
    },
    I32Store {
        mem_arg: MemArg,
    },
    I64Store {
        mem_arg: MemArg,
    },
    F32Store {
        mem_arg: MemArg,
    },
    F64Store {
        mem_arg: MemArg,
    },
    I32Store8 {
        mem_arg: MemArg,
    },
    I32Store16 {
        mem_arg: MemArg,
    },
    I64Store8 {
        mem_arg: MemArg,
    },
    I64Store16 {
        mem_arg: MemArg,
    },
    I64Store32 {
        mem_arg: MemArg,
    },
    MemorySize,
    MemoryGrow,
    MemoryInit {
        data_index: u32,
    },
    DataDrop {
        data_index: u32,
    },
    MemoryCopy,
    MemoryFill,
    /* Numerics */
    I32Const {
        value: i32,
    },
    I64Const {
        value: i64,
    },
    F32Const {
        value: f32,
    },
    F64Const {
        value: f64,
    },
    I32Eqz,
    I32Eq,
    I32Ne,
    I32LtS,
    I32LtU,
    I32GtS,
    I32GtU,
    I32LeS,
    I32LeU,
    I32GeS,
    I32GeU,
    I64Eqz,
    I64Eq,
    I64Ne,
    I64LtS,
    I64LtU,
    I64GtS,
    I64GtU,
    I64LeS,
    I64LeU,
    I64GeS,
    I64GeU,
    F32Eq,
    F32Ne,
    F32Lt,
    F32Gt,
    F32Le,
    F32Ge,
    F64Eq,
    F64Ne,
    F64Lt,
    F64Gt,
    F64Le,
    F64Ge,
    I32Clz,
    I32Ctz,
    I32Popcnt,
    I32Add,
    I32Sub,
    I32Mul,
    I32DivS,
    I32DivU,
    I32RemS,
    I32RemU,
    I32And,
    I32Or,
    I32Xor,
    I32Shl,
    I32ShrS,
    I32ShrU,
    I32Rotl,
    I32Rotr,
    I64Clz,
    I64Ctz,
    I64Popcnt,
    I64Add,
    I64Sub,
    I64Mul,
    I64DivS,
    I64DivU,
    I64RemS,
    I64RemU,
    I64And,
    I64Or,
    I64Xor,
    I64Shl,
    I64ShrS,
    I64ShrU,
    I64Rotl,
    I64Rotr,
    F32Abs,
    F32Neg,
    F32Ceil,
    F32Floor,
    F32Trunc,
    F32Nearest,
    F32Sqrt,
    F32Add,
    F32Sub,
    F32Mul,
    F32Div,
    F32Min,
    F32Max,
    F32Copysign,
    F64Abs,
    F64Neg,
    F64Ceil,
    F64Floor,
    F64Trunc,
    F64Nearest,
    F64Sqrt,
    F64Add,
    F64Sub,
    F64Mul,
    F64Div,
    F64Min,
    F64Max,
    F64Copysign,
    I32WrapI64,
    I32TruncF32S,
    I32TruncF32U,
    I32TruncF64S,
    I32TruncF64U,
    I64ExtendI32S,
    I64ExtendI32U,
    I64TruncF32S,
    I64TruncF32U,
    I64TruncF64S,
    I64TruncF64U,
    F32ConvertI32S,
    F32ConvertI32U,
    F32ConvertI64S,
    F32ConvertI64U,
    F32DemoteF64,
    F64ConvertI32S,
    F64ConvertI32U,
    F64ConvertI64S,
    F64ConvertI64U,
    F64PromoteF32,
    I32ReinterpretF32,
    I64ReinterpretF64,
    F32ReinterpretI32,
    F64ReinterpretI64,
    I32Extend8S,
    I32Extend16S,
    I64Extend8S,
    I64Extend16S,
    I64Extend32S,
    I32TruncSatF32S,
    I32TruncSatF32U,
    I32TruncSatF64S,
    I32TruncSatF64U,
    I64TruncSatF32S,
    I64TruncSatF32U,
    I64TruncSatF64S,
    I64TruncSatF64U,
    /* Vectors */
}
