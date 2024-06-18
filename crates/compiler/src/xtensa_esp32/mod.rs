mod asm;

use std::collections::HashMap;

use asm::*;
use wasm_parser::{
    decoder::{instructions::Instruction, types::ValueType},
    parser::module::{Function, Module},
};

fn push_stack(insts_writer: &mut AsmWriter, reg: usize) {
    insts_writer
        .comment("-- push stack")
        .op("s32i.n", vec![RegA(reg), RegA(7), Imm(0)])
        .inline_comment(format!("stack[wsp] = a{};", reg))
        .op("addi", vec![RegA(7), RegA(7), Imm(4)])
        .inline_comment("wsp += 4;");
}

fn pop_stack(insts_writer: &mut AsmWriter, reg: usize) {
    insts_writer
        .comment("-- pop stack")
        .op("addi", vec![RegA(7), RegA(7), Imm(-4)])
        .inline_comment("wsp -= 4;")
        .op("l32i.n", vec![RegA(reg), RegA(7), Imm(0)])
        .inline_comment(format!("a{} = stack[wsp];", reg));
}

pub struct XtensaEsp32 {
    symbol_count: usize,
    asm: AsmWriter,
}

impl XtensaEsp32 {
    pub fn new() -> Self {
        XtensaEsp32 {
            symbol_count: 0,
            asm: AsmWriter::new(),
        }
    }

    pub fn compile(&mut self, module: Module) -> String {
        let mut literals_writer = AsmWriter::new();
        literals_writer.op(".literal_position", vec![]);

        let mut literal_i32_map = HashMap::<String, String>::new();
        for func in &module.functions {
            let Some(insts) = &func.raw_body else {
                continue;
            };

            for inst in insts {
                if let Instruction::I32Const { value } = inst {
                    let key = value.to_string();

                    if literal_i32_map.contains_key(&key) {
                        continue;
                    }

                    let label = self.gen_symbol();
                    literal_i32_map.insert(key, label.clone());
                    literals_writer.op(".literal", vec![Symbol(label.clone()), LiteralI32(*value)]);
                }
            }
        }
        self.asm.extend(literals_writer);

        for func in &module.functions {
            let Some(insts) = &func.raw_body else {
                continue;
            };

            let func_label = func.label.clone();
            let name = func
                .export_name
                .clone()
                .unwrap_or_else(|| func_label.clone().to_string());

            self.asm
                .op(".align", vec![Imm(4)])
                .op(".global", vec![Symbol(func_label.clone())])
                .op(
                    ".type",
                    vec![Symbol(func_label.clone()), symbol("@function")],
                )
                .label(func_label.clone())
                .inline_comment(name.clone())
                .op("entry", vec![SP, Imm(64)]);

            let mut offset = 0;
            for (idx, _) in func.params.iter().enumerate() {
                self.asm
                    .op("s32i.n", vec![RegA(idx + 2), SP, Imm(offset)])
                    .inline_comment(format!("param#{}", idx));
                offset += 4; // 32bit
            }
            self.asm
                .op("addi", vec![RegA(7), SP, Imm(offset)])
                .inline_comment("a7 is wsp(wasm stack pointer)");

            let mut insts_writer = AsmWriter::new();
            let mut insts = insts.iter();

            loop {
                let Some(inst) = insts.next() else {
                    break;
                };

                match inst {
                    Instruction::LocalGet { local_index } => {
                        insts_writer
                            .comment(format!("local.get {}", local_index))
                            .op("l32i.n", vec![RegA(6), SP, Imm((local_index * 4) as i32)])
                            .inline_comment(format!(
                                "a6 = stack[sp + {}(local offset)];",
                                local_index * 4
                            ));
                        push_stack(&mut insts_writer, 6);
                    }
                    Instruction::I32Add => {
                        insts_writer.comment("i32.add");
                        pop_stack(&mut insts_writer, 2);
                        pop_stack(&mut insts_writer, 3);
                        insts_writer.op("add", vec![RegA(2), RegA(2), RegA(3)]);
                        push_stack(&mut insts_writer, 2);
                    }
                    Instruction::Unreachable => todo!(),
                    Instruction::Nop => todo!(),
                    Instruction::Block { block } => todo!(),
                    Instruction::Loop { block } => todo!(),
                    Instruction::If { block } => todo!(),
                    Instruction::Else => todo!(),
                    Instruction::Br { level } => todo!(),
                    Instruction::BrIf { level } => todo!(),
                    Instruction::BrTable {
                        label_indexes,
                        default_index,
                    } => todo!(),
                    Instruction::Return => {
                        pop_stack(&mut insts_writer, 2);
                        insts_writer.op("retw.n", vec![]).inline_comment("return");
                        // insts_writer.comment("____RETURN____");
                    }
                    Instruction::Call { func_index } => {
                        insts_writer.comment("call");
                        let func = &module.functions[*func_index as usize];
                        for i in 0..func.params.len() {
                            pop_stack(&mut insts_writer, 10 + i); // a10, a11, a12, ...
                        }
                        insts_writer.op("call8", vec![Symbol(func.label.clone())]);
                        push_stack(&mut insts_writer, 10);
                        // let func = insts_writer.comment("____CALL____");
                    }
                    Instruction::End => {
                        // insts_writer.comment("____END____");
                        insts_writer.op("retw.n", vec![]).inline_comment("end");
                    }
                    Instruction::CallIndirect {
                        type_index,
                        table_index,
                    } => todo!(),
                    Instruction::RefNull { ref_type } => todo!(),
                    Instruction::RefIsNull => todo!(),
                    Instruction::RefFunc { func_index } => todo!(),
                    Instruction::Drop => todo!(),
                    Instruction::Select { result_types } => todo!(),
                    Instruction::SelectResult { result_type } => todo!(),
                    Instruction::LocalSet { local_index } => todo!(),
                    Instruction::LocalTee { local_index } => todo!(),
                    Instruction::GlobalGet { global_index } => {
                        insts_writer.comment("global.get");
                    }
                    Instruction::GlobalSet { global_index } => {
                        insts_writer.comment("global.set");
                    }
                    Instruction::TableGet { table_index } => todo!(),
                    Instruction::TableSet { table_index } => todo!(),
                    Instruction::TableInit {
                        element_index,
                        table_index,
                    } => todo!(),
                    Instruction::ElemDrop { element_index } => todo!(),
                    Instruction::TableCopy {
                        dst_table_index,
                        src_table_index,
                    } => todo!(),
                    Instruction::TableGrow { table_index } => todo!(),
                    Instruction::TableSize { table_index } => todo!(),
                    Instruction::TableFill { table_index } => todo!(),
                    Instruction::I32Load { mem_arg } => todo!(),
                    Instruction::I64Load { mem_arg } => todo!(),
                    Instruction::F32Load { mem_arg } => todo!(),
                    Instruction::F64Load { mem_arg } => todo!(),
                    Instruction::I32Load8S { mem_arg } => todo!(),
                    Instruction::I32Load8U { mem_arg } => todo!(),
                    Instruction::I32Load16S { mem_arg } => todo!(),
                    Instruction::I32Load16U { mem_arg } => todo!(),
                    Instruction::I64Load8S { mem_arg } => todo!(),
                    Instruction::I64Load8U { mem_arg } => todo!(),
                    Instruction::I64Load16S { mem_arg } => todo!(),
                    Instruction::I64Load16U { mem_arg } => todo!(),
                    Instruction::I64Load32S { mem_arg } => todo!(),
                    Instruction::I64Load32U { mem_arg } => todo!(),
                    Instruction::I32Store { mem_arg } => todo!(),
                    Instruction::I64Store { mem_arg } => todo!(),
                    Instruction::F32Store { mem_arg } => todo!(),
                    Instruction::F64Store { mem_arg } => todo!(),
                    Instruction::I32Store8 { mem_arg } => todo!(),
                    Instruction::I32Store16 { mem_arg } => todo!(),
                    Instruction::I64Store8 { mem_arg } => todo!(),
                    Instruction::I64Store16 { mem_arg } => todo!(),
                    Instruction::I64Store32 { mem_arg } => todo!(),
                    Instruction::MemorySize => todo!(),
                    Instruction::MemoryGrow => todo!(),
                    Instruction::MemoryInit { data_index } => todo!(),
                    Instruction::DataDrop { data_index } => todo!(),
                    Instruction::MemoryCopy => todo!(),
                    Instruction::MemoryFill => todo!(),
                    Instruction::I32Const { value } => {
                        let key = value.to_string();
                        let label = literal_i32_map.get(&key).unwrap();
                        insts_writer
                            .comment(format!("i32.const {}", value))
                            .op("l32r", vec![RegA(2), Symbol(label.clone())]);
                        push_stack(&mut insts_writer, 2);
                    }
                    Instruction::I64Const { value } => todo!(),
                    Instruction::F32Const { value } => todo!(),
                    Instruction::F64Const { value } => todo!(),
                    Instruction::I32Eqz => todo!(),
                    Instruction::I32Eq => todo!(),
                    Instruction::I32Ne => todo!(),
                    Instruction::I32LtS => {
                        insts_writer.comment("i32.lts");
                        pop_stack(&mut insts_writer, 2);
                        pop_stack(&mut insts_writer, 3);
                    }
                    Instruction::I32LtU => todo!(),
                    Instruction::I32GtS => todo!(),
                    Instruction::I32GtU => todo!(),
                    Instruction::I32LeS => todo!(),
                    Instruction::I32LeU => todo!(),
                    Instruction::I32GeS => todo!(),
                    Instruction::I32GeU => todo!(),
                    Instruction::I64Eqz => todo!(),
                    Instruction::I64Eq => todo!(),
                    Instruction::I64Ne => todo!(),
                    Instruction::I64LtS => todo!(),
                    Instruction::I64LtU => todo!(),
                    Instruction::I64GtS => todo!(),
                    Instruction::I64GtU => todo!(),
                    Instruction::I64LeS => todo!(),
                    Instruction::I64LeU => todo!(),
                    Instruction::I64GeS => todo!(),
                    Instruction::I64GeU => todo!(),
                    Instruction::F32Eq => todo!(),
                    Instruction::F32Ne => todo!(),
                    Instruction::F32Lt => todo!(),
                    Instruction::F32Gt => todo!(),
                    Instruction::F32Le => todo!(),
                    Instruction::F32Ge => todo!(),
                    Instruction::F64Eq => todo!(),
                    Instruction::F64Ne => todo!(),
                    Instruction::F64Lt => todo!(),
                    Instruction::F64Gt => todo!(),
                    Instruction::F64Le => todo!(),
                    Instruction::F64Ge => todo!(),
                    Instruction::I32Clz => todo!(),
                    Instruction::I32Ctz => todo!(),
                    Instruction::I32Popcnt => todo!(),
                    Instruction::I32Sub => todo!(),
                    Instruction::I32Mul => todo!(),
                    Instruction::I32DivS => todo!(),
                    Instruction::I32DivU => todo!(),
                    Instruction::I32RemS => todo!(),
                    Instruction::I32RemU => todo!(),
                    Instruction::I32And => {
                        insts_writer.comment("____UNIMPLEMENTED(i32.and)____");
                    }
                    Instruction::I32Or => {
                        insts_writer.comment("____UNIMPLEMENTED(i32.or)____");
                    }
                    Instruction::I32Xor => {
                        insts_writer.comment("____UNIMPLEMENTED(i32.xor)____");
                    }
                    Instruction::I32Shl => {
                        insts_writer.comment("____UNIMPLEMENTED(i32.shl)____");
                    }
                    Instruction::I32ShrS => todo!(),
                    Instruction::I32ShrU => todo!(),
                    Instruction::I32Rotl => todo!(),
                    Instruction::I32Rotr => todo!(),
                    Instruction::I64Clz => todo!(),
                    Instruction::I64Ctz => todo!(),
                    Instruction::I64Popcnt => todo!(),
                    Instruction::I64Add => todo!(),
                    Instruction::I64Sub => todo!(),
                    Instruction::I64Mul => todo!(),
                    Instruction::I64DivS => todo!(),
                    Instruction::I64DivU => todo!(),
                    Instruction::I64RemS => todo!(),
                    Instruction::I64RemU => todo!(),
                    Instruction::I64And => todo!(),
                    Instruction::I64Or => todo!(),
                    Instruction::I64Xor => todo!(),
                    Instruction::I64Shl => todo!(),
                    Instruction::I64ShrS => todo!(),
                    Instruction::I64ShrU => todo!(),
                    Instruction::I64Rotl => todo!(),
                    Instruction::I64Rotr => todo!(),
                    Instruction::F32Abs => todo!(),
                    Instruction::F32Neg => todo!(),
                    Instruction::F32Ceil => todo!(),
                    Instruction::F32Floor => todo!(),
                    Instruction::F32Trunc => todo!(),
                    Instruction::F32Nearest => todo!(),
                    Instruction::F32Sqrt => todo!(),
                    Instruction::F32Add => todo!(),
                    Instruction::F32Sub => todo!(),
                    Instruction::F32Mul => todo!(),
                    Instruction::F32Div => todo!(),
                    Instruction::F32Min => todo!(),
                    Instruction::F32Max => todo!(),
                    Instruction::F32Copysign => todo!(),
                    Instruction::F64Abs => todo!(),
                    Instruction::F64Neg => todo!(),
                    Instruction::F64Ceil => todo!(),
                    Instruction::F64Floor => todo!(),
                    Instruction::F64Trunc => todo!(),
                    Instruction::F64Nearest => todo!(),
                    Instruction::F64Sqrt => todo!(),
                    Instruction::F64Add => todo!(),
                    Instruction::F64Sub => todo!(),
                    Instruction::F64Mul => todo!(),
                    Instruction::F64Div => todo!(),
                    Instruction::F64Min => todo!(),
                    Instruction::F64Max => todo!(),
                    Instruction::F64Copysign => todo!(),
                    Instruction::I32WrapI64 => todo!(),
                    Instruction::I32TruncF32S => todo!(),
                    Instruction::I32TruncF32U => todo!(),
                    Instruction::I32TruncF64S => todo!(),
                    Instruction::I32TruncF64U => todo!(),
                    Instruction::I64ExtendI32S => todo!(),
                    Instruction::I64ExtendI32U => todo!(),
                    Instruction::I64TruncF32S => todo!(),
                    Instruction::I64TruncF32U => todo!(),
                    Instruction::I64TruncF64S => todo!(),
                    Instruction::I64TruncF64U => todo!(),
                    Instruction::F32ConvertI32S => todo!(),
                    Instruction::F32ConvertI32U => todo!(),
                    Instruction::F32ConvertI64S => todo!(),
                    Instruction::F32ConvertI64U => todo!(),
                    Instruction::F32DemoteF64 => todo!(),
                    Instruction::F64ConvertI32S => todo!(),
                    Instruction::F64ConvertI32U => todo!(),
                    Instruction::F64ConvertI64S => todo!(),
                    Instruction::F64ConvertI64U => todo!(),
                    Instruction::F64PromoteF32 => todo!(),
                    Instruction::I32ReinterpretF32 => todo!(),
                    Instruction::I64ReinterpretF64 => todo!(),
                    Instruction::F32ReinterpretI32 => todo!(),
                    Instruction::F64ReinterpretI64 => todo!(),
                    Instruction::I32Extend8S => todo!(),
                    Instruction::I32Extend16S => todo!(),
                    Instruction::I64Extend8S => todo!(),
                    Instruction::I64Extend16S => todo!(),
                    Instruction::I64Extend32S => todo!(),
                    Instruction::I32TruncSatF32S => todo!(),
                    Instruction::I32TruncSatF32U => todo!(),
                    Instruction::I32TruncSatF64S => todo!(),
                    Instruction::I32TruncSatF64U => todo!(),
                    Instruction::I64TruncSatF32S => todo!(),
                    Instruction::I64TruncSatF32U => todo!(),
                    Instruction::I64TruncSatF64S => todo!(),
                    Instruction::I64TruncSatF64U => todo!(),
                }
            }

            self.asm.extend(insts_writer);
            self.asm.op(
                ".size",
                vec![
                    Symbol(func_label.clone()),
                    symbol(format!(".-{}", func_label.to_string())),
                ],
            );
        }

        self.asm.write_to_string()
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
