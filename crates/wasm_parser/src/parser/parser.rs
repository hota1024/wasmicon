use crate::decoder;

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

        for func_idx in &self.module_binary.function_section {
            let func_type = &self.module_binary.type_section[*func_idx as usize];
            let func_body = &self.module_binary.code_section[*func_idx as usize];
            let export = &self.module_binary.export_section.get(*func_idx as usize);
            let mut params_locals = func_type.params.clone();
            params_locals.append(&mut func_body.locals.clone());

            let func = Function {
                export_name: export.map(|e| e.name.clone()),
                params: func_type.params.clone(),
                results: func_type.results.clone(),
                locals: func_body.locals.clone(),
                params_locals,
                raw_instructions: func_body.code.clone(),
            };
            funcs.push(func);
        }

        funcs
    }
}
