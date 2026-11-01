//! Locks in that a port without `wasmicon:device/touch` refuses its imports at
//! instantiate.
//!
//! No port implements `touch` yet (docs/TODO.md §7), the host included, so an
//! app that imports it must fail with a link error before `run`
//! (docs/device-layer.md §6, docs/decisions.md P-13).

mod common;

use common::wat_to_wasm;

#[test]
fn touch_import_is_a_link_error() {
    let wasm = wat_to_wasm("touch_unlinked.wat");
    let opts = wasmicon_host::Options {
        trace: true,
        ..Default::default()
    };
    let (out, result) = wasmicon_host::run_wasm_capture(&wasm, opts);
    let Err(e) = result else {
        panic!("a touch import linked on a port without touch");
    };
    assert_eq!(e.kind().name(), "unlinkable", "{}", e.reason());
    assert!(out.trace.is_empty(), "the guest ran: {}", out.trace);
}
