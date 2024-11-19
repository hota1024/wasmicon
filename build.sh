# wat2wasm examples/sandbox.wat -o examples/sandbox.wasm
wat2wasm examples/$1.wat -o examples/$1.wasm
cargo run -- examples/$1.wasm
