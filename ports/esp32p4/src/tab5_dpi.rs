//! The DSI side of the `tab5` backend (`tab5`): the ILI9881C in video mode
//! from a PSRAM frame buffer, with no interrupt.
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
/// CTL1: AXI read and write burst lengths of 16. No IOC_BlkTfr (bit 26): with
/// it, the controller holds the next frame until the block-done status is
/// cleared, which nothing here does (docs/verification-report.md §28).
const CTL1: u32 = (1 << 6) | (16 << 7) | (1 << 15) | (16 << 16);

/// One init list: `(command, params)` pairs.
pub type List = &'static [(u8, &'static [u8])];

/// The init lists in order, with the wait in milliseconds after each.
pub const INIT: [(List, u32); 4] = [
    (init::LANES, 0),
    (init::AFTER_SLEEP_OUT, 120),
    (init::GIP, 0),
    (init::POWER_GAMMA_ON, 0),
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

/// Starts video mode from the frame buffer at `fb`, at `PIXEL_MHZ`.
///
/// # Errors
/// The VDMA controller did not come out of reset.
///
/// # Safety
/// `fb` must point to `FB_BYTES` of PSRAM that stays allocated and written
/// back (`writeback`) while the panel shows it. The DSI host must be up at
/// `LANE_MBPS` (`retune_pll`), and nothing else may use VDMA channel 0.
pub unsafe fn start(fb: *const u8) -> Result<(), ()> {
    let (lane_mbps, pixel_mhz) = (LANE_MBPS as f32, PIXEL_MHZ);
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
            w.lp_vact_en().set_bit();
            w.lp_cmd_en().set_bit();
            // A bus turnaround after every frame, so the panel can acknowledge
            // or report errors, as ESP-IDF does (esp-hal leaves it off).
            w.frame_bta_ack_en().set_bit()
        });
        // The turnaround above needs BTA enabled, as ESP-IDF leaves it.
        host.pckhdl_cfg().modify(|_, w| w.bta_en().set_bit());
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
        // What the bridge sends when its FIFO runs dry; black instead of the
        // reset value 0x3FFF, which shows as cyan.
        bridge
            .dpi_rsv_dpi_data()
            .modify(|_, w| w.dpi_rsv_data().bits(0));
        bridge.en().modify(|_, w| w.dsi_en().set_bit());
        bridge
            .dpi_config_update()
            .write(|w| w.dpi_config_update().set_bit());

        // VDMA channel 0 in reload mode: one block = one frame, replayed by
        // the controller with no link list and no interrupt.
        let dma = VDMA::regs();
        dma.reset0().write(|w| w.dmac_rst().set_bit());
        let delay = esp_hal::delay::Delay::new();
        let mut waited = 0;
        while dma.reset0().read().dmac_rst().bit_is_set() {
            if waited == 1000 {
                return Err(());
            }
            delay.delay_micros(1);
            waited += 1;
        }
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
            // and the bridge FIFO must not run dry at 80 MHz.
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
        dma.chen0().write(|w| w.bits(0x0101)); // channel 0 enable + write enable

        // Video mode on.
        host.mode_cfg()
            .modify(|_, w| w.cmd_video_mode().clear_bit());
        bridge.dpi_misc_config().modify(|_, w| w.dpi_en().set_bit());
        bridge
            .dpi_config_update()
            .write(|w| w.dpi_config_update().set_bit());
    }
    Ok(())
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

    /// Which wait a packet stalled in.
    #[derive(Clone, Copy, Debug)]
    pub struct Stall(pub &'static str);

    /// Waits until `ok` holds, at most `WAIT_US`.
    fn wait(at: &'static str, ok: impl Fn() -> bool) -> Result<(), Stall> {
        let delay = Delay::new();
        for _ in 0..WAIT_US {
            if ok() {
                return Ok(());
            }
            delay.delay_micros(1);
        }
        Err(Stall(at))
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

    /// Waits until every queued command has gone out.
    pub fn drain() -> Result<(), Stall> {
        let h = MIPI_DSI_HOST::regs();
        wait("cmd fifo not empty", || {
            h.cmd_pkt_status().read().gen_cmd_empty().bit_is_set()
        })
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
}

/// D-PHY PLL reference on silicon before v3.0: PLL_F20M, shared with the PHY
/// configuration clock (ESP-IDF v6.0.1 `mipi_dsi_ll.h`, the legacy branch).
/// esp-hal 1.2.2 computes M/N for a 40 MHz XTAL reference, which these chips
/// cannot select, so every rate it programs comes out at half.
pub const PLL_REF_MHZ: u32 = 20;

/// Lane rate this module programs: 20 MHz x 52 / 1 (ESP-IDF's limits:
/// 5 MHz <= ref / N <= 40 MHz, M even).
pub const LANE_MBPS: u32 = PLL_REF_MHZ * PLL_M / PLL_N;
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

/// Brings the D-PHY up again for `LANE_MBPS` from the 20 MHz reference:
/// held in shutdown and reset while the PLL and the HS range are written,
/// then released, so nothing of esp-hal's first setup (for half the rate)
/// stays in effect; the D-PHY supply is set to `ldo` meanwhile. Call right
/// after `MipiDsi::new`. Returns whether the PLL locked and the lanes reached
/// stop state.
pub fn retune_pll(delay: &esp_hal::delay::Delay, ldo: Ldo) -> bool {
    let h = MIPI_DSI_HOST::regs();
    h.phy_rstz().modify(|_, w| {
        w.phy_rstz().clear_bit();
        w.phy_shutdownz().clear_bit()
    });
    ldo.apply();
    delay.delay_micros(1000);
    phy_write(0x44, HS_FREQ_RANGE << 1);
    phy_write(0x19, 0x30); // use the N and M written below
    phy_write(0x17, PLL_N - 1);
    phy_write(0x18, (PLL_M - 1) & 0x1F);
    phy_write(0x18, 0x80 | (((PLL_M - 1) >> 5) & 0x0F));
    h.phy_rstz().modify(|_, w| w.phy_shutdownz().set_bit());
    delay.delay_micros(10);
    h.phy_rstz().modify(|_, w| {
        w.phy_rstz().set_bit();
        w.phy_enableclk().set_bit();
        w.phy_forcepll().set_bit()
    });
    let h = MIPI_DSI_HOST::regs();
    for _ in 0..10_000 {
        // PLL lock, and the clock lane and both data lanes in stop state.
        const STOPPED: u32 = (1 << 2) | (1 << 4) | (1 << 7);
        let st = h.phy_status().read();
        if st.phy_lock().bit_is_set() && st.bits() & STOPPED == STOPPED {
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

/// A setting of the LDO that supplies the D-PHY (VO3, `ext_ldo_p0_0p2a`).
#[derive(Clone, Copy, Debug)]
pub struct Ldo {
    pub dref: u8,
    pub mul: u8,
}

/// eFuse BLOCK1 words (ESP-IDF `efuse_reg.h`): `RD_MAC_SYS_2` holds the
/// block version, `RD_MAC_SYS_3` the VO3 calibration.
const EFUSE_RD_MAC_SYS_2: u32 = 0x5012_D04C;
const EFUSE_RD_MAC_SYS_3: u32 = 0x5012_D050;
/// `PMU_EXT_LDO_P0_0P2A_ANA_REG` (ESP-IDF `pmu_reg.h`): `mul` in bits 25:23,
/// `en_vdet` in bit 26, `dref` in bits 31:28.
const PMU_EXT_LDO_P0_0P2A_ANA: u32 = 0x5011_51C4;

impl Ldo {
    /// The setting for 2.5 V, as ESP-IDF v6.0.1 computes it
    /// (`ldo_ll_voltage_to_dref_mul`, unit 2): from the chip's eFuse
    /// calibration when the block version is at least 1, else uncalibrated.
    /// esp-hal 1.2.2 writes the uncalibrated dref 9 / mul 6 instead
    /// (docs/verification-report.md §28).
    #[must_use]
    pub fn for_2v5() -> Ldo {
        // SAFETY: two eFuse read registers of this chip; reading has no side
        // effect.
        let (sys2, sys3) = unsafe {
            (
                (EFUSE_RD_MAC_SYS_2 as *const u32).read_volatile(),
                (EFUSE_RD_MAC_SYS_3 as *const u32).read_volatile(),
            )
        };
        let blk_version = ((sys2 >> 11) & 0x3) * 100 + ((sys2 >> 8) & 0x7);
        let fuses = (blk_version >= 1).then_some((
            ((sys3 >> 6) & 0xFF) as i32,
            ((sys3 >> 14) & 0x3F) as i32,
            ((sys3 >> 20) & 0x3F) as i32,
        ));
        Ldo::for_mv(2500, fuses)
    }

    /// The `(dref, mul)` whose output is nearest `mv`, for the calibration
    /// fuses `(k, vos, c)` (signed as ESP-IDF reads them; 0 = not burnt).
    fn for_mv(mv: i32, fuses: Option<(i32, i32, i32)>) -> Ldo {
        let (mut k, mut vos, mut c) = (1000, 0, 1000);
        if let Some((fk, fvos, fc)) = fuses {
            if fk != 0 {
                k = if fk & 0x80 != 0 {
                    975 - (fk & 0x7F)
                } else {
                    975 + fk
                };
            }
            if fvos != 0 {
                vos = if fvos & 0x20 != 0 {
                    -(fvos & 0x1F) - 3
                } else {
                    fvos - 3
                };
            }
            if fc != 0 {
                c = if fc & 0x20 != 0 {
                    990 - (fc & 0x1F)
                } else {
                    990 + fc
                };
            }
        }
        // Vout = (Vref * K + Vos) * (1 + 0.25 * mul * C), in units that keep
        // it integral: Vref * 20, K, Vos and C times 1000.
        let mut best = (i32::MAX, Ldo { dref: 0, mul: 0 });
        for dref in 0..16u8 {
            let vref_20 = if dref < 9 {
                10 + i32::from(dref)
            } else {
                20 + (i32::from(dref) - 9) * 2
            };
            for mul in 0..8u8 {
                let vout = (vref_20 * k + 20 * vos) * (4000 + i32::from(mul) * c);
                let diff = (mv * 80_000 - vout).abs();
                if diff < best.0 {
                    best = (diff, Ldo { dref, mul });
                }
            }
        }
        best.1
    }

    /// Writes the setting, with ripple suppression on (`en_vdet`), as
    /// ESP-IDF's `esp_ldo_acquire_channel` leaves it.
    fn apply(self) {
        let reg = PMU_EXT_LDO_P0_0P2A_ANA as *mut u32;
        const FIELDS: u32 = (0xF << 28) | (1 << 26) | (0x7 << 23);
        // SAFETY: the LDO analog register of the D-PHY supply; only its dref,
        // en_vdet and mul fields change, each within its width (4, 1, 3 bits).
        unsafe {
            let v = reg.read_volatile() & !FIELDS;
            reg.write_volatile(
                v | (u32::from(self.dref) << 28) | (1 << 26) | (u32::from(self.mul) << 23),
            );
        }
    }
}
