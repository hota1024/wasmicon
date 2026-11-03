//! Runs a hand-written `indexed8` display guest (`display_indexed.wat`) on the
//! host port and checks its trace and the mock display's picture.
//!
//! Locks in `set-palette` (wit-device/display.wit): the length check (511 and
//! 513 bytes), the switch of `format`, an index as `fill-rect`'s color, one
//! byte per pixel in `write-rect`, and that a new open starts in `rgb565`. The picture must be
//! what the palette gives each index, at 320 columns and at the Tab5's 720,
//! which the default `Board::display_write_expanded` splits into pieces.

mod common;

use common::wat_to_wasm;

const EXPECTED: &str = "\
> wasmicon:device/display@0.1.0/[static]screen.open(0)
< 0 [1]
> wasmicon:device/display@0.1.0/[method]screen.width(1)
< WIDTH
> wasmicon:device/display@0.1.0/[method]screen.set-palette(1, 0x00ff01fe02fd03fc04fb05fa06f907f8..len=511 crc32=SHORT)
< 1
> wasmicon:device/display@0.1.0/[method]screen.set-palette(1, 0x00ff01fe02fd03fc04fb05fa06f907f8..len=513 crc32=LONG)
< 1
> wasmicon:device/display@0.1.0/[method]screen.format(1)
< 0
> wasmicon:device/display@0.1.0/[method]screen.set-palette(1, 0x00ff01fe02fd03fc04fb05fa06f907f8..len=512 crc32=PALETTE)
< 0
> wasmicon:device/display@0.1.0/[method]screen.format(1)
< 1
> wasmicon:device/display@0.1.0/[method]screen.fill-rect(1, 0, 0, WIDTH, 240, 3)
< 0
> wasmicon:device/display@0.1.0/[method]screen.fill-rect(1, 0, 0, 1, 1, 256)
< 1
> wasmicon:device/display@0.1.0/[method]screen.write-rect(1, 0, 10, WIDTH, 3, crc32=ROWS)
< 0
> wasmicon:device/display@0.1.0/[method]screen.write-rect(1, 0, 20, 3, 1, crc32=FOUR)
< 1
> wasmicon:device/display@0.1.0/[method]screen.write-rect(1, 0, 20, 3, 1, crc32=SIX)
< 1
> wasmicon:device/display@0.1.0/[method]screen.write-rect(1, 5, 30, 7, 2, crc32=ODD)
< 0
> wasmicon:device/display@0.1.0/[method]screen.present(1)
< 0
> wasmicon:device/display@0.1.0/[resource-drop]screen(1)
<
> wasmicon:device/display@0.1.0/[static]screen.open(0)
< 0 [1]
> wasmicon:device/display@0.1.0/[method]screen.format(1)
< 0
> wasmicon:device/display@0.1.0/[resource-drop]screen(1)
<
";

/// The guest's indices: byte `i` is `i * 7 mod 256`.
fn index(i: usize) -> u8 {
    (i * 7) as u8
}

/// The guest's palette: entry `k` is `k << 8 | (255 - k)`.
fn color(k: u8) -> u16 {
    u16::from(k) << 8 | u16::from(255 - k)
}

fn crc(bytes: impl IntoIterator<Item = u8>) -> String {
    format!(
        "{:08x}",
        wasmicon_port::crc32(&bytes.into_iter().collect::<Vec<_>>())
    )
}

fn run_at(width: u32, height: u32) {
    let wasm = wat_to_wasm("display_indexed.wat");
    let opts = wasmicon_host::Options {
        trace: true,
        display_size: Some((width, height)),
        ..Default::default()
    };
    let out = wasmicon_host::run_wasm_opts(&wasm, opts)
        .unwrap_or_else(|e| panic!("execution failed: {} [{}]", e.reason(), e.kind().name()));

    let w = width as usize;
    let palette: Vec<u8> = (0..=255u8).flat_map(|k| color(k).to_be_bytes()).collect();
    let expected = EXPECTED
        .replace("WIDTH", &width.to_string())
        .replace("SHORT", &crc(palette[..511].iter().copied()))
        // The byte after the palette in the guest's memory is 0.
        .replace("LONG", &crc(palette.iter().copied().chain([0])))
        .replace("PALETTE", &crc(palette.iter().copied()))
        .replace("ROWS", &crc((0..3 * w).map(index)))
        .replace("FOUR", &crc((0..4).map(index)))
        .replace("SIX", &crc((0..6).map(index)))
        .replace("ODD", &crc((0..14).map(index)));
    assert_eq!(out.trace, expected);

    let frame = out.frame.expect("the host has a display");
    let at = |x: usize, y: usize| frame.pixels[y * w + x];
    // The fill took index 3; the rejected calls drew nothing.
    assert_eq!(at(0, 0), color(3));
    assert_eq!(at(0, 20), color(3));
    assert_eq!(at(w - 1, 239), color(3));
    if height > 240 {
        assert_eq!(at(0, 240), 0, "the fill stopped at row 240");
    }
    for y in 0..3 {
        for x in 0..w {
            assert_eq!(
                at(x, 10 + y),
                color(index(y * w + x)),
                "row pixel ({x}, {})",
                10 + y
            );
        }
    }
    for y in 0..2 {
        for x in 0..7 {
            assert_eq!(
                at(5 + x, 30 + y),
                color(index(y * 7 + x)),
                "odd pixel ({}, {})",
                5 + x,
                30 + y
            );
        }
    }
    assert_eq!(at(4, 30), color(3));
    assert_eq!(at(12, 31), color(3));
}

#[test]
fn indexed_calls_trace_and_draw_as_documented_at_320_columns() {
    run_at(320, 240);
}

#[test]
fn indexed_rows_wider_than_one_piece_draw_whole_at_720_columns() {
    run_at(720, 1280);
}
