use crate::decoder::binary::types::ValueType;

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub block_type: BlockType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BlockType {
    Empty,
    Value(Vec<ValueType>),
    TypeIndex(u32),
}

impl BlockType {
    pub fn result_count(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Value(types) => types.len(),
            Self::TypeIndex(_) => 1,
        }
    }
}
