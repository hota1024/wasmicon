//! Locks in that a port without a device interface refuses its imports at instantiate.
//!
//! An app that imports `wasmicon:device/display@0.1.0` must fail with a link
//! error before `run`, not run half-way and get `unsupported` from every call
//! (docs/device-layer.md §6, docs/decisions.md P-13).

mod common;

use common::wat_to_wasm;

#[test]
fn display_import_is_a_link_error() {
    let wasm = wat_to_wasm("display_unlinked.wat");
    let opts = wasmicon_host::Options {
        trace: true,
        ..Default::default()
    };
    let (out, result) = wasmicon_host::run_wasm_capture(&wasm, opts);
    let Err(e) = result else {
        panic!("a display import linked on a port without a display");
    };
    assert_eq!(e.kind().name(), "unlinkable", "{}", e.reason());
    assert!(out.trace.is_empty(), "the guest ran: {}", out.trace);
}
