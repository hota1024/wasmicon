use crate::decoder::{instructions::Instruction, types::ValueType};

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub export_name: Option<String>,
    pub params: Vec<ValueType>,
    pub results: Vec<ValueType>,
    pub params_locals: Vec<ValueType>,
    pub locals: Vec<ValueType>,
    pub raw_instructions: Vec<Instruction>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub functions: Vec<Function>,
}
