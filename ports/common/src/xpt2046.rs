//! XPT2046 touch backend: `wasmicon:device/touch` over the bus of the ILI9341
//! backend (docs/device-layer.md §11.4).
//!
//! - The XPT2046 shares the ILI9341's SPI bus with a chip select of its own.
//!   The panel runs at 16 MHz and the XPT2046 at about 2 MHz at most, so a
//!   read closes the bus, opens it at `SPI_HZ`, samples, and opens it again
//!   at the panel's clock. Both chip selects are high while the bus changes.
//! - With `irq` wired, a read that finds PENIRQ high (no touch) touches no
//!   bus, except every `IDLE_CHECK`th one, which samples anyway. PENIRQ works
//!   only after a control byte with PD = 00: out of power-on it stayed high
//!   under a finger until one was sent, so `init` sends one. In some runs
//!   with the panel's SDO also on MISO, touches then went unseen for
//!   minutes; the idle samples are a fallback for that, whose cause was not
//!   found (docs/verification-report.md §39).
//! - Each conversion is a control byte and two reply bytes; the 12-bit
//!   result is their top bits (the XPT2046 data sheet's 24-clock cycle).
//!   Every control byte has PD1 = PD0 = 0, so PENIRQ stays enabled between
//!   conversions.
//! - A touch is a pressure (`pressure`) of at least `Z_MIN`, before and
//!   after the samples, and the point is the median of `SAMPLES` readings
//!   per axis. The XPT2046 reports one point.
//! - Raw readings map to the pixels of the panel at rotation 0 with the
//!   calibration, then to the configured rotation, as MADCTL lays the
//!   picture out (`Ili9341::madctl`). Only rotation 90 has run on hardware.

use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::spi::Mode as SpiMode;

use crate::ili9341;
use crate::{Board, BoardResult};

/// The XPT2046's clock while sampling.
pub const SPI_HZ: u32 = 2_000_000;

/// Control bytes: start bit, channel, 12-bit differential, PD = 00.
const READ_X: u8 = 0xD0;
const READ_Y: u8 = 0x90;
const READ_Z1: u8 = 0xB0;
const READ_Z2: u8 = 0xC0;

/// Readings per axis; the point is their median.
const SAMPLES: usize = 5;

/// With PENIRQ high, every this many reads samples anyway (see the module
/// header). A touch is seen within this many reads even if PENIRQ fails.
const IDLE_CHECK: u32 = 16;

/// Least pressure that counts as a touch.
pub const Z_MIN: u16 = 400;

/// The panel at rotation 0, in pixels.
const NATIVE: (u32, u32) = (240, 320);

/// The raw readings at the edges of the panel at rotation 0. A minimum above
/// its maximum flips the axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Calibration {
    pub x_min: u32,
    pub x_max: u32,
    pub y_min: u32,
    pub y_max: u32,
}

/// An XPT2046 on SPI bus `spi` (the ILI9341's) with chip select `cs` and,
/// optionally, PENIRQ on `irq`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Xpt2046 {
    spi: u32,
    cs: u32,
    irq: Option<u32>,
    cal: Calibration,
    rotation: u32,
    ready: bool,
    /// Reads since the last one that sampled.
    idle: u32,
}

impl Xpt2046 {
    /// A controller not yet set up. `spi` and `rotation` are the display
    /// entry's; `roles::TouchConfig::check` has checked the rest.
    #[must_use]
    pub const fn new(spi: u32, cs: u32, irq: Option<u32>, cal: Calibration, rotation: u32) -> Self {
        Xpt2046 {
            spi,
            cs,
            irq,
            cal,
            rotation,
            ready: false,
            idle: 0,
        }
    }

    /// Sets the pins up and enables PENIRQ, once. The bus must be open: the
    /// ILI9341 backend's `init` opens it.
    ///
    /// # Errors
    /// Whatever the board's SPI or GPIO returns.
    pub fn init<B: Board>(&mut self, b: &mut B) -> BoardResult<()> {
        if self.ready {
            return Ok(());
        }
        b.gpio_configure(self.cs, PinMode::Output)?;
        b.gpio_write(self.cs, Level::High)?;
        if let Some(irq) = self.irq {
            b.gpio_configure(irq, PinMode::InputPullUp)?;
        }
        // One conversion with PD = 00 enables PENIRQ.
        self.on_slow_bus(b, |t, b| t.convert(b, READ_Z1).map(drop))?;
        self.ready = true;
        Ok(())
    }

    /// Writes the point touched now into `buf` (x then y, high byte first)
    /// and returns 1, or returns 0 for no touch or a `buf` under 4 bytes.
    ///
    /// # Errors
    /// Whatever the board's SPI or GPIO returns. A failure to open the bus
    /// at the panel's clock again leaves the display without a bus.
    pub fn read<B: Board>(&mut self, b: &mut B, buf: &mut [u8]) -> BoardResult<usize> {
        if buf.len() < 4 {
            return Ok(0);
        }
        if let Some(irq) = self.irq
            && b.gpio_read(irq)? == Level::High
        {
            self.idle += 1;
            if self.idle < IDLE_CHECK {
                return Ok(0);
            }
        }
        self.idle = 0;
        let Some((x, y)) = self.on_slow_bus(b, Self::sample_selected)? else {
            return Ok(0);
        };
        let (x, y) = to_pixels(x, y, &self.cal, self.rotation);
        buf[..2].copy_from_slice(&x.to_be_bytes());
        buf[2..4].copy_from_slice(&y.to_be_bytes());
        Ok(1)
    }

    /// Runs `f` with the bus at `SPI_HZ` and `cs` low, then gives the bus
    /// back at the panel's clock.
    fn on_slow_bus<B: Board, T>(
        &self,
        b: &mut B,
        f: impl FnOnce(&Self, &mut B) -> BoardResult<T>,
    ) -> BoardResult<T> {
        b.spi_close(self.spi);
        let r = match b.spi_open(self.spi, SPI_HZ, SpiMode::Mode0) {
            Ok(()) => match b.gpio_write(self.cs, Level::Low) {
                Ok(()) => {
                    let r = f(self, b);
                    b.gpio_write(self.cs, Level::High).and(r)
                }
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        };
        b.spi_close(self.spi);
        b.spi_open(self.spi, ili9341::SPI_HZ, SpiMode::Mode0)?;
        r
    }

    /// The raw point, or `None` under `Z_MIN`; `cs` is low.
    fn sample_selected<B: Board>(&self, b: &mut B) -> BoardResult<Option<(u16, u16)>> {
        let z1 = self.convert(b, READ_Z1)?;
        let z2 = self.convert(b, READ_Z2)?;
        if pressure(z1, z2) < Z_MIN {
            return Ok(None);
        }
        let mut xs = [0u16; SAMPLES];
        let mut ys = [0u16; SAMPLES];
        for (x, y) in xs.iter_mut().zip(&mut ys) {
            *x = self.convert(b, READ_X)?;
            *y = self.convert(b, READ_Y)?;
        }
        // Lifted while sampling: the last readings are not a point.
        let z1 = self.convert(b, READ_Z1)?;
        let z2 = self.convert(b, READ_Z2)?;
        if pressure(z1, z2) < Z_MIN {
            return Ok(None);
        }
        Ok(Some((median(xs), median(ys))))
    }

    /// One conversion: the control byte, then 12 bits in the next two bytes.
    fn convert<B: Board>(&self, b: &mut B, control: u8) -> BoardResult<u16> {
        let mut rx = [0u8; 3];
        b.spi_transfer(self.spi, &[control, 0, 0], &mut rx)?;
        Ok(u16::from_be_bytes([rx[1], rx[2]]) >> 3)
    }
}

/// Pressure from Z1 and Z2: larger when pressed harder, 0 when lifted, and
/// 0 for readings no touch gives. Under a finger 0 < Z1 < Z2; MISO stuck low
/// reads Z1 = Z2 = 0, which the plain formula takes for the hardest press
/// (measured on the ESP32-S3 with the panel's SDO on the same pin).
fn pressure(z1: u16, z2: u16) -> u16 {
    if z1 == 0 || z2 <= z1 {
        return 0;
    }
    (4095 + z1).saturating_sub(z2)
}

fn median(mut v: [u16; SAMPLES]) -> u16 {
    v.sort_unstable();
    v[SAMPLES / 2]
}

/// Maps a raw point to the pixels of the display at `rotation`: the
/// calibration gives the pixel at rotation 0 (240x320), and the rotation lays
/// it out as `Ili9341::madctl` lays the picture out.
#[must_use]
pub fn to_pixels(x: u16, y: u16, cal: &Calibration, rotation: u32) -> (u16, u16) {
    let (w, h) = NATIVE;
    let x0 = scale(u32::from(x), cal.x_min, cal.x_max, w);
    let y0 = scale(u32::from(y), cal.y_min, cal.y_max, h);
    let (x, y) = match rotation {
        90 => (y0, w - 1 - x0),
        180 => (w - 1 - x0, h - 1 - y0),
        270 => (h - 1 - y0, x0),
        _ => (x0, y0),
    };
    (x as u16, y as u16)
}

/// `raw` from `[from, to]` (either order) onto `0..len`, clamped.
fn scale(raw: u32, from: u32, to: u32, len: u32) -> u32 {
    let (raw, from, to) = (i64::from(raw), i64::from(from), i64::from(to));
    let span = to - from;
    if span == 0 {
        return 0;
    }
    let v = (raw - from) * i64::from(len - 1) / span;
    v.clamp(0, i64::from(len - 1)) as u32
}
