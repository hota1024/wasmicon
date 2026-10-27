//! Demo screen drawn through `wasmicon:device/display` (docs/device-layer.md §4).
//!
//! The host owns the panel, so this app has no driver, no pins and no bus: it
//! asks the screen for its size and lays the picture out from it. The same
//! `.wasm` draws on the 320x240 host mock, an ILI9341 and the 720x1280 panel of
//! the M5Stack Tab5; traces compare between displays of the same size
//! (docs/device-layer.md §7).
//!
//! - A title band with scaled text, and the screen size as text.
//! - Color bars: color order. Red and blue swapped means the backend's color
//!   order is wrong.
//! - A gray ramp: RGB565 packing and the byte order of `write-rect`.
//! - A 1 px frame: the exact edge (`x + w == width`).
//! - A bouncing square, one frame per `present`.
//!
//! Nothing is visible before `present` on a frame buffer backend, so every
//! frame ends with it (docs/device-layer.md §4).

#![no_std]

mod font;

use wasmicon_hal::display::{PixelFormat, Screen};
use wasmicon_hal::{ErrorCode, log, time};

type Result<T> = core::result::Result<T, ErrorCode>;

const WHITE: u16 = 0xFFFF;
/// Background and title band.
const NAVY: u16 = 0x0010;
const BAND: u16 = 0x001F;
/// The bouncing square.
const BOX: u16 = 0x07E0;

/// Color bars, left to right: red, orange, yellow, green, cyan, blue,
/// magenta, white.
const BARS: [u16; 8] = [
    0xF800, 0xFD20, 0xFFE0, 0x07E0, 0x07FF, 0x001F, 0xF81F, 0xFFFF,
];

/// Largest text scale. One glyph at this scale is the biggest `write_rect`.
const MAX_SCALE: u32 = 4;
/// Bytes of one glyph at `MAX_SCALE`.
const GLYPH_BYTES: usize = (8 * MAX_SCALE as usize) * (8 * MAX_SCALE as usize) * 2;

/// Frames of the bouncing square.
const FRAMES: u32 = 48;

/// Entry point. The host calls it once, right after instantiation
/// (docs/abi-spec.md §3.3).
#[unsafe(no_mangle)]
pub extern "C" fn run() {
    log::info("display-demo start");
    let Ok(screen) = Screen::open(0) else {
        log::error("no display");
        return;
    };
    if screen.format() != Some(PixelFormat::Rgb565) {
        log::error("unknown pixel format");
        return;
    }
    if draw(&screen).is_err() {
        log::error("display failed");
        return;
    }
    log::info("display-demo done");
}

/// Layout derived from the screen size.
struct Layout {
    w: u32,
    h: u32,
    /// Text scale: 2 on a 320 px wide screen, 4 from 640 px on.
    scale: u32,
    /// One text line in pixels.
    line: u32,
    margin: u32,
}

impl Layout {
    fn of(screen: &Screen) -> Layout {
        let (w, h) = (screen.width(), screen.height());
        let scale = (w / 160).clamp(1, MAX_SCALE);
        Layout {
            w,
            h,
            scale,
            line: 8 * scale,
            margin: 4 * scale,
        }
    }
}

/// Draws the screen and runs the animation. Stops at the first failure.
fn draw(s: &Screen) -> Result<()> {
    let l = Layout::of(s);
    fill(s, 0, 0, l.w, l.h, NAVY)?;

    // Title band.
    let band_h = l.line + 2 * l.margin;
    fill(s, 0, 0, l.w, band_h, BAND)?;
    text(s, &l, l.margin, l.margin, b"WASMICON", WHITE, BAND)?;
    let mut y = band_h + l.margin;

    // The screen size, as `320X240`.
    let mut buf = [0u8; 24];
    let n = size_text(l.w, l.h, &mut buf);
    text(s, &l, l.margin, y, &buf[..n], WHITE, NAVY)?;
    y += l.line + l.margin;

    // Color bars across the full width. The last bar takes the remainder,
    // so the bars end exactly at the right edge.
    let bar_h = l.h / 6;
    let bar_w = l.w / BARS.len() as u32;
    for (i, &c) in BARS.iter().enumerate() {
        let x = i as u32 * bar_w;
        let w = if i + 1 == BARS.len() { l.w - x } else { bar_w };
        fill(s, x, y, w, bar_h, c)?;
    }
    y += bar_h + l.margin;

    // Gray ramp: 16 steps through write-rect, one row repeated, so the byte
    // order of write-rect shows up as tinted steps if it is wrong.
    let ramp_h = l.h / 10;
    gray_ramp(s, &l, y, ramp_h)?;
    y += ramp_h + l.margin;

    text(s, &l, l.margin, y, b"DEVICE DISPLAY", WHITE, NAVY)?;
    y += l.line + l.margin;

    // Frame: four rectangles that end exactly at the edges.
    fill(s, 0, 0, l.w, 1, WHITE)?;
    fill(s, 0, l.h - 1, l.w, 1, WHITE)?;
    fill(s, 0, 0, 1, l.h, WHITE)?;
    fill(s, l.w - 1, 0, 1, l.h, WHITE)?;
    s.present()?;

    bounce(s, &l, y)
}

/// `Screen::fill_rect` with an RGB565 color.
fn fill(s: &Screen, x: u32, y: u32, w: u32, h: u32, c: u16) -> Result<()> {
    s.fill_rect(x, y, w, h, u32::from(c))
}

/// Draws a 16-step gray ramp at `y`, `h` rows high, through `write_rect`.
fn gray_ramp(s: &Screen, l: &Layout, y: u32, h: u32) -> Result<()> {
    let step_w = l.w / 16;
    let mut row = [0u8; GLYPH_BYTES];
    let row_px = (step_w as usize).min(GLYPH_BYTES / 2);
    for step in 0..16u32 {
        let r = (step * 31) / 15;
        let g = (step * 63) / 15;
        let c = ((r << 11) | (g << 5) | r) as u16;
        for p in row.chunks_exact_mut(2).take(row_px) {
            p.copy_from_slice(&c.to_be_bytes());
        }
        // One row at a time keeps the buffer small on a wide screen.
        for dy in 0..h {
            s.write_rect(step * step_w, y + dy, row_px as u32, 1, &row[..row_px * 2])?;
        }
    }
    Ok(())
}

/// Moves a square left and right below `top`, one `present` per frame.
fn bounce(s: &Screen, l: &Layout, top: u32) -> Result<()> {
    let size = 8 * l.scale;
    let bottom = l.h.saturating_sub(l.margin + 1);
    if top + size > bottom {
        return Ok(());
    }
    let y = top + (bottom - top - size) / 2;
    let (x0, x1) = (l.margin, l.w - l.margin - size - 1);
    let step = (x1 - x0) / (FRAMES / 2);

    let mut prev = x0;
    for n in 0..=FRAMES {
        // Right for the first half, left for the second; ends where it started.
        let i = if n <= FRAMES / 2 { n } else { FRAMES - n };
        let x = x0 + i * step;
        fill(s, prev, y, size, size, NAVY)?;
        fill(s, x, y, size, size, BOX)?;
        s.present()?;
        prev = x;
        time::sleep_ms(20);
    }
    Ok(())
}

/// Draws `t` at `(x, y)` in the 8x8 font, scaled by `l.scale`, one glyph per
/// `write_rect`. Text that would leave the screen is not drawn.
fn text(s: &Screen, l: &Layout, x: u32, y: u32, t: &[u8], fg: u16, bg: u16) -> Result<()> {
    let k = l.scale as usize;
    let side = 8 * k;
    let mut buf = [0u8; GLYPH_BYTES];
    for (i, &c) in t.iter().enumerate() {
        let gx = x + (i * side) as u32;
        if gx + side as u32 > l.w || y + side as u32 > l.h {
            return Ok(());
        }
        let glyph = font::GLYPHS[font::index_of(c)];
        for py in 0..side {
            let bits = glyph[py / k];
            for px in 0..side {
                let on = bits & (0x80 >> (px / k)) != 0;
                let color = if on { fg } else { bg };
                let at = (py * side + px) * 2;
                buf[at..at + 2].copy_from_slice(&color.to_be_bytes());
            }
        }
        s.write_rect(gx, y, side as u32, side as u32, &buf[..side * side * 2])?;
    }
    Ok(())
}

/// Writes `<w>X<h>` into `out` and returns its length.
fn size_text(w: u32, h: u32, out: &mut [u8; 24]) -> usize {
    let mut n = decimal(w, &mut out[..]);
    out[n] = b'X';
    n += 1;
    n + decimal(h, &mut out[n..])
}

/// Writes `v` in decimal at the start of `out` and returns the digit count.
fn decimal(mut v: u32, out: &mut [u8]) -> usize {
    let mut digits = [0u8; 10];
    let mut n = 0;
    loop {
        digits[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    for i in 0..n {
        out[i] = digits[n - 1 - i];
    }
    n
}

/// Required because the crate is `no_std` (docs/decisions.md A-04).
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

#[cfg(test)]
mod tests {
    use super::size_text;

    #[test]
    fn size_text_formats_both_numbers() {
        let mut b = [0u8; 24];
        let n = size_text(320, 240, &mut b);
        assert_eq!(&b[..n], b"320X240");
        let n = size_text(720, 1280, &mut b);
        assert_eq!(&b[..n], b"720X1280");
    }
}
