use super::{
    section::Code,
    types::{Data, Element, Export, FuncType, Global, Import, MemoryType, TableType},
};

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub version: u32,
    pub custom_section: Option<()>,
    pub type_section: Vec<FuncType>,
    pub import_section: Vec<Import>,
    pub function_section: Vec<u32>,
    pub table_section: Vec<TableType>,
    pub memory_section: Vec<MemoryType>,
    pub global_section: Vec<Global>,
    pub export_section: Vec<Export>,
    pub start_section: Option<u32>,
    pub element_section: Vec<Element>,
    pub code_section: Vec<Code>,
    pub data_section: Vec<Data>,
    pub data_count_section: Option<u32>,
}

impl Default for Module {
    fn default() -> Self {
        Module {
            version: 0,
            custom_section: None,
            type_section: vec![],
            import_section: vec![],
            function_section: vec![],
            table_section: vec![],
            memory_section: vec![],
            global_section: vec![],
            export_section: vec![],
            start_section: None,
            element_section: vec![],
            code_section: vec![],
            data_section: vec![],
            data_count_section: None,
        }
    }
}
