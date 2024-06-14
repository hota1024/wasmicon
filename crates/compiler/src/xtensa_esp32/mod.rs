use std::fmt::format;

use wasm_parser::{
    decoder::{instructions::Instruction, types::ValueType},
    parser::module::{Function, Module},
};

pub struct XtensaEsp32 {
    indent_size: usize,
    symbol_count: usize,
}

impl XtensaEsp32 {
    pub fn new() -> Self {
        XtensaEsp32 {
            indent_size: 4,
            symbol_count: 0,
        }
    }

    pub fn compile(&mut self, module: Module) -> String {
        let mut lines = vec![];

        for func in &module.functions {
            lines.push(self.gen_function(func));
        }

        lines.join("\n")
    }

    fn gen_function(&mut self, func: &Function) -> String {
        let mut lines = vec![];
        let name = func
            .export_name
            .clone()
            .unwrap_or_else(|| self.gen_symbol());

        lines.push(format!("\t.align 4"));
        lines.push(format!("\t.global\t{}", name));
        lines.push(format!("\t.type\t{}, @function", name));

        lines.push(format!("{}:", name));
        lines.push(format!("\tentry\tsp, 32"));

        let mut offset = 0;
        for (idx, param) in func.params.iter().enumerate() {
            lines.push(format!("\t# param#{}", idx + 1));
            lines.push(format!("\ts32i.n\ta{}, sp, {}", idx + 2, offset));
            offset += 4; // 32bit
        }
        lines.push(format!("\t# a7 = wasm sp"));
        lines.push(format!("\taddi\ta7, sp, {}", offset));

        let mut insts = func.raw_instructions.iter();

        loop {
            let Some(inst) = insts.next() else {
                break;
            };

            match inst {
                Instruction::LocalGet { local_index } => {
                    lines.push(format!("\t# local.get {}", local_index));
                    lines.push(format!("\tl32i.n\ta6, sp, {}", local_index * 4));
                    lines.push(format!("\ts32i.n\ta6, a7, 0"));
                    lines.push(format!("\taddi\ta7, a7, 4"));
                }
                Instruction::I32Add => {
                    lines.push(format!("\t# i32.add"));
                    // pop
                    lines.push(format!("\taddi\ta7, a7, -4"));
                    lines.push(format!("\tl32i.n\ta2, a7, 0"));
                    // pop
                    lines.push(format!("\taddi\ta7, a7, -4"));
                    lines.push(format!("\tl32i.n\ta3, a7, 0"));

                    // a2 = add
                    lines.push(format!("\tadd\ta2, a2, a3"));

                    // push a2
                    lines.push(format!("\ts32i.n\ta2, a7, 0"));
                    lines.push(format!("\taddi\ta7, a7, 4"));
                }
                Instruction::End => {
                    // lines.push(format!("\t// pop result"));
                    // // pop
                    // lines.push(format!("\taddi\ta7, a7, -4"));
                    // lines.push(format!("\tl32i.n\ta2, a6, 0"));
                    lines.push(format!("\tretw.n"));
                    break;
                }
                _ => {
                    lines.push(format!("UNIMPLEMENTED: {:?}", inst));
                }
            }
        }

        lines.push(format!("\t.size {}, .-{}", name, name));

        lines.join("\n")
    }

    fn indent(&self) -> String {
        " ".repeat(self.indent_size)
    }

    fn gen_symbol(&mut self) -> String {
        let s = format!("L{}", self.symbol_count);
        self.symbol_count += 1;

        s
    }

    fn get_value_type_byte_siize(&self, value_type: &ValueType) -> usize {
        match value_type {
            ValueType::I32 => 4,
            ValueType::I64 => 8,
            ValueType::F32 => 4,
            ValueType::F64 => 8,
            _ => unimplemented!(),
        }
    }
}
