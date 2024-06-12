#![no_main]

use libfuzzer_sys::fuzz_target;
use wasm_parser::decoder::Decoder;
use wasm_smith::Module;

fuzz_target!(|module: Module| {
    let wasm_bytes = module.to_bytes();

    // println!("------------------------------WASM------------------------------");
    // std::fs::write("test.wasm", wasm_bytes.as_slice()).unwrap();
    // let output = Command::new("wasm2wat").arg("test.wasm").output().unwrap();
    // std::fs::write("test.wat", output.stdout.as_slice()).unwrap();
    // println!("{}", std::str::from_utf8(&output.stdout).unwrap());
    // println!("----------------------------------------------------------------");

    let mut decoder = Decoder::new(wasm_bytes.as_slice());

    decoder.decode().unwrap();
});
