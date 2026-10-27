//! Bring-up probe for the M5Stack Tab5's own display, before the guest runs.
//!
//! Called from `main` only with `--features tab5-probe`, then the firmware
//! halts. It answers, one power cycle at a time, what a display backend for
//! the Tab5 needs to know (docs/TODO.md §6, stage 4):
//!
//! - whether the internal I2C (SDA=GPIO31, SCL=GPIO32) reaches the two
//!   PI4IOE5V6408 IO expanders that power and reset the panel;
//! - which panel this unit has (ILI9881C, ST7121 or ST7123), told apart by
//!   its touch controller as M5GFX 0.2.32 does (`M5GFX.cpp`, board_M5Tab5);
//! - whether the DSI host and the D-PHY come up on this silicon, by reading
//!   the panel's ID over DSI with no interrupt and no PSRAM;
//! - whether PSRAM comes up and holds data.
//!
//! Each step prints `tab5: step N ...` before it starts, so a step that hangs
//! or resets the chip still shows where. The expander register values are
//! the ones M5GFX writes. Diagnosis only; results go to
//! docs/verification-report.md.

use esp_hal::delay::Delay;
use esp_hal::gpio::{Flex, Level, Output, OutputConfig};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::mipi_dsi::{Config as DsiConfig, DataLanes, MipiDsi};
use esp_hal::peripherals::{GPIO22, GPIO23, GPIO31, GPIO32, I2C1, MIPI_DSI, PSRAM, VDMA_CH0};
use esp_hal::psram::{Psram, PsramConfig, PsramMode, PsramSize, PsramTimingParams};
use esp_hal::time::Rate;
use wasmicon_port::fmt::Buf;

use crate::board::Serial;

/// PI4IOE5V6408 addresses: the first drives the LCD reset, the second the
/// panel power rails.
const PI4IO1: u8 = 0x43;
const PI4IO2: u8 = 0x44;

/// Register writes for the expanders, as `(register, value)`, in M5GFX order.
/// Reset outputs are preloaded low before they become outputs.
const IO1_FIRST: &[(u8, u8)] = &[
    (0x05, 0b0100_0110), // output: LCD reset and touch reset low
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
const IO1_SECOND: &[(u8, u8)] = &[(0x03, 0b0110_1111), (0x05, 0b0111_0110)];

/// Touch controllers that tell the panel apart.
const ST_TOUCH: u8 = 0x55;
const GT911: u8 = 0x14;

/// The panel kinds M5GFX knows on the Tab5.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel {
    Ili9881c,
    St7121,
    St7123,
    Unknown,
}

impl Panel {
    fn name(self) -> &'static str {
        match self {
            Panel::Ili9881c => "ili9881c",
            Panel::St7121 => "st7121",
            Panel::St7123 => "st7123",
            Panel::Unknown => "unknown",
        }
    }

    /// Lane rate M5GFX uses: the ST7121 runs slower.
    fn lane_mbps(self) -> f32 {
        if self == Panel::St7121 {
            900.0
        } else {
            1040.0
        }
    }
}

/// The peripherals the probe takes. They are not given back: the firmware
/// halts after the probe.
pub struct Parts {
    pub i2c: I2C1<'static>,
    pub sda: GPIO31<'static>,
    pub scl: GPIO32<'static>,
    pub tp_int: GPIO23<'static>,
    pub backlight: GPIO22<'static>,
    pub dsi: MIPI_DSI<'static>,
    pub vdma: VDMA_CH0<'static>,
    pub psram: PSRAM<'static>,
}

/// Runs every step and returns; the caller halts.
pub fn run(serial: &mut impl Serial, p: Parts) {
    let delay = Delay::new();
    say(serial, "step 1 internal i2c and io expanders", &[]);
    // GT911 address select: high = 0x14 (M5GFX drives it before the scan).
    let mut tp_int = Flex::new(p.tp_int);
    tp_int.set_high();
    tp_int.set_output_enable(true);
    let Ok(i2c) = I2c::new(
        p.i2c,
        I2cConfig::default().with_frequency(Rate::from_khz(100)),
    ) else {
        say(serial, "i2c1 config refused", &[]);
        return;
    };
    let mut i2c = i2c.with_sda(p.sda).with_scl(p.scl);
    for (name, addr) in [("pi4io1", PI4IO1), ("pi4io2", PI4IO2)] {
        let mut b = [0u8; 1];
        match i2c.write_read(addr, &[0x01], &mut b) {
            Ok(()) => say(serial, name, &[("id", u32::from(b[0]))]),
            Err(_) => say(serial, name, &[("nack", u32::from(addr))]),
        }
    }
    let mut failed = 0u32;
    for (addr, table) in [(PI4IO1, IO1_FIRST), (PI4IO2, IO2)] {
        for &(reg, val) in table {
            failed += u32::from(i2c.write(addr, &[reg, val]).is_err());
        }
    }
    delay.delay_millis(10);
    for &(reg, val) in IO1_SECOND {
        failed += u32::from(i2c.write(PI4IO1, &[reg, val]).is_err());
    }
    // Back to an input, as M5GFX leaves it: the touch controller drives it.
    tp_int.set_output_enable(false);
    tp_int.set_input_enable(true);
    delay.delay_millis(100);
    say(serial, "expanders written", &[("failed writes", failed)]);

    say(serial, "step 2 which panel", &[]);
    let mut panel = Panel::Unknown;
    for _ in 0..60 {
        let mut fw = [0u8; 1];
        if i2c.write_read(ST_TOUCH, &[0, 0], &mut fw).is_ok() {
            say(serial, "st touch", &[("fw", u32::from(fw[0]))]);
            panel = match fw[0] {
                1 => Panel::St7121,
                3 => Panel::St7123,
                _ => Panel::Unknown,
            };
            if panel != Panel::Unknown {
                break;
            }
        } else {
            // GT911 product ID register: "911\0".
            let mut id = [0u8; 4];
            if i2c.write_read(GT911, &[0x81, 0x40], &mut id).is_ok() {
                say(serial, "gt911", &[("id", u32::from_le_bytes(id))]);
                panel = Panel::Ili9881c;
                break;
            }
        }
        delay.delay_millis(10);
    }
    say(serial, panel.name(), &[]);

    say(
        serial,
        "step 3 dsi host, no dpi",
        &[("lane mbps", panel.lane_mbps() as u32)],
    );
    let config = DsiConfig::default()
        .with_num_data_lanes(DataLanes::_2)
        .with_lane_bit_rate_mbps(panel.lane_mbps());
    match MipiDsi::new(p.dsi, p.vdma, config) {
        Err(_) => say(serial, "dsi config refused", &[]),
        Ok(mut bus) => {
            say(serial, "dsi up", &[]);
            delay.delay_millis(80);
            let mut dbi = bus.dbi(0);
            say(serial, "read st id (0xf4)", &[]);
            let mut st = [0u8; 2];
            let n = dbi.read_cmd(0xF4, &mut st).unwrap_or(0);
            say(
                serial,
                "st id",
                &[
                    ("n", n as u32),
                    ("b0", u32::from(st[0])),
                    ("b1", u32::from(st[1])),
                ],
            );
            say(serial, "read ili id (page 1, 0x00..0x02)", &[]);
            let _ = dbi.write_cmd(0xFF, &[0x98, 0x81, 0x01]);
            let mut ili = [0u8; 3];
            for (i, b) in ili.iter_mut().enumerate() {
                let mut one = [0u8; 1];
                let _ = dbi.read_cmd(i as u8, &mut one);
                *b = one[0];
            }
            let _ = dbi.write_cmd(0xFF, &[0x98, 0x81, 0x00]);
            say(
                serial,
                "ili id",
                &[
                    ("b0", u32::from(ili[0])),
                    ("b1", u32::from(ili[1])),
                    ("b2", u32::from(ili[2])),
                ],
            );
            // The bus would power the D-PHY down on drop; keep it for step 4.
            core::mem::forget(bus);
        }
    }

    say(serial, "step 4 backlight on (gpio22 high)", &[]);
    // `Output` has no `Drop`: the pin stays high after the value is gone.
    let _backlight = Output::new(p.backlight, Level::High, OutputConfig::default());

    say(serial, "step 5 psram hex at 200 mhz", &[]);
    let psram = Psram::new(
        p.psram,
        PsramConfig {
            mode: PsramMode::Hex,
            size: PsramSize::AutoDetect,
            timing: PsramTimingParams::MHZ_200,
        },
    );
    let (base, size) = psram.raw_parts();
    say(
        serial,
        "psram",
        &[("base", base as u32), ("size", size as u32)],
    );
    if size == 0 {
        return;
    }
    // 8 MiB: far past the 256 KiB L2 cache, so the read-back comes from PSRAM.
    let words = (size.min(8 << 20)) / 4;
    say(serial, "psram pattern", &[("bytes", (words * 4) as u32)]);
    let mem = base.cast::<u32>();
    for i in 0..words {
        // SAFETY: `raw_parts` maps `size` bytes from `base`, and `i * 4` stays
        // below `size`; nothing else uses PSRAM in this build.
        unsafe { mem.add(i).write_volatile(pattern(i)) };
    }
    let mut bad = 0u32;
    for i in 0..words {
        // SAFETY: as above.
        let v = unsafe { mem.add(i).read_volatile() };
        bad += u32::from(v != pattern(i));
    }
    say(serial, "psram checked", &[("bad words", bad)]);
}

/// Address-dependent, so a wrong mapping or a stuck line shows.
fn pattern(i: usize) -> u32 {
    (i as u32).wrapping_mul(0x9E37_79B9) ^ 0xA5A5_5A5A
}

/// Prints `tab5: <what> k=v ...` (values in hex).
fn say(serial: &mut impl Serial, what: &str, values: &[(&str, u32)]) {
    let mut line = [0u8; 160];
    let mut out = Buf::new(&mut line);
    out.str("tab5: ");
    out.str(what);
    for (k, v) in values {
        out.byte(b' ');
        out.str(k);
        out.str("=0x");
        out.hex(*v, 1);
    }
    serial.write(out.as_bytes());
    serial.write(b"\r\n");
}
