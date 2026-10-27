//! Locks in that a port without a device interface refuses its imports at instantiate.
//!
//! An app that imports `wasmicon:device/display@0.1.0` must fail with a link
//! error before `run` on a firmware without a display, not run half-way and
//! get `unsupported` from every call (docs/device-layer.md §6,
//! docs/decisions.md P-13). The host has the mock display, so
//! `Options::no_display` stands in for such a firmware.

mod common;

use common::wat_to_wasm;

#[test]
fn display_import_is_a_link_error_without_a_display() {
    let wasm = wat_to_wasm("display_unlinked.wat");
    let opts = wasmicon_host::Options {
        trace: true,
        no_display: true,
        ..Default::default()
    };
    let (out, result) = wasmicon_host::run_wasm_capture(&wasm, opts);
    let Err(e) = result else {
        panic!("a display import linked on a port without a display");
    };
    assert_eq!(e.kind().name(), "unlinkable", "{}", e.reason());
    assert!(out.trace.is_empty(), "the guest ran: {}", out.trace);
}

#[test]
fn display_import_links_on_the_host() {
    let wasm = wat_to_wasm("display_unlinked.wat");
    let opts = wasmicon_host::Options {
        trace: true,
        ..Default::default()
    };
    match wasmicon_host::run_wasm_opts(&wasm, opts) {
        Ok(out) => assert_eq!(
            out.trace,
            "> wasmicon:device/display@0.1.0/[static]screen.open(0)\n< 0 [1]\n"
        ),
        Err(e) => panic!("execution failed: {} [{}]", e.reason(), e.kind().name()),
    }
}
