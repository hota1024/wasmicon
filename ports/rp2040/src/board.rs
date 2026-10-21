//! Board implementation for the Raspberry Pi Pico WH.
//!
//! GPIOs are addressed by number at run time, so instead of typed pins this
//! drives the SIO / IO_BANK0 / PADS_BANK0 registers directly. The shared part
//! of the HAL lives in `wasmicon-port`.

use rp2040_hal::pac;
use wasmicon_core::generated::ErrorCode;
use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::i2c::Speed;
use wasmicon_core::generated::log::Level as LogLevel;
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_port::{Board, BoardResult};

/// The RP2040 has GP0..GP29.
const NUM_GPIO: u32 = 30;

// The wiring-table check (CLI and firmware, `wasmicon_port::roles`) uses the
// profile's `gpio_count` / `reserved`. If they disagreed with the values here,
// `pin.open` would fail on a table that passed the check, so they are
// cross-checked at compile time. The firmware holds no role table of its own
// (docs/app-workflow.md §3.9).
const _: () = assert!(NUM_GPIO == wasmicon_port::profile::RP2040.gpio_count);
const _: () = assert!(wasmicon_port::profile::same_pins(
    RESERVED,
    wasmicon_port::profile::RP2040.reserved
));

// Ties the profile's "implemented interfaces" to the `unsupported` stubs in
// the `Board` impl below. Fixing only one side fails here.
const _: () = {
    let i = wasmicon_port::profile::RP2040.interfaces;
    assert!(
        !i.i2c && !i.spi,
        "set profile::RP2040.interfaces.i2c / .spi once I2C / SPI are implemented"
    );
    assert!(i.gpio && i.time && i.log && i.board);
};

/// SIO FUNCSEL value: software-controlled GPIO.
const FUNCSEL_SIO: u8 = 5;

/// GPIOs not released to the guest.
///
/// GP0/GP1 are UART0, which carries the trace; opening them would cut it off.
/// GP23/24/25/29 connect to the CYW43439 (radio and on-board LED) on the
/// Pico W(H).
const RESERVED: &[u32] = &[0, 1, 23, 24, 25, 29];

/// Where traces and logs go.
pub trait Serial {
    fn write(&mut self, bytes: &[u8]);
}

/// The Pico board.
pub struct PicoBoard<S: Serial> {
    serial: S,
    /// Pins open as open-drain outputs.
    ///
    /// RP2040 pads have no open-drain control (`OD` is output disable), so it
    /// is emulated with output enable: low is OE=1 and OUT=0, high is OE=0
    /// (high impedance).
    open_drain: u32,
    /// The assets slot as memory-mapped flash (docs/app-workflow.md §3.10).
    /// `None` until `set_assets`, and when the slot could not be read.
    assets: Option<&'static [u8]>,
}

impl<S: Serial> PicoBoard<S> {
    /// # Safety
    /// This board must have exclusive use of SIO / IO_BANK0 / PADS_BANK0 /
    /// TIMER. The caller must not touch those peripherals anywhere else.
    #[must_use]
    pub unsafe fn new(serial: S) -> Self {
        PicoBoard {
            serial,
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
        // SAFETY: by the contract of PicoBoard::new, only this board touches SIO.
        unsafe { pac::Peripherals::steal().SIO }
    }

    fn timer() -> pac::TIMER {
        // SAFETY: by the contract of PicoBoard::new, only this board touches
        // TIMER, and it is only read.
        unsafe { pac::Peripherals::steal().TIMER }
    }
}

impl<S: Serial> Board for PicoBoard<S> {
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
        // SAFETY: by the contract of PicoBoard::new, only this board touches
        // SIO / IO_BANK0 / PADS_BANK0.
        let p = unsafe { pac::Peripherals::steal() };

        // Pad setup: input buffer, pulls, output disable.
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
            w
        });

        // Hand the pin to software control.
        p.IO_BANK0
            .gpio(n)
            .gpio_ctrl()
            // SAFETY: FUNCSEL_SIO (5) fits the 5-bit FUNCSEL field.
            .write(|w| unsafe { w.funcsel().bits(FUNCSEL_SIO) });

        let mask = 1u32 << index;
        match mode {
            PinMode::OutputOpenDrain => {
                // Start in high impedance; the external pull-up holds the line.
                self.open_drain |= mask;
                // SAFETY: index < NUM_GPIO (30), so mask is one bit of the
                // 30-bit GPIO_OUT_CLR field.
                p.SIO.gpio_out_clr().write(|w| unsafe { w.bits(mask) });
                // SAFETY: index < NUM_GPIO (30), so mask is one bit of the
                // 30-bit GPIO_OE_CLR field.
                p.SIO.gpio_oe_clr().write(|w| unsafe { w.bits(mask) });
            }
            PinMode::Output => {
                self.open_drain &= !mask;
                // SAFETY: index < NUM_GPIO (30), so mask is one bit of the
                // 30-bit GPIO_OE_SET field.
                p.SIO.gpio_oe_set().write(|w| unsafe { w.bits(mask) });
            }
            _ => {
                self.open_drain &= !mask;
                // SAFETY: index < NUM_GPIO (30), so mask is one bit of the
                // 30-bit GPIO_OE_CLR field.
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
                    // 30-bit GPIO_OUT_CLR field.
                    sio.gpio_out_clr().write(|w| unsafe { w.bits(mask) });
                    // SAFETY: index < NUM_GPIO (30), so mask is one bit of the
                    // 30-bit GPIO_OE_SET field.
                    sio.gpio_oe_set().write(|w| unsafe { w.bits(mask) });
                }
                // SAFETY: index < NUM_GPIO (30), so mask is one bit of the
                // 30-bit GPIO_OE_CLR field.
                Level::High => sio.gpio_oe_clr().write(|w| unsafe { w.bits(mask) }),
            }
            return Ok(());
        }
        // A pin that is not configured as an output cannot be written.
        if sio.gpio_oe().read().bits() & mask == 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        match level {
            // SAFETY: index < NUM_GPIO (30), so mask is one bit of the 30-bit
            // GPIO_OUT_SET field.
            Level::High => sio.gpio_out_set().write(|w| unsafe { w.bits(mask) }),
            // SAFETY: index < NUM_GPIO (30), so mask is one bit of the 30-bit
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

    // --- I2C / SPI are not implemented on this port ---
    //
    // When they are, also set i2c / spi in `profile::RP2040.interfaces`. The
    // const above ties the two together, so fixing only one side fails to
    // compile. While they disagree, `wasmicon check --board rp2040` keeps
    // rejecting working apps as "not implemented on this port"
    // (docs/TODO.md §1.2).

    fn i2c_open(&mut self, _index: u32, _speed: Speed) -> BoardResult<()> {
        Err(ErrorCode::Unsupported)
    }
    fn i2c_write(&mut self, _index: u32, _address: u16, _data: &[u8]) -> BoardResult<()> {
        Err(ErrorCode::Unsupported)
    }
    fn i2c_read(&mut self, _index: u32, _address: u16, _buf: &mut [u8]) -> BoardResult<usize> {
        Err(ErrorCode::Unsupported)
    }
    fn i2c_write_read(
        &mut self,
        _index: u32,
        _address: u16,
        _data: &[u8],
        _buf: &mut [u8],
    ) -> BoardResult<usize> {
        Err(ErrorCode::Unsupported)
    }
    fn i2c_close(&mut self, _index: u32) {}

    fn spi_open(&mut self, _index: u32, _frequency_hz: u32, _mode: SpiMode) -> BoardResult<()> {
        Err(ErrorCode::Unsupported)
    }
    fn spi_write(&mut self, _index: u32, _data: &[u8]) -> BoardResult<()> {
        Err(ErrorCode::Unsupported)
    }
    fn spi_transfer(&mut self, _index: u32, _data: &[u8], _buf: &mut [u8]) -> BoardResult<usize> {
        Err(ErrorCode::Unsupported)
    }
    fn spi_close(&mut self, _index: u32) {}

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

    // --- Time. TIMER runs at 1 MHz, so the count is microseconds. ---

    fn now_us(&mut self) -> u64 {
        let t = Self::timer();
        // Reading TIMELR latches TIMEHR, so the order matters.
        let lo = t.timelr().read().bits();
        let hi = t.timehr().read().bits();
        (u64::from(hi) << 32) | u64::from(lo)
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
