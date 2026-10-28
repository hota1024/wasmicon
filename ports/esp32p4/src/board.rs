//! Board implementation for the ESP32-P4.
//!
//! Built like `board.rs` of `ports/esp32c5` (GPIO through the registers, SPI
//! / I2C through the `esp-hal` drivers); the reasons, and what is aligned
//! with `ports/rp2350`, are in the module docs of `ports/esp32s3`. The P4
//! differs only in these points:
//!
//! - 55 GPIOs, 0..=54, in two register banks: GPIO 0..=31 in `enable` /
//!   `out` / `in`, GPIO 32..=54 in `enable1` / `out1` / `in1` (as on the S3).
//! - `MCU_SEL` = 1 is the GPIO function and the "GPIO output" signal of the
//!   GPIO matrix is 256, the same values as on the S3 and C5
//!   (`gpio.gpio_function` and `OutputSignal::GPIO` in
//!   `esp-metadata-generated` 0.5.3).
//! - Not tied to one board. Only the chip's own pins are reserved; the bus
//!   pins are free on the M5Stack Tab5 (docs/abi-spec.md §8): I2C0 is
//!   SDA=GPIO54 / SCL=GPIO53, SPI2 is SCK=GPIO5 / MOSI=GPIO18 / MISO=GPIO19.
//!   There is no on-board RGB LED to serve, so `spi.bus` index 1 is
//!   `unsupported`.
//!
//! Port in progress; hardware status in docs/TODO.md §1.3.2.

use esp_hal::gpio::{AnyPin, Flex};
use esp_hal::i2c::master::{Config as HalI2cConfig, Error as HalI2cError, I2c, SoftwareTimeout};
use esp_hal::peripherals::{GPIO, I2C0, IO_MUX, SPI2};
use esp_hal::spi::master::{Config as HalSpiConfig, Spi};
use esp_hal::spi::Mode as HalSpiMode;
use esp_hal::time::{Duration, Rate};
use esp_hal::Blocking;
use esp_storage::FlashStorage;
use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::i2c::Speed;
use wasmicon_core::generated::log::Level as LogLevel;
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_core::generated::ErrorCode;
use wasmicon_port::fmt::Buf;
use wasmicon_port::{Board, BoardResult, Rect};

use crate::tab5::{self, Tab5};

/// The ESP32-P4 has GPIO 0..=54.
const NUM_GPIO: u32 = 55;

/// The "GPIO function" value for MCU_SEL in the IO_MUX. 1 on the ESP32-P4 as
/// well (`gpio.gpio_function` in `esp-metadata-generated`).
const MCU_SEL_GPIO: u8 = 1;

/// FUN_DRV in the IO_MUX. 2 = 20 mA, the drive strength the S3 and C5 ports
/// use. Set explicitly: the P4 SVD gives the register a reset value of 0,
/// which is not the pad default.
const FUN_DRV_DEFAULT: u8 = 2;

/// The "GPIO output" signal of the GPIO matrix (`GPIO_FUNCn_OUT_SEL`).
///
/// 256 on the ESP32-P4 (`OutputSignal::GPIO` in `esp-metadata-generated`
/// 0.5.3). The value differs per chip, so never copy it blindly: writing 128
/// on the S3 left every pin dead on hardware (see the comment on the same
/// constant in `ports/esp32s3`).
const SIG_GPIO_OUT: u16 = 256;

/// GPIOs not released to the guest. Only the chip's own uses:
///
/// - 24, 25: the USB-Serial-JTAG pads ("may be used to debug the chip using
///   USB" in `esp-metadata-generated`)
/// - 37, 38: UART0, which carries the trace
///
/// Flash and the in-package PSRAM have dedicated pins outside 0..=54. What a
/// board wires to the other pins (on the Tab5: the Wi-Fi chip, the internal
/// I2C, the SD card) is left open; keeping the wiring table off them is the
/// wiring's job (docs/TODO.md §1.3.2).
const fn reserved(index: u32) -> bool {
    matches!(index, 24 | 25 | 37 | 38)
}

// The wiring-table check (CLI and firmware, `wasmicon_port::roles`) uses the
// profile's `gpio_count` / `reserved`. If they disagreed with `NUM_GPIO` /
// `reserved` here, `pin.open` would fail on a table that passed the check, so
// they are cross-checked at compile time. `reserved` is a match, so every
// number is compared one by one. The firmware holds no role table of its own
// (docs/app-workflow.md §3.9).
const _: () = {
    let p = &wasmicon_port::profile::ESP32P4;
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
            "profile::ESP32P4.reserved disagrees with reserved() in board.rs"
        );
        n += 1;
    }
};

/// Pins assigned to SPI2 (docs/abi-spec.md §8). CS is not included: the
/// guest toggles the `lcd-cs` GPIO itself (the design of `wit/spi.wit`).
///
/// They are not in `reserved`. Reserving them would make GPIO availability
/// differ from the host mock and the other ports, so trace comparison would
/// differ per board. If the guest opens one of these numbers through `gpio`
/// while SPI is open, the GPIO matrix routing is overwritten and SPI silently
/// stops; that counts as a guest bug (the same as on `ports/rp2350`).
const SPI2_SCK: u8 = 5;
const SPI2_MOSI: u8 = 18;
const SPI2_MISO: u8 = 19;

/// Pins assigned to I2C0 (docs/abi-spec.md §8). Not in `reserved`, for the
/// same reason as `SPI2_*`. On the Tab5 these are PORT.A; which of the two is
/// SDA there is not settled (docs/TODO.md §1.3.2).
const I2C0_SDA: u8 = 54;
const I2C0_SCL: u8 = 53;

// The bus pins above must be the profile's `bus_pins`, or the wiring-table
// check would let a role land on a bus pin.
const _: () = {
    let bus = wasmicon_port::profile::ESP32P4.bus_pins;
    let ours = [SPI2_SCK, SPI2_MOSI, SPI2_MISO, I2C0_SDA, I2C0_SCL];
    assert!(bus.len() == ours.len());
    let mut i = 0;
    while i < ours.len() {
        let mut listed = false;
        let mut j = 0;
        while j < bus.len() {
            if bus[j] == ours[i] as u32 {
                listed = true;
            }
            j += 1;
        }
        assert!(
            listed,
            "profile::ESP32P4.bus_pins disagrees with the bus pins in board.rs"
        );
        i += 1;
    }
};

/// Where traces and logs go.
pub trait Serial {
    fn write(&mut self, bytes: &[u8]);
}

/// An ESP32-P4 board.
pub struct EspBoard<S: Serial> {
    serial: S,
    /// Flash, for the guest's asset reads (docs/app-workflow.md §3.10).
    /// `main` reads the app and config slots through it at boot as well
    /// (`parts`).
    storage: FlashStorage<'static>,
    /// Flash offset of the assets slot; `assets_read` offsets count from it.
    assets_base: u32,
    /// The open SPI2 driver. Dropping it in `spi_close` restores the pin
    /// routing and the peripheral clock.
    spi: Option<Spi<'static, Blocking>>,
    /// The open I2C0 driver. As for `spi`, cleanup is left to `Drop`; the
    /// pins are not reset by hand.
    i2c: Option<I2c<'static, Blocking>>,
    /// The Tab5's own panel (`tab5`), when the config slot names it.
    tab5: Tab5State,
}

/// Where the Tab5 panel is: its parts are held from boot (PSRAM up) and taken
/// at the guest's first `open`; the panel, once up, is never dropped.
enum Tab5State {
    /// Not configured.
    None,
    Pending(tab5::Pending),
    Up(Tab5),
    /// Bring-up failed (the reason was printed); `open` returns `io`.
    Failed,
}

impl<S: Serial> EspBoard<S> {
    /// # Safety
    /// This board must have exclusive use of GPIO / IO_MUX / SPI2 / I2C0 and
    /// of the GPIOs assigned to SPI2 and I2C0 (`SPI2_SCK` / `SPI2_MOSI` /
    /// `SPI2_MISO` / `I2C0_SDA` / `I2C0_SCL`). The one exception is the Tab5
    /// panel's parts, if `configure_tab5` hands them over: its esp-hal drivers
    /// then write the GPIO / IO_MUX registers of GPIO22, 31 and 32, which the
    /// guest cannot open while the display is configured (`Hal` reserves
    /// `DisplayConfig::pins`).
    #[must_use]
    pub unsafe fn new(serial: S, storage: FlashStorage<'static>, assets_base: u32) -> Self {
        EspBoard {
            serial,
            storage,
            assets_base,
            spi: None,
            i2c: None,
            tab5: Tab5State::None,
        }
    }

    /// Makes the Tab5 panel display 0 and brings PSRAM up for it, before the
    /// app runs. The panel comes up at the guest's first `open`
    /// (`display_open`). A PSRAM failure is printed here and returned by
    /// that `open`.
    pub fn configure_tab5(&mut self, parts: tab5::Parts) {
        self.tab5 = match tab5::Pending::new(parts) {
            Ok(pending) => Tab5State::Pending(pending),
            Err(e) => {
                self.print_tab5_failure(e);
                Tab5State::Failed
            }
        };
    }

    fn print_tab5_failure(&mut self, e: tab5::UpError) {
        let mut line = [0u8; 128];
        let mut out = Buf::new(&mut line);
        out.str("wasmicon: display tab5 failed: ");
        e.describe(&mut out);
        self.serial.write(out.as_bytes());
        self.serial.write(b"\r\n");
    }

    fn tab5_up(&mut self) -> BoardResult<&mut Tab5> {
        match &mut self.tab5 {
            Tab5State::Up(panel) => Ok(panel),
            _ => Err(ErrorCode::Unsupported),
        }
    }

    /// The serial port, used to print the reason for a failure.
    pub fn serial(&mut self) -> &mut S {
        &mut self.serial
    }

    /// The flash and the serial port together, for the slot reads at boot
    /// (they print what they find).
    pub fn parts(&mut self) -> (&mut FlashStorage<'static>, &mut S) {
        (&mut self.storage, &mut self.serial)
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

/// A word-aligned window for the ragged edges of `read_flash`.
#[repr(align(4))]
struct Window([u8; 256]);

/// Reads `buf.len()` bytes of flash from `at` into `buf`.
///
/// Not `FlashStorage::read`: that puts a 4 KiB sector buffer on the stack on
/// every call, and the guest's asset reads happen while the interpreter is
/// running, with the native stack already in use. `read_nor` wants a
/// word-aligned flash offset and length and reads straight into a
/// word-aligned destination, so an aligned request goes straight into the
/// guest's memory and anything else goes through a 256-byte window. The same
/// as on the ESP32-S3 and C5.
fn read_flash(storage: &mut FlashStorage<'static>, at: u32, buf: &mut [u8]) -> BoardResult<()> {
    const WORD: u32 = 4;
    const MASK: usize = WORD as usize - 1;
    let aligned =
        (at as usize) & MASK == 0 && buf.len() & MASK == 0 && (buf.as_ptr() as usize) & MASK == 0;
    if aligned {
        return storage.read_nor(at, buf).map_err(|_| ErrorCode::Io);
    }
    let cap = u32::try_from(storage.capacity()).unwrap_or(u32::MAX);
    let mut window = Window([0; 256]);
    let mut done = 0usize;
    while done < buf.len() {
        let pos = at.checked_add(done as u32).ok_or(ErrorCode::Io)?;
        let start = pos & !(WORD - 1);
        let skip = (pos - start) as usize;
        let want = (buf.len() - done).min(window.0.len() - skip);
        // The span is rounded up to whole words and clipped to the flash, so
        // the padding never reads past its end (the wanted bytes are inside
        // the slot, which ends with the flash).
        let span = ((skip + want + WORD as usize - 1) & !(WORD as usize - 1))
            .min(cap.saturating_sub(start) as usize);
        if span < skip + want {
            return Err(ErrorCode::Io);
        }
        storage
            .read_nor(start, &mut window.0[..span])
            .map_err(|_| ErrorCode::Io)?;
        buf[done..done + want].copy_from_slice(&window.0[skip..skip + want]);
        done += want;
    }
    Ok(())
}

/// Which register bank a GPIO number belongs to, and its bit in that bank.
/// GPIO 0..=31 are bank 0, 32..=54 bank 1.
const fn bank(index: u32) -> (bool, u32) {
    if index < 32 {
        (false, 1u32 << index)
    } else {
        (true, 1u32 << (index - 32))
    }
}

impl<S: Serial> Board for EspBoard<S> {
    fn display_supported(&self) -> bool {
        // The ILI9341 backend in ports/common runs on this port's SPI, and the
        // Tab5 panel is the board's own (docs/device-layer.md §5.3); the
        // profile says whether there is either.
        wasmicon_port::profile::ESP32P4.interfaces.display
    }

    // The Tab5 panel. `Hal` asks the board only when no ILI9341 is configured.
    fn display_size(&self, index: u32) -> Option<(u32, u32)> {
        match self.tab5 {
            Tab5State::None => None,
            _ if index != 0 => None,
            _ => Some(Tab5::SIZE),
        }
    }

    fn display_open(&mut self, _index: u32) -> BoardResult<()> {
        let pending = match core::mem::replace(&mut self.tab5, Tab5State::Failed) {
            Tab5State::Pending(pending) => pending,
            Tab5State::Failed => return Err(ErrorCode::Io),
            other => {
                self.tab5 = other;
                return Ok(());
            }
        };
        // Printed first: a bring-up that hangs inside esp-hal (its D-PHY
        // waits have no limit) still shows where.
        self.serial.write(b"wasmicon: display tab5 bring-up\r\n");
        match Tab5::up(pending) {
            Ok((panel, ldo)) => {
                self.tab5 = Tab5State::Up(panel);
                let mut line = [0u8; 64];
                let mut out = Buf::new(&mut line);
                out.str("wasmicon: display tab5 up, d-phy ldo dref=");
                out.u32(u32::from(ldo.dref));
                out.str(" mul=");
                out.u32(u32::from(ldo.mul));
                self.serial.write(out.as_bytes());
                self.serial.write(b"\r\n");
                Ok(())
            }
            Err(e) => {
                self.print_tab5_failure(e);
                Err(ErrorCode::Io)
            }
        }
    }

    fn display_fill_rect(&mut self, _index: u32, rect: Rect, color: u16) -> BoardResult<()> {
        self.tab5_up()?.fill_rect(rect, color);
        Ok(())
    }

    fn display_write_rect(&mut self, _index: u32, rect: Rect, pixels: &[u8]) -> BoardResult<()> {
        self.tab5_up()?.write_rect(rect, pixels);
        Ok(())
    }

    fn display_present(&mut self, _index: u32) -> BoardResult<()> {
        self.tab5_up()?.present();
        Ok(())
    }

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

        // Reset the pad the way esp-hal does before any use (`Flex::new` runs
        // its `init_gpio`): it releases a hold the low-power domain may still
        // keep on the pad, switches an LP pad back to the digital side, and
        // turns off the USB pull resistors on the USB pads. The register
        // writes below do none of that. `Flex` has no `Drop`, so letting it go
        // leaves the pad as is.
        //
        // SAFETY: by the contract of EspBoard::new, only this board touches
        // the GPIOs it opens, and `index < NUM_GPIO` (55) was checked above,
        // so the number fits in a `u8` and names an existing pad.
        let _ = Flex::new(unsafe { AnyPin::steal(index as u8) });

        // SAFETY: by the contract of EspBoard::new, only this board touches
        // GPIO / IO_MUX.
        let (gpio, io_mux) = unsafe { (GPIO::steal(), IO_MUX::steal()) };
        let g = gpio.register_block();
        let m = io_mux.register_block();

        // IO_MUX: select the GPIO function and set the input buffer and pulls.
        //
        // `write()` resets every field not named. That is deliberate, so that
        // a reopen reliably drops the pulls, and the drive strength is set
        // explicitly rather than left to the SVD's reset value (0 on the P4).
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

        let output = matches!(mode, PinMode::Output | PinMode::OutputOpenDrain);
        match bank(index) {
            // SAFETY: index < 32, so mask is one bit of the 32-bit
            // ENABLE_W1TS / ENABLE_W1TC fields.
            (false, mask) if output => g.enable_w1ts().write(|w| unsafe { w.bits(mask) }),
            // SAFETY: as above.
            (false, mask) => g.enable_w1tc().write(|w| unsafe { w.bits(mask) }),
            // SAFETY: 32 <= index < NUM_GPIO (55), so mask is one of the low
            // 23 bits, inside the 25-bit ENABLE1_W1TS / ENABLE1_W1TC fields.
            (true, mask) if output => g.enable1_w1ts().write(|w| unsafe { w.bits(mask) }),
            // SAFETY: as above.
            (true, mask) => g.enable1_w1tc().write(|w| unsafe { w.bits(mask) }),
        };
        Ok(())
    }

    fn gpio_write(&mut self, index: u32, level: Level) -> BoardResult<()> {
        if index >= NUM_GPIO {
            return Err(ErrorCode::InvalidArgument);
        }
        // SAFETY: by the contract of EspBoard::new, only this board touches GPIO.
        let gpio = unsafe { GPIO::steal() };
        let g = gpio.register_block();
        let (high_bank, mask) = bank(index);
        let enable = if high_bank {
            g.enable1().read().bits()
        } else {
            g.enable().read().bits()
        };
        if enable & mask == 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        let high = matches!(level, Level::High);
        match (high_bank, high) {
            // SAFETY: index < 32, so mask is one bit of the 32-bit OUT_W1TS
            // / OUT_W1TC fields.
            (false, true) => g.out_w1ts().write(|w| unsafe { w.bits(mask) }),
            // SAFETY: as above.
            (false, false) => g.out_w1tc().write(|w| unsafe { w.bits(mask) }),
            // SAFETY: 32 <= index < NUM_GPIO (55), so mask is one of the low
            // 23 bits, inside the 25-bit OUT1_W1TS / OUT1_W1TC fields.
            (true, true) => g.out1_w1ts().write(|w| unsafe { w.bits(mask) }),
            // SAFETY: as above.
            (true, false) => g.out1_w1tc().write(|w| unsafe { w.bits(mask) }),
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
        let (high_bank, mask) = bank(index);
        let (enable, out, input) = if high_bank {
            (
                g.enable1().read().bits(),
                g.out1().read().bits(),
                g.in1().read().bits(),
            )
        } else {
            (
                g.enable().read().bits(),
                g.out().read().bits(),
                g.in_().read().bits(),
            )
        };
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
        // docs/abi-spec.md §5.2: back to input with no pulls. An LCD `RESET`
        // line then floats; that is the external circuit's job (the comment
        // on the same function in `ports/esp32c5`).
        let _ = self.gpio_configure(index, PinMode::Input);
    }

    // --- I2C. index 0 = I2C0 (SDA=GPIO54, SCL=GPIO53) ---

    fn i2c_open(&mut self, index: u32, speed: Speed) -> BoardResult<()> {
        // There is an I2C1 too, but v0.1 only assigns I2C0
        // (docs/abi-spec.md §8). The Hal has already checked index < MAX_I2C.
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
        // The timeout is set explicitly; the reason is on the same constant
        // in `ports/esp32c5`.
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
        // Length 0 is rejected here as well as in the Hal, so that it agrees
        // with `ports/rp2350` (the reason is in `ports/esp32c5`). All three
        // methods make the same test.
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
        bus.write_read(addr, data, buf).map_err(i2c_error)?;
        Ok(n)
    }

    fn i2c_close(&mut self, index: u32) {
        if index != 0 {
            return;
        }
        // `Drop` restores the pin routing and the peripheral clock.
        self.i2c = None;
    }

    // --- SPI. index 0 = SPI2 (SCK=GPIO5, MOSI=GPIO18, MISO=GPIO19) ---

    fn spi_open(&mut self, index: u32, frequency_hz: u32, mode: SpiMode) -> BoardResult<()> {
        // SPI2 is the only general-purpose SPI that v0.1 assigns
        // (docs/abi-spec.md §8). The Hal has already checked index < MAX_SPI.
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        if frequency_hz == 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        // Drop a leftover first (never hold two drivers for a stolen
        // peripheral). The Hal rejects a double open with `busy`.
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
        // SPI2 and the SPI2_* pins. self.spi was set to None above, so no two
        // drivers refer to the same peripheral or pins at once.
        let (spi2, sck, mosi, miso) = unsafe {
            (
                SPI2::steal(),
                AnyPin::steal(SPI2_SCK),
                AnyPin::steal(SPI2_MOSI),
                AnyPin::steal(SPI2_MISO),
            )
        };
        // The only failure is a requested frequency outside the reachable
        // range.
        let spi = Spi::new(spi2, config)
            .map_err(|_| ErrorCode::Unsupported)?
            .with_sck(sck)
            .with_mosi(mosi)
            .with_miso(miso);
        self.spi = Some(spi);
        Ok(())
    }

    fn spi_write(&mut self, index: u32, data: &[u8]) -> BoardResult<()> {
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        let spi = self.spi.as_mut().ok_or(ErrorCode::InvalidHandle)?;
        // Split every 64 bytes (the FIFO), but the guest keeps CS held, so to
        // the ILI9341 it is still one transfer.
        spi.write(data).map_err(|_| ErrorCode::Io)
    }

    fn spi_transfer(&mut self, index: u32, data: &[u8], buf: &mut [u8]) -> BoardResult<usize> {
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        let spi = self.spi.as_mut().ok_or(ErrorCode::InvalidHandle)?;
        let n = data.len().min(buf.len());
        // The driver's full-duplex transfer works in place, so the outgoing
        // bytes are copied in first.
        buf[..n].copy_from_slice(&data[..n]);
        spi.transfer(&mut buf[..n]).map_err(|_| ErrorCode::Io)?;
        Ok(n)
    }

    fn spi_close(&mut self, index: u32) {
        if index != 0 || self.spi.is_none() {
            return;
        }
        // Drop restores the pin routing and the peripheral clock.
        self.spi = None;
        // Back to input with no pulls, as in gpio_release. The driver's Drop
        // only disconnects the GPIO matrix signals; it does not touch the pad
        // direction or pulls.
        for n in [SPI2_SCK, SPI2_MOSI, SPI2_MISO] {
            let _ = self.gpio_configure(u32::from(n), PinMode::Input);
        }
    }

    // --- Assets: read from flash by range (docs/app-workflow.md §3.10) ---

    fn assets_read(&mut self, offset: u32, buf: &mut [u8]) -> BoardResult<()> {
        let at = self.assets_base.checked_add(offset).ok_or(ErrorCode::Io)?;
        read_flash(&mut self.storage, at, buf)
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
