use std::{env::args, fs, process};

use compiler::xtensa_esp32;
use wasm_parser::{
    decoder::{instructions::Instruction, Decoder},
    parser::Parser,
};

fn main() {
    // let wasm = fs::read("examples/reg.wasm").unwrap();
    // let wasm = fs::read("examples/led.wasm").unwrap();
    // let wasm = fs::read("examples/add_two.wasm").unwrap();
    let wasm = fs::read(args().nth(1).unwrap()).unwrap();
    // let wasm = fs::read("examples/sandbox.wasm").unwrap();

    let mut decoder = Decoder::new(&wasm[..]);
    let module = decoder.decode().unwrap();

    let mut parser = Parser::new(module);
    let module = parser.parse();

    let mut compiler = xtensa_esp32::XtensaEsp32::new();
    let result = compiler.compile(module);
    println!("{}", &result);

    // fs::write(
    //     "/Users/hota1024/GitHub/hota1024/esp32_nolib_c/src/main.s",
    //     &result,
    // )
    // .unwrap();
}
