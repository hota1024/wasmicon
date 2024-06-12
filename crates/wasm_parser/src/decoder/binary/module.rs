use super::{
    section::Code,
    types::{Data, Element, Export, FuncType, Global, Import, MemoryType, TableType},
};

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub version: u32,
    pub custom_section: Option<()>,
    pub type_section: Option<Vec<FuncType>>,
    pub import_section: Option<Vec<Import>>,
    pub function_section: Option<Vec<u32>>,
    pub table_section: Option<Vec<TableType>>,
    pub memory_section: Option<Vec<MemoryType>>,
    pub global_section: Option<Vec<Global>>,
    pub export_section: Option<Vec<Export>>,
    pub start_section: Option<u32>,
    pub element_section: Option<Vec<Element>>,
    pub code_section: Option<Vec<Code>>,
    pub data_section: Option<Vec<Data>>,
    pub data_count_section: Option<u32>,
}

impl Default for Module {
    fn default() -> Self {
        Module {
            version: 0,
            custom_section: None,
            type_section: None,
            import_section: None,
            function_section: None,
            table_section: None,
            memory_section: None,
            global_section: None,
            export_section: None,
            start_section: None,
            element_section: None,
            code_section: None,
            data_section: None,
            data_count_section: None,
        }
    }
}
