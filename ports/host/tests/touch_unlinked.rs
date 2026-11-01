//! Locks in that a port without `wasmicon:device/touch` refuses its imports at
//! instantiate.
//!
//! An app that imports `wasmicon:device/touch@0.1.0` must fail with a link
//! error before `run` on a firmware without touch, not run half-way and get
//! `unsupported` from every call (docs/device-layer.md §6,
//! docs/decisions.md P-13). The host has the mock touch panel, so
//! `Options::no_touch` stands in for such a firmware.

mod common;

use common::wat_to_wasm;

#[test]
fn touch_import_is_a_link_error_without_touch() {
    let wasm = wat_to_wasm("touch_unlinked.wat");
    let opts = wasmicon_host::Options {
        trace: true,
        no_touch: true,
        ..Default::default()
    };
    let (out, result) = wasmicon_host::run_wasm_capture(&wasm, opts);
    let Err(e) = result else {
        panic!("a touch import linked on a port without touch");
    };
    assert_eq!(e.kind().name(), "unlinkable", "{}", e.reason());
    assert!(out.trace.is_empty(), "the guest ran: {}", out.trace);
}

#[test]
fn touch_import_links_on_the_host() {
    let wasm = wat_to_wasm("touch_unlinked.wat");
    let opts = wasmicon_host::Options {
        trace: true,
        ..Default::default()
    };
    match wasmicon_host::run_wasm_opts(&wasm, opts) {
        Ok(out) => assert_eq!(
            out.trace,
            "> wasmicon:device/touch@0.1.0/[static]panel.open(0)\n< 0 [1]\n"
        ),
        Err(e) => panic!("execution failed: {} [{}]", e.reason(), e.kind().name()),
    }
}
