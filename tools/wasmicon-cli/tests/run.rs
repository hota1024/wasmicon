//! `wasmicon run`'s usage report: which boards a run's stacks do not fit.

use wasmicon_cli::run::usage_report;
use wasmicon_host::{Options, Usage, run_wasm_capture};
use wasmicon_port::profile;

#[test]
fn a_run_past_the_mcu_call_depth_names_the_boards_it_does_not_fit() {
    // The call depth DOOM's title demos reach on the host.
    let u = Usage {
        max_control_depth: 10,
        max_value_stack: 21,
        max_locals: 80,
        max_call_depth: 47,
        operand_stack_slots: 306,
    };
    let lines = usage_report(
        &u,
        &[&profile::ESP32S3, &profile::ESP32S3_PSRAM, &profile::HOST],
    );
    assert_eq!(
        lines,
        [
            "usage: max_control_depth 10, max_value_stack 21, max_locals 80, \
             max_call_depth 47, operand_stack_slots 306",
            "  over esp32s3: max_call_depth 47 > 32",
        ]
    );
}

#[test]
fn a_shallow_run_fits_every_board() {
    let lines = usage_report(&Usage::default(), profile::PROFILES);
    assert_eq!(lines[1], "  within every board's limits");
    assert_eq!(lines.len(), 2);
}

#[test]
fn the_host_run_fills_in_the_run_time_fields() {
    // `(func (export "run") call 1) (func)` with a memory: two calls deep.
    let wasm: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // header
        0x01, 0x04, 0x01, 0x60, 0x00, 0x00, // type: () -> ()
        0x03, 0x03, 0x02, 0x00, 0x00, // two functions
        0x05, 0x03, 0x01, 0x00, 0x01, // memory 1
        0x07, 0x07, 0x01, 0x03, b'r', b'u', b'n', 0x00, 0x00, // export "run"
        0x0a, 0x09, 0x02, // code: two bodies
        0x04, 0x00, 0x10, 0x01, 0x0b, // call 1
        0x02, 0x00, 0x0b, // empty
    ];
    let opts = Options {
        trace: false,
        i2c_replay: Vec::new(),
        spi_unsupported: false,
        assets: Vec::new(),
        no_display: false,
        display_size: None,
    };
    let (out, result) = run_wasm_capture(wasm, opts);
    assert!(result.is_ok());
    let u = out.usage.expect("validated");
    assert_eq!(u.max_call_depth, 2);
    assert_eq!(u.max_control_depth, 1);
}
