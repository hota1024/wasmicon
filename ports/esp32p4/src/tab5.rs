//! The M5Stack Tab5's own panel as `wasmicon:device/display`: the `tab5`
//! backend of docs/device-layer.md §5.3.
//!
//! - Only when the config slot names it (`@display=tab5`), so a firmware that
//!   runs no display app leaves PSRAM, the internal I2C and the backlight
//!   alone (docs/TODO.md §6-6). PSRAM comes up at boot (`Pending::new`): its
//!   init rewrites every external LDO, which must not happen under a running
//!   guest. The panel comes up at the guest's first `open` (`Tab5::up`); the
//!   board holds the parts until then (`board.rs`).
//! - The order matters on v1.0 silicon: PSRAM before DSI (esp-hal's PSRAM
//!   init turns the D-PHY supply off), the panel lit only once video runs.
//!   `tab5_dpi` has the DSI side.
//! - Every driver it takes lives as long as the firmware: dropping one stops
//!   the picture.
//! - The frame buffer is 720x1280 RGB565 in PSRAM, in memory order
//!   (little-endian), scanned by the VDMA without end. Drawing goes through
//!   the CPU cache; `present` writes the rows drawn since the last one back to
//!   PSRAM, which is when the panel shows them.
//! - The panel is portrait, with no rotation.
//! - The expander values are M5GFX 0.2.32's (`M5GFX.cpp`, board_M5Tab5),
//!   except the speaker amplifier, which stays off.

use esp_hal::delay::Delay;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::i2c::master::{Config as I2cConfig, I2c, SoftwareTimeout};
use esp_hal::mipi_dsi::{Config as DsiConfig, DataLanes, MipiDsi};
use esp_hal::peripherals::{GPIO22, GPIO31, GPIO32, I2C1, MIPI_DSI, PSRAM, VDMA_CH0};
use esp_hal::psram::{Psram, PsramConfig, PsramMode, PsramSize, PsramTimingParams};
use esp_hal::time::{Duration, Rate};
use esp_hal::Blocking;
use wasmicon_port::fmt::Buf;
use wasmicon_port::Rect;

use crate::tab5_dpi::{self, FB_BYTES, HEIGHT, WIDTH};

/// PI4IOE5V6408 addresses: the first drives the LCD and touch resets, the
/// second the panel's power rails.
const PI4IO1: u8 = 0x43;
const PI4IO2: u8 = 0x44;

/// Register writes for the expanders, as `(register, value)`, in M5GFX order.
/// Reset outputs are preloaded low before they become outputs. Output bit 1
/// of 0x43 is the speaker amplifier (M5Unified's "AMP on"); M5GFX sets it,
/// but it stays off here: M5Unified enables it only after configuring the
/// ES8388 codec, and nothing here does (docs/verification-report.md §28).
const IO1_FIRST: &[(u8, u8)] = &[
    (0x05, 0b0100_0100), // output: LCD reset, touch reset and amplifier low
    (0x03, 0b0111_1111), // direction
    (0x07, 0b0000_0000), // output high impedance
    (0x0D, 0b0111_1111), // pull select
    (0x0B, 0b0111_1111), // pull enable
];
const IO2: &[(u8, u8)] = &[
    (0x03, 0b1011_1001),
    (0x07, 0b0000_0110),
    (0x0D, 0b1011_1001),
    (0x0B, 0b1111_1001),
    (0x09, 0b0100_0000),
    (0x11, 0b1011_1111),
    (0x05, 0b1000_1001),
];
/// After 10 ms: the LCD reset becomes an input (its pull-up releases the
/// panel) and the touch reset goes high.
const IO1_SECOND: &[(u8, u8)] = &[(0x03, 0b0110_1111), (0x05, 0b0111_0100)];

/// Touch controllers that tell the panel apart, as M5GFX does: an ST touch
/// controller comes with the ST7121 / ST7123 panels, a GT911 with the
/// ILI9881C. M5GFX pins the GT911 at 0x14 by driving its INT line (GPIO23)
/// during reset; this backend does not own GPIO23, so it looks at both of the
/// GT911's addresses.
const ST_TOUCH: u8 = 0x55;
const GT911: [u8; 2] = [0x14, 0x5D];

/// Lane rate asked of esp-hal. It assumes a 40 MHz PLL reference this
/// silicon does not have, so this runs at half until `tab5_dpi::retune_pll`
/// sets `LANE_MBPS`.
const ESP_HAL_LANE_MBPS: f32 = 520.0;

/// The peripherals the backend takes for good.
pub struct Parts {
    pub i2c: I2C1<'static>,
    pub sda: GPIO31<'static>,
    pub scl: GPIO32<'static>,
    pub backlight: GPIO22<'static>,
    pub dsi: MIPI_DSI<'static>,
    pub vdma: VDMA_CH0<'static>,
    pub psram: PSRAM<'static>,
}

/// Why the panel did not come up.
#[derive(Clone, Copy, Debug)]
pub enum UpError {
    I2c,
    Expanders,
    /// An ST touch controller: an ST7121 / ST7123 panel, which this backend
    /// does not drive.
    OtherPanel,
    NoPanel,
    Psram,
    Dsi,
    PllLock,
    /// The DCS command that stalled, and the wait it stalled in.
    Init(u8, &'static str),
    /// The init commands were still queued when video was to start.
    Queued(&'static str),
    /// The VDMA controller did not come out of reset.
    Vdma,
}

impl UpError {
    /// Writes the reason, with the command and wait for a stall.
    pub fn describe(self, out: &mut Buf) {
        out.str(self.reason());
        match self {
            UpError::Init(cmd, at) => {
                out.str(" (command 0x");
                out.hex(u32::from(cmd), 2);
                out.str(", ");
                out.str(at);
                out.str(")");
            }
            UpError::Queued(at) => {
                out.str(" (");
                out.str(at);
                out.str(")");
            }
            _ => {}
        }
    }

    fn reason(self) -> &'static str {
        match self {
            UpError::I2c => "internal I2C refused",
            UpError::Expanders => "IO expander write failed",
            UpError::OtherPanel => "ST7121 / ST7123 panel, not driven",
            UpError::NoPanel => "no touch controller answered, panel unknown",
            UpError::Psram => "PSRAM too small or absent",
            UpError::Dsi => "DSI host refused",
            UpError::PllLock => "D-PHY PLL did not lock",
            UpError::Init(..) => "panel init command stalled",
            UpError::Queued(_) => "init commands not sent before video",
            UpError::Vdma => "VDMA did not reset",
        }
    }
}

/// The parts, with PSRAM up and the frame buffer cleared: what the board
/// holds from boot until the first `open`.
pub struct Pending {
    i2c: I2C1<'static>,
    sda: GPIO31<'static>,
    scl: GPIO32<'static>,
    backlight: GPIO22<'static>,
    dsi: MIPI_DSI<'static>,
    vdma: VDMA_CH0<'static>,
    psram: Psram,
    fb: &'static mut [u16],
}

impl Pending {
    /// Brings PSRAM up and clears the frame buffer to black. Drives no pin.
    ///
    /// # Errors
    /// `UpError::Psram`. The parts are consumed either way.
    pub fn new(p: Parts) -> Result<Pending, UpError> {
        let psram = Psram::new(
            p.psram,
            PsramConfig {
                mode: PsramMode::Hex,
                size: PsramSize::AutoDetect,
                // esp-hal clocks PSRAM from a 500 MHz MPLL, which 200 MHz
                // does not divide (docs/verification-report.md §28).
                timing: PsramTimingParams::MHZ_125,
            },
        );
        let (base, size) = psram.raw_parts();
        if size < FB_BYTES || !(base as usize).is_multiple_of(64) {
            return Err(UpError::Psram);
        }
        // SAFETY: `raw_parts` maps `size` bytes from `base`, at least FB_BYTES
        // and 64-byte aligned (checked above); nothing else in this firmware
        // uses PSRAM, and `psram` is kept with the slice for good.
        let fb = unsafe { core::slice::from_raw_parts_mut(base.cast::<u16>(), FB_BYTES / 2) };
        fb.fill(0);
        // SAFETY: the same mapped range.
        unsafe { tab5_dpi::writeback(base as u32, FB_BYTES as u32) };
        Ok(Pending {
            i2c: p.i2c,
            sda: p.sda,
            scl: p.scl,
            backlight: p.backlight,
            dsi: p.dsi,
            vdma: p.vdma,
            psram,
            fb,
        })
    }
}

/// The running panel.
pub struct Tab5 {
    /// The frame buffer, `WIDTH * HEIGHT` pixels.
    fb: &'static mut [u16],
    /// Rows drawn since the last `present`, as `[first, end)`.
    dirty: Option<(u32, u32)>,
    _psram: Psram,
    _i2c: I2c<'static, Blocking>,
    _backlight: Output<'static>,
}

impl Tab5 {
    /// `(width, height)`: the panel is portrait.
    pub const SIZE: (u32, u32) = (WIDTH, HEIGHT);

    /// Powers the panel, starts video from the black frame buffer and lights
    /// it. Returns the D-PHY supply setting used, for the log.
    ///
    /// # Errors
    /// The step that failed. The parts are consumed either way.
    pub fn up(p: Pending) -> Result<(Tab5, tab5_dpi::Ldo), UpError> {
        let delay = Delay::new();
        // Held low from the start: the backlight stays off until video runs,
        // and the pin is not left floating while the panel powers up.
        let mut backlight = Output::new(p.backlight, Level::Low, OutputConfig::default());
        // A per-byte limit, as the guest's I2C has: a stuck bus fails the
        // bring-up instead of hanging it.
        let config = I2cConfig::default()
            .with_frequency(Rate::from_khz(100))
            .with_software_timeout(SoftwareTimeout::PerByte(Duration::from_millis(25)));
        let i2c = I2c::new(p.i2c, config).map_err(|_| UpError::I2c)?;
        let mut i2c = i2c.with_sda(p.sda).with_scl(p.scl);
        for (addr, table) in [(PI4IO1, IO1_FIRST), (PI4IO2, IO2)] {
            for &(reg, val) in table {
                i2c.write(addr, &[reg, val])
                    .map_err(|_| UpError::Expanders)?;
            }
        }
        delay.delay_millis(10);
        for &(reg, val) in IO1_SECOND {
            i2c.write(PI4IO1, &[reg, val])
                .map_err(|_| UpError::Expanders)?;
        }
        delay.delay_millis(100);
        detect(&mut i2c, &delay)?;

        // DSI after PSRAM (`Pending::new`): esp-hal's PSRAM init rewrites
        // every external LDO and leaves the one that powers the D-PHY off.
        let config = DsiConfig::default()
            .with_num_data_lanes(DataLanes::_2)
            .with_lane_bit_rate_mbps(ESP_HAL_LANE_MBPS);
        let bus = MipiDsi::new(p.dsi, p.vdma, config).map_err(|_| UpError::Dsi)?;
        // The D-PHY powers down when the bus drops; the picture needs it for
        // as long as the firmware runs.
        core::mem::forget(bus);
        let ldo = tab5_dpi::Ldo::for_2v5();
        if !tab5_dpi::retune_pll(&delay, ldo) {
            return Err(UpError::PllLock);
        }
        delay.delay_millis(80);
        tab5_dpi::dbi::setup();
        for (list, wait_ms) in tab5_dpi::INIT {
            tab5_dpi::send(list).map_err(|(cmd, e)| UpError::Init(cmd, e.0))?;
            delay.delay_millis(wait_ms);
        }
        tab5_dpi::send(tab5_dpi::PIXEL_FORMAT).map_err(|(cmd, e)| UpError::Init(cmd, e.0))?;
        // Video mode sends queued commands only in blanking, so the last ones
        // must be out first.
        tab5_dpi::dbi::drain().map_err(|e| UpError::Queued(e.0))?;

        // SAFETY: the frame buffer stays allocated (it moves into the
        // returned `Tab5` with its `Psram`), the DSI host is up at LANE_MBPS,
        // and VDMA channel 0 came with the DSI bus, which is never dropped.
        unsafe { tab5_dpi::start(p.fb.as_ptr().cast()) }.map_err(|()| UpError::Vdma)?;
        // A few frames first, so the panel has a whole picture to show.
        delay.delay_millis(50);
        backlight.set_high();

        Ok((
            Tab5 {
                fb: p.fb,
                dirty: None,
                _psram: p.psram,
                _i2c: i2c,
                _backlight: backlight,
            },
            ldo,
        ))
    }

    /// Fills `r` with `color`. `Hal` has checked `r` against `SIZE`.
    pub fn fill_rect(&mut self, r: Rect, color: u16) {
        for y in r.y..r.y + r.h {
            self.row(y, r.x, r.w).fill(color);
        }
        self.touch(r);
    }

    /// Writes `r.w * r.h` pixels, high byte first (docs/device-layer.md §4),
    /// swapped to the frame buffer's memory order. `Hal` has checked `r` and
    /// the length.
    pub fn write_rect(&mut self, r: Rect, pixels: &[u8]) {
        let stride = r.w as usize * 2;
        for (y, src) in (r.y..r.y + r.h).zip(pixels.chunks_exact(stride)) {
            for (p, b) in self.row(y, r.x, r.w).iter_mut().zip(src.as_chunks::<2>().0) {
                *p = u16::from_be_bytes(*b);
            }
        }
        self.touch(r);
    }

    /// Writes the rows drawn since the last call back to PSRAM, where the
    /// VDMA reads them. The picture can tear: there is one buffer, and the
    /// write-back does not wait for vertical blank (docs/TODO.md §6-7).
    pub fn present(&mut self) {
        if let Some((first, end)) = self.dirty.take() {
            let row = WIDTH as usize * 2;
            let at = self.fb.as_ptr() as usize + first as usize * row;
            // SAFETY: rows `first..end` lie inside the frame buffer, which is
            // mapped PSRAM.
            unsafe { tab5_dpi::writeback(at as u32, ((end - first) as usize * row) as u32) };
        }
    }

    fn row(&mut self, y: u32, x: u32, w: u32) -> &mut [u16] {
        let at = (y * WIDTH + x) as usize;
        &mut self.fb[at..at + w as usize]
    }

    fn touch(&mut self, r: Rect) {
        let (first, end) = (r.y, r.y + r.h);
        self.dirty = Some(match self.dirty {
            Some((a, b)) => (a.min(first), b.max(end)),
            None => (first, end),
        });
    }
}

/// Tells the panel apart by its touch controller, waiting up to 600 ms for it
/// to come out of reset.
fn detect(i2c: &mut I2c<'static, Blocking>, delay: &Delay) -> Result<(), UpError> {
    for _ in 0..60 {
        let mut fw = [0u8; 1];
        if i2c.write_read(ST_TOUCH, &[0, 0], &mut fw).is_ok() {
            return Err(UpError::OtherPanel);
        }
        for addr in GT911 {
            // Product ID register; the read only has to be acknowledged.
            let mut id = [0u8; 4];
            if i2c.write_read(addr, &[0x81, 0x40], &mut id).is_ok() {
                return Ok(());
            }
        }
        delay.delay_millis(10);
    }
    Err(UpError::NoPanel)
}
