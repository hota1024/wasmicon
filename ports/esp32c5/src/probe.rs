//! Hardware diagnosis: drives the port layer (`Board`) directly, before the
//! guest runs.
//!
//! Called from `main` only with `--features hw-probe`. When the trace matches
//! the host exactly but the screen stays blank, the trace cannot tell whether
//! the port's register setup reaches the pads. This is the one place that can:
//!
//! - Read the ILI9341 ID (`0xD3` / `0x04`). With `SDO`/`MISO` wired to
//!   `GPIO5`, `0xD3` returns `00 00 93 41`; a reply proves that CS / DC /
//!   SCK / MOSI / MISO all work. `00 00 00 00` or `ff ff ff ff` means a dead
//!   path.
//! - Toggle DC at 1 Hz, so a meter or an LED shows whether the GPIO reaches
//!   the pad.
//!
//! Diagnosis only; the feature can be dropped once it has served.

use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_core::generated::ErrorCode;
use wasmicon_port::fmt::Buf;
use wasmicon_port::Board;

use esp_hal::peripherals::{GPIO, IO_MUX};

use crate::board::{EspBoard, Serial};

/// These can be looked up by role, but this drives `Board` directly, so they
/// are constants (the ESP32-C5 table in docs/abi-spec.md §8).
const CS: u32 = 10;
const DC: u32 = 0;
const RST: u32 = 1;

/// Slower reads are more likely to succeed, so this is below the demo's rate.
const SPI_HZ: u32 = 1_000_000;

/// Number of 1 Hz DC toggles.
const WIGGLE: u32 = 10;

/// Screen size (landscape).
const WIDTH: u16 = 320;
const HEIGHT: u16 = 240;

/// Bytes per row.
const ROW_BYTES: usize = WIDTH as usize * 2;

fn say<S: Serial>(b: &mut EspBoard<S>, s: &str) {
    b.serial().write(s.as_bytes());
    b.serial().write(b"\r\n");
}

/// Prints `<label>: ok` or `<label>: err=<n>`.
///
/// `<n>` is the status of docs/abi-spec.md §4.1 (discriminant + 1), through
/// `status()` rather than `e as u32`: the discriminant of `invalid-argument`
/// is 0, so the raw value would print a failure as `err=0`, indistinguishable
/// from success and off by one from the number in the trace.
fn say_result<S: Serial>(b: &mut EspBoard<S>, label: &str, r: Result<(), ErrorCode>) {
    let mut raw = [0u8; 64];
    let mut out = Buf::new(&mut raw);
    out.str(label);
    match r {
        Ok(()) => out.str(": ok"),
        Err(e) => {
            out.str(": err=");
            out.u32(e.status());
        }
    }
    b.serial().write(out.as_bytes());
    b.serial().write(b"\r\n");
}

/// Prints the bytes read, in hex.
fn say_bytes<S: Serial>(b: &mut EspBoard<S>, label: &str, data: &[u8]) {
    let mut raw = [0u8; 96];
    let mut out = Buf::new(&mut raw);
    out.str(label);
    for &x in data {
        out.byte(b' ');
        out.hex(u32::from(x), 2);
    }
    b.serial().write(out.as_bytes());
    b.serial().write(b"\r\n");
}

/// Sends one command (plus arguments) with CS held; the same sequence as the
/// guest's `ili9341.rs`.
fn send<S: Serial>(b: &mut EspBoard<S>, cmd: u8, args: &[u8]) -> Result<(), ErrorCode> {
    b.gpio_write(CS, Level::Low)?;
    b.gpio_write(DC, Level::Low)?;
    b.spi_write(0, &[cmd])?;
    if !args.is_empty() {
        b.gpio_write(DC, Level::High)?;
        b.spi_write(0, args)?;
    }
    b.gpio_write(CS, Level::High)
}

/// Sends a command and reads `n` bytes. The first byte from the ILI9341 is a
/// dummy.
fn read_id<S: Serial>(b: &mut EspBoard<S>, cmd: u8, n: usize) -> Result<[u8; 5], ErrorCode> {
    let mut rx = [0u8; 5];
    b.gpio_write(CS, Level::Low)?;
    b.gpio_write(DC, Level::Low)?;
    b.spi_write(0, &[cmd])?;
    b.gpio_write(DC, Level::High)?;
    let tx = [0u8; 5];
    b.spi_transfer(0, &tx[..n], &mut rx[..n])?;
    b.gpio_write(CS, Level::High)?;
    Ok(rx)
}

/// Fills the whole screen with one color, to see with your own eyes whether
/// SPI works without the guest or the interpreter. Red means the port layer's
/// SPI and pins all work.
fn fill_screen<S: Serial>(b: &mut EspBoard<S>, color: u16) -> Result<(), ErrorCode> {
    let x1 = WIDTH - 1;
    let y1 = HEIGHT - 1;
    send(b, 0x2A, &[0, 0, (x1 >> 8) as u8, x1 as u8])?; // CASET
    send(b, 0x2B, &[0, 0, (y1 >> 8) as u8, y1 as u8])?; // PASET

    let mut row = [0u8; ROW_BYTES];
    let mut i = 0;
    while i < ROW_BYTES {
        row[i] = (color >> 8) as u8;
        row[i + 1] = color as u8;
        i += 2;
    }

    b.gpio_write(CS, Level::Low)?;
    b.gpio_write(DC, Level::Low)?;
    b.spi_write(0, &[0x2C])?; // RAMWR
    b.gpio_write(DC, Level::High)?;
    let mut r = 0;
    while r < HEIGHT {
        b.spi_write(0, &row)?;
        r += 1;
    }
    b.gpio_write(CS, Level::High)
}

/// Drives `pin` as an output, turns its input buffer on as well, and reads
/// the pad back through `GPIO_IN`. A pad that follows High and Low proves the
/// firmware reaches it (pad mux, LP mux, hold, output enable); whatever is
/// wired to it is not involved, so a wrong ILI9341 reply with a good loopback
/// points at the wiring.
fn loopback<S: Serial>(b: &mut EspBoard<S>, label: &str, pin: u32) {
    let ok = b.gpio_configure(pin, PinMode::Output).is_ok();
    // SAFETY: diagnosis runs before the guest, and only this board touches
    // IO_MUX; `pin` is below 29, so `gpio(pin)` is in range. FUN_IE is a
    // single bit, so the write cannot touch another field.
    unsafe { IO_MUX::steal() }
        .register_block()
        .gpio(pin as usize)
        .modify(|_, w| w.fun_ie().set_bit());
    let mut seen = [0u8; 2];
    for (i, level) in [Level::High, Level::Low].into_iter().enumerate() {
        let _ = b.gpio_write(pin, level);
        b.sleep_us(10);
        // SAFETY: reading GPIO_IN has no side effects.
        let bits = unsafe { GPIO::steal() }
            .register_block()
            .in_()
            .read()
            .bits();
        seen[i] = u8::from(bits & (1 << pin) != 0);
    }
    let mut raw = [0u8; 80];
    let mut out = Buf::new(&mut raw);
    out.str("probe: loopback ");
    out.str(label);
    out.str(if ok { "" } else { " (configure failed)" });
    out.str(" wrote 1,0 read ");
    out.u32(u32::from(seen[0]));
    out.byte(b',');
    out.u32(u32::from(seen[1]));
    out.str(if seen == [1, 0] { " ok" } else { " BAD" });
    b.serial().write(out.as_bytes());
    b.serial().write(b"\r\n");
    let _ = b.gpio_configure(pin, PinMode::Input);
}

/// Runs the checks in order and does not stop on failure (the point is to see
/// how far they get).
pub fn run<S: Serial>(b: &mut EspBoard<S>) {
    say(b, "probe: start (hw-probe)");
    say(
        b,
        "probe: cs=GPIO10 dc=GPIO0 rst=GPIO1 sck=GPIO24 mosi=GPIO23 miso=GPIO5",
    );

    // First the pads themselves, before SPI owns any of them.
    for (label, pin) in [
        ("cs GPIO10", CS),
        ("dc GPIO0", DC),
        ("rst GPIO1", RST),
        ("sck GPIO24", 24),
        ("mosi GPIO23", 23),
        ("miso GPIO5", 5),
    ] {
        loopback(b, label, pin);
    }

    let r = b.gpio_configure(CS, PinMode::Output);
    say_result(b, "probe: configure cs", r);
    let r = b.gpio_configure(DC, PinMode::Output);
    say_result(b, "probe: configure dc", r);
    let r = b.gpio_configure(RST, PinMode::Output);
    say_result(b, "probe: configure rst", r);

    let _ = b.gpio_write(CS, Level::High);
    let _ = b.gpio_write(RST, Level::Low);
    b.sleep_ms(10);
    let _ = b.gpio_write(RST, Level::High);
    b.sleep_ms(120);
    say(b, "probe: reset pulsed");

    let r = b.spi_open(0, SPI_HZ, SpiMode::Mode0);
    say_result(b, "probe: spi_open(0, 1MHz, mode0)", r);

    // Init sequence (the same as the guest's; apps/README.md §2).
    let mut init = send(b, 0x01, &[]); // SWRESET
    b.sleep_ms(120);
    init = init.and_then(|()| send(b, 0x11, &[])); // SLPOUT
    b.sleep_ms(120);
    init = init.and_then(|()| send(b, 0x3A, &[0x55])); // PIXFMT = RGB565
    init = init.and_then(|()| send(b, 0x36, &[0x28])); // MADCTL = landscape
    init = init.and_then(|()| send(b, 0x29, &[])); // DISPON
    say_result(b, "probe: init sequence", init);

    // The main check. With MISO wired, 0xD3 returns 00 00 93 41.
    match read_id(b, 0xD3, 5) {
        Ok(rx) => say_bytes(b, "probe: 0xD3 (want 00 00 93 41) ->", &rx[..5]),
        Err(e) => say_result(b, "probe: 0xD3", Err(e)),
    }
    match read_id(b, 0x04, 4) {
        Ok(rx) => say_bytes(b, "probe: 0x04 RDDID             ->", &rx[..4]),
        Err(e) => say_result(b, "probe: 0x04", Err(e)),
    }

    // Fill the screen red. Red here means the port layer's SPI and pins all
    // work. It also puts 150 KB worth of clocks on SCK / MOSI, so a USB-serial
    // RXD on either of them shows a flood of bytes.
    say(b, "probe: filling screen red (port layer only)");
    let r = fill_screen(b, 0xF800);
    say_result(b, "probe: fill_screen", r);

    // 1 Hz on DC, so a meter or an LED shows whether the GPIO reaches the pad.
    say(b, "probe: wiggling DC (GPIO0) at 1Hz for 10s");
    let mut i = 0;
    while i < WIGGLE {
        let _ = b.gpio_write(DC, Level::High);
        b.sleep_ms(500);
        let _ = b.gpio_write(DC, Level::Low);
        b.sleep_ms(500);
        i += 1;
    }

    b.spi_close(0);
    say(b, "probe: done, handing over to the guest");
}
