use crate::decoder::{self, types::ExportDesc};

use super::module::{Function, Module};

pub struct Parser {
    pub module_binary: decoder::module::Module,
}

impl Parser {
    pub fn new(module_binary: decoder::module::Module) -> Self {
        Parser { module_binary }
    }

    pub fn parse(&mut self) -> Module {
        Module {
            functions: self.parse_functions(),
        }
    }

    fn parse_functions(&mut self) -> Vec<Function> {
        let mut funcs = vec![];
        let mut func_idx: usize = 0;

        for import in &self.module_binary.import_section {
            if let decoder::types::ImportDesc::Func(type_idx) = import.desc {
                func_idx += 1;
            }
        }

        for func_sig_idx in &self.module_binary.function_section {
            let func_type = &self.module_binary.type_section[*func_sig_idx as usize];
            let func_body = &self.module_binary.code_section[func_idx as usize];
            let mut params_locals = func_type.params.clone();
            params_locals.append(&mut func_body.locals.clone());
            let export_name = self
                .module_binary
                .export_section
                .iter()
                .find(|a| matches!(a.desc, ExportDesc::Func(f) if f == func_idx as u32))
                .map(|a| a.name.clone());

            let func = Function {
                index: func_idx,
                label: export_name.clone().unwrap_or(format!("func_{}", func_idx)),
                export_name,
                params: func_type.params.clone(),
                results: func_type.results.clone(),
                locals: func_body.locals.clone(),
                params_locals,
                raw_body: Some(func_body.code.clone()),
            };
            funcs.push(func);

            func_idx += 1;
        }

        funcs
    }
}
