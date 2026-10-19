//! Demo that only draws on an ILI9341. No sensor needed.
//!
//! `sensor-display-rs` reads an SHT4x over I2C, so it cannot drive the
//! display without the sensor attached (it returns as soon as I2C fails to
//! open). This demo uses only SPI and GPIO, so one LCD is enough.
//!
//! Each element is chosen to expose a different way of failing:
//!
//! - Color bars: color order (the BGR bit of MADCTL). If red and blue are
//!   swapped, this is where it shows.
//! - A 1 px frame around the screen: the edge of the address window. Checks
//!   that `fill_rect` handles an exact fit (x+w == 320, y+h == 240).
//! - A gray ramp: RGB565 packing. Tinted steps mean a bit position is off.
//! - Text: the `draw_text` path.
//! - A moving square: `time.sleep-ms` and repeated drawing.
//!
//! Pin numbers are not hard-coded; they come from `board.pin-by-role`, so the
//! same `.wasm` runs on any board that wires `lcd-cs` / `lcd-dc` / `lcd-rst`
//! (docs/abi-spec.md §8). The same `.wasm` is deployed to the app slot on
//! every board (docs/app-workflow.md §3.3).
//!
//! The on-screen `RP2350` / `HELLO PICO 2` stay the same on every board.
//! The point of this demo is to confirm that one binary produces the same
//! host-call sequence on two boards; per-board text would leave nothing to
//! compare. Changing the text also invalidates the traces recorded in
//! docs/verification-report.md §6. Wiring: `apps/lcd-demo-rs/README.md`.

#![no_std]

mod font;
mod ili9341;

use wasmicon_hal::gpio::{Pin, PinMode};
use wasmicon_hal::spi::{Bus as SpiBus, Mode};
use wasmicon_hal::{board, log, time};

use ili9341::{Display, HEIGHT, WIDTH};

/// Background.
const BLACK: u16 = 0x0000;
/// Text and frame.
const WHITE: u16 = 0xFFFF;
/// Header band.
const BAND: u16 = 0x001F;

/// Color bars, left to right: red, orange, yellow, green, cyan, blue,
/// magenta, white. If red and blue come out swapped, suspect the BGR bit of
/// MADCTL (`0x36`).
const BARS: [u16; 8] = [
    0xF800, 0xFD20, 0xFFE0, 0x07E0, 0x07FF, 0x001F, 0xF81F, 0xFFFF,
];

/// SPI clock.
///
/// Lower than the 24 MHz of apps/README.md §2. This app does not take part in
/// the two-implementation cross-check, so it takes the value that is less
/// likely to fail during bring-up. RP2350 (clk_peri 150 MHz) rounds it to
/// 15 MHz.
const SPI_HZ: u32 = 16_000_000;

/// Frames per direction of the moving square (a round trip is twice this).
const FRAMES: u16 = 16;

/// Entry point. The host calls it once, right after instantiation
/// (docs/abi-spec.md §3.3).
///
/// The handles are declared in the order `cs`, `dc`, `rst`, `spi`. `Drop`
/// runs in reverse declaration order, so every exit path releases them as
/// `spi`, `rst`, `dc`, `cs` (the same order as apps/README.md §4).
#[unsafe(no_mangle)]
pub extern "C" fn run() {
    log::info("lcd-demo start");

    let (Ok(cs_i), Ok(dc_i), Ok(rst_i)) = (
        board::pin_by_role("lcd-cs"),
        board::pin_by_role("lcd-dc"),
        board::pin_by_role("lcd-rst"),
    ) else {
        log::error("role not found");
        return;
    };
    let Ok(cs) = Pin::open(cs_i, PinMode::Output) else {
        log::error("gpio open failed");
        return;
    };
    let Ok(dc) = Pin::open(dc_i, PinMode::Output) else {
        log::error("gpio open failed");
        return;
    };
    let Ok(rst) = Pin::open(rst_i, PinMode::Output) else {
        log::error("gpio open failed");
        return;
    };
    let Ok(spi) = SpiBus::open(0, SPI_HZ, Mode::Mode0) else {
        log::error("spi open failed");
        return;
    };

    let display = Display::new(&spi, &cs, &dc);
    if display.init(&rst).is_err() || draw(&display).is_err() {
        log::error("display failed");
        return;
    }
    log::info("lcd-demo done");
}

/// Draws the screen. Stops at the first failure and returns it.
fn draw(d: &Display<'_>) -> Result<(), wasmicon_hal::ErrorCode> {
    d.fill_rect(0, 0, WIDTH, HEIGHT, BLACK)?;

    // Header band.
    d.fill_rect(0, 0, WIDTH, 32, BAND)?;
    d.draw_text(8, 12, b"WASMICON", WHITE, BAND)?;
    d.draw_text(232, 12, b"RP2350", WHITE, BAND)?;

    // Color bars: 8 x 40 px is exactly 320 px.
    let bar_w = WIDTH / BARS.len() as u16;
    let mut i = 0u16;
    while (i as usize) < BARS.len() {
        d.fill_rect(i * bar_w, 40, bar_w, 56, BARS[i as usize])?;
        i += 1;
    }
    d.draw_text(8, 104, b"RGB565 BARS", WHITE, BLACK)?;

    // Gray ramp: 16 steps x 20 px. R and B have 5 bits, G has 6, so each
    // channel is scaled separately to get the same brightness.
    let step_w = WIDTH / 16;
    let mut s = 0u16;
    while s < 16 {
        let r = (s * 31) / 15;
        let g = (s * 63) / 15;
        let b = (s * 31) / 15;
        d.fill_rect(s * step_w, 120, step_w, 32, (r << 11) | (g << 5) | b)?;
        s += 1;
    }
    d.draw_text(8, 160, b"GRAY RAMP", WHITE, BLACK)?;

    d.draw_text(8, 208, b"HELLO PICO 2", WHITE, BLACK)?;
    d.draw_text(8, 224, b"ILI9341 SPI", WHITE, BLACK)?;

    // Frame: four rectangles that fit the screen exactly. `fill_rect` fails
    // on `x + w > 320`, so if these pass, the edge handling is right.
    d.fill_rect(0, 0, WIDTH, 1, WHITE)?;
    d.fill_rect(0, HEIGHT - 1, WIDTH, 1, WHITE)?;
    d.fill_rect(0, 0, 1, HEIGHT, WHITE)?;
    d.fill_rect(WIDTH - 1, 0, 1, HEIGHT, WHITE)?;

    sweep(d)
}

/// Moves a 16 px square right and back once.
///
/// Each frame erases the previous position with the background color and
/// then paints the next one. There is no full-screen frame buffer, so these
/// two rectangles are all that moves.
fn sweep(d: &Display<'_>) -> Result<(), wasmicon_hal::ErrorCode> {
    /// Side of the square.
    const SIZE: u16 = 16;
    /// Top of the track: the gap between the color bars and the text.
    const TRACK_Y: u16 = 176;
    /// Left end and the step per frame. The right end is
    /// `8 + 16 * 18 + 16 = 312`, clear of the frame.
    const X0: u16 = 8;
    const STEP: u16 = 18;
    /// Square color (green).
    const BOX: u16 = 0x07E0;

    let mut prev = X0;
    d.fill_rect(prev, TRACK_Y, SIZE, SIZE, BOX)?;

    let mut n = 0u16;
    while n < FRAMES * 2 {
        // Right for the first half, left for the second; ends where it
        // started.
        let i = if n < FRAMES {
            n + 1
        } else {
            FRAMES * 2 - n - 1
        };
        let x = X0 + i * STEP;

        time::sleep_ms(40);
        d.fill_rect(prev, TRACK_Y, SIZE, SIZE, BLACK)?;
        d.fill_rect(x, TRACK_Y, SIZE, SIZE, BOX)?;
        prev = x;
        n += 1;
    }
    Ok(())
}

/// Required because the crate is `no_std` (docs/decisions.md A-04).
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
