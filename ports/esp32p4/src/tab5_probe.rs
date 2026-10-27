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
//! - whether PSRAM comes up and holds data (hex mode at 125 MHz);
//! - whether the ILI9881C shows a test picture from a PSRAM frame buffer in
//!   video mode with no interrupt (`tab5_dpi`), and frames keep coming.
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
use crate::tab5_dpi;

/// PI4IOE5V6408 addresses: the first drives the LCD reset, the second the
/// panel power rails.
const PI4IO1: u8 = 0x43;
const PI4IO2: u8 = 0x44;

/// Register writes for the expanders, as `(register, value)`, in M5GFX order.
/// Reset outputs are preloaded low before they become outputs. Output bit 1
/// of 0x43 is the speaker amplifier (M5Unified's "AMP on"); M5GFX sets it,
/// but it stays off here: enabled over an unconfigured ES8388 codec, it pops.
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

    /// Lane rate asked of esp-hal. It assumes a 40 MHz PLL reference this
    /// silicon does not have, so 520 asked runs at 260; `retune_pll` then sets
    /// the real `LANE_MBPS`.
    fn lane_mbps(self) -> f32 {
        520.0
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

    say(serial, "step 3 backlight off until video runs", &[]);
    // Lit before video, the panel shows a dark haze over a faint old picture
    // for as long as the PSRAM test, the DSI bring-up and the init take.
    let mut backlight = Output::new(p.backlight, Level::Low, OutputConfig::default());

    say(serial, "step 4 psram hex at 125 mhz", &[]);
    let psram = Psram::new(
        p.psram,
        PsramConfig {
            mode: PsramMode::Hex,
            size: PsramSize::AutoDetect,
            // esp-hal clocks PSRAM from MPLL at 500 MHz, which 200 MHz does not
            // divide (its init then gives up). At 125 MHz the frame buffer
            // reads topped out near 70 MB/s (38 frames a second at 80 MHz).
            timing: PsramTimingParams::MHZ_125,
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

    // DSI after PSRAM: esp-hal's PSRAM init rewrites every external LDO and
    // leaves the one that powers the D-PHY (LDO 3, `ext_ldo_p0_0p2a`) off.
    // With DSI brought up first, the host sent nothing after PSRAM init.
    say(
        serial,
        "step 5 dsi host, no dpi",
        &[("lane mbps", panel.lane_mbps() as u32)],
    );
    let config = DsiConfig::default()
        .with_num_data_lanes(DataLanes::_2)
        .with_lane_bit_rate_mbps(panel.lane_mbps());
    let mut dsi_bus = None;
    match MipiDsi::new(p.dsi, p.vdma, config) {
        Err(_) => say(serial, "dsi config refused", &[]),
        Ok(bus) => {
            say(serial, "dsi up", &[]);
            let locked = tab5_dpi::retune_pll(&delay);
            say(
                serial,
                "pll retuned",
                &[
                    ("lane mbps", tab5_dpi::LANE_MBPS),
                    ("locked", u32::from(locked)),
                    ("phy 44", tab5_dpi::phy_read(0x44)),
                    ("phy 17", tab5_dpi::phy_read(0x17)),
                    ("phy 18", tab5_dpi::phy_read(0x18)),
                ],
            );
            delay.delay_millis(80);
            tab5_dpi::dbi::setup();
            // No ID read here: a read turns the bus around (BTA), and after
            // one the host sent nothing more on this unit (the ILI9881C ID
            // 98 81 5C was read in earlier runs; the touch controller already
            // tells the panel apart).
            dsi_bus = Some(bus);
        }
    }

    let Some(bus) = dsi_bus else { return };
    if size < tab5_dpi::FB_BYTES {
        return;
    }
    say(serial, "step 6 ili9881c init, then video from psram", &[]);
    for (name, list, wait_ms) in tab5_dpi::INIT {
        say(serial, name, &[("commands", list.len() as u32)]);
        if let Err((cmd, e)) = tab5_dpi::send(list) {
            stalled(serial, cmd, e);
            return;
        }
        delay.delay_millis(wait_ms);
    }
    if let Err((cmd, e)) = tab5_dpi::send(tab5_dpi::PIXEL_FORMAT) {
        stalled(serial, cmd, e);
        return;
    }
    let h = esp_hal::peripherals::MIPI_DSI_HOST::regs();
    say(
        serial,
        "panel init sent",
        &[
            ("pkt status", h.cmd_pkt_status().read().bits()),
            ("int st0", h.int_st0().read().bits()),
            ("int st1", h.int_st1().read().bits()),
        ],
    );
    // SAFETY: `base` is 64-byte aligned PSRAM of at least FB_BYTES (checked
    // above), used by nothing else; the slice ends before `size`.
    let fb = unsafe { core::slice::from_raw_parts_mut(base.cast::<u16>(), tab5_dpi::FB_BYTES / 2) };
    tab5_dpi::test_picture(fb);
    // SAFETY: the same mapped range.
    unsafe { tab5_dpi::writeback(base as u32, tab5_dpi::FB_BYTES as u32) };
    say(serial, "picture written back", &[]);
    // SAFETY: the frame buffer stays allocated (the firmware halts after the
    // probe), the DSI host is up at `lane_mbps`, and VDMA channel 0 is ours.
    // If the DPI side outpaces the DSI host, the host drops payload
    // (`int st1` bit 7) and the picture shears; at half the clock it should not.
    // One clock per boot: reprogramming a running stream stalled the DMA.
    // Leave it running: the D-PHY would power down on drop.
    core::mem::forget(bus);
    for pixel_mhz in [tab5_dpi::PIXEL_MHZ] {
        // SAFETY: the frame buffer stays allocated (the firmware halts after
        // the probe), the DSI host is up at `lane_mbps`, and VDMA channel 0 is
        // ours; `start` resets the channel before reprogramming it.
        let khz = unsafe { tab5_dpi::start(base, tab5_dpi::LANE_MBPS as f32, pixel_mhz) };
        say(serial, "video on", &[("dpi khz", khz)]);
        // A few frames first, so the panel has a whole picture to show.
        delay.delay_millis(50);
        backlight.set_high();
        let _ = tab5_dpi::health();
        // Never returns: dropping the drivers here would change what the
        // stream depends on, and the picture must outlive the probe.
        for second in 0..=u32::MAX {
            if second == 3 {
                dump_regs(serial);
            }
            let mut frames = 0u32;
            let mut fifo_min = u32::MAX;
            let mut underruns = 0u32;
            for _ in 0..2000 {
                let (vsync, underrun) = tab5_dpi::bridge_flags();
                frames += u32::from(vsync);
                fifo_min = fifo_min.min(tab5_dpi::fifo_depth());
                underruns += u32::from(underrun);
                delay.delay_micros(500);
            }
            // The frame buffer held (every pixel read back for two minutes);
            // the read-back is left out so it does not compete with the DMA.
            let bad = 0;
            let _ = bad;
            let st1 = esp_hal::peripherals::MIPI_DSI_HOST::regs()
                .int_st1()
                .read()
                .bits();
            // int_st0: what the panel reported in the turnaround after each
            // frame (ack_with_err_*), and D-PHY errors.
            let st0 = esp_hal::peripherals::MIPI_DSI_HOST::regs()
                .int_st0()
                .read()
                .bits();
            // The expanders that power and reset the panel: output (0x05) and
            // direction (0x03) registers, as read back now.
            let mut regs = [0u8; 4];
            for (i, (addr, reg)) in [
                (PI4IO1, 0x05),
                (PI4IO1, 0x03),
                (PI4IO2, 0x05),
                (PI4IO2, 0x03),
            ]
            .into_iter()
            .enumerate()
            {
                let mut b = [0xEEu8; 1];
                let _ = i2c.write_read(addr, &[reg], &mut b);
                regs[i] = b[0];
            }
            say(
                serial,
                "health",
                &[
                    ("s", second),
                    ("dma frames", frames),
                    ("underruns", underruns),
                    ("fifo min", fifo_min),
                    ("host int st0", st0),
                    ("host int st1", st1),
                    ("io1 out dir", u32::from(regs[0]) << 8 | u32::from(regs[1])),
                    ("io2 out dir", u32::from(regs[2]) << 8 | u32::from(regs[3])),
                ],
            );
        }
    }
}

/// Registers that drive the display, in the format of the M5GFX dump
/// sketch, so the two firmwares' hardware state can be diffed.
fn dump_regs(serial: &mut impl Serial) {
    // Whole blocks where every word reads back; DW-GDMA only by its
    // readable registers (its write-only ones hang the bus on a read).
    const REGIONS: [(u32, u32); 5] = [
        (0x500A_0000, 0x200), // DSI host
        (0x500A_0800, 0x100), // DSI bridge
        (0x500E_6000, 0x200), // HP_SYS_CLKRST
        (0x5011_5180, 0x80),  // PMU external LDOs
        (0x5008_E000, 0x400), // PSRAM MSPI0
    ];
    // Common: ID, version, config, channel enable, interrupt status.
    const GDMA_COMMON: [u32; 10] = [0x00, 0x04, 0x08, 0x0C, 0x10, 0x14, 0x18, 0x1C, 0x30, 0x34];
    // Per channel: SAR, DAR, BLOCK_TS, CTL, CFG, LLP, STATUS, INTSTATUS_ENABLE,
    // INTSTATUS, INTSIGNAL_ENABLE (low and high words).
    const GDMA_CH: [u32; 20] = [
        0x00, 0x04, 0x08, 0x0C, 0x10, 0x14, 0x18, 0x1C, 0x20, 0x24, 0x28, 0x2C, 0x30, 0x34, 0x80,
        0x84, 0x88, 0x8C, 0x90, 0x94,
    ];
    let read = |serial: &mut _, a: u32| {
        // SAFETY: every address is a readable peripheral register of this
        // chip (ESP-IDF's reg_base.h and the DW-GDMA map); a 32-bit read has
        // no side effect other than clearing read-to-clear status bits, which
        // the probe does not rely on after this.
        let v = unsafe { (a as *const u32).read_volatile() };
        say(serial, "reg", &[("a", a), ("v", v)]);
    };
    say(serial, "dump begin", &[]);
    for a in GDMA_COMMON {
        read(serial, 0x5008_1000 + a);
    }
    for ch in 0..4 {
        for a in GDMA_CH {
            read(serial, 0x5008_1100 + ch * 0x100 + a);
        }
    }
    for (base, bytes) in REGIONS {
        for off in (0..bytes).step_by(4) {
            read(serial, base + off);
        }
    }
    for a in 0..0x80 {
        say(serial, "phy", &[("a", a), ("v", tab5_dpi::phy_read(a))]);
    }
    say(serial, "dump end", &[]);
}

/// Reports a DSI packet that did not go through.
fn stalled(serial: &mut impl Serial, cmd: u8, e: tab5_dpi::dbi::Stall) {
    say(
        serial,
        e.at,
        &[
            ("cmd", u32::from(cmd)),
            ("pkt status", e.pkt_status),
            ("int st0", e.int_st0),
            ("int st1", e.int_st1),
        ],
    );
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
        out.hex(*v, 8);
    }
    serial.write(out.as_bytes());
    serial.write(b"\r\n");
}
