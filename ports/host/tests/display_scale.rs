//! Runs the display guests on a scaled mock display (docs/TODO.md §6-10).
//!
//! A scale is configuration, not API: the guest of a panel twice the size at
//! `scale = 2` must trace exactly as on the unscaled panel, and every guest
//! pixel must cover `scale` x `scale` panel pixels, for `rgb565`
//! (`display.wat`) and `indexed8` (`display_indexed.wat`), for `fill-rect`
//! and `write-rect`.

mod common;

use common::wat_to_wasm;
use wasmicon_host::hal::Frame;

fn run(wat: &str, size: (u32, u32), scale: Option<u32>) -> (String, Frame) {
    let opts = wasmicon_host::Options {
        trace: true,
        display_size: Some(size),
        display_scale: scale,
        ..Default::default()
    };
    let out = wasmicon_host::run_wasm_opts(&wat_to_wasm(wat), opts)
        .unwrap_or_else(|e| panic!("execution failed: {} [{}]", e.reason(), e.kind().name()));
    (out.trace, out.frame.expect("the host has a display"))
}

/// Runs `wat` at 1x on `size` and at `n`x on `n` times `size`, and compares.
fn same_as_unscaled(wat: &str, size: (u32, u32), n: u32) {
    let (trace, one) = run(wat, size, None);
    let (scaled_trace, big) = run(wat, (size.0 * n, size.1 * n), Some(n));
    assert_eq!(
        scaled_trace, trace,
        "{wat} at {n}x traced differently from 1x"
    );
    let (w, n) = (one.width as usize, n as usize);
    for (i, &px) in big.pixels.iter().enumerate() {
        let (x, y) = (i % (w * n), i / (w * n));
        assert_eq!(
            px,
            one.pixels[(y / n) * w + x / n],
            "{wat} at {n}x: panel pixel ({x}, {y})"
        );
    }
}

#[test]
fn rgb565_at_2x_and_4x_draws_each_pixel_as_a_block() {
    same_as_unscaled("display.wat", (320, 240), 2);
    same_as_unscaled("display.wat", (320, 240), 4);
}

#[test]
fn indexed8_at_2x_and_4x_draws_each_pixel_as_a_block() {
    same_as_unscaled("display_indexed.wat", (320, 240), 2);
    same_as_unscaled("display_indexed.wat", (320, 240), 4);
}

#[test]
fn rows_wider_than_one_piece_draw_whole_at_2x() {
    // 720 guest pixels at 2x are 1,440 panel pixels: five pieces per row.
    same_as_unscaled("display_indexed.wat", (720, 1280), 2);
}
