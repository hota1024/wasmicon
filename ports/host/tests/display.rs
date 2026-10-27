//! Runs a hand-written display guest (`display.wat`) on the host port and checks
//! its trace and the mock display's picture.
//!
//! Locks in the error paths of `wasmicon:device/display@0.1.0`
//! (wit-device/display.wit), the trace rules of docs/abi-spec.md §9 for it, the
//! byte order of `write-rect` (docs/device-layer.md §4) and that only `present`
//! makes a drawing visible (docs/device-layer.md §5.3).

mod common;

use common::wat_to_wasm;

const EXPECTED: &str = "\
> wasmicon:device/display@0.1.0/[static]screen.open(0)
< 0 [1]
> wasmicon:device/display@0.1.0/[method]screen.width(1)
< 320
> wasmicon:device/display@0.1.0/[method]screen.height(1)
< 240
> wasmicon:device/display@0.1.0/[method]screen.format(1)
< 0
> wasmicon:device/display@0.1.0/[method]screen.fill-rect(1, 0, 0, 320, 240, 65535)
< 0
> wasmicon:device/display@0.1.0/[method]screen.fill-rect(1, 318, 238, 2, 2, 31)
< 0
> wasmicon:device/display@0.1.0/[method]screen.fill-rect(1, 319, 0, 2, 1, 0)
< 1
> wasmicon:device/display@0.1.0/[method]screen.fill-rect(1, 0, 0, 0, 1, 0)
< 1
> wasmicon:device/display@0.1.0/[method]screen.fill-rect(1, 0, 0, 1, 1, 65536)
< 1
> wasmicon:device/display@0.1.0/[method]screen.write-rect(1, 0, 0, 2, 2, crc32=CRC)
< 0
> wasmicon:device/display@0.1.0/[method]screen.write-rect(1, 0, 0, 3, 1, crc32=CRC)
< 1
> wasmicon:device/display@0.1.0/[method]screen.present(1)
< 0
> wasmicon:device/display@0.1.0/[method]screen.fill-rect(1, 0, 0, 1, 1, 0)
< 0
> wasmicon:device/display@0.1.0/[static]screen.open(0)
< 3
> wasmicon:device/display@0.1.0/[static]screen.open(1)
< 7
> wasmicon:device/display@0.1.0/[method]screen.width(7)
< 0
> wasmicon:device/display@0.1.0/[method]screen.present(7)
< 2
> wasmicon:device/display@0.1.0/[resource-drop]screen(1)
<
> wasmicon:device/display@0.1.0/[method]screen.height(1)
< 0
";

#[test]
fn display_calls_trace_and_draw_as_documented() {
    let wasm = wat_to_wasm("display.wat");
    let opts = wasmicon_host::Options {
        trace: true,
        ..Default::default()
    };
    let out = match wasmicon_host::run_wasm_opts(&wasm, opts) {
        Ok(out) => out,
        Err(e) => panic!("execution failed: {} [{}]", e.reason(), e.kind().name()),
    };
    let crc = format!(
        "{:08x}",
        wasmicon_port::crc32(&[0xf8, 0x00, 0x07, 0xe0, 0x00, 0x1f, 0xff, 0xff])
    );
    assert_eq!(out.trace, EXPECTED.replace("CRC", &crc));

    let frame = out.frame.expect("the host has a display");
    let at = |x: usize, y: usize| frame.pixels[y * frame.width as usize + x];
    // write-rect: high byte first, row by row. (0, 0) is still red although
    // a black fill came after the last present: only present makes it visible.
    assert_eq!(
        [at(0, 0), at(1, 0), at(0, 1), at(1, 1)],
        [0xf800, 0x07e0, 0x001f, 0xffff]
    );
    // The failed calls drew nothing; the corner square reaches the last pixel.
    assert_eq!(at(2, 0), 0xffff);
    assert_eq!(at(319, 0), 0xffff);
    assert_eq!(at(318, 238), 0x001f);
    assert_eq!(at(319, 239), 0x001f);
}
