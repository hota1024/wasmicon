//! Step 6 of the Tab5 probe: the ILI9881C in video mode from a PSRAM frame
//! buffer, with no interrupt.
//!
//! - esp-hal's `DsiDpi` keeps the frame running by re-arming DMA link-list
//!   items in an interrupt, and an interrupt faults on this silicon
//!   (docs/TODO.md §1.3.2). Here the VDMA channel runs one block per frame in
//!   reload mode instead: the controller replays the same source and
//!   destination after every block, so nothing has to run per frame.
//! - The host and bridge set-up follows `DsiDpi::new` of esp-hal 1.2.2
//!   (`src/mipi_dsi/dpi.rs`), with its two interrupts left out.
//! - The frame buffer is RGB565 in memory order (little-endian), so a
//!   big-endian `write-rect` swaps on copy (docs/device-layer.md §4).

use esp_hal::clock::ll::{ClockTree, MipiDsiDpiClkConfig, MipiDsiDpiClkSclk, MipiDsiInstance};
use esp_hal::peripherals::{MIPI_DSI_BRIDGE, MIPI_DSI_HOST, VDMA};

use crate::tab5_ili9881c as init;

pub const WIDTH: u32 = 720;
pub const HEIGHT: u32 = 1280;
pub const FB_BYTES: usize = (WIDTH * HEIGHT * 2) as usize;

/// DPI timing for the ILI9881C on the Tab5 (M5GFX 0.2.32).
pub const PIXEL_MHZ: f32 = 80.0;
const HSW: u32 = 40;
const HBP: u32 = 140;
const HFP: u32 = 40;
const VSW: u32 = 4;
const VBP: u32 = 20;
const VFP: u32 = 20;

/// Bridge FIFO: as in esp-hal.
const DMA_BURST_LEN: u32 = 256;
const FIFO_EMPTY_THRESHOLD: u32 = 1024 - DMA_BURST_LEN;

/// The DSI bridge pixel FIFO, the fixed VDMA destination.
const DSI_BRG_MEM_BASE: u32 = 0x5010_5000;

/// VDMA CTL0 for memory -> DSI: source memory, destination DSI, source
/// increments, destination fixed, 64-bit widths, bursts of 512 / 256 items.
const CTL0: u32 = 1 | (1 << 6) | (3 << 8) | (3 << 11) | (8 << 14) | (7 << 18);
/// CTL1: AXI read and write burst lengths of 16, and IOC_BlkTfr (bit 26) so
/// the block-done status is raised. No interrupt signal is enabled, so it only
/// sets a status bit the probe polls.
const CTL1: u32 = (1 << 6) | (16 << 7) | (1 << 15) | (16 << 16) | (1 << 26);

/// One init list: `(command, params)` pairs.
pub type List = &'static [(u8, &'static [u8])];

/// The init lists in order, with the wait after each.
pub const INIT: [(&str, List, u32); 4] = [
    ("lanes", init::LANES, 0),
    ("sleep out", init::AFTER_SLEEP_OUT, 120),
    ("gip", init::GIP, 0),
    ("power gamma on", init::POWER_GAMMA_ON, 0),
];

/// Sends one init list in command mode; stops at the first stalled packet.
pub fn send(list: &[(u8, &[u8])]) -> Result<(), (u8, dbi::Stall)> {
    for &(cmd, params) in list {
        dbi::write(cmd, params).map_err(|e| (cmd, e))?;
    }
    Ok(())
}

/// MADCTL (RGB order) and COLMOD (16 bits per pixel), after the init lists.
pub const PIXEL_FORMAT: &[(u8, &[u8])] = &[(0x36, &[0x00]), (0x3A, &[0x55])];

/// Writes the frame buffer back from the CPU caches to PSRAM.
///
/// # Safety
/// `addr..addr + len` must be mapped memory.
pub unsafe fn writeback(addr: u32, len: u32) {
    extern "C" {
        // ROM (in the pre-ECO5 tables the port links).
        fn Cache_WriteBack_Addr(bus: u32, addr: u32, size: u32);
    }
    // L1 data cache and L2 cache: PSRAM is behind both.
    const L1_DCACHE: u32 = 1 << 4;
    const L2_CACHE: u32 = 1 << 5;
    // SAFETY: the caller guarantees the range is mapped; the ROM routine
    // only writes dirty lines of that range back.
    unsafe { Cache_WriteBack_Addr(L1_DCACHE | L2_CACHE, addr, len) };
}

/// Starts video mode from the frame buffer at `fb`, at `pixel_mhz` (the
/// Tab5 panel's own is `PIXEL_MHZ`). Returns the DPI clock
/// actually set, in kHz.
///
/// # Safety
/// `fb` must point to `FB_BYTES` of PSRAM that stays allocated and written
/// back (`writeback`) while the panel shows it. The DSI host must be up
/// (`MipiDsi::new`) at `lane_mbps`, and nothing else may use VDMA channel 0.
pub unsafe fn start(fb: *const u8, lane_mbps: f32, pixel_mhz: f32) -> u32 {
    // DPI clock from PLL_F240M, divided to the nearest 80 MHz.
    let real_mhz = ClockTree::with(|clocks| {
        let src_hz = MipiDsiInstance::dpi_clk_config_frequency(
            clocks,
            MipiDsiDpiClkConfig::new(MipiDsiDpiClkSclk::PllF240m, 0),
        );
        let src_mhz = src_hz as f32 / 1_000_000.0;
        let div = ((src_mhz / pixel_mhz) + 0.5) as u32;
        let div = div.max(1);
        let cfg = MipiDsiDpiClkConfig::new(MipiDsiDpiClkSclk::PllF240m, div - 1);
        MipiDsiInstance::MipiDsi.configure_dpi_clk(clocks, cfg);
        MipiDsiInstance::MipiDsi.request_dpi_clk(clocks);
        src_mhz / div as f32
    });

    let host = MIPI_DSI_HOST::regs();
    let bridge = MIPI_DSI_BRIDGE::regs();

    // SAFETY: every register write below goes to the DSI host, the DSI
    // bridge or VDMA channel 0, which the caller gives this function alone,
    // and each value fits its PAC field (sizes up to 1280, timings in lane
    // byte clocks well under 0xffff, the frame block under the 22-bit limit).
    unsafe {
        host.dpi_vcid().modify(|_, w| w.dpi_vcid().bits(0));
        host.dpi_color_coding()
            .modify(|_, w| w.dpi_color_coding().bits(0)); // 16-bit config 1
        host.dpi_cfg_pol().write(|w| w.bits(0));
        host.vid_mode_cfg().modify(|_, w| {
            w.vid_mode_type().bits(2); // burst with sync pulses
            w.lp_vsa_en().set_bit();
            w.lp_vbp_en().set_bit();
            w.lp_vfp_en().set_bit();
            w.lp_hbp_en().set_bit();
            w.lp_hfp_en().set_bit();
            w.lp_cmd_en().set_bit();
            // A bus turnaround after every frame, so the panel can acknowledge
            // or report errors, as ESP-IDF does (esp-hal leaves it off).
            w.frame_bta_ack_en().set_bit()
        });
        host.vid_pkt_size()
            .modify(|_, w| w.vid_pkt_size().bits(WIDTH as u16));
        host.vid_num_chunks()
            .modify(|_, w| w.vid_num_chunks().bits(0));
        host.vid_null_size()
            .modify(|_, w| w.vid_null_size().bits(0));

        // Horizontal timing in lane byte clocks.
        let ratio = lane_mbps / pixel_mhz / 8.0;
        let htotal = HSW + HBP + WIDTH + HFP;
        let r = |v: u32| (v as f32 * ratio + 0.5) as u32;
        let (hsa, hbp, act, hfp) = (r(HSW), r(HBP), r(WIDTH), r(HFP));
        let comp = r(htotal) as i32 - (hsa + hbp + act + hfp) as i32;
        let act = (act as i32 + comp).max(0) as u32;
        host.vid_hsa_time()
            .modify(|_, w| w.vid_hsa_time().bits(hsa as u16));
        host.vid_hbp_time()
            .modify(|_, w| w.vid_hbp_time().bits(hbp as u16));
        host.vid_hline_time()
            .modify(|_, w| w.vid_hline_time().bits((hsa + hbp + act + hfp) as u16));
        host.vid_vsa_lines()
            .modify(|_, w| w.vsa_lines().bits(VSW as u16));
        host.vid_vbp_lines()
            .modify(|_, w| w.vbp_lines().bits(VBP as u16));
        host.vid_vactive_lines()
            .modify(|_, w| w.v_active_lines().bits(HEIGHT as u16));
        host.vid_vfp_lines()
            .modify(|_, w| w.vfp_lines().bits(VFP as u16));

        // Bridge timing; HFP compensates for the DPI clock actually set.
        let brg_hfp = {
            let c = ((real_mhz / pixel_mhz) * htotal as f32 + 0.5) as i32 - htotal as i32;
            (HFP as i32 + c).max(0) as u32
        };
        bridge.dpi_h_cfg0().modify(|_, w| {
            w.htotal().bits((HSW + HBP + WIDTH + brg_hfp) as u16);
            w.hdisp().bits(WIDTH as u16)
        });
        bridge.dpi_h_cfg1().modify(|_, w| {
            w.hsync().bits(HSW as u16);
            w.hbank().bits(HBP as u16)
        });
        bridge.dpi_v_cfg0().modify(|_, w| {
            w.vtotal().bits((VSW + VBP + HEIGHT + VFP) as u16);
            w.vdisp().bits(HEIGHT as u16)
        });
        bridge.dpi_v_cfg1().modify(|_, w| {
            w.vsync().bits(VSW as u16);
            w.vbank().bits(VBP as u16)
        });

        let total_bits = WIDTH * HEIGHT * 16;
        bridge.raw_num_cfg().modify(|_, w| {
            w.raw_num_total().bits(total_bits.div_ceil(64));
            w.unalign_64bit_en().bit(!total_bits.is_multiple_of(64));
            w.raw_num_total_set().set_bit()
        });
        bridge
            .dpi_misc_config()
            .modify(|_, w| w.fifo_underrun_discard_vcnt().bits(WIDTH as u16));
        bridge.pixel_type().modify(|_, w| {
            w.raw_type().bits(2); // RGB565 in
            w.dpi_type().bits(2); // RGB565 out
            w.data_in_type().clear_bit()
        });
        bridge.dma_flow_ctrl().modify(|_, w| {
            w.dsi_dma_flow_controller().clear_bit();
            w.dma_flow_multiblk_num().bits(1)
        });
        bridge
            .dma_frame_interval()
            .modify(|_, w| w.dma_multiblk_en().clear_bit());
        bridge
            .dma_req_cfg()
            .modify(|_, w| w.dma_burst_len().bits(DMA_BURST_LEN as u16));
        bridge.raw_buf_almost_empty_thrd().modify(|_, w| {
            w.dsi_raw_buf_almost_empty_thrd()
                .bits(FIFO_EMPTY_THRESHOLD as u16)
        });
        bridge.en().modify(|_, w| w.dsi_en().set_bit());
        bridge
            .dpi_config_update()
            .write(|w| w.dpi_config_update().set_bit());

        // VDMA channel 0 in reload mode: one block = one frame, replayed by
        // the controller with no link list and no interrupt.
        let dma = VDMA::regs();
        dma.reset0().write(|w| w.dmac_rst().set_bit());
        while dma.reset0().read().dmac_rst().bit_is_set() {}
        dma.cfg0().modify(|_, w| w.dmac_en().set_bit());
        let ch = dma.ch(0);
        ch.cfg0().write(|w| {
            w.ch1_src_multblk_type().bits(1);
            w.ch1_dst_multblk_type().bits(1)
        });
        ch.cfg1().write(|w| {
            w.ch1_tt_fc().bits(1); // memory to peripheral, DMA flow control
            w.ch1_hs_sel_src().clear_bit();
            w.ch1_hs_sel_dst().clear_bit();
            w.ch1_dst_per().bits(0);
            w.ch1_ch_prior().bits(1);
            // 16 outstanding reads (esp-hal uses 5): PSRAM latency is long,
            // and with 5 the bridge FIFO ran dry at 80 MHz.
            w.ch1_src_osr_lmt().bits(15);
            w.ch1_dst_osr_lmt().bits(1)
        });
        ch.sar0().write(|w| w.bits(fb as u32));
        ch.sar1().write(|w| w.bits(0));
        ch.dar0().write(|w| w.bits(DSI_BRG_MEM_BASE));
        ch.dar1().write(|w| w.bits(0));
        ch.block_ts0().write(|w| w.bits((FB_BYTES / 8) as u32 - 1));
        ch.ctl0().write(|w| w.bits(CTL0));
        ch.ctl1().write(|w| w.bits(CTL1));
        // Status only (no signal): lets the probe count frames.
        ch.intstatus_enable0()
            .write(|w| w.ch1_enable_block_tfr_done_intstat().set_bit());
        dma.chen0().write(|w| w.bits(0x0101)); // channel 0 enable + write enable

        // Video mode on.
        host.mode_cfg()
            .modify(|_, w| w.cmd_video_mode().clear_bit());
        bridge.dpi_misc_config().modify(|_, w| w.dpi_en().set_bit());
        bridge
            .dpi_config_update()
            .write(|w| w.dpi_config_update().set_bit());
    }
    (real_mhz * 1000.0) as u32
}

/// Whether the VDMA finished a block (a frame) since the last call; clears it.
pub fn frame_done() -> bool {
    let ch = VDMA::regs().ch(0);
    let done = ch.intstatus0().read().bits() & 1 != 0;
    if done {
        // SAFETY: writing 1 clears the block-done status bit only.
        unsafe { ch.intclear0().write(|w| w.bits(1)) };
    }
    done
}

/// One second of link health: DSI host errors (`int_st1`, read clears),
/// bridge underrun and vsync flags (cleared after), VDMA channel status.
pub fn health() -> [u32; 4] {
    let host = MIPI_DSI_HOST::regs();
    let bridge = MIPI_DSI_BRIDGE::regs();
    let ch = VDMA::regs().ch(0);
    let st1 = host.int_st1().read().bits();
    let brg = bridge.int_raw().read().bits();
    // SAFETY: writing 1s clears the bridge's raw underrun and vsync flags only.
    bridge.int_clr().write(|w| unsafe { w.bits(0b11) });
    [
        st1,
        brg,
        ch.status0().read().bits(),
        ch.intstatus0().read().bits(),
    ]
}

/// Bridge flags since the last call: (vsync, underrun); clears both.
#[allow(dead_code)]
pub fn bridge_flags() -> (bool, bool) {
    let bridge = MIPI_DSI_BRIDGE::regs();
    let raw = bridge.int_raw().read().bits();
    if raw & 0b11 != 0 {
        // SAFETY: writing 1s clears the bridge's raw underrun and vsync flags only.
        bridge.int_clr().write(|w| unsafe { w.bits(raw & 0b11) });
    }
    (raw & 0b10 != 0, raw & 0b01 != 0)
}

/// Words in the bridge's pixel FIFO now.
pub fn fifo_depth() -> u32 {
    MIPI_DSI_BRIDGE::regs().fifo_flow_status().read().bits() & 0x3FFF
}

/// VDMA channel 0 enabled, as the controller reports it.
#[allow(dead_code)]
pub fn dma_running() -> bool {
    VDMA::regs().chen0().read().bits() & 1 != 0
}

/// Fills a portrait test picture: eight horizontal color bars, a 16-step
/// gray ramp at the bottom and a white frame.
pub fn test_picture(fb: &mut [u16]) {
    let w = WIDTH as usize;
    for (i, p) in fb.iter_mut().enumerate() {
        *p = test_pixel(i % w, i / w);
    }
}

/// Pixels of the frame buffer that no longer match the test picture.
#[allow(dead_code)]
pub fn damaged(fb: &[u16]) -> u32 {
    let w = WIDTH as usize;
    let mut bad = 0;
    for (i, p) in fb.iter().enumerate() {
        // SAFETY: `p` is a valid reference; the volatile read keeps the
        // compiler from assuming the buffer is unchanged since it was written.
        let v = unsafe { core::ptr::read_volatile(p) };
        bad += u32::from(v != test_pixel(i % w, i / w));
    }
    bad
}

fn test_pixel(x: usize, y: usize) -> u16 {
    const BARS: [u16; 8] = [
        0xF800, 0xFD20, 0xFFE0, 0x07E0, 0x07FF, 0x001F, 0xF81F, 0xFFFF,
    ];
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
        0xFFFF
    } else if y < h * 3 / 4 {
        BARS[y * 8 / (h * 3 / 4)]
    } else {
        let s = (x * 16 / w) as u16;
        ((s * 31 / 15) << 11) | ((s * 63 / 15) << 5) | (s * 31 / 15)
    }
}

/// DSI command-mode packets with bounded waits.
///
/// esp-hal's `DsiDbi` polls the host's FIFO flags with no limit and disables
/// every DSI timeout, so one lost packet hangs the firmware silently. These
/// give up after `WAIT_US` and report the host's state instead.
pub mod dbi {
    use esp_hal::delay::Delay;
    use esp_hal::peripherals::MIPI_DSI_HOST;

    /// Longest wait for one FIFO flag.
    const WAIT_US: u32 = 20_000;

    const DCS_SHORT_WRITE_0: u8 = 0x05;
    const DCS_SHORT_WRITE_1: u8 = 0x15;
    const DCS_LONG_WRITE: u8 = 0x39;
    #[allow(dead_code)]
    const DCS_READ_0: u8 = 0x06;
    #[allow(dead_code)]
    const SET_MAX_RETURN_PKT: u8 = 0x37;

    /// Where a packet stalled, with the host's status registers then.
    #[derive(Clone, Copy)]
    pub struct Stall {
        pub at: &'static str,
        pub pkt_status: u32,
        pub int_st0: u32,
        pub int_st1: u32,
    }

    fn stall(at: &'static str) -> Stall {
        let h = MIPI_DSI_HOST::regs();
        Stall {
            at,
            pkt_status: h.cmd_pkt_status().read().bits(),
            // Reading clears them; that is fine for a diagnosis.
            int_st0: h.int_st0().read().bits(),
            int_st1: h.int_st1().read().bits(),
        }
    }

    /// Waits until `ok` holds, at most `WAIT_US`.
    fn wait(at: &'static str, ok: impl Fn() -> bool) -> Result<(), Stall> {
        let delay = Delay::new();
        for _ in 0..WAIT_US {
            if ok() {
                return Ok(());
            }
            delay.delay_micros(1);
        }
        Err(stall(at))
    }

    /// Command mode set up as esp-hal does, but with no acknowledgement
    /// request: every packet in low-power mode.
    pub fn setup() {
        MIPI_DSI_HOST::regs().cmd_mode_cfg().modify(|_, w| {
            w.tear_fx_en().clear_bit();
            w.ack_rqst_en().clear_bit();
            w.gen_sw_0p_tx().set_bit();
            w.gen_sw_1p_tx().set_bit();
            w.gen_sw_2p_tx().set_bit();
            w.gen_sr_0p_tx().set_bit();
            w.gen_sr_1p_tx().set_bit();
            w.gen_sr_2p_tx().set_bit();
            w.gen_lw_tx().set_bit();
            w.dcs_sw_0p_tx().set_bit();
            w.dcs_sw_1p_tx().set_bit();
            w.dcs_sr_0p_tx().set_bit();
            w.dcs_lw_tx().set_bit();
            w.max_rd_pkt_size().set_bit()
        });
    }

    fn cmd_room() -> Result<(), Stall> {
        let h = MIPI_DSI_HOST::regs();
        wait("cmd fifo full", || {
            h.cmd_pkt_status().read().gen_cmd_full().bit_is_clear()
        })
    }

    fn header(dt: u8, lo: u8, hi: u8) -> Result<(), Stall> {
        cmd_room()?;
        // SAFETY: virtual channel 0, a DSI data type and two payload bytes,
        // each within its 2- / 6- / 8-bit field.
        MIPI_DSI_HOST::regs().gen_hdr().write(|w| unsafe {
            w.gen_vc().bits(0);
            w.gen_dt().bits(dt);
            w.gen_wc_lsbyte().bits(lo);
            w.gen_wc_msbyte().bits(hi)
        });
        Ok(())
    }

    fn payload(word: u32) -> Result<(), Stall> {
        let h = MIPI_DSI_HOST::regs();
        wait("payload fifo full", || {
            h.cmd_pkt_status().read().gen_pld_w_full().bit_is_clear()
        })?;
        // SAFETY: the payload FIFO takes any 32-bit word.
        h.gen_pld_data().write(|w| unsafe { w.bits(word) });
        Ok(())
    }

    /// Sends a DCS command with its parameters.
    pub fn write(cmd: u8, params: &[u8]) -> Result<(), Stall> {
        match params {
            [] => header(DCS_SHORT_WRITE_0, cmd, 0),
            [p] => header(DCS_SHORT_WRITE_1, cmd, *p),
            _ => {
                // The command byte, then the parameters, packed little-endian.
                let mut bytes = [0u8; 64];
                let n = params.len() + 1;
                bytes[0] = cmd;
                bytes[1..n].copy_from_slice(params);
                for chunk in bytes[..n].chunks(4) {
                    let mut w = [0u8; 4];
                    w[..chunk.len()].copy_from_slice(chunk);
                    payload(u32::from_le_bytes(w))?;
                }
                header(DCS_LONG_WRITE, n as u8, 0)
            }
        }
    }

    /// Reads `out.len()` bytes of a DCS register.
    #[allow(dead_code)]
    pub fn read(cmd: u8, out: &mut [u8]) -> Result<usize, Stall> {
        let h = MIPI_DSI_HOST::regs();
        header(SET_MAX_RETURN_PKT, out.len() as u8, 0)?;
        h.mode_cfg().modify(|_, w| w.cmd_video_mode().set_bit());
        h.pckhdl_cfg().modify(|_, w| w.bta_en().set_bit());
        // SAFETY: virtual channel 0 fits the 2-bit field.
        h.gen_vcid().modify(|_, w| unsafe { w.rx().bits(0) });
        header(DCS_READ_0, cmd, 0)?;
        wait("read busy", || {
            h.cmd_pkt_status().read().gen_rd_cmd_busy().bit_is_clear()
        })?;
        wait("no read data", || {
            h.cmd_pkt_status().read().gen_pld_r_empty().bit_is_clear()
        })?;
        let mut n = 0;
        while h.cmd_pkt_status().read().gen_pld_r_empty().bit_is_clear() {
            let word = h.gen_pld_data().read().bits().to_le_bytes();
            for b in word {
                if n < out.len() {
                    out[n] = b;
                    n += 1;
                }
            }
        }
        Ok(n)
    }
}

/// D-PHY PLL reference on silicon before v3.0: PLL_F20M, shared with the PHY
/// configuration clock (ESP-IDF v6.0.1 `mipi_dsi_ll.h`, the legacy branch).
/// esp-hal 1.2.2 computes M/N for a 40 MHz XTAL reference, which these chips
/// cannot select, so every rate it programs comes out at half.
#[allow(dead_code)]
pub const PLL_REF_MHZ: u32 = 20;

/// Lane rate this module programs: 20 MHz x 52 / 1 (ESP-IDF's limits:
/// 5 MHz <= ref / N <= 40 MHz, M even).
pub const LANE_MBPS: u32 = 1040;
const PLL_M: u32 = 52;
const PLL_N: u32 = 1;
/// `hsfreqrange` for 1000..1050 Mbps (ESP-IDF `mipi_dsi_periph.c`).
const HS_FREQ_RANGE: u32 = 0x2A;
/// Escape clock at most 20 MHz: byte clock 130 MHz / 7.
const ESC_DIV: u32 = 7;

/// Writes a D-PHY register through the host's test interface (ESP-IDF
/// `mipi_dsi_hal_phy_write_register`).
fn phy_write(addr: u32, val: u32) {
    let h = MIPI_DSI_HOST::regs();
    // SAFETY: PHY_TST_CTRL0 takes the test clock in bit 1 and clear in bit 0;
    // PHY_TST_CTRL1 takes an 8-bit address with bit 16 set, or an 8-bit value.
    unsafe {
        h.phy_tst_ctrl0().write(|w| w.bits(0));
        h.phy_tst_ctrl1()
            .write(|w| w.bits((1 << 16) | (addr & 0xFF)));
        h.phy_tst_ctrl0().write(|w| w.bits(1 << 1));
        h.phy_tst_ctrl0().write(|w| w.bits(0));
        h.phy_tst_ctrl1().write(|w| w.bits(val & 0xFF));
        h.phy_tst_ctrl0().write(|w| w.bits(1 << 1));
        h.phy_tst_ctrl0().write(|w| w.bits(0));
    }
}

/// Reprograms the D-PHY PLL for `LANE_MBPS` from the 20 MHz reference and
/// waits for lock. Call right after `MipiDsi::new`. Returns whether it locked.
pub fn retune_pll(delay: &esp_hal::delay::Delay) -> bool {
    phy_write(0x44, HS_FREQ_RANGE << 1);
    phy_write(0x19, 0x30); // use the N and M written below
    phy_write(0x17, PLL_N - 1);
    phy_write(0x18, (PLL_M - 1) & 0x1F);
    phy_write(0x18, 0x80 | (((PLL_M - 1) >> 5) & 0x0F));
    let h = MIPI_DSI_HOST::regs();
    for _ in 0..10_000 {
        if h.phy_status().read().phy_lock().bit_is_set() {
            // SAFETY: both dividers fit their 8-bit fields; the timeout clock
            // divider keeps esp-hal's value.
            h.clkmgr_cfg()
                .modify(|_, w| unsafe { w.tx_esc_clk_division().bits(ESC_DIV as u8) });
            return true;
        }
        delay.delay_micros(1);
    }
    false
}
