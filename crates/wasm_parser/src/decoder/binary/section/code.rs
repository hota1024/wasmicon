use crate::decoder::binary::{instructions::Instruction, types::ValueType};

#[derive(Debug, Clone, PartialEq)]
pub struct Code {
    pub locals: Vec<ValueType>,
    pub code: Vec<Instruction>,
}
