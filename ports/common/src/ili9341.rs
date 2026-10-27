//! ILI9341 display backend: `wasmicon:device/display` over the port's own SPI
//! and GPIO (docs/device-layer.md §5.3).
//!
//! - The init sequence and the send order are those of apps/README.md §2, the
//!   ones `apps/lcd-demo-rs` runs on hardware. This is the fourth copy of the
//!   driver (apps/README.md §0); the guest copies stay, since they are the
//!   HAL's evaluation apps.
//! - The backend calls `Board` directly, not the guest's handle tables, so
//!   `Hal::release_all` never closes its bus or floats its reset line (a
//!   floating `RESET` blanks the panel; docs/TODO.md §2).
//! - Writes go out in pieces of at most `CHUNK` bytes, the row size that
//!   lcd-demo-rs sends on hardware.
//! - Only rotation 90 (320x240 landscape, MADCTL 0x28) has run on hardware.

use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::spi::Mode as SpiMode;

use crate::{Board, BoardResult, Rect};

/// SPI clock: the 16 MHz of apps/lcd-demo-rs (RP2350 rounds it to 15 MHz).
pub const SPI_HZ: u32 = 16_000_000;

/// Largest single `spi_write`: one 320-pixel row.
pub const CHUNK: usize = 640;

const SWRESET: u8 = 0x01;
const SLPOUT: u8 = 0x11;
const DISPON: u8 = 0x29;
const CASET: u8 = 0x2A;
const PASET: u8 = 0x2B;
const RAMWR: u8 = 0x2C;
const MADCTL: u8 = 0x36;
const PIXFMT: u8 = 0x3A;

/// An ILI9341 on SPI bus `spi` with chip select, data/command and reset on
/// GPIOs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ili9341 {
    spi: u32,
    cs: u32,
    dc: u32,
    rst: u32,
    rotation: u32,
    ready: bool,
}

impl Ili9341 {
    /// A panel not yet initialized. `rotation` is 0, 90, 180 or 270
    /// (`roles::DisplayConfig::check` has checked it).
    #[must_use]
    pub const fn new(spi: u32, cs: u32, dc: u32, rst: u32, rotation: u32) -> Self {
        Ili9341 {
            spi,
            cs,
            dc,
            rst,
            rotation,
            ready: false,
        }
    }

    /// `(width, height)` after rotation.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        if self.rotation % 180 == 0 {
            (240, 320)
        } else {
            (320, 240)
        }
    }

    /// MADCTL for the rotation, BGR order (the modules at hand are BGR).
    fn madctl(&self) -> u8 {
        match self.rotation {
            0 => 0x48,
            180 => 0x88,
            270 => 0xE8,
            _ => 0x28,
        }
    }

    /// Opens the bus and the pins and runs the init sequence, once. Later calls
    /// return at once: the panel keeps its picture between apps.
    ///
    /// # Errors
    /// Whatever the board's SPI or GPIO returns.
    pub fn init<B: Board>(&mut self, b: &mut B) -> BoardResult<()> {
        if self.ready {
            return Ok(());
        }
        b.spi_open(self.spi, SPI_HZ, SpiMode::Mode0)?;
        for pin in [self.cs, self.dc, self.rst] {
            b.gpio_configure(pin, PinMode::Output)?;
        }
        b.gpio_write(self.cs, Level::High)?;
        b.gpio_write(self.rst, Level::Low)?;
        b.sleep_ms(10);
        b.gpio_write(self.rst, Level::High)?;
        b.sleep_ms(120);

        self.send(b, SWRESET, &[])?;
        b.sleep_ms(120);
        self.send(b, SLPOUT, &[])?;
        b.sleep_ms(120);
        self.send(b, PIXFMT, &[0x55])?; // RGB565
        self.send(b, MADCTL, &[self.madctl()])?;
        self.send(b, DISPON, &[])?;
        self.ready = true;
        Ok(())
    }

    /// Fills `r` with one color.
    ///
    /// # Errors
    /// Whatever the board's SPI or GPIO returns.
    pub fn fill_rect<B: Board>(&mut self, b: &mut B, r: Rect, color: u16) -> BoardResult<()> {
        let mut row = [0u8; CHUNK];
        for px in row.chunks_exact_mut(2) {
            px.copy_from_slice(&color.to_be_bytes());
        }
        let mut left = r.w as usize * r.h as usize * 2;
        self.begin_write(b, r)?;
        while left > 0 {
            let n = left.min(CHUNK);
            b.spi_write(self.spi, &row[..n])?;
            left -= n;
        }
        b.gpio_write(self.cs, Level::High)
    }

    /// Writes `pixels` (RGB565, high byte first, as the panel takes them) into `r`.
    ///
    /// # Errors
    /// Whatever the board's SPI or GPIO returns.
    pub fn write_rect<B: Board>(&mut self, b: &mut B, r: Rect, pixels: &[u8]) -> BoardResult<()> {
        self.begin_write(b, r)?;
        for piece in pixels.chunks(CHUNK) {
            b.spi_write(self.spi, piece)?;
        }
        b.gpio_write(self.cs, Level::High)
    }

    /// Sets the address window and starts RAMWR; CS stays low, DC high.
    fn begin_write<B: Board>(&self, b: &mut B, r: Rect) -> BoardResult<()> {
        let (x1, y1) = (r.x + r.w - 1, r.y + r.h - 1);
        self.send(b, CASET, &span(r.x, x1))?;
        self.send(b, PASET, &span(r.y, y1))?;
        b.gpio_write(self.cs, Level::Low)?;
        b.gpio_write(self.dc, Level::Low)?;
        b.spi_write(self.spi, &[RAMWR])?;
        b.gpio_write(self.dc, Level::High)
    }

    /// One command and its arguments within one CS assertion.
    fn send<B: Board>(&self, b: &mut B, cmd: u8, args: &[u8]) -> BoardResult<()> {
        b.gpio_write(self.cs, Level::Low)?;
        b.gpio_write(self.dc, Level::Low)?;
        b.spi_write(self.spi, &[cmd])?;
        if !args.is_empty() {
            b.gpio_write(self.dc, Level::High)?;
            b.spi_write(self.spi, args)?;
        }
        b.gpio_write(self.cs, Level::High)
    }
}

/// CASET / PASET arguments: start and end, 16 bits each, high byte first.
fn span(a: u32, b: u32) -> [u8; 4] {
    let (a, b) = (a as u16, b as u16);
    let (a, b) = (a.to_be_bytes(), b.to_be_bytes());
    [a[0], a[1], b[0], b[1]]
}
