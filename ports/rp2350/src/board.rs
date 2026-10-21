//! Board implementation for the Raspberry Pi Pico 2 / Pico 2 W.
//!
//! GPIOs are addressed by number at run time, so instead of typed pins this
//! drives the SIO / IO_BANK0 / PADS_BANK0 registers directly. The shared part
//! of the HAL lives in `wasmicon-port`.
//!
//! Nearly the same as the RP2040 port (`ports/rp2040`), with these differences:
//!
//! - The pads have an isolation latch (`ISO`) whose reset value is 1. Until it
//!   is cleared the pad stays disconnected and the GPIO is silently dead. A
//!   `write()` to PADS_BANK0 starts from the reset value, so every write must
//!   clear `iso()` explicitly.
//! - FUNCSEL is a typed enum, so `funcsel().sio()` needs no `unsafe`.
//! - The timer is `TIMER0` (the RP2350 has TIMER0 and TIMER1).
//!
//! SPI uses SPI0 (PL022) through its registers. The `Spi` of `rp235x-hal`
//! wants typed pins, but the board works through `Peripherals::steal()` and
//! cannot hand over ownership, so SPI is written the same way as GPIO.
//!
//! I2C uses I2C0 (DW_apb_i2c) the same way. Setup and transfers follow
//! `i2c/controller.rs` in `rp235x-hal`, but return error codes instead of
//! `assert!`-ing, because the panic handler cannot print a reason
//! (docs/TODO.md §1.4).

use rp235x_hal::pac;
use wasmicon_core::generated::ErrorCode;
use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::i2c::Speed;
use wasmicon_core::generated::log::Level as LogLevel;
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_port::{Board, BoardResult};

/// The RP2350A (QFN-60, on the Pico 2 / Pico 2 W) has GP0..GP29.
/// The QFN-80 RP2350B has GP0..GP47, but no target board carries it.
const NUM_GPIO: u32 = 30;

// The wiring-table check (CLI and firmware, `wasmicon_port::roles`) uses the
// profile's `gpio_count` / `reserved`. If they disagreed with the values here,
// `pin.open` would fail on a table that passed the check, so they are
// cross-checked at compile time. The firmware holds no role table of its own
// (docs/app-workflow.md §3.9).
const _: () = assert!(NUM_GPIO == wasmicon_port::profile::RP2350.gpio_count);
const _: () = assert!(wasmicon_port::profile::same_pins(
    RESERVED,
    wasmicon_port::profile::RP2350.reserved
));

/// GPIOs not released to the guest.
///
/// GP0/GP1 are UART0, which carries the trace; opening them would cut it off.
/// GP23/24/25/29 connect to the CYW43439 (radio and on-board LED) on the
/// Pico 2 W, and to the SMPS PS pin / VBUS sense / on-board LED / VSYS
/// monitor on the Pico 2. Both sets are closed so that the same build runs on
/// either board.
const RESERVED: &[u32] = &[0, 1, 23, 24, 25, 29];

/// Pins assigned to SPI0 (docs/abi-spec.md §8). On the RP2350, FUNCSEL=1 on
/// these three connects them to SPI0. CS is not included: the guest toggles
/// the `lcd-cs` GPIO itself (the design of `wit/spi.wit`).
///
/// They are not in `RESERVED`. Reserving them would make GPIO availability
/// differ from the host mock and the other ports, so trace comparison would
/// differ per board. If the guest opens one of these numbers through `gpio`
/// while SPI is open, FUNCSEL falls back to SIO and SPI silently stops; that
/// counts as a guest bug.
const SPI0_SCK: usize = 18;
const SPI0_MOSI: usize = 19;
const SPI0_MISO: usize = 16;

/// Pins assigned to I2C0 (docs/abi-spec.md §8). FUNCSEL=3 connects them to I2C0.
///
/// Not in `RESERVED`, for the same reason as `SPI0_*`.
const I2C0_SDA: usize = 4;
const I2C0_SCL: usize = 5;

/// Upper bound on the wait for one I2C byte.
///
/// A diagnostic value, not part of the spec. It only keeps the firmware from
/// hanging silently when the clock stops or the peer holds SCL. At 100 kHz
/// one byte (9 bits) takes 90 us, so 25 ms is more than 250 times that and is
/// never reached in normal operation. When it is, the result is `timeout`.
///
/// It is measured in TIMER0 time rather than loop iterations, because an
/// iteration count would change meaning with `opt-level`. `ports/esp32s3`
/// gets the equivalent `Error::Timeout` from `esp-hal`, so the two ports
/// agree here.
const I2C_TIMEOUT_US: u64 = 25_000;

/// Where traces and logs go.
pub trait Serial {
    fn write(&mut self, bytes: &[u8]);
}

/// The Pico 2 board.
pub struct Pico2Board<S: Serial> {
    serial: S,
    /// Frequency of `clk_peri`, which sets the SPI divider. The actual value
    /// decided at boot is passed in rather than assuming 150 MHz.
    peri_clock_hz: u32,
    /// Frequency of `clk_sys`, which sets the I2C SCL counters.
    ///
    /// A different clock from SPI's: the PL022 runs from `clk_peri`, the
    /// DW_apb_i2c from `clk_sys` (`new_controller` in `rp235x-hal` takes
    /// `system_clock`, and `i2c_set_baudrate` in the pico-sdk uses
    /// `clock_get_hz(clk_sys)`). With the default `init_clocks_and_plls` both
    /// are 150 MHz, so mixing them up still works, until `clk_peri` is changed
    /// and SCL drifts from the requested rate.
    sys_clock_hz: u32,
    /// Pins open as open-drain outputs.
    ///
    /// RP2350 pads have no open-drain control either (`OD` is output
    /// disable), so it is emulated with output enable: low is OE=1 and OUT=0,
    /// high is OE=0 (high impedance).
    open_drain: u32,
    /// The assets slot as memory-mapped flash (docs/app-workflow.md §3.10).
    /// `None` until `set_assets`, and when the slot could not be read.
    assets: Option<&'static [u8]>,
}

impl<S: Serial> Pico2Board<S> {
    /// # Safety
    /// This board must have exclusive use of SIO / IO_BANK0 / PADS_BANK0 /
    /// TIMER0 / SPI0 / I2C0, of the SPI0 / I2C0 bits of RESETS, and of the
    /// GPIOs assigned to SPI0 (`SPI0_SCK` / `SPI0_MOSI` / `SPI0_MISO`) and
    /// I2C0 (`I2C0_SDA` / `I2C0_SCL`). The caller must not touch those
    /// peripherals or pins anywhere else.
    ///
    /// `peri_clock_hz` is `clocks.peripheral_clock.freq()` (for SPI) and
    /// `sys_clock_hz` is `clocks.system_clock.freq()` (for I2C). They are
    /// different clocks; do not swap them (with the defaults the values
    /// match, so a swap would still appear to work).
    #[must_use]
    pub unsafe fn new(serial: S, peri_clock_hz: u32, sys_clock_hz: u32) -> Self {
        Pico2Board {
            serial,
            peri_clock_hz,
            sys_clock_hz,
            open_drain: 0,
            assets: None,
        }
    }

    /// The serial port, used to print the reason for a failure.
    pub fn serial(&mut self) -> &mut S {
        &mut self.serial
    }

    /// Hands the board the assets slot (`assets::read_xip` in `main`). The
    /// guest's reads copy out of this slice in place.
    pub fn set_assets(&mut self, region: Option<&'static [u8]>) {
        self.assets = region;
    }

    fn sio() -> pac::SIO {
        // SAFETY: by the contract of Pico2Board::new, only this board touches SIO.
        unsafe { pac::Peripherals::steal().SIO }
    }

    fn timer() -> pac::TIMER0 {
        // SAFETY: by the contract of Pico2Board::new, only this board touches
        // TIMER0, and it is only read.
        unsafe { pac::Peripherals::steal().TIMER0 }
    }

    fn spi0() -> pac::SPI0 {
        // SAFETY: by the contract of Pico2Board::new, only this board touches SPI0.
        unsafe { pac::Peripherals::steal().SPI0 }
    }

    fn i2c0() -> pac::I2C0 {
        // SAFETY: by the contract of Pico2Board::new, only this board touches I2C0.
        unsafe { pac::Peripherals::steal().I2C0 }
    }

    /// Current time in microseconds. TIMER0 runs at 1 MHz, so the count is
    /// used as is.
    ///
    /// Reading TIMELR latches TIMEHR, so the order matters.
    fn micros() -> u64 {
        let t = Self::timer();
        let lo = t.timelr().read().bits();
        let hi = t.timehr().read().bits();
        (u64::from(hi) << 32) | u64::from(lo)
    }

    /// Waits for STOP_DET and clears it. If the wait times out, the flag is
    /// cleared anyway.
    ///
    /// It returns nothing: the real reason the caller holds (a NACK, say) is
    /// more useful for diagnosis than a failure to wait here.
    ///
    /// Leaving the flag set breaks the next transfer. `IC_RAW_INTR_STAT` is
    /// not cleared until `IC_CLR_*` is read (toggling `IC_ENABLE` does not
    /// clear it either), and a stale flag makes the next `i2c_wait_stop`
    /// return at once instead of waiting for its own STOP.
    fn i2c_drain_stop(i2c: &pac::I2C0) {
        let deadline = Self::micros().saturating_add(I2C_TIMEOUT_US);
        while i2c.ic_raw_intr_stat().read().stop_det().is_inactive() {
            if Self::micros() > deadline {
                break;
            }
            core::hint::spin_loop();
        }
        let _ = i2c.ic_clr_stop_det().read();
    }

    /// Tears down the transfer in progress.
    ///
    /// Setting `IC_ENABLE.ABORT` makes the controller issue a STOP and flush
    /// the TX FIFO (the same as `abort` in `rp235x-hal`).
    ///
    /// Always run this after a timeout. Returning without it leaves the
    /// transfer in progress, the next `i2c_set_target` never gets past its
    /// `IC_EN` wait, and every later I2C call waits 25 ms and returns
    /// `timeout`. The NACK path does not need it: the hardware issues the
    /// STOP itself.
    fn i2c_abort(i2c: &pac::I2C0) {
        i2c.ic_enable().modify(|_, w| w.abort().set_bit());
        // The controller clears ABORT itself. Go on even if that never
        // happens (the caller returns `timeout` either way).
        let deadline = Self::micros().saturating_add(I2C_TIMEOUT_US);
        while i2c.ic_enable().read().abort().bit_is_set() {
            if Self::micros() > deadline {
                break;
            }
            core::hint::spin_loop();
        }
        // Clear every flag the abort raised.
        let _ = i2c.ic_clr_tx_abrt().read();
        let _ = i2c.ic_tx_abrt_source().read();
        Self::i2c_drain_stop(i2c);
    }

    /// Reads `IC_TX_ABRT_SOURCE` and, if any bit is set, clears it and turns
    /// it into an error.
    ///
    /// The register is not cleared until `IC_CLR_TX_ABRT` is read
    /// (`IC_CLR_TX_ABRT` itself always reads 0). Forgetting the clear leaves
    /// the TX FIFO disarmed and the next transfer fails.
    ///
    /// On an abort the hardware issues the STOP itself, so none is sent here,
    /// but its flag is waited for and cleared here. `write_internal` in
    /// `rp235x-hal` does the same with
    /// `if abort_reason.is_err() || do_stop { ... clr_stop_det }`; dropping
    /// the first half breaks the next transfer.
    fn i2c_take_abort(i2c: &pac::I2C0) -> BoardResult<()> {
        let src = i2c.ic_tx_abrt_source().read();
        if src.bits() == 0 {
            return Ok(());
        }
        // Reading clears it.
        let _ = i2c.ic_clr_tx_abrt().read();
        // Was the address or the data NACKed, or was it something else (lost
        // arbitration, say)? The host mock returns `nack` when no peer
        // answers, so match that.
        let nacked = src.abrt_7b_addr_noack().bit_is_set()
            || src.abrt_10addr1_noack().bit_is_set()
            || src.abrt_10addr2_noack().bit_is_set()
            || src.abrt_txdata_noack().bit_is_set()
            || src.abrt_gcall_noack().bit_is_set();
        Self::i2c_drain_stop(i2c);
        Err(if nacked {
            ErrorCode::Nack
        } else {
            ErrorCode::Io
        })
    }

    /// Waits until `cond` holds, for at most `I2C_TIMEOUT_US`.
    ///
    /// Every wait goes through this. Copies of the same loop drift apart in
    /// how they check for aborts and clean up the STOP; that happened once,
    /// when the abort path forgot to clear STOP_DET.
    ///
    /// With `check_abort` set, `IC_TX_ABRT_SOURCE` is checked while waiting
    /// and the wait ends with that reason (used in the middle of a transfer).
    /// STOP waits pass false, because the STOP an abort issues is waited for
    /// there as well.
    ///
    /// On timeout the transfer is torn down with `i2c_abort` before returning.
    fn i2c_wait(
        i2c: &pac::I2C0,
        check_abort: bool,
        cond: impl Fn(&pac::I2C0) -> bool,
    ) -> BoardResult<()> {
        let deadline = Self::micros().saturating_add(I2C_TIMEOUT_US);
        while !cond(i2c) {
            if check_abort {
                Self::i2c_take_abort(i2c)?;
            }
            if Self::micros() > deadline {
                Self::i2c_abort(i2c);
                return Err(ErrorCode::Timeout);
            }
            core::hint::spin_loop();
        }
        Ok(())
    }

    /// Waits for the STOP and clears the flag.
    fn i2c_wait_stop(i2c: &pac::I2C0) -> BoardResult<()> {
        Self::i2c_wait(i2c, false, |i2c| {
            i2c.ic_raw_intr_stat().read().stop_det().is_active()
        })?;
        let _ = i2c.ic_clr_stop_det().read();
        Ok(())
    }

    /// Sets the target address.
    ///
    /// `IC_TAR` cannot be changed while ENABLE=1, so the controller is
    /// disabled first, and `IC_ENABLE_STATUS.IC_EN` is waited on until it is
    /// really clear (`rp235x-hal` does not wait; without the wait the write
    /// can be ignored across `open` -> `close` -> `open`).
    ///
    /// This does not use `i2c_wait`, so a timeout here does not `i2c_abort`.
    /// Nothing should be in flight at this point: after a NACK the hardware
    /// issued a STOP and returned to idle, and after a timeout the previous
    /// call already tore the transfer down. If `IC_EN` still does not clear,
    /// the bus itself is held, so report `timeout` rather than hide it
    /// behind an abort.
    fn i2c_set_target(i2c: &pac::I2C0, address: u16) -> BoardResult<()> {
        // 7-bit addresses only (`wit/i2c.wit`). `ports/esp32s3` matches:
        // `esp-hal` rejects out-of-range addresses with `AddressInvalid`. The
        // host mock does not check (docs/TODO.md §2.1).
        if address > 0x7f {
            return Err(ErrorCode::InvalidArgument);
        }
        i2c.ic_enable().write(|w| w.enable().disabled());
        let deadline = Self::micros().saturating_add(I2C_TIMEOUT_US);
        while i2c.ic_enable_status().read().ic_en().bit_is_set() {
            if Self::micros() > deadline {
                return Err(ErrorCode::Timeout);
            }
            core::hint::spin_loop();
        }
        i2c.ic_con()
            .modify(|_, w| w.ic_10bitaddr_master().addr_7bits());
        // SAFETY: address is at most 0x7f (checked above), so it fits the
        // 10-bit IC_TAR field.
        i2c.ic_tar().write(|w| unsafe { w.ic_tar().bits(address) });
        i2c.ic_enable().write(|w| w.enable().enabled());
        Ok(())
    }

    /// Sends `data`. With `stop` false no STOP is issued (the first half of
    /// `write-read`; the following read becomes a Repeated START).
    ///
    /// `data` must not be empty (the caller rejects that with
    /// `invalid-argument`); `data.len() - 1` would underflow.
    fn i2c_send(i2c: &pac::I2C0, data: &[u8], stop: bool) -> BoardResult<()> {
        let last = data.len() - 1;
        for (i, &b) in data.iter().enumerate() {
            // The FIFO only stays full when the peer has stalled. If an abort
            // is pending, return that as the reason.
            Self::i2c_wait(i2c, true, |i2c| i2c.ic_status().read().tfnf().bit_is_set())?;
            i2c.ic_data_cmd().write(|w| {
                w.stop().bit(stop && i == last);
                // SAFETY: b is a u8, so it fits the 8-bit DAT field.
                unsafe { w.dat().bits(b) }
            });
        }
        // Wait until the shift register has drained before checking for an
        // abort.
        Self::i2c_wait(i2c, false, |i2c| {
            i2c.ic_raw_intr_stat().read().tx_empty().is_active()
        })?;
        Self::i2c_take_abort(i2c)?;
        if stop {
            Self::i2c_wait_stop(i2c)?;
        }
        Ok(())
    }

    /// Reads until `buf` is full. Each byte needs one READ command pushed to
    /// `IC_DATA_CMD`; without it no clock is generated.
    ///
    /// `buf` must not be empty (the caller rejects that with
    /// `invalid-argument`); `n - 1` would underflow.
    fn i2c_recv(i2c: &pac::I2C0, buf: &mut [u8]) -> BoardResult<usize> {
        let n = buf.len();
        let last = n - 1;
        for (i, byte) in buf.iter_mut().enumerate() {
            Self::i2c_wait(i2c, true, |i2c| i2c.ic_status().read().tfnf().bit_is_set())?;
            i2c.ic_data_cmd().write(|w| {
                w.stop().bit(i == last);
                w.cmd().read()
            });
            Self::i2c_wait(i2c, true, |i2c| i2c.ic_rxflr().read().bits() != 0)?;
            *byte = i2c.ic_data_cmd().read().dat().bits();
        }
        Self::i2c_wait_stop(i2c)?;
        Ok(n)
    }

    /// Waits until transmission has fully finished and empties the receive
    /// FIFO.
    ///
    /// Always run this before returning from `spi.write` / `spi.transfer`.
    /// The guest moves DC or CS right after the call returns; if the last
    /// byte is still in the shift register at that point, the ILI9341 mixes
    /// up command and data (the registers look right but the picture is
    /// corrupt).
    fn spi_drain(spi: &pac::SPI0) {
        while spi.sspsr().read().bsy().bit_is_set() {
            core::hint::spin_loop();
        }
        while spi.sspsr().read().rne().bit_is_set() {
            let _ = spi.sspdr().read().bits();
        }
        // Clear the receive overrun raised while discarding (SSPICR is
        // write-1-to-clear).
        spi.sspicr().write(|w| w.roric().clear_bit_by_one());
    }
}

/// Picks the PL022 divisors `(cpsdvsr, scr)`. The output rate is
/// `clk_peri / (cpsdvsr * (1 + scr))`; `cpsdvsr` is even in 2..=254 and
/// `scr` is 0..=255.
///
/// Picks the fastest combination that does not exceed the request.
/// `wit/spi.wit` says "round to the nearest", but rounding up could exceed a
/// display's limit, so this rounds down (24 MHz requested from a 150 MHz
/// `clk_peri` gives 18.75 MHz, say). Use the same rule when adding SPI to the
/// rp2040 port.
///
/// `cpsdvsr` is tried from the smallest up, and the first `scr` (total
/// divider 1..=256) at or below the request wins. Up to a total divider of
/// 512 (what `cpsdvsr` = 2 can express) the total divider is always even, so
/// this is the largest rate at or below the request. For slower requests only
/// multiples of `cpsdvsr` are tried, so the result can be one step slow (a
/// required total divider of 517 gives 4 x 130 = 520, although 14 x 37 = 518
/// is closer). The error is toward not exceeding the display's limit, so it
/// is left alone. `set_baudrate` in rp235x-hal guesses `cpsdvsr` first and
/// can exceed the request at the boundaries (100 kHz from 150 MHz, say); this
/// is exhaustive instead.
///
/// Requests outside the reachable range clamp to the ends: `clk_peri / 2` at
/// the top, `clk_peri / (254 * 256)` at the bottom.
fn spi_divisors(clk_hz: u32, want_hz: u32) -> (u8, u8) {
    // want_hz is a divisor below, so 0 is rejected (the caller checks too).
    let want = u64::from(want_hz.max(1));
    let clk = u64::from(clk_hz);

    let mut cpsdvsr: u32 = 2;
    while cpsdvsr <= 254 {
        let mut div: u32 = 1;
        while div <= 256 {
            if clk / (u64::from(cpsdvsr) * u64::from(div)) <= want {
                return (cpsdvsr as u8, (div - 1) as u8);
            }
            div += 1;
        }
        cpsdvsr += 2;
    }
    // The request is below the minimum; clamp to the slowest combination.
    (254, 255)
}

/// Picks the DW_apb_i2c SCL counters `(hcnt, lcnt, spklen, sda_tx_hold)`.
///
/// The arithmetic is that of `i2c/controller.rs` in `rp235x-hal` (itself
/// from `hardware_i2c` in the pico-sdk), except that out-of-range input
/// returns `None`, which the caller turns into `unsupported`, instead of
/// `assert!`-ing (the panic handler cannot print a reason; docs/TODO.md §1.4).
///
/// The period is split 60% low / 40% high. There is no "do not exceed"
/// rounding as in `spi_divisors`: I2C tolerates clock stretching, and
/// `wit/i2c.wit` only offers the `speed` enum. The actual SCL is a few
/// percent below `clk / (hcnt + lcnt)`, because the controller adds the
/// spike-filter length and fixed cycles to each phase; the pico-sdk and
/// `rp235x-hal` ignore this too ("There are some subtleties to I2C timing
/// which we are completely ignoring here"), so measure with a scope if the
/// exact rate matters.
///
/// With `clk_sys` at 150 MHz:
///
/// | speed | hcnt | lcnt | spklen | sda_hold |
/// |---|---|---|---|---|
/// | `standard` | 600 | 900 | 56 | 46 |
/// | `fast` | 150 | 225 | 14 | 46 |
/// | `fast-plus` | 60 | 90 | 5 | 19 |
///
/// Returns `None` for 0 or above 1 MHz, for `hcnt` / `lcnt` outside
/// 8..=0xffff (happens once `clk_sys` drops to about 2 MHz), for `fast-plus`
/// with `clk_sys` below 32 MHz, and for `sda_hold` above `lcnt - 2`.
fn i2c_timing(clk_hz: u32, freq_hz: u32) -> Option<(u16, u16, u8, u16)> {
    if freq_hz == 0 || freq_hz > 1_000_000 {
        return None;
    }
    let clk = u64::from(clk_hz);
    let freq = u64::from(freq_hz);
    // Round to nearest, then split 3:2.
    let period = (clk + freq / 2) / freq;
    let lcnt = period * 3 / 5;
    let hcnt = period - lcnt;
    if !(8..=0xffff).contains(&hcnt) || !(8..=0xffff).contains(&lcnt) {
        return None;
    }

    // The SDA hold time the I2C spec requires (300 ns for standard / fast,
    // 120 ns for fast-plus) in clock cycles; the +1 avoids rounding down.
    let sda_hold = if freq < 1_000_000 {
        (clk * 3) / 10_000_000 + 1
    } else {
        // fast-plus cannot produce the required hold time unless clk_in > 32 MHz.
        if clk < 32_000_000 {
            return None;
        }
        (clk * 3) / 25_000_000 + 1
    };
    // lcnt >= 8 was checked above, so the subtraction cannot underflow.
    if sda_hold > lcnt - 2 {
        return None;
    }

    // Spike filter length.
    let spklen = if lcnt < 16 { 1 } else { lcnt / 16 };
    if spklen > 0xff {
        return None;
    }

    Some((hcnt as u16, lcnt as u16, spklen as u8, sda_hold as u16))
}

impl<S: Serial> Board for Pico2Board<S> {
    fn gpio_count(&self) -> u32 {
        NUM_GPIO
    }

    fn gpio_reserved(&self, index: u32) -> bool {
        RESERVED.contains(&index)
    }

    fn gpio_configure(&mut self, index: u32, mode: PinMode) -> BoardResult<()> {
        if index >= NUM_GPIO {
            return Err(ErrorCode::InvalidArgument);
        }
        let n = index as usize;
        // SAFETY: by the contract of Pico2Board::new, only this board touches
        // SIO / IO_BANK0 / PADS_BANK0.
        let p = unsafe { pac::Peripherals::steal() };

        // Pad setup: input buffer, pulls, output disable, and isolation off.
        // `write()` starts from the reset value (ISO=1, PDE=1), so every
        // field is written.
        p.PADS_BANK0.gpio(n).write(|w| {
            w.ie().set_bit();
            w.od().clear_bit();
            match mode {
                PinMode::InputPullUp => {
                    w.pue().set_bit();
                    w.pde().clear_bit();
                }
                PinMode::InputPullDown => {
                    w.pue().clear_bit();
                    w.pde().set_bit();
                }
                _ => {
                    w.pue().clear_bit();
                    w.pde().clear_bit();
                }
            }
            // Connect the pad last. Left set, the GPIO stays unresponsive.
            w.iso().clear_bit();
            w
        });

        // Hand the pin to software control.
        p.IO_BANK0.gpio(n).gpio_ctrl().write(|w| w.funcsel().sio());

        let mask = 1u32 << index;
        match mode {
            PinMode::OutputOpenDrain => {
                // Start in high impedance; the external pull-up holds the line.
                self.open_drain |= mask;
                // SAFETY: index < NUM_GPIO (30), so mask is one bit of the
                // 32-bit GPIO_OUT_CLR field.
                p.SIO.gpio_out_clr().write(|w| unsafe { w.bits(mask) });
                // SAFETY: index < NUM_GPIO (30), so mask is one bit of the
                // 32-bit GPIO_OE_CLR field.
                p.SIO.gpio_oe_clr().write(|w| unsafe { w.bits(mask) });
            }
            PinMode::Output => {
                self.open_drain &= !mask;
                // SAFETY: index < NUM_GPIO (30), so mask is one bit of the
                // 32-bit GPIO_OE_SET field.
                p.SIO.gpio_oe_set().write(|w| unsafe { w.bits(mask) });
            }
            _ => {
                self.open_drain &= !mask;
                // SAFETY: index < NUM_GPIO (30), so mask is one bit of the
                // 32-bit GPIO_OE_CLR field.
                p.SIO.gpio_oe_clr().write(|w| unsafe { w.bits(mask) });
            }
        }
        Ok(())
    }

    fn gpio_write(&mut self, index: u32, level: Level) -> BoardResult<()> {
        if index >= NUM_GPIO {
            return Err(ErrorCode::InvalidArgument);
        }
        let sio = Self::sio();
        let mask = 1u32 << index;
        if self.open_drain & mask != 0 {
            // Open drain: low is driven, high is high impedance.
            match level {
                Level::Low => {
                    // SAFETY: index < NUM_GPIO (30), so mask is one bit of the
                    // 32-bit GPIO_OUT_CLR field.
                    sio.gpio_out_clr().write(|w| unsafe { w.bits(mask) });
                    // SAFETY: index < NUM_GPIO (30), so mask is one bit of the
                    // 32-bit GPIO_OE_SET field.
                    sio.gpio_oe_set().write(|w| unsafe { w.bits(mask) });
                }
                // SAFETY: index < NUM_GPIO (30), so mask is one bit of the
                // 32-bit GPIO_OE_CLR field.
                Level::High => sio.gpio_oe_clr().write(|w| unsafe { w.bits(mask) }),
            }
            return Ok(());
        }
        // A pin that is not configured as an output cannot be written.
        if sio.gpio_oe().read().bits() & mask == 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        match level {
            // SAFETY: index < NUM_GPIO (30), so mask is one bit of the 32-bit
            // GPIO_OUT_SET field.
            Level::High => sio.gpio_out_set().write(|w| unsafe { w.bits(mask) }),
            // SAFETY: index < NUM_GPIO (30), so mask is one bit of the 32-bit
            // GPIO_OUT_CLR field.
            Level::Low => sio.gpio_out_clr().write(|w| unsafe { w.bits(mask) }),
        }
        Ok(())
    }

    fn gpio_read(&mut self, index: u32) -> BoardResult<Level> {
        if index >= NUM_GPIO {
            return Err(ErrorCode::InvalidArgument);
        }
        let sio = Self::sio();
        let mask = 1u32 << index;
        // Output mode reads the output value, input mode the input value (the
        // definition of read in the WIT).
        let bits = if self.open_drain & mask != 0 {
            // Open drain reads the actual state of the line.
            sio.gpio_in().read().bits()
        } else if sio.gpio_oe().read().bits() & mask != 0 {
            sio.gpio_out().read().bits()
        } else {
            sio.gpio_in().read().bits()
        };
        Ok(if bits & mask != 0 {
            Level::High
        } else {
            Level::Low
        })
    }

    fn gpio_release(&mut self, index: u32) {
        if index >= NUM_GPIO {
            return;
        }
        // docs/abi-spec.md §5.2: back to input with no pulls.
        self.open_drain &= !(1u32 << index);
        let _ = self.gpio_configure(index, PinMode::Input);
    }

    // --- I2C. index 0 = I2C0 (SDA=GP4, SCL=GP5) ---

    fn i2c_open(&mut self, index: u32, speed: Speed) -> BoardResult<()> {
        // i2c1 is on the header too, but v0.1 only assigns i2c0
        // (docs/abi-spec.md §8). The Hal has already checked index < MAX_I2C.
        // Same test as for SPI (docs/TODO.md §2.1).
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        let freq_hz = match speed {
            Speed::Standard => 100_000,
            Speed::Fast => 400_000,
            Speed::FastPlus => 1_000_000,
        };
        let Some((hcnt, lcnt, spklen, sda_hold)) = i2c_timing(self.sys_clock_hz, freq_hz) else {
            return Err(ErrorCode::Unsupported);
        };

        // SAFETY: by the contract of Pico2Board::new, only this board touches
        // RESETS (its I2C0 bit), I2C0, IO_BANK0 and PADS_BANK0.
        let p = unsafe { pac::Peripherals::steal() };

        // Pulse the reset. Unlike SPI0 it is asserted and then released, so
        // that open -> close -> open does not keep the previous setup.
        p.RESETS.reset().modify(|_, w| w.i2c0().set_bit());
        p.RESETS.reset().modify(|_, w| w.i2c0().clear_bit());
        while p.RESETS.reset_done().read().i2c0().bit_is_clear() {
            core::hint::spin_loop();
        }

        let i2c = p.I2C0;
        // Configure while ENABLE=0.
        i2c.ic_enable().write(|w| w.enable().disabled());

        // Run as a master. `ic_restart_en` is needed for the Repeated START
        // of `write-read`. `rx_fifo_full_hld_ctrl` makes the controller hold
        // the clock while the receive FIFO is full, so nothing is lost.
        i2c.ic_con().modify(|_, w| {
            w.speed().fast();
            w.master_mode().enabled();
            w.ic_slave_disable().slave_disabled();
            w.ic_restart_en().enabled();
            w.tx_empty_ctrl().enabled();
            w.rx_fifo_full_hld_ctrl().enabled();
            w
        });

        // SAFETY: i2c_timing bounded hcnt and lcnt to 8..=0xffff (16-bit
        // fields), spklen to at most 0xff (8-bit field) and sda_hold to at
        // most lcnt - 2 (16-bit field); the TX / RX thresholds are 0, which
        // fits their 8-bit fields.
        unsafe {
            i2c.ic_fs_scl_hcnt()
                .write(|w| w.ic_fs_scl_hcnt().bits(hcnt));
            i2c.ic_fs_scl_lcnt()
                .write(|w| w.ic_fs_scl_lcnt().bits(lcnt));
            i2c.ic_fs_spklen().write(|w| w.ic_fs_spklen().bits(spklen));
            i2c.ic_sda_hold()
                .modify(|_, w| w.ic_sda_tx_hold().bits(sda_hold));
            // The thresholds are unused (transfers are synchronous loops on
            // IC_STATUS), so they are left at 0.
            i2c.ic_tx_tl().write(|w| w.tx_tl().bits(0));
            i2c.ic_rx_tl().write(|w| w.rx_tl().bits(0));
        }

        // Route SDA / SCL to the pads. As for GPIO / SPI, `write()` starts
        // from the reset value (ISO=1, PDE=1), so ISO is cleared.
        //
        // The internal pull-ups are enabled. I2C is a bus whose lines are held
        // high by pull-ups, and the RP2350's internal pulls are weak
        // (50..80 kOhm). On `ports/esp32s3` the `connect_pin` of `esp-hal`
        // always applies `Pull::Up` to SDA/SCL (`i2c/master/low_level/mod.rs`
        // in esp-hal 1.2.1), so this keeps the bus conditions the same on
        // both ports; the pico-sdk i2c example does the same. It is only a
        // weak aid: an external 10 kOhm pull-up is required (100 kHz over
        // short wires may work on the internal pulls alone, `fast` /
        // `fast-plus` do not).
        for n in [I2C0_SDA, I2C0_SCL] {
            p.PADS_BANK0.gpio(n).write(|w| {
                w.ie().set_bit();
                w.od().clear_bit();
                w.pue().set_bit();
                w.pde().clear_bit();
                w.iso().clear_bit();
                w
            });
            p.IO_BANK0.gpio(n).gpio_ctrl().write(|w| w.funcsel().i2c());
        }

        i2c.ic_enable().write(|w| w.enable().enabled());
        Ok(())
    }

    fn i2c_write(&mut self, index: u32, address: u16, data: &[u8]) -> BoardResult<()> {
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        // Length 0 is invalid-argument on the Hal side (docs/abi-spec.md §8).
        if data.is_empty() {
            return Err(ErrorCode::InvalidArgument);
        }
        let i2c = Self::i2c0();
        Self::i2c_set_target(&i2c, address)?;
        Self::i2c_send(&i2c, data, true)
    }

    fn i2c_read(&mut self, index: u32, address: u16, buf: &mut [u8]) -> BoardResult<usize> {
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        if buf.is_empty() {
            return Err(ErrorCode::InvalidArgument);
        }
        let i2c = Self::i2c0();
        Self::i2c_set_target(&i2c, address)?;
        Self::i2c_recv(&i2c, buf)
    }

    fn i2c_write_read(
        &mut self,
        index: u32,
        address: u16,
        data: &[u8],
        buf: &mut [u8],
    ) -> BoardResult<usize> {
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        if data.is_empty() || buf.is_empty() {
            return Err(ErrorCode::InvalidArgument);
        }
        let i2c = Self::i2c0();
        Self::i2c_set_target(&i2c, address)?;
        // No STOP after the first half; the next read command becomes a
        // Repeated START (`ic_restart_en` is set).
        Self::i2c_send(&i2c, data, false)?;
        Self::i2c_recv(&i2c, buf)
    }

    fn i2c_close(&mut self, index: u32) {
        if index != 0 {
            return;
        }
        let i2c = Self::i2c0();
        i2c.ic_enable().write(|w| w.enable().disabled());
        // Back to input with no pulls, as in gpio_release / spi_close (FUNCSEL
        // returns to SIO as well, so a bus left open does not hold the lines).
        for n in [I2C0_SDA, I2C0_SCL] {
            let _ = self.gpio_configure(n as u32, PinMode::Input);
        }
    }

    // --- SPI. index 0 = SPI0 (SCK=GP18, MOSI=GP19, MISO=GP16) ---

    fn spi_open(&mut self, index: u32, frequency_hz: u32, mode: SpiMode) -> BoardResult<()> {
        // spi1 is on the header too, but v0.1 only assigns spi0
        // (docs/abi-spec.md §8). The Hal has already checked index < MAX_SPI.
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        if frequency_hz == 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        // SAFETY: by the contract of Pico2Board::new, only this board touches
        // RESETS (its SPI0 bit), SPI0, IO_BANK0 and PADS_BANK0.
        let p = unsafe { pac::Peripherals::steal() };

        // SPI0 comes out of boot held in reset. Release it and wait for the
        // release to complete. Unlike GPIO this is the only place that does
        // so; without it, register writes are silently dropped.
        p.RESETS.reset().modify(|_, w| w.spi0().clear_bit());
        while p.RESETS.reset_done().read().spi0().bit_is_clear() {
            core::hint::spin_loop();
        }

        // Route SCK / MOSI / MISO to the pads. As for GPIO, `write()` starts
        // from the reset value (ISO=1, PDE=1); if ISO is not cleared, the pad
        // stays disconnected.
        for n in [SPI0_SCK, SPI0_MOSI, SPI0_MISO] {
            p.PADS_BANK0.gpio(n).write(|w| {
                // The input buffer stays on for the outputs too (the PL022
                // only reads MISO, but rp235x-hal sets it on all three).
                w.ie().set_bit();
                w.od().clear_bit();
                w.pue().clear_bit();
                w.pde().clear_bit();
                w.iso().clear_bit();
                w
            });
            p.IO_BANK0.gpio(n).gpio_ctrl().write(|w| w.funcsel().spi());
        }

        let (cpsdvsr, scr) = spi_divisors(self.peri_clock_hz, frequency_hz);
        let (spo, sph) = match mode {
            SpiMode::Mode0 => (false, false),
            SpiMode::Mode1 => (false, true),
            SpiMode::Mode2 => (true, false),
            SpiMode::Mode3 => (true, true),
        };

        let spi = p.SPI0;
        // Always disable before configuring (touching CR0 with SSE=1 is
        // undefined behavior of the PL022).
        spi.sspcr1().write(|w| w);
        spi.sspcpsr()
            // SAFETY: cpsdvsr is a u8 in 2..=254 from spi_divisors, so it fits
            // the 8-bit CPSDVSR field.
            .write(|w| unsafe { w.cpsdvsr().bits(cpsdvsr) });
        spi.sspcr0().write(|w| {
            // SAFETY: 7 fits the 4-bit DSS field and selects 8-bit frames.
            unsafe { w.dss().bits(7) };
            w.frf().motorola();
            w.spo().bit(spo);
            w.sph().bit(sph);
            // SAFETY: scr is a u8, so it fits the 8-bit SCR field.
            unsafe { w.scr().bits(scr) };
            w
        });
        // Enable as a master. The bit order is fixed at MSB first (the
        // Motorola format of the PL022 has no other; matches the premise of
        // wit/spi.wit).
        spi.sspcr1().write(|w| w.ms().clear_bit().sse().set_bit());

        // Discard anything received under the previous setup.
        Self::spi_drain(&spi);
        Ok(())
    }

    fn spi_write(&mut self, index: u32, data: &[u8]) -> BoardResult<()> {
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        let spi = Self::spi0();
        for &b in data {
            while spi.sspsr().read().tnf().bit_is_clear() {
                core::hint::spin_loop();
            }
            spi.sspdr()
                // SAFETY: frames are 8 bits, so u16::from(b) fits the 16-bit
                // DATA field.
                .write(|w| unsafe { w.data().bits(u16::from(b)) });
            // The received bytes are discarded, but drained as they come so
            // the 8-deep FIFO never overruns.
            while spi.sspsr().read().rne().bit_is_set() {
                let _ = spi.sspdr().read().bits();
            }
        }
        Self::spi_drain(&spi);
        Ok(())
    }

    fn spi_transfer(&mut self, index: u32, data: &[u8], buf: &mut [u8]) -> BoardResult<usize> {
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        let spi = Self::spi0();
        let n = data.len().min(buf.len());
        // Full duplex: send one byte, receive one byte, repeat. Filling the
        // FIFO first would be faster, but this form never has to worry about
        // dropped receive bytes (v0.1 transfers are at most 128 bytes).
        for i in 0..n {
            while spi.sspsr().read().tnf().bit_is_clear() {
                core::hint::spin_loop();
            }
            spi.sspdr()
                // SAFETY: frames are 8 bits, so u16::from(data[i]) fits the
                // 16-bit DATA field.
                .write(|w| unsafe { w.data().bits(u16::from(data[i])) });
            while spi.sspsr().read().rne().bit_is_clear() {
                core::hint::spin_loop();
            }
            buf[i] = spi.sspdr().read().data().bits() as u8;
        }
        Self::spi_drain(&spi);
        Ok(n)
    }

    fn spi_close(&mut self, index: u32) {
        if index != 0 {
            return;
        }
        let spi = Self::spi0();
        Self::spi_drain(&spi);
        spi.sspcr1().write(|w| w.sse().clear_bit());
        // Back to input with no pulls, as in gpio_release (FUNCSEL returns to
        // SIO as well).
        for n in [SPI0_SCK, SPI0_MOSI, SPI0_MISO] {
            let _ = self.gpio_configure(n as u32, PinMode::Input);
        }
    }

    // --- Assets. The slot is memory-mapped, so bodies are read in place. ---

    fn assets_read(&mut self, offset: u32, buf: &mut [u8]) -> BoardResult<()> {
        let region = self.assets.ok_or(ErrorCode::Unsupported)?;
        let at = offset as usize;
        let src = at
            .checked_add(buf.len())
            .and_then(|end| region.get(at..end))
            .ok_or(ErrorCode::Io)?;
        buf.copy_from_slice(src);
        Ok(())
    }

    fn assets_crc32(&mut self, offset: u32, len: u32) -> BoardResult<u32> {
        let region = self.assets.ok_or(ErrorCode::Unsupported)?;
        let at = offset as usize;
        let body = at
            .checked_add(len as usize)
            .and_then(|end| region.get(at..end))
            .ok_or(ErrorCode::Io)?;
        Ok(wasmicon_port::crc32(body))
    }

    // --- Time. TIMER0 runs at 1 MHz, so the count is microseconds. ---

    fn now_us(&mut self) -> u64 {
        Self::micros()
    }

    fn sleep_ms(&mut self, ms: u32) {
        self.sleep_us(ms.saturating_mul(1000));
    }

    fn sleep_us(&mut self, us: u32) {
        let start = self.now_us();
        let target = start.saturating_add(u64::from(us));
        while self.now_us() < target {
            core::hint::spin_loop();
        }
    }

    // --- Output ---

    fn log(&mut self, _level: LogLevel, message: &[u8]) {
        self.serial.write(b"[wasm] ");
        self.serial.write(message);
        self.serial.write(b"\r\n");
    }

    fn trace(&mut self, line: &[u8]) {
        self.serial.write(line);
        self.serial.write(b"\r\n");
    }
}
