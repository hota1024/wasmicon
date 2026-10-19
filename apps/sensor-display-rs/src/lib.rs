//! Reads an SHT4x (SHT40) and shows the values on an ILI9341.
//!
//! apps/README.md is normative for the drawing. This app must produce the
//! same host-call sequence and the same pixels as `sensor-display-as`.
//!
//! Pin numbers are not hard-coded; they come from `board.pin-by-role`, so the
//! same `.wasm` runs on both ESP32-S3 and RP2040 (docs/abi-spec.md §8).

#![no_std]

mod font;
mod ili9341;
mod sht4x;

use wasmicon_hal::gpio::{Pin, PinMode};
use wasmicon_hal::i2c::{Bus as I2cBus, Speed};
use wasmicon_hal::spi::{Bus as SpiBus, Mode};
use wasmicon_hal::{ErrorCode, board, log};

use ili9341::Display;

/// Background.
const BG: u16 = 0x0841;
/// Header band.
const HEADER: u16 = 0x1A3F;
/// Card background.
const CARD: u16 = 0x2104;
/// Text of the values and the header.
const FG: u16 = 0xFFFF;
/// Labels and tick marks.
const MUTED: u16 = 0xA514;
/// Accent of the temperature card.
const TEMP_ACCENT: u16 = 0xFC00;
/// Accent of the humidity card.
const HUM_ACCENT: u16 = 0x07FF;
/// Gauge track.
const TRACK: u16 = 0x4208;

/// Card positions and size (screen layout in apps/README.md §2).
const TEMP_X: u16 = 4;
const HUM_X: u16 = 162;
const CARD_Y: u16 = 36;
const CARD_W: u16 = 154;
const CARD_H: u16 = 116;
/// Scale and y of the value text.
const VALUE_SCALE: u16 = 3;
const VALUE_Y: u16 = 84;

/// Gauge. `bar_px` (0..=308) is the fill width in pixels.
const GAUGE_X: u16 = 6;
const GAUGE_Y: u16 = 178;
const GAUGE_H: u16 = 16;
/// Color zones of the gauge (end on the `bar_px` axis, color):
/// -45..10 / ..30 / ..50 / ..175 °C.
const ZONES: [(u16, u16); 4] = [(77, 0x041F), (105, 0x07E0), (133, 0xFFE0), (308, 0xF800)];
/// Tick marks (positions on the `bar_px` axis): 0 / 25 / 50 / 100 °C.
const TICKS: [u16; 4] = [63, 98, 133, 203];

/// SPI clock.
const SPI_HZ: u32 = 24_000_000;

/// Entry point. The host calls it once, right after instantiation
/// (docs/abi-spec.md §3.3).
///
/// All handles live in one scope, declared in the order `cs`, `dc`, `rst`,
/// `spi`, `i2c`. `Drop` runs in reverse declaration order, so every exit path
/// releases them in the order apps/README.md §4 requires (i2c, spi, rst, dc,
/// cs).
#[unsafe(no_mangle)]
pub extern "C" fn run() {
    log::info("sensor-display start");

    let (Ok(cs_i), Ok(dc_i), Ok(rst_i)) = (
        board::pin_by_role("lcd-cs"),
        board::pin_by_role("lcd-dc"),
        board::pin_by_role("lcd-rst"),
    ) else {
        log::error("role not found");
        return;
    };
    // Open the pins one at a time. Opening them in one tuple would call all
    // three even after a failure, and the host-call sequence would differ
    // from the AssemblyScript version.
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
    let Ok(i2c) = I2cBus::open(0, Speed::Standard) else {
        log::error("i2c open failed");
        return;
    };

    let display = Display::new(&spi, &cs, &dc);
    if display.init(&rst).is_err() || draw_frame(&display).is_err() {
        log::error("display failed");
        return;
    }

    let reading = match sht4x::read(&i2c) {
        Ok(r) => r,
        Err(sht4x::Error::Crc) => {
            log::error("sensor crc failed");
            return;
        }
        Err(sht4x::Error::Io) => {
            log::error("sensor read failed");
            return;
        }
    };

    let temp = sht4x::temp_centi(reading.raw_t);
    let humidity = sht4x::humidity_centi(reading.raw_h);
    if draw_values(&display, temp, humidity).is_err() {
        log::error("display failed");
    }
}

/// The part drawn before the sensor is read: header, cards, gauge track and
/// tick marks.
fn draw_frame(d: &Display) -> Result<(), ErrorCode> {
    d.fill_rect(0, 0, ili9341::WIDTH, ili9341::HEIGHT, BG)?;

    d.fill_rect(0, 0, ili9341::WIDTH, 28, HEADER)?;
    d.draw_text(8, 6, b"WASMICON", 2, FG, HEADER)?;
    d.draw_text(272, 10, b"SHT40", 1, FG, HEADER)?;

    draw_card(d, TEMP_X, b"TEMP", b"C", TEMP_ACCENT)?;
    draw_card(d, HUM_X, b"HUMIDITY", b"%", HUM_ACCENT)?;

    d.fill_rect(4, 162, 312, 72, CARD)?;
    d.fill_rect(GAUGE_X, GAUGE_Y, 308, GAUGE_H, TRACK)?;
    for t in TICKS {
        d.fill_rect(GAUGE_X + t, 196, 1, 5, MUTED)?;
    }
    d.draw_text(6, 206, b"-45", 1, MUTED, CARD)?;
    d.draw_text(65, 206, b"0", 1, MUTED, CARD)?;
    d.draw_text(131, 206, b"50", 1, MUTED, CARD)?;
    d.draw_text(197, 206, b"100", 1, MUTED, CARD)?;
    d.draw_text(282, 206, b"175C", 1, MUTED, CARD)
}

/// One card: background, accent strip at the top, label, unit.
fn draw_card(d: &Display, x: u16, label: &[u8], unit: &[u8], accent: u16) -> Result<(), ErrorCode> {
    d.fill_rect(x, CARD_Y, CARD_W, CARD_H, CARD)?;
    d.fill_rect(x, CARD_Y, CARD_W, 4, accent)?;
    d.draw_text(x + 8, 48, label, 1, MUTED, CARD)?;
    d.draw_text(x + CARD_W - 24, 46, unit, 2, accent, CARD)
}

/// The values read: the numbers on the cards and the gauge fill.
fn draw_values(d: &Display, temp: i32, humidity: i32) -> Result<(), ErrorCode> {
    let mut buf = [0u8; ili9341::MAX_TEXT];
    let n = format_value(temp, &mut buf);
    draw_value(d, TEMP_X, &buf[..n])?;
    let n = format_value(humidity, &mut buf);
    draw_value(d, HUM_X, &buf[..n])?;

    // For each zone, fill only its overlap with [0, bar).
    let bar = bar_px(temp) as u16;
    let mut start = 0;
    for (end, color) in ZONES {
        if bar > start {
            let stop = if bar < end { bar } else { end };
            d.fill_rect(GAUGE_X + start, GAUGE_Y, stop - start, GAUGE_H, color)?;
        }
        start = end;
    }
    Ok(())
}

/// Draws a value centered horizontally in its card.
fn draw_value(d: &Display, card_x: u16, text: &[u8]) -> Result<(), ErrorCode> {
    let w = text.len() as u16 * 8 * VALUE_SCALE;
    let x = card_x + (CARD_W - w) / 2;
    d.draw_text(x, VALUE_Y, text, VALUE_SCALE, FG, CARD)
}

/// Length of the temperature bar. The only place in this app that uses f32
/// (apps/README.md §1).
///
/// The clamp is done in f32. Narrowing an out-of-range value to `i32` would
/// behave differently in Rust (saturating conversion) and AssemblyScript
/// (trapping conversion).
///
/// clippy suggests `f32::clamp`, but this must compile to the same
/// instruction sequence as the AssemblyScript version. `clamp` may lower to
/// `f32.min` / `f32.max`, which differ from the compare-and-branch on the AS
/// side. This is a determinism test case, so the shape is kept identical.
#[allow(clippy::manual_clamp)]
fn bar_px(temp_centi: i32) -> i32 {
    let t = temp_centi as f32 / 100.0;
    let mut b = (t + 45.0) * 1.4;
    if b < 0.0 {
        b = 0.0;
    }
    if b > 308.0 {
        b = 308.0;
    }
    b as i32
}

/// Formats a centi value as `23.44` / `-5.07`. Returns the number of bytes
/// written.
fn format_value(centi: i32, out: &mut [u8]) -> usize {
    let mut n = 0;
    let put = |b: u8, out: &mut [u8], n: &mut usize| {
        if *n < out.len() {
            out[*n] = b;
            *n += 1;
        }
    };

    let neg = centi < 0;
    let v = if neg { -centi } else { centi };
    if neg {
        put(b'-', out, &mut n);
    }

    let int = v / 100;
    let frac = v % 100;
    // Integer part, no zero padding.
    let mut digits = [0u8; 10];
    let mut d = 0;
    let mut x = int;
    loop {
        digits[d] = b'0' + (x % 10) as u8;
        d += 1;
        x /= 10;
        if x == 0 {
            break;
        }
    }
    while d > 0 {
        d -= 1;
        put(digits[d], out, &mut n);
    }

    put(b'.', out, &mut n);
    put(b'0' + (frac / 10) as u8, out, &mut n);
    put(b'0' + (frac % 10) as u8, out, &mut n);
    n
}

/// Required because the crate is `no_std` (docs/decisions.md A-04).
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
