//! Minimal ILI9341 driver. The init sequence and the send order follow
//! apps/README.md §2.
//!
//! No full-screen frame buffer (it does not fit on RP2040). Pixels are
//! streamed into a rectangle one row at a time.
//!
//! A copy of the file of the same name in `sensor-display-rs`. That copy is
//! checked pixel for pixel against the AssemblyScript version, so this one is
//! kept separate rather than shared (editing this file does not run that
//! check). Only the font differs: this copy uses the extended `crate::font`.

use wasmicon_hal::gpio::{Level, Pin};
use wasmicon_hal::spi::Bus;
use wasmicon_hal::{ErrorCode, time};

use crate::font;

pub const WIDTH: u16 = 320;
pub const HEIGHT: u16 = 240;

/// One row of pixels (320 px x 2 bytes).
const ROW_BYTES: usize = WIDTH as usize * 2;

/// Longest text `draw_text` sends at once.
pub const MAX_TEXT: usize = 12;

/// Text buffer (MAX_TEXT x 8 px wide, 8 rows, 2 bytes per pixel).
const TEXT_BYTES: usize = MAX_TEXT * 8 * 8 * 2;

type Result<T> = core::result::Result<T, ErrorCode>;

/// Returns whether `start + len` fits in `limit`. Widens to u32 before adding.
/// Adding in u16 wraps, and an off-screen coordinate would then pass the
/// bounds check (Wasm arithmetic in release builds is wrapping).
///
/// The inputs that slip through have `start + len` in 65536..65856. There are
/// two outcomes, for different input ranges:
///
/// - `fill_rect(65530, 0, 10, 1, c)`: `65530 + 10 == 4` passes, and `window`
///   sends an inverted rectangle (`x0 > x1`) in CASET. The buffer is not
///   exceeded (`n == 20`).
/// - `fill_rect(65216, 0, 640, 1, c)`: wraps to 320 and passes, and then
///   `n = w * 2 == 1280` exceeds the 640-byte `row`. Only this case is an
///   out-of-bounds write (it needs `w > 320`).
///
/// `draw_text` has `w <= MAX_TEXT * 8 == 96`, so it cannot exceed `buf` (the
/// largest `p` is 1534 < 1536). It gets as far as sending the inverted
/// rectangle.
fn in_bounds(start: u16, len: u16, limit: u16) -> bool {
    u32::from(start) + u32::from(len) <= u32::from(limit)
}

/// An ILI9341 attached over SPI.
pub struct Display<'a> {
    spi: &'a Bus,
    cs: &'a Pin,
    dc: &'a Pin,
}

impl<'a> Display<'a> {
    #[must_use]
    pub fn new(spi: &'a Bus, cs: &'a Pin, dc: &'a Pin) -> Self {
        Display { spi, cs, dc }
    }

    fn begin(&self) -> Result<()> {
        self.cs.write(Level::Low)
    }

    fn end(&self) -> Result<()> {
        self.cs.write(Level::High)
    }

    fn cmd(&self, c: u8) -> Result<()> {
        self.dc.write(Level::Low)?;
        self.spi.write(&[c])
    }

    fn data(&self, d: &[u8]) -> Result<()> {
        self.dc.write(Level::High)?;
        self.spi.write(d)
    }

    /// Sends a command and its arguments within one CS assertion.
    fn send(&self, c: u8, args: &[u8]) -> Result<()> {
        self.begin()?;
        self.cmd(c)?;
        if !args.is_empty() {
            self.data(args)?;
        }
        self.end()
    }

    /// Resets the panel and sends the init sequence.
    ///
    /// # Errors
    /// If a GPIO or SPI call fails.
    pub fn init(&self, rst: &Pin) -> Result<()> {
        self.cs.write(Level::High)?;
        rst.write(Level::Low)?;
        time::sleep_ms(10);
        rst.write(Level::High)?;
        time::sleep_ms(120);

        self.send(0x01, &[])?; // SWRESET
        time::sleep_ms(120);
        self.send(0x11, &[])?; // SLPOUT
        time::sleep_ms(120);
        self.send(0x3A, &[0x55])?; // PIXFMT = RGB565
        self.send(0x36, &[0x28])?; // MADCTL = landscape
        self.send(0x29, &[]) // DISPON
    }

    /// Sets the rectangle that the next RAMWR writes to.
    fn window(&self, x: u16, y: u16, w: u16, h: u16) -> Result<()> {
        let x1 = x + w - 1;
        let y1 = y + h - 1;
        self.send(0x2A, &[(x >> 8) as u8, x as u8, (x1 >> 8) as u8, x1 as u8])?;
        self.send(0x2B, &[(y >> 8) as u8, y as u8, (y1 >> 8) as u8, y1 as u8])
    }

    /// Fills a rectangle with one color, sending it row by row.
    ///
    /// # Errors
    /// If a GPIO or SPI call fails.
    pub fn fill_rect(&self, x: u16, y: u16, w: u16, h: u16, color: u16) -> Result<()> {
        if w == 0 || h == 0 {
            return Ok(());
        }
        // Do not draw off screen (apps/README.md §2). This also keeps the
        // write within the row buffer.
        if !in_bounds(x, w, WIDTH) || !in_bounds(y, h, HEIGHT) {
            return Err(ErrorCode::InvalidArgument);
        }
        self.window(x, y, w, h)?;

        let mut row = [0u8; ROW_BYTES];
        let n = w as usize * 2;
        let mut i = 0;
        while i < n {
            row[i] = (color >> 8) as u8;
            row[i + 1] = color as u8;
            i += 2;
        }

        self.begin()?;
        self.cmd(0x2C)?; // RAMWR
        let mut r = 0;
        while r < h {
            self.data(&row[..n])?;
            r += 1;
        }
        self.end()
    }

    /// Draws text in the monospace 8x8 font. Up to `MAX_TEXT` characters.
    ///
    /// # Errors
    /// If a GPIO or SPI call fails.
    pub fn draw_text(&self, x: u16, y: u16, text: &[u8], fg: u16, bg: u16) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        // Never truncate silently (apps/README.md §2).
        if text.len() > MAX_TEXT {
            return Err(ErrorCode::InvalidArgument);
        }
        let len = text.len();
        let w = (len * 8) as u16;
        if !in_bounds(x, w, WIDTH) || !in_bounds(y, 8, HEIGHT) {
            return Err(ErrorCode::InvalidArgument);
        }
        self.window(x, y, w, 8)?;

        // Build all 8 rows at once; within a row each character is 8 px.
        let mut buf = [0u8; TEXT_BYTES];
        let stride = len * 8 * 2;
        let mut row = 0;
        while row < 8 {
            let mut col = 0;
            while col < len {
                let bits = font::GLYPHS[font::index_of(text[col])][row];
                let mut bit = 0;
                while bit < 8 {
                    let on = bits & (0x80 >> bit) != 0;
                    let c = if on { fg } else { bg };
                    let p = row * stride + (col * 8 + bit) * 2;
                    buf[p] = (c >> 8) as u8;
                    buf[p + 1] = c as u8;
                    bit += 1;
                }
                col += 1;
            }
            row += 1;
        }

        self.begin()?;
        self.cmd(0x2C)?; // RAMWR
        self.data(&buf[..stride * 8])?;
        self.end()
    }
}

#[cfg(test)]
mod tests {
    use super::{HEIGHT, WIDTH, in_bounds};

    /// Inputs that slip through by wrapping (`start + len` in 65536..65856).
    /// apps/README.md §2 says `x + w > 320` must fail without drawing.
    #[test]
    fn wrapping_coordinates_are_rejected() {
        // Sums of exactly 65536, which wrap to 0.
        assert!(!in_bounds(65000, 536, WIDTH));
        assert!(!in_bounds(65528, 8, HEIGHT));
        // A sum that wraps to 4.
        assert!(!in_bounds(65530, 10, WIDTH));
        // The top of the range that slips through (65856 -> 320). This one
        // also overruns the row buffer.
        assert!(!in_bounds(65216, 640, WIDTH));
    }

    /// Plain off-screen inputs that do not wrap. The original check already
    /// rejected these.
    #[test]
    fn plain_out_of_range_is_rejected() {
        assert!(!in_bounds(65000, 1000, WIDTH));
        assert!(!in_bounds(1, WIDTH, WIDTH));
        assert!(!in_bounds(0, HEIGHT + 1, HEIGHT));
    }

    /// An exact fit passes. `fill_rect(0, 0, WIDTH, HEIGHT, ...)` hits this
    /// boundary on every frame in every app.
    #[test]
    fn exact_fit_is_allowed() {
        assert!(in_bounds(0, WIDTH, WIDTH));
        assert!(in_bounds(0, HEIGHT, HEIGHT));
        assert!(in_bounds(WIDTH - 1, 1, WIDTH));
    }
}
