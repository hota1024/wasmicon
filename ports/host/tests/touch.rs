//! `wasmicon:device/touch@0.1.0` on the host's mock touch panel
//! (docs/device-layer.md §11, docs/abi-spec.md §7).
//!
//! The mock replays a script, one entry per read, so the trace is exact.

mod common;

use common::wat_to_wasm;

#[test]
fn the_mock_replays_its_script_and_checks_the_arguments() {
    let wasm = wat_to_wasm("touch.wat");
    let script = wasmicon_host::parse_touch_script("10,20 300,200 0,239\n-\n").unwrap();
    let opts = wasmicon_host::Options {
        trace: true,
        touch_script: script,
        ..Default::default()
    };
    let out = match wasmicon_host::run_wasm_opts(&wasm, opts) {
        Ok(out) => out,
        Err(e) => panic!("execution failed: {} [{}]", e.reason(), e.kind().name()),
    };
    let m = "wasmicon:device/touch@0.1.0";
    let expected = [
        format!("> {m}/[static]panel.open(1)"),
        "< 7".to_string(),
        format!("> {m}/[static]panel.open(0)"),
        "< 0 [1]".to_string(),
        format!("> {m}/[static]panel.open(0)"),
        "< 3".to_string(),
        format!("> {m}/[method]panel.read(1, 0)"),
        "< 1".to_string(),
        format!("> {m}/[method]panel.read(1, 3)"),
        "< 1".to_string(),
        // Two of the three points: x, y as u16 high byte first.
        format!("> {m}/[method]panel.read(1, 2)"),
        "< 0 [len=8 data=0x000a0014012c00c8]".to_string(),
        format!("> {m}/[method]panel.read(1, 2)"),
        "< 0 [len=0 data=0x]".to_string(),
        format!("> {m}/[method]panel.read(1, 2)"),
        "< 0 [len=0 data=0x]".to_string(),
        format!("> {m}/[method]panel.read(9, 1)"),
        "< 2".to_string(),
        format!("> {m}/[resource-drop]panel(1)"),
        "<".to_string(),
        format!("> {m}/[method]panel.read(1, 1)"),
        "< 2".to_string(),
    ];
    let lines: Vec<&str> = out.trace.lines().collect();
    assert_eq!(lines, expected, "trace:\n{}", out.trace);
}

#[test]
fn points_are_in_guest_pixels_on_a_scaled_display() {
    // The script is in guest pixels, as a board's trace records them, so a
    // 2x board's trace replays as is (docs/device-layer.md §11.5). The mock
    // reports panel pixels and `Hal` divides them back.
    let wasm = wat_to_wasm("touch.wat");
    let script = wasmicon_host::parse_touch_script("5,10 150,100 0,119\n").unwrap();
    let opts = wasmicon_host::Options {
        trace: true,
        display_scale: Some(2),
        touch_script: script,
        ..Default::default()
    };
    let out = wasmicon_host::run_wasm_opts(&wasm, opts)
        .unwrap_or_else(|e| panic!("execution failed: {} [{}]", e.reason(), e.kind().name()));
    assert!(
        out.trace.contains("< 0 [len=8 data=0x0005000a00960064]\n"),
        "trace:\n{}",
        out.trace
    );
}

#[test]
fn script_lines_parse_and_bad_ones_name_their_line() {
    let s = wasmicon_host::parse_touch_script("# comment\n\n-\n1,2\n3,4 5,6\n").unwrap();
    assert_eq!(s, vec![vec![], vec![(1, 2)], vec![(3, 4), (5, 6)]]);
    assert_eq!(
        wasmicon_host::parse_touch_script("-\n1;2\n").unwrap_err().0,
        2
    );
    assert_eq!(
        wasmicon_host::parse_touch_script("70000,1\n")
            .unwrap_err()
            .0,
        1
    );
    let nine = ["1,1"; 9].join(" ");
    assert_eq!(wasmicon_host::parse_touch_script(&nine).unwrap_err().0, 1);
}

#[test]
fn a_point_outside_the_display_is_found() {
    let s = wasmicon_host::parse_touch_script("-\n319,239\n320,0\n").unwrap();
    assert_eq!(
        wasmicon_host::touch_outside(&s, (320, 240)),
        Some((2, 320, 0))
    );
    assert_eq!(wasmicon_host::touch_outside(&s, (321, 240)), None);
}

/// A port returns at most 8 points whatever `max-points` asks for, so the
/// trace carries every byte and never folds a read into a CRC
/// (docs/abi-spec.md §7, §9).
#[test]
fn a_read_returns_at_most_eight_points_in_full() {
    let wasm = wat_to_wasm("touch_many.wat");
    let line = (0..8)
        .map(|i| format!("{i},{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let opts = wasmicon_host::Options {
        trace: true,
        touch_script: wasmicon_host::parse_touch_script(&line).unwrap(),
        ..Default::default()
    };
    let out = match wasmicon_host::run_wasm_opts(&wasm, opts) {
        Ok(out) => out,
        Err(e) => panic!("execution failed: {} [{}]", e.reason(), e.kind().name()),
    };
    let data: String = (0..8u16).map(|i| format!("{i:04x}{i:04x}")).collect();
    let expected = format!("< 0 [len=32 data=0x{data}]");
    assert!(
        out.trace.lines().any(|l| l == expected),
        "trace:\n{}",
        out.trace
    );
}

/// `[buf, buf+cap)` outside memory traps whether or not a finger is down,
/// though a read with no touch writes nothing (docs/abi-spec.md §4.4).
#[test]
fn a_buffer_past_memory_traps_without_a_touch_too() {
    let wasm = wat_to_wasm("touch_oob.wat");
    for script in ["-\n", "1,2\n"] {
        let opts = wasmicon_host::Options {
            touch_script: wasmicon_host::parse_touch_script(script).unwrap(),
            ..Default::default()
        };
        match wasmicon_host::run_wasm_opts(&wasm, opts) {
            Ok(_) => panic!("{script:?}: ran to the end"),
            Err(e) => assert_eq!(e.kind().name(), "trap", "{script:?}: {}", e.reason()),
        }
    }
}
