//! Helpers shared by the hand-written `.wat` tests.

use std::path::PathBuf;
use std::process::Command;

/// Assembles `tests/<name>` with `wasm-tools` into `target/<name>.wasm` and
/// returns the bytes.
///
/// The tests of one file run on several threads and may ask for the same
/// `.wat`; the parse and the read are serialized so that no test reads a
/// half-written file.
pub fn wat_to_wasm(name: &str) -> Vec<u8> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

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
