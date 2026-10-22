//! Runs a hand-written blink guest (`blink.wat`) on the host port and checks its trace.
//!
//! This is the smallest end-to-end test of the port: it needs no app build,
//! only `wasm-tools`, and locks in that a guest links against the mock HAL,
//! that GPIO host calls produce the docs/abi-spec.md §9 trace lines and that
//! `time` stays out of the trace. The real apps are covered by `tests/apps.rs`.

mod common;

use std::path::PathBuf;
use std::process::Command;

use common::wat_to_wasm;

#[test]
fn blink_runs_and_traces() {
    let wasm = wat_to_wasm("blink.wat");

    // Check that it stays within the supported feature set (the same check as
    // tests/apps.rs).
    let ok = Command::new("wasm-tools")
        .args([
            "validate",
            "--features=mvp,sign-extension,saturating-float-to-int,bulk-memory,multi-value,mutable-global",
        ])
        .arg(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/blink.wat.wasm"),
        )
        .status()
        .expect("cannot run wasm-tools");
    assert!(
        ok.success(),
        "contains an instruction outside the supported feature set"
    );

    // The core uses no core::fmt, so Error has no Debug. Print the reason by hand.
    let trace = match wasmicon_host::run_wasm(&wasm, true) {
        Ok(out) => out.trace,
        Err(e) => panic!("execution failed: {} [{}]", e.reason(), e.kind().name()),
    };
    println!("{trace}");

    // Opening one pin yields handle 1.
    assert!(
        trace.contains("> wasmicon:hal/gpio@0.1.0/[static]pin.open(2, 3)\n< 0 [1]"),
        "wrong pin.open trace:\n{trace}"
    );
    // time is not traced (docs/abi-spec.md §9).
    assert!(!trace.contains("time@0.1.0"), "time appears in the trace");
    // Blinking is 3 writes and 3 toggles.
    assert_eq!(
        trace.matches("[method]pin.write(1, 1)").count(),
        3,
        "wrong pin.write count:\n{trace}"
    );
    assert_eq!(
        trace.matches("[method]pin.toggle(1)").count(),
        3,
        "wrong pin.toggle count:\n{trace}"
    );
    // drop at the end.
    assert!(
        trace.contains("[resource-drop]pin(1)"),
        "missing drop trace:\n{trace}"
    );
    // Every host call succeeded.
    assert!(!trace.contains("\n< 1"), "a host call failed:\n{trace}");
}

/// docs/abi-spec.md §9 makes one host call two lines (request and result). A
/// guest that logs a newline or `"` must not break that shape, or the diff
/// between the two boards shifts by whole lines.
#[test]
fn trace_escapes_guest_strings() {
    let wasm = wat_to_wasm("logesc.wat");
    let trace = match wasmicon_host::run_wasm(&wasm, true) {
        Ok(out) => out.trace,
        Err(e) => panic!("execution failed: {} [{}]", e.reason(), e.kind().name()),
    };
    println!("{trace}");

    let lines: Vec<&str> = trace.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "one host call should be two lines:\n{trace}"
    );
    assert_eq!(
        lines[0], r#"> wasmicon:hal/log@0.1.0/log(2, "a\nb\"c")"#,
        "not escaped"
    );
    assert_eq!(lines[1], "<");
}
