//! Runs a hand-written assets guest (`assets.wat`) on the host port and checks its trace.
//!
//! The smallest end-to-end test of `wasmicon:hal/assets@0.1.0`: it needs no
//! app build, only `wasm-tools`, and locks in that the host mock serves the
//! `Options::assets` list and that the calls produce the docs/abi-spec.md §9
//! trace lines. The sample app is covered by `tests/apps.rs`.

use std::path::PathBuf;
use std::process::Command;

fn wat_to_wasm(name: &str) -> Vec<u8> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(name);
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("{name}.wasm"));
    let status = Command::new("wasm-tools")
        .arg("parse")
        .arg(&src)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("cannot run wasm-tools");
    assert!(status.success(), "wasm-tools parse failed");
    std::fs::read(&out).unwrap()
}

fn run(assets: Vec<(String, Vec<u8>)>) -> String {
    let wasm = wat_to_wasm("assets.wat");
    let opts = wasmicon_host::Options {
        trace: true,
        assets,
        ..Default::default()
    };
    // The core uses no core::fmt, so Error has no Debug. Print the reason by hand.
    match wasmicon_host::run_wasm_opts(&wasm, opts) {
        Ok(out) => out.trace,
        Err(e) => panic!("execution failed: {} [{}]", e.reason(), e.kind().name()),
    }
}

#[test]
fn open_len_read_and_drop_trace_as_documented() {
    let body: Vec<u8> = (0..32).collect();
    let trace = run(vec![("a.bin".to_string(), body)]);
    println!("{trace}");
    assert_eq!(
        trace,
        "> wasmicon:hal/assets@0.1.0/[static]blob.open(\"a.bin\")\n\
         < 0 [1]\n\
         > wasmicon:hal/assets@0.1.0/[method]blob.len(1)\n\
         < 0 [32]\n\
         > wasmicon:hal/assets@0.1.0/[method]blob.read(1, 4, 8)\n\
         < 0 [len=8 data=0x0405060708090a0b]\n\
         > wasmicon:hal/assets@0.1.0/[static]blob.open(\"a.bi\")\n\
         < 9\n\
         > wasmicon:hal/assets@0.1.0/[resource-drop]blob(1)\n\
         <\n"
    );
}

#[test]
fn without_the_option_every_name_is_not_found() {
    let trace = run(Vec::new());
    println!("{trace}");
    // open fails with not-found (status 9), so the guest holds handle 0 and
    // the calls on it are invalid-handle (status 2). Nothing traps.
    assert!(
        trace.contains("[static]blob.open(\"a.bin\")\n< 9\n"),
        "{trace}"
    );
    assert!(trace.contains("[method]blob.len(0)\n< 2\n"), "{trace}");
    assert!(
        trace.contains("[method]blob.read(0, 4, 8)\n< 2\n"),
        "{trace}"
    );
    assert!(trace.ends_with("[resource-drop]blob(0)\n<\n"), "{trace}");
}
