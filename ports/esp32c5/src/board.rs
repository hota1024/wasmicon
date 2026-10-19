//! Board implementation for the ESP32-C5 (Waveshare ESP32-C5-WIFI6-KIT).
//!
//! Built like `board.rs` of `ports/esp32s3` (GPIO through the registers, SPI
//! / I2C through the `esp-hal` drivers); the reasons, and what is aligned
//! with `ports/rp2350`, are in that file's module docs. The C5 differs only
//! in these points:
//!
//! - 29 GPIOs, 0..=28, in a single register bank (no `enable1` / `out1` /
//!   `in1` set for GPIO 32 and up as on the S3).
//! - The IO_MUX fields and defaults (`MCU_SEL` = 1 is the GPIO function,
//!   `FUN_DRV` = 2) and the "GPIO output" signal of the GPIO matrix (256) have
//!   the same values as on the S3 (`gpio.gpio_function` and
//!   `OutputSignal::GPIO` in `esp-metadata-generated` 0.5.1; reset value
//!   0x1800 in the esp32c5 PAC 0.3).
//! - Pin assignment (docs/abi-spec.md §8): I2C0 is SDA=GPIO8 / SCL=GPIO9, SPI2
//!   is SCK=GPIO24 / MOSI=GPIO23 / MISO=GPIO5. `spi.bus` index 1 serves the
//!   on-board RGB LED (WS2812B, GPIO27) by routing only the SPI2 MOSI to
//!   GPIO27. GPIO6 is tied through 0 Ohm to the battery voltage divider on the
//!   board (per the schematic), so it is not used.
//!
//! Port in progress; hardware status in docs/TODO.md §1.3.1.

use esp_hal::gpio::AnyPin;
use esp_hal::i2c::master::{Config as HalI2cConfig, Error as HalI2cError, I2c, SoftwareTimeout};
use esp_hal::peripherals::{GPIO, I2C0, IO_MUX, SPI2};
use esp_hal::spi::master::{Config as HalSpiConfig, Spi};
use esp_hal::spi::Mode as HalSpiMode;
use esp_hal::time::{Duration, Rate};
use esp_hal::Blocking;
use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::i2c::Speed;
use wasmicon_core::generated::log::Level as LogLevel;
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_core::generated::ErrorCode;
use wasmicon_port::{Board, BoardResult};

/// The ESP32-C5 has GPIO 0..=28.
const NUM_GPIO: u32 = 29;

/// The "GPIO function" value for MCU_SEL in the IO_MUX. 1 on the ESP32-C5 as
/// well (`gpio.gpio_function` in `esp-metadata-generated`).
const MCU_SEL_GPIO: u8 = 1;

/// FUN_DRV in the IO_MUX. 2 = 20 mA, the reset value of the ESP32-C5 pads
/// (bits 10..11 of the PAC reset value 0x1800).
const FUN_DRV_DEFAULT: u8 = 2;

/// The "GPIO output" signal of the GPIO matrix (`GPIO_FUNCn_OUT_SEL`).
///
/// 256 on the ESP32-C5 (`OutputSignal::GPIO` in `esp-metadata-generated`
/// 0.5.1). The value differs per chip, so never copy it blindly: writing 128
/// on the S3 left every pin dead on hardware (see the comment on the same
/// constant in `ports/esp32s3`).
const SIG_GPIO_OUT: u16 = 256;

/// GPIOs not released to the guest.
///
/// - 11, 12: UART0, which carries the trace (wired to the CH343 on the WIFI6-KIT)
/// - 13, 14: native USB (wired to the Type-C port on the WIFI6-KIT)
/// - 15..=22: SPI flash and PSRAM; touching them while executing from XIP
///   crashes the firmware
const fn reserved(index: u32) -> bool {
    matches!(index, 11..=22)
}

// The wiring-table check (CLI and firmware, `wasmicon_port::roles`) uses the
// profile's `gpio_count` / `reserved`. If they disagreed with `NUM_GPIO` /
// `reserved` here, `pin.open` would fail on a table that passed the check, so
// they are cross-checked at compile time. `reserved` is a range test, so
// every number is compared one by one. The firmware holds no role table of
// its own (docs/app-workflow.md §3.9).
const _: () = {
    let p = &wasmicon_port::profile::ESP32C5;
    assert!(NUM_GPIO == p.gpio_count);
    let mut n = 0;
    while n < NUM_GPIO {
        let mut listed = false;
        let mut j = 0;
        while j < p.reserved.len() {
            if p.reserved[j] == n {
                listed = true;
            }
            j += 1;
        }
        assert!(
            listed == reserved(n),
            "profile::ESP32C5.reserved disagrees with reserved() in board.rs"
        );
        n += 1;
    }
};

/// Pins assigned to SPI2 (FSPI) (docs/abi-spec.md §8). CS is not included:
/// the guest toggles the `lcd-cs` GPIO itself (the design of `wit/spi.wit`).
///
/// They are not in `reserved`. Reserving them would make GPIO availability
/// differ from the host mock and the other ports, so trace comparison would
/// differ per board. If the guest opens one of these numbers through `gpio`
/// while SPI is open, the GPIO matrix routing is overwritten and SPI silently
/// stops; that counts as a guest bug (the same as on `ports/rp2350`).
const SPI2_SCK: u8 = 24;
const SPI2_MOSI: u8 = 23;
const SPI2_MISO: u8 = 5;

/// MOSI of `spi.bus` index 1. Tied through 0 Ohm to the DIN of the on-board
/// RGB LED (WS2812B; R3 on the WIFI6-KIT schematic).
///
/// SPI2 drives this pin with MOSI only (no SCK / MISO). The guest expands
/// each WS2812 bit into several SPI bits (`apps/rgb-led-rs`). The GPIO matrix
/// can route to any pin, so no wiring is needed. It shares SPI2 with index 0,
/// so the two cannot be open at the same time (the second open gets `busy`).
const SPI2_LED_MOSI: u8 = 27;

/// Pins assigned to I2C0 (docs/abi-spec.md §8). Not in `reserved`, for the
/// same reason as `SPI2_*`.
const I2C0_SDA: u8 = 8;
const I2C0_SCL: u8 = 9;

/// Where traces and logs go.
pub trait Serial {
    fn write(&mut self, bytes: &[u8]);
}

/// The ESP32-C5-WIFI6-KIT board.
pub struct EspBoard<S: Serial> {
    serial: S,
    /// The open SPI2 driver and the `index` it was opened with (0 = general
    /// purpose, 1 = RGB LED). Dropping it in `spi_close` restores the pin
    /// routing and the peripheral clock.
    spi: Option<(u32, Spi<'static, Blocking>)>,
    /// The open I2C0 driver. As for `spi`, cleanup is left to `Drop`; GPIO8/9
    /// are not reset by hand.
    i2c: Option<I2c<'static, Blocking>>,
}

impl<S: Serial> EspBoard<S> {
    /// # Safety
    /// This board must have exclusive use of GPIO / IO_MUX / SPI2 / I2C0 and
    /// of the GPIOs assigned to SPI2 and I2C0 (`SPI2_SCK` / `SPI2_MOSI` /
    /// `SPI2_MISO` / `SPI2_LED_MOSI` / `I2C0_SDA` / `I2C0_SCL`).
    #[must_use]
    pub unsafe fn new(serial: S) -> Self {
        EspBoard {
            serial,
            spi: None,
            i2c: None,
        }
    }

    /// The serial port, used to print the reason for a failure.
    pub fn serial(&mut self) -> &mut S {
        &mut self.serial
    }

    /// The SPI2 driver open under `index`. `invalid-handle` if it is not open
    /// (also when the other index holds it).
    fn open_spi(&mut self, index: u32) -> BoardResult<&mut Spi<'static, Blocking>> {
        match self.spi.as_mut() {
            Some((open, spi)) if *open == index => Ok(spi),
            _ => Err(ErrorCode::InvalidHandle),
        }
    }
}

/// Converts a 7-bit address to the `esp-hal` type.
///
/// `wit/i2c.wit` defines 7-bit addresses only; anything else is
/// `invalid-argument`. `ports/rp2350` makes the same check; only the host
/// mock does not (docs/TODO.md §2.1).
fn i2c_addr(address: u16) -> BoardResult<u8> {
    if address > 0x7f {
        return Err(ErrorCode::InvalidArgument);
    }
    Ok(address as u8)
}

/// Maps an `esp-hal` I2C error to an ABI error code (docs/abi-spec.md §7).
///
/// `ports/rp2350` makes the same split from `IC_TX_ABRT_SOURCE`: an address
/// or data NACK is `nack`, lost arbitration and the like are `io`. The host
/// mock also returns `nack` when no peer answers.
fn i2c_error(e: HalI2cError) -> ErrorCode {
    match e {
        // No peer answered. The first error to expect on hardware.
        HalI2cError::AcknowledgeCheckFailed(_) => ErrorCode::Nack,
        HalI2cError::Timeout => ErrorCode::Timeout,
        // Zero length and out-of-range addresses are rejected by the Hal and
        // the caller, but the driver may notice first.
        HalI2cError::ZeroLengthInvalid | HalI2cError::AddressInvalid(_) => {
            ErrorCode::InvalidArgument
        }
        // A transfer longer than the FIFO. The Hal caps transfers at SCRATCH
        // (128), so this cannot happen in v0.1; if it does, it is "not
        // possible on this platform".
        HalI2cError::FifoExceeded => ErrorCode::Unsupported,
        // Bus-side failures: lost arbitration, inconsistent command sequence.
        _ => ErrorCode::Io,
    }
}

/// The bit of a GPIO number. The C5 has 29 GPIOs, so one register bank is enough.
const fn mask(index: u32) -> u32 {
    1u32 << index
}

impl<S: Serial> Board for EspBoard<S> {
    fn gpio_count(&self) -> u32 {
        NUM_GPIO
    }

    fn gpio_reserved(&self, index: u32) -> bool {
        reserved(index)
    }

    fn gpio_configure(&mut self, index: u32, mode: PinMode) -> BoardResult<()> {
        if index >= NUM_GPIO {
            return Err(ErrorCode::InvalidArgument);
        }
        let n = index as usize;
        // SAFETY: by the contract of EspBoard::new, only this board touches
        // GPIO / IO_MUX.
        let (gpio, io_mux) = unsafe { (GPIO::steal(), IO_MUX::steal()) };
        let g = gpio.register_block();
        let m = io_mux.register_block();

        // IO_MUX: select the GPIO function and set the input buffer and pulls.
        //
        // `write()` resets every field not named. That is deliberate, so that
        // a reopen reliably drops the pulls, but the drive strength is set
        // explicitly rather than left to the reset value (no dependence on
        // the SVD's reset value). 2 = 20 mA, which is also the pad default on
        // the ESP32-C5.
        m.gpio(n).write(|w| {
            // SAFETY: MCU_SEL_GPIO (1) fits the 3-bit MCU_SEL field.
            unsafe { w.mcu_sel().bits(MCU_SEL_GPIO) };
            // SAFETY: FUN_DRV_DEFAULT (2) fits the 2-bit FUN_DRV field.
            unsafe { w.fun_drv().bits(FUN_DRV_DEFAULT) };
            w.fun_ie()
                .bit(!matches!(mode, PinMode::Output | PinMode::OutputOpenDrain));
            w.fun_wpu().bit(matches!(mode, PinMode::InputPullUp));
            w.fun_wpd().bit(matches!(mode, PinMode::InputPullDown));
            w
        });

        // GPIO matrix: route the output to the GPIO_OUT signal.
        g.func_out_sel_cfg(n)
            // SAFETY: SIG_GPIO_OUT (256) fits the 9-bit OUT_SEL field.
            .write(|w| unsafe { w.out_sel().bits(SIG_GPIO_OUT) });

        // Open drain is switched with PIN_PAD_DRIVER.
        g.pin(n)
            .modify(|_, w| w.pad_driver().bit(matches!(mode, PinMode::OutputOpenDrain)));

        let mask = mask(index);
        match mode {
            PinMode::Output | PinMode::OutputOpenDrain => {
                // SAFETY: index < NUM_GPIO (29), so mask is one bit of the
                // 32-bit ENABLE_W1TS field.
                g.enable_w1ts().write(|w| unsafe { w.bits(mask) });
            }
            _ => {
                // SAFETY: index < NUM_GPIO (29), so mask is one bit of the
                // 32-bit ENABLE_W1TC field.
                g.enable_w1tc().write(|w| unsafe { w.bits(mask) });
            }
        }
        Ok(())
    }

    fn gpio_write(&mut self, index: u32, level: Level) -> BoardResult<()> {
        if index >= NUM_GPIO {
            return Err(ErrorCode::InvalidArgument);
        }
        // SAFETY: by the contract of EspBoard::new, only this board touches GPIO.
        let gpio = unsafe { GPIO::steal() };
        let g = gpio.register_block();
        let mask = mask(index);
        if g.enable().read().bits() & mask == 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        match level {
            // SAFETY: index < NUM_GPIO (29), so mask is one bit of the 32-bit
            // OUT_W1TS field.
            Level::High => g.out_w1ts().write(|w| unsafe { w.bits(mask) }),
            // SAFETY: index < NUM_GPIO (29), so mask is one bit of the 32-bit
            // OUT_W1TC field.
            Level::Low => g.out_w1tc().write(|w| unsafe { w.bits(mask) }),
        };
        Ok(())
    }

    fn gpio_read(&mut self, index: u32) -> BoardResult<Level> {
        if index >= NUM_GPIO {
            return Err(ErrorCode::InvalidArgument);
        }
        // SAFETY: by the contract of EspBoard::new, only this board touches GPIO.
        let gpio = unsafe { GPIO::steal() };
        let g = gpio.register_block();
        let mask = mask(index);
        let (enable, out, input) = (
            g.enable().read().bits(),
            g.out().read().bits(),
            g.in_().read().bits(),
        );
        // Output mode reads the output value, input mode the input value (the
        // definition of read in the WIT).
        let bits = if enable & mask != 0 { out } else { input };
        Ok(if bits & mask != 0 {
            Level::High
        } else {
            Level::Low
        })
    }

    fn gpio_release(&mut self, index: u32) {
        // docs/abi-spec.md §5.2: back to input with no pulls.
        //
        // This lets the LCD `RESET` float. Seen on the ESP32-S3: once the demo
        // has finished drawing and the handles are released, `lcd-rst` goes
        // back to input, the line floats, and with no pull-up on the module
        // the panel resets and the screen turns white. It is not specific to
        // one port (a consequence of docs/abi-spec.md §5.2; `ports/rp2350`
        // does the same under the same conditions). The idle level of the
        // line is the external circuit's job: a pull-up of about 10 kOhm on
        // `RESET` fixes it (docs/verification-report.md §7,
        // apps/lcd-demo-rs/README.md).
        let _ = self.gpio_configure(index, PinMode::Input);
    }

    // --- I2C. index 0 = I2C0 (SDA=GPIO8, SCL=GPIO9) ---

    fn i2c_open(&mut self, index: u32, speed: Speed) -> BoardResult<()> {
        // There is an I2C1 too, but v0.1 only assigns I2C0
        // (docs/abi-spec.md §8). The Hal has already checked index < MAX_I2C.
        // Same test as for SPI.
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        // The Hal rejects a double open with Busy, so normally nothing is
        // left here; if something is, drop it first (never hold two drivers
        // for a stolen peripheral).
        self.i2c = None;

        let hz = match speed {
            Speed::Standard => 100_000,
            Speed::Fast => 400_000,
            Speed::FastPlus => 1_000_000,
        };
        // The timeout is set explicitly. `Config::default()` has
        // `software_timeout: SoftwareTimeout::None` (`i2c/master/mod.rs` in
        // esp-hal 1.2.1), so there is no software bound. Only the FSM's
        // hardware timeout (2^23 bus cycles by default, about 0.2 s) would
        // remain, so a bus with SCL stuck low would take orders of magnitude
        // longer to fail than on `ports/rp2350`. `PerByte` gives the same
        // budget as `I2C_TIMEOUT_US` on the rp2350 (25 ms per byte).
        //
        // Change both or neither: `I2C_TIMEOUT_US` in
        // `ports/rp2350/src/board.rs` (its doc comment explains the value).
        const I2C_TIMEOUT_MS: u64 = 25;
        let config = HalI2cConfig::default()
            .with_frequency(Rate::from_hz(hz))
            .with_software_timeout(SoftwareTimeout::PerByte(Duration::from_millis(
                I2C_TIMEOUT_MS,
            )));

        // SAFETY: by the contract of EspBoard::new, only this board touches
        // I2C0 and the I2C0_* pins. self.i2c was set to None above, so no two
        // drivers refer to the same peripheral at once.
        let (i2c0, sda, scl) = unsafe {
            (
                I2C0::steal(),
                AnyPin::steal(I2C0_SDA),
                AnyPin::steal(I2C0_SCL),
            )
        };
        // `with_sda` / `with_scl` apply open drain and the internal pull-up.
        // The module docs of `ports/esp32s3` say why an external 10 kOhm is
        // still needed.
        let i2c = I2c::new(i2c0, config)
            .map_err(|_| ErrorCode::Unsupported)?
            .with_sda(sda)
            .with_scl(scl);
        self.i2c = Some(i2c);
        Ok(())
    }

    fn i2c_write(&mut self, index: u32, address: u16, data: &[u8]) -> BoardResult<()> {
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        // Length 0 is invalid-argument on the Hal side, but it is rejected
        // here as well. `transaction_impl` in `esp-hal` drops empty reads
        // from the transfer (`.filter(|op| op.is_write() || !op.is_empty())`),
        // so left to the driver a zero-length read would return `Ok(0)` and
        // disagree with `ports/rp2350`. All three methods make the same test.
        if data.is_empty() {
            return Err(ErrorCode::InvalidArgument);
        }
        let addr = i2c_addr(address)?;
        let bus = self.i2c.as_mut().ok_or(ErrorCode::InvalidHandle)?;
        bus.write(addr, data).map_err(i2c_error)
    }

    fn i2c_read(&mut self, index: u32, address: u16, buf: &mut [u8]) -> BoardResult<usize> {
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        if buf.is_empty() {
            return Err(ErrorCode::InvalidArgument);
        }
        let addr = i2c_addr(address)?;
        let n = buf.len();
        let bus = self.i2c.as_mut().ok_or(ErrorCode::InvalidHandle)?;
        bus.read(addr, buf).map_err(i2c_error)?;
        Ok(n)
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
        let addr = i2c_addr(address)?;
        let n = buf.len();
        let bus = self.i2c.as_mut().ok_or(ErrorCode::InvalidHandle)?;
        // write -> Repeated START -> read as one transaction.
        //
        // The SHT4x cannot be read this way: after the measurement command
        // it needs a STOP and the conversion time, so the guest splits it
        // into `write` / `sleep-ms` / `read` (apps/README.md §1). Nothing in
        // v0.1 uses `write-read` (the ILI9341 ID read is on the SPI side).
        bus.write_read(addr, data, buf).map_err(i2c_error)?;
        Ok(n)
    }

    fn i2c_close(&mut self, index: u32) {
        if index != 0 {
            return;
        }
        // `Drop` restores the pin routing and the peripheral clock. As in
        // `spi_close`, GPIO8/9 are not touched by hand.
        self.i2c = None;
    }

    // --- SPI. Both indexes use SPI2, so they cannot be open at the same time ---
    //   index 0 = general purpose (SCK=GPIO24, MOSI=GPIO23, MISO=GPIO5)
    //   index 1 = on-board RGB LED (MOSI=GPIO27 only)

    fn spi_open(&mut self, index: u32, frequency_hz: u32, mode: SpiMode) -> BoardResult<()> {
        // SPI2 is the only general-purpose SPI (SPI0/1 serve the flash). The
        // Hal has already checked index < MAX_SPI.
        if index > 1 {
            return Err(ErrorCode::Unsupported);
        }
        if frequency_hz == 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        // `busy` if the other index holds SPI2. The Hal rejects a double open
        // of the same index with `busy`, so whatever is left here belongs to
        // the other index.
        if let Some((open, _)) = self.spi {
            if open != index {
                return Err(ErrorCode::Busy);
            }
        }
        // Drop a leftover first (never hold two drivers for a stolen
        // peripheral).
        self.spi = None;

        let config = HalSpiConfig::default()
            .with_frequency(Rate::from_hz(frequency_hz))
            .with_mode(match mode {
                SpiMode::Mode0 => HalSpiMode::_0,
                SpiMode::Mode1 => HalSpiMode::_1,
                SpiMode::Mode2 => HalSpiMode::_2,
                SpiMode::Mode3 => HalSpiMode::_3,
            });

        // SAFETY: by the contract of EspBoard::new, only this board touches
        // SPI2. self.spi was set to None above, so no two drivers refer to
        // the same peripheral at once.
        let spi2 = unsafe { SPI2::steal() };
        // The only failure is a requested frequency outside the reachable
        // range.
        let spi = Spi::new(spi2, config).map_err(|_| ErrorCode::Unsupported)?;
        let spi = if index == 0 {
            // SAFETY: by the contract of EspBoard::new, only this board
            // touches the SPI2_SCK / SPI2_MOSI / SPI2_MISO pins, and no driver
            // holds them while self.spi is None.
            let (sck, mosi, miso) = unsafe {
                (
                    AnyPin::steal(SPI2_SCK),
                    AnyPin::steal(SPI2_MOSI),
                    AnyPin::steal(SPI2_MISO),
                )
            };
            spi.with_sck(sck).with_mosi(mosi).with_miso(miso)
        } else {
            // SAFETY: by the contract of EspBoard::new, only this board
            // touches the SPI2_LED_MOSI pin, and no driver holds it while
            // self.spi is None.
            let mosi = unsafe { AnyPin::steal(SPI2_LED_MOSI) };
            spi.with_mosi(mosi)
        };
        self.spi = Some((index, spi));
        Ok(())
    }

    fn spi_write(&mut self, index: u32, data: &[u8]) -> BoardResult<()> {
        if index > 1 {
            return Err(ErrorCode::Unsupported);
        }
        let spi = self.open_spi(index)?;
        // Split every 64 bytes (the FIFO), but the guest keeps CS held, so to
        // the ILI9341 it is still one transfer. A flush follows each chunk.
        spi.write(data).map_err(|_| ErrorCode::Io)
    }

    fn spi_transfer(&mut self, index: u32, data: &[u8], buf: &mut [u8]) -> BoardResult<usize> {
        if index > 1 {
            return Err(ErrorCode::Unsupported);
        }
        let spi = self.open_spi(index)?;
        let n = data.len().min(buf.len());
        // The driver's full-duplex transfer works in place, so the outgoing
        // bytes are copied in first.
        buf[..n].copy_from_slice(&data[..n]);
        spi.transfer(&mut buf[..n]).map_err(|_| ErrorCode::Io)?;
        Ok(n)
    }

    fn spi_close(&mut self, index: u32) {
        if !matches!(self.spi, Some((open, _)) if open == index) {
            return;
        }
        // Drop restores the pin routing and the peripheral clock.
        self.spi = None;
        // Back to input with no pulls, as in gpio_release. The driver's Drop
        // only disconnects the GPIO matrix signals; it does not touch the pad
        // direction or pulls.
        let pins: &[u8] = if index == 0 {
            &[SPI2_SCK, SPI2_MOSI, SPI2_MISO]
        } else {
            &[SPI2_LED_MOSI]
        };
        for &n in pins {
            let _ = self.gpio_configure(u32::from(n), PinMode::Input);
        }
    }

    // --- Time ---

    fn now_us(&mut self) -> u64 {
        esp_hal::time::Instant::now()
            .duration_since_epoch()
            .as_micros()
    }

    fn sleep_ms(&mut self, ms: u32) {
        self.sleep_us(ms.saturating_mul(1000));
    }

    fn sleep_us(&mut self, us: u32) {
        let target = self.now_us().saturating_add(u64::from(us));
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
