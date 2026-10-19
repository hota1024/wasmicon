//! Minimal ILI9341 driver (init and rectangle fills only).
//!
//! The init sequence is the one in apps/README.md §2. Unlike the other three
//! copies (apps/README.md §0), the pixels of a rectangle are not sent row by
//! row but in as few SPI transfers as possible (640 bytes each). The game of
//! life redraws dozens of cells per generation, and calling `dc` and
//! `spi.write` per row would multiply the host calls by 7, more than the UART
//! of a tracing build can carry.

use wasmicon_hal::gpio::{Level, Pin};
use wasmicon_hal::spi::Bus;
use wasmicon_hal::{ErrorCode, time};

use crate::font;

pub const WIDTH: u16 = 320;
pub const HEIGHT: u16 = 240;

/// Pixel buffer for one transfer (320 px).
const CHUNK: usize = WIDTH as usize * 2;

type Result<T> = core::result::Result<T, ErrorCode>;

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

    /// Sends a command and its arguments within one CS assertion.
    fn send(&self, c: u8, args: &[u8]) -> Result<()> {
        self.cs.write(Level::Low)?;
        self.dc.write(Level::Low)?;
        self.spi.write(&[c])?;
        if !args.is_empty() {
            self.dc.write(Level::High)?;
            self.spi.write(args)?;
        }
        self.cs.write(Level::High)
    }

    /// Resets the panel and sends the init sequence (apps/README.md §2).
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
        self.send(0x36, &[0x28])?; // MADCTL = landscape, BGR
        self.send(0x29, &[]) // DISPON
    }

    /// Fills a rectangle with one color, sending the pixels 640 bytes at a
    /// time.
    ///
    /// # Errors
    /// If the rectangle leaves the screen (`invalid-argument`), or a GPIO or
    /// SPI call fails.
    pub fn fill_rect(&self, x: u16, y: u16, w: u16, h: u16, color: u16) -> Result<()> {
        if w == 0 || h == 0 {
            return Ok(());
        }
        // Widen to u32 before comparing (a u16 sum wraps; apps/README.md §0).
        if u32::from(x) + u32::from(w) > u32::from(WIDTH)
            || u32::from(y) + u32::from(h) > u32::from(HEIGHT)
        {
            return Err(ErrorCode::InvalidArgument);
        }
        let (x1, y1) = (x + w - 1, y + h - 1);
        self.send(0x2A, &[(x >> 8) as u8, x as u8, (x1 >> 8) as u8, x1 as u8])?;
        self.send(0x2B, &[(y >> 8) as u8, y as u8, (y1 >> 8) as u8, y1 as u8])?;

        // Fill only what is needed. A 7x7 cell is 98 bytes; filling all 640
        // bytes every time cost thousands of instructions per cell.
        let total = usize::from(w) * usize::from(h) * 2;
        let fill = if total < CHUNK { total } else { CHUNK };
        let mut buf = [0u8; CHUNK];
        let mut i = 0;
        while i < fill {
            buf[i] = (color >> 8) as u8;
            buf[i + 1] = color as u8;
            i += 2;
        }
        self.cs.write(Level::Low)?;
        self.dc.write(Level::Low)?;
        self.spi.write(&[0x2C])?; // RAMWR
        self.dc.write(Level::High)?;
        let mut left = total;
        while left > 0 {
            let n = if left < CHUNK { left } else { CHUNK };
            self.spi.write(&buf[..n])?;
            left -= n;
        }
        self.cs.write(Level::High)
    }

    /// Draws text in the monospace 8x8 font, one row per transfer. Up to the
    /// 40 characters that fit in one screen row.
    ///
    /// # Errors
    /// If the text leaves the screen (`invalid-argument`), or a GPIO or SPI
    /// call fails.
    pub fn draw_text(&self, x: u16, y: u16, text: &[u8], fg: u16, bg: u16) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        let w = text.len() * 8;
        if u32::from(x) + w as u32 > u32::from(WIDTH) || u32::from(y) + 8 > u32::from(HEIGHT) {
            return Err(ErrorCode::InvalidArgument);
        }
        let (x1, y1) = (x + w as u16 - 1, y + 7);
        self.send(0x2A, &[(x >> 8) as u8, x as u8, (x1 >> 8) as u8, x1 as u8])?;
        self.send(0x2B, &[(y >> 8) as u8, y as u8, (y1 >> 8) as u8, y1 as u8])?;
        self.cs.write(Level::Low)?;
        self.dc.write(Level::Low)?;
        self.spi.write(&[0x2C])?; // RAMWR
        self.dc.write(Level::High)?;
        let mut buf = [0u8; CHUNK];
        for row in 0..8 {
            let mut p = 0;
            for &c in text {
                let bits = font::glyph(c)[row];
                for bit in 0..8 {
                    let color = if bits & (0x80 >> bit) != 0 { fg } else { bg };
                    buf[p] = (color >> 8) as u8;
                    buf[p + 1] = color as u8;
                    p += 2;
                }
            }
            self.spi.write(&buf[..p])?;
        }
        self.cs.write(Level::High)
    }

    /// Streams prepared pixels (RGB565, big-endian) into a rectangle.
    ///
    /// The window commands and the write go out back to back within one CS
    /// assertion (14 host calls). Meant for drawing the same image many
    /// times, such as the cells of the game of life: the pixels are not
    /// rebuilt each time, which also saves interpreter instructions.
    ///
    /// # Errors
    /// If the rectangle leaves the screen or `pixels` is not `w * h * 2`
    /// bytes long (`invalid-argument`), or a GPIO or SPI call fails.
    pub fn blit(&self, x: u16, y: u16, w: u16, h: u16, pixels: &[u8]) -> Result<()> {
        if w == 0
            || h == 0
            || u32::from(x) + u32::from(w) > u32::from(WIDTH)
            || u32::from(y) + u32::from(h) > u32::from(HEIGHT)
            || pixels.len() != usize::from(w) * usize::from(h) * 2
        {
            return Err(ErrorCode::InvalidArgument);
        }
        let (x1, y1) = (x + w - 1, y + h - 1);
        self.cs.write(Level::Low)?;
        for (cmd, a, b) in [(0x2A, x, x1), (0x2B, y, y1)] {
            self.dc.write(Level::Low)?;
            self.spi.write(&[cmd])?;
            self.dc.write(Level::High)?;
            self.spi
                .write(&[(a >> 8) as u8, a as u8, (b >> 8) as u8, b as u8])?;
        }
        self.dc.write(Level::Low)?;
        self.spi.write(&[0x2C])?; // RAMWR
        self.dc.write(Level::High)?;
        self.spi.write(pixels)?;
        self.cs.write(Level::High)
    }
}
