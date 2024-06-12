#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SectionId {
    Custom,
    Type,
    Import,
    Function,
    Table,
    Memory,
    Global,
    Export,
    Start,
    Element,
    Code,
    Data,
    DataCount,
    Unknown(u8),
}

impl From<u8> for SectionId {
    fn from(value: u8) -> Self {
        match value {
            0 => SectionId::Custom,
            1 => SectionId::Type,
            2 => SectionId::Import,
            3 => SectionId::Function,
            4 => SectionId::Table,
            5 => SectionId::Memory,
            6 => SectionId::Global,
            7 => SectionId::Export,
            8 => SectionId::Start,
            9 => SectionId::Element,
            10 => SectionId::Code,
            11 => SectionId::Data,
            12 => SectionId::DataCount,
            _ => SectionId::Unknown(value),
        }
    }
}

impl SectionId {
    pub fn is_unknown(&self) -> bool {
        match self {
            SectionId::Unknown(_) => true,
            _ => false,
        }
    }
}
