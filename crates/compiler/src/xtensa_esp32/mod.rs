mod asm;

use std::collections::HashMap;

use asm::*;
use wasm_parser::{
    decoder::{
        instructions::Instruction,
        types::{GlobalType, Import, ImportDesc, ValueType},
    },
    parser::module::{Function, Module},
};

fn push_stack(insts_writer: &mut AsmWriter, reg: usize) {
    insts_writer
        .comment("### push stack")
        .op("s32i.n", vec![RegA(reg), RegA(7), Imm(0)])
        .inline_comment(format!("stack[wsp] = a{};", reg))
        .op("addi", vec![RegA(7), RegA(7), Imm(4)])
        .inline_comment("wsp += 4;");
}

fn pop_stack(insts_writer: &mut AsmWriter, reg: usize) {
    insts_writer
        .comment("### pop stack")
        .op("addi", vec![RegA(7), RegA(7), Imm(-4)])
        .inline_comment("wsp -= 4;")
        .op("l32i.n", vec![RegA(reg), RegA(7), Imm(0)])
        .inline_comment(format!("a{} = stack[wsp];", reg));
}

#[derive(Debug, Clone)]
enum FuncDecl {
    Imported(Import),
    UserDefined(Function),
}

#[derive(Debug, Clone)]
struct Global {
    label: String,
    global_type: GlobalType,
}

pub struct XtensaEsp32 {
    symbol_count: usize,
    asm: AsmWriter,
    literal_i32_map: HashMap<String, String>,
    global_map: HashMap<usize, Global>,
    function_map: HashMap<u32, FuncDecl>,
}

impl XtensaEsp32 {
    pub fn new() -> Self {
        XtensaEsp32 {
            symbol_count: 0,
            asm: AsmWriter::new(),
            literal_i32_map: HashMap::new(),
            function_map: HashMap::new(),
            global_map: HashMap::new(),
        }
    }

    pub fn compile(&mut self, module: Module) -> String {
        let mut literals_writer = AsmWriter::new();
        literals_writer.op(".literal_position", vec![]);

        let mut global_idx = 0;
        for global in &module.globals {
            let label = self.gen_symbol();
            self.global_map.insert(
                global_idx,
                Global {
                    label: label.clone(),
                    global_type: global.global_type.clone(),
                },
            );

            match &global.init_expr {
                Instruction::I32Const { value } => {
                    literals_writer.op(".literal", vec![Symbol(label), LiteralI32(*value)]);
                }
                _ => {
                    panic!("Unsupported global init expression");
                }
            }

            global_idx += 1;
        }

        let mut func_idx = 0;
        for import in &module.imports {
            if matches!(import.desc, ImportDesc::Func(_)) {
                self.function_map
                    .insert(func_idx as u32, FuncDecl::Imported(import.clone()));
                func_idx += 1;
            }
        }

        for func in &module.functions {
            self.function_map
                .insert(func.index as u32, FuncDecl::UserDefined(func.clone()));

            let Some(insts) = &func.raw_body else {
                continue;
            };

            for inst in insts {
                if let Instruction::I32Const { value } = inst {
                    let key = value.to_string();

                    if self.literal_i32_map.contains_key(&key) {
                        continue;
                    }

                    let label = self.gen_symbol();
                    self.literal_i32_map.insert(key, label.clone());
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

            for (idx, _) in func.locals.iter().enumerate() {
                self.asm
                    .op("s32i.n", vec![RegA(idx + 2), SP, Imm(offset)])
                    .inline_comment(format!("local#{}", idx));
                offset += 4; // 32bit
            }
            self.asm
                .op("addi", vec![RegA(7), SP, Imm(offset)])
                .inline_comment("a7 is wsp(wasm stack pointer)");

            let mut insts_writer = AsmWriter::new();
            if let Some(last) =
                self.compile_instructions(&mut insts_writer, &mut insts.clone().into_iter())
            {
                match last {
                    Instruction::End => {
                        insts_writer.op("retw.n", vec![]);
                    }
                    _ => {}
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

        // self.asm.write_to_string(true)
        self.asm.write_to_string(false)
    }

    fn compile_instructions(
        &mut self,
        insts_writer: &mut AsmWriter,
        insts: &mut impl Iterator<Item = Instruction>,
    ) -> Option<Instruction> {
        loop {
            let Some(inst) = insts.next() else {
                break;
            };

            match inst {
                Instruction::If { block } => {
                    insts_writer.comment("if");
                    pop_stack(insts_writer, 2);
                    let falsy_case_label = self.gen_symbol();
                    insts_writer
                        .op("beqz", vec![RegA(2), Symbol(falsy_case_label.clone())])
                        .inline_comment(format!(
                            "if a2 == false then jump to {}(falsy case)",
                            &falsy_case_label
                        ));
                    insts_writer.comment("truthy case");
                    if let Some(last) = self.compile_instructions(insts_writer, insts) {
                        if matches!(last, Instruction::Else) {
                            let end_label = self.gen_symbol();
                            insts_writer.op("j", vec![Symbol(end_label.clone())]);
                            insts_writer
                                .label(falsy_case_label.clone())
                                .inline_comment("else");
                            self.compile_instructions(insts_writer, insts);

                            insts_writer
                                .label(end_label.clone())
                                .inline_comment("end if");
                        } else {
                            insts_writer
                                .label(falsy_case_label.clone())
                                .inline_comment("end if");
                        }
                    }
                }
                Instruction::Else => return Some(inst.clone()),
                Instruction::End => return Some(inst.clone()),
                Instruction::Return => {
                    pop_stack(insts_writer, 2);
                    insts_writer.comment("return").op("retw.n", vec![]);
                }
                _ => {
                    self.compile_instruction(insts_writer, &inst);
                }
            }
        }

        None
    }

    fn compile_instruction(&mut self, insts_writer: &mut AsmWriter, inst: &Instruction) {
        match inst {
            Instruction::Unreachable => todo!(),
            Instruction::Nop => todo!(),
            Instruction::Block { block } => todo!(),
            Instruction::Loop { block } => todo!(),
            Instruction::If { block } => {
                // implemented in compile_instructions
            }
            Instruction::Else => todo!(),
            Instruction::Br { level } => todo!(),
            Instruction::BrIf { level } => todo!(),
            Instruction::BrTable {
                label_indexes,
                default_index,
            } => todo!(),
            Instruction::Return => {
                // implemented in compile_instructions
            }
            Instruction::Call { func_index } => {
                let func = self.function_map.get(func_index).cloned().unwrap();
                match func {
                    FuncDecl::Imported(import) => {
                        match (import.module.as_str(), import.field.as_str()) {
                            ("wasmicon", "reg32_write") => {
                                insts_writer.comment("call wasmicon::reg32_write");
                                pop_stack(insts_writer, 3); // second arg
                                pop_stack(insts_writer, 2); // first arg
                                insts_writer
                                    .op("memw", vec![])
                                    .op("s32i.n", vec![RegA(3), RegA(2), Imm(0)]);
                            }
                            ("wasmicon", "reg32_read") => {
                                insts_writer.comment("call wasmicon::reg32_read");
                                pop_stack(insts_writer, 2); // first arg
                                insts_writer
                                    .op("l32i.n", vec![RegA(2), RegA(2), Imm(0)])
                                    .op("memw", vec![]);
                                push_stack(insts_writer, 2);
                            }
                            ("wasmicon", "sleep_ms") => {
                                insts_writer.comment("call wasmicon::sleep_ms");
                                pop_stack(insts_writer, 2); // first arg
                                insts_writer
                                    .op("l32i.n", vec![RegA(2), RegA(2), Imm(0)])
                                    .op("memw", vec![]);
                                push_stack(insts_writer, 2);
                            }
                            _ => {
                                insts_writer.unimplemented(format!(
                                    "call {}::{}",
                                    import.module, import.field
                                ));
                            }
                        }
                        // insts_writer.comment(format!("call {}::{}", module, name));
                    }
                    FuncDecl::UserDefined(func) => {
                        for i in 0..func.params.len() {
                            pop_stack(insts_writer, 10 + i); // a10, a11, a12, ...
                        }
                        insts_writer.op("call8", vec![Symbol(func.label.clone())]);
                        push_stack(insts_writer, 10);
                    }
                }
            }
            Instruction::End => {
                // insts_writer.comment("____END____");
                // insts_writer.op("retw.n", vec![]).inline_comment("end");
                // implemented in compile_instructions.
            }
            Instruction::CallIndirect {
                type_index,
                table_index,
            } => todo!(),
            Instruction::RefNull { ref_type } => todo!(),
            Instruction::RefIsNull => todo!(),
            Instruction::RefFunc { func_index } => todo!(),
            Instruction::Drop => {
                insts_writer
                    .comment("drop")
                    .op("addi", vec![RegA(7), RegA(7), Imm(-4)])
                    .inline_comment("wsp -= 4;");
            }
            Instruction::Select { result_types } => todo!(),
            Instruction::SelectResult { result_type } => todo!(),
            Instruction::LocalGet { local_index } => {
                insts_writer
                    .comment(format!("local.get {}", local_index))
                    .op("l32i.n", vec![RegA(6), SP, Imm((local_index * 4) as i32)])
                    .inline_comment(format!(
                        "a6 = stack[sp + {}(local offset)];",
                        local_index * 4
                    ));
                push_stack(insts_writer, 6);
            }
            Instruction::LocalSet { local_index } => {
                insts_writer
                    .comment(format!("local.set {}", local_index))
                    .op("s32i.n", vec![RegA(2), SP, Imm((local_index * 4) as i32)])
                    .inline_comment(format!(
                        "stack[sp + {}(local offset)] = a2;",
                        local_index * 4
                    ));
                pop_stack(insts_writer, 2);
            }
            Instruction::LocalTee { local_index } => todo!(),
            Instruction::GlobalGet { global_index } => {
                let idx = *global_index;
                let global = self.global_map.get(&(idx as usize)).unwrap();
                insts_writer
                    .comment(format!("global.get {}", global_index))
                    .op("l32r", vec![RegA(2), Symbol(global.label.clone())]);
                push_stack(insts_writer, 2);

                // let key = value.to_string();
                // let label = self.literal_i32_map.get(&key).unwrap();
                // insts_writer
                //     .comment(format!("i32.const {}", value))
                //     .op("l32r", vec![RegA(2), Symbol(label.clone())]);
                // push_stack(insts_writer, 2);
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
                let label = self.literal_i32_map.get(&key).unwrap();
                insts_writer
                    .comment(format!("i32.const {}", value))
                    .op("l32r", vec![RegA(2), Symbol(label.clone())]);
                push_stack(insts_writer, 2);
            }
            Instruction::I64Const { value } => todo!(),
            Instruction::F32Const { value } => todo!(),
            Instruction::F64Const { value } => todo!(),
            Instruction::I32Eqz => todo!(),
            Instruction::I32Eq => todo!(),
            Instruction::I32Ne => todo!(),
            Instruction::I32LtS => {
                insts_writer.comment("i32.lts");
                pop_stack(insts_writer, 2);
                pop_stack(insts_writer, 3);
                let label = self.gen_symbol();
                insts_writer
                    .op("movi.n", vec![RegA(4), Imm(1)])
                    .inline_comment("a4 = true(1)")
                    .op("blt", vec![RegA(3), RegA(2), Symbol(label.clone())])
                    .inline_comment(format!("if a3 < a2 then jump to {}", &label))
                    .op("movi.n", vec![RegA(4), Imm(0)])
                    .inline_comment("a4 = true(0)")
                    .label(label);
                push_stack(insts_writer, 4);
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
            Instruction::I32Ctz => {
                insts_writer.unimplemented("i32.ctz");
            }
            Instruction::I32Popcnt => todo!(),
            Instruction::I32Add => {
                insts_writer.comment("i32.add");
                pop_stack(insts_writer, 2);
                pop_stack(insts_writer, 3);
                insts_writer.op("add", vec![RegA(2), RegA(2), RegA(3)]);
                push_stack(insts_writer, 2);
            }
            Instruction::I32Sub => {
                insts_writer.comment("i32.sub");
                pop_stack(insts_writer, 3);
                pop_stack(insts_writer, 2);
                insts_writer.op("sub", vec![RegA(2), RegA(2), RegA(3)]);
                push_stack(insts_writer, 2);
            }
            Instruction::I32Mul => todo!(),
            Instruction::I32DivS => todo!(),
            Instruction::I32DivU => todo!(),
            Instruction::I32RemS => todo!(),
            Instruction::I32RemU => todo!(),
            Instruction::I32And => {
                pop_stack(insts_writer, 2);
                pop_stack(insts_writer, 3);
                insts_writer
                    .comment("i32.and")
                    .op("and", vec![RegA(2), RegA(3), RegA(2)]);
                push_stack(insts_writer, 2);
            }
            Instruction::I32Or => {
                pop_stack(insts_writer, 2);
                pop_stack(insts_writer, 3);
                insts_writer
                    .comment("i32.or")
                    .op("or", vec![RegA(2), RegA(3), RegA(2)]);
                push_stack(insts_writer, 2);
            }
            Instruction::I32Xor => {
                pop_stack(insts_writer, 2);
                pop_stack(insts_writer, 3);
                insts_writer
                    .comment("i32.xor")
                    .op("xor", vec![RegA(2), RegA(3), RegA(2)]);
                push_stack(insts_writer, 2);
            }
            Instruction::I32Shl => {
                insts_writer.comment("i32.shl");
                pop_stack(insts_writer, 2);
                pop_stack(insts_writer, 3);
                insts_writer
                    .op("ssl", vec![RegA(2)])
                    .inline_comment("Sets Shift Amount Register(SAR)")
                    .op("sll", vec![RegA(2), RegA(3)]);
                push_stack(insts_writer, 2);
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
