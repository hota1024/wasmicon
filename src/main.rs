use std::fs;

use compiler::xtensa_esp32;
use wasm_parser::{decoder::Decoder, parser::Parser};

fn main() {
    // let wasm = fs::read("examples/led.wasm").unwrap();
    // let wasm = fs::read("examples/add_two.wasm").unwrap();
    let wasm = fs::read("examples/fib.wasm").unwrap();

    let mut decoder = Decoder::new(&wasm[..]);
    let module = decoder.decode().unwrap();

    let mut parser = Parser::new(module);
    let module = parser.parse();

    let mut compiler = xtensa_esp32::XtensaEsp32::new();
    let result = compiler.compile(module);
    println!("{}", &result);
}
