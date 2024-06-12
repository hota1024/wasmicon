use wasm_parser::decoder::Decoder;

fn main() {
    let wasm = include_bytes!("../test.wasm");

    let mut decoder = Decoder::new(&wasm[..]);
    let module = decoder.decode().unwrap();
    dbg!(module);
}
