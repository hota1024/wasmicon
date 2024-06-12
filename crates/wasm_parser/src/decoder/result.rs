use super::binary::section::SectionId;

#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    #[error("invalid magic header")]
    InvalidMagicHeader,
    #[error("invalid version")]
    InvalidVersion,
    #[error("invalid section id: {0:?}")]
    InvalidSectionId(SectionId),
    #[error("invalid reference type")]
    InvalidRefType,
    #[error("invalid table instruction id")]
    InvalidTableInstructionId,
    #[error("invalid sub instruction id: {0:#x}")]
    InvalidSubInstructionId(u32), // 0xFC xx
    #[error("unxpected EOF")]
    UnexpectedEof,
    #[error("expected {0}")]
    Expected(String),

    #[error("invalid block type")]
    InvalidBlockType,

    #[error("invalid type type")]
    InvalidTypeKind,

    #[error("invalid value type")]
    InvalidValueType(u8),

    #[error("invalid import description: 0x{0:#x}")]
    InvalidImportDescription(u8),
    #[error("invalid export description")]
    InvalidExportDescription,

    #[error("invalid global init expression")]
    InvalidGlobalInitExpr,
    #[error("invalid limits kind")]
    InvalidLimitsKind,
    #[error("invalid element kind")]
    InvalidElementKind,

    #[error("expected const expression")]
    ExpectedConstExpression,
    #[error("unsupported element prefix")]
    UnsupportedElementPrefix,

    #[error("unsupported data prefix")]
    UnsupportedDataPrefix(u32),
}

pub type Result<T> = anyhow::Result<T>;
