use crate::decoder::{
    instructions::Instruction,
    types::{Global, Import, ValueType},
};

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDecl {
    index: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub index: usize,
    pub label: String,
    pub export_name: Option<String>,
    pub params: Vec<ValueType>,
    pub results: Vec<ValueType>,
    pub params_locals: Vec<ValueType>,
    pub locals: Vec<ValueType>,
    pub raw_body: Option<Vec<Instruction>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub functions: Vec<Function>,
    pub imports: Vec<Import>,
    pub globals: Vec<Global>,
}
