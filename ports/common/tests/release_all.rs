//! Locks in that `Hal::release_all` satisfies docs/abi-spec.md §5.2.
//!
//! That section says the host drops every remaining handle when `run`
//! returns. The cleanup was missing for a long time, hidden by the guest-side
//! `Drop`; with a loader the next app would hit `busy`, so this test locks it
//! in.
//!
//! `ports/common` is `no_std`, so the tests are integration tests (a separate
//! crate), the same shape as `runtime`'s `tests/`.

use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::i2c::Speed;
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_core::generated::{self, ErrorCode};
use wasmicon_core::instance::Resolver;
use wasmicon_port::roles::{Limits, from_table};
use wasmicon_port::{Board, BoardResult, Hal};

/// A board that only records what was released, in order.
#[derive(Default)]
struct RecordingBoard {
    released: Vec<String>,
}

impl Board for RecordingBoard {
    fn gpio_count(&self) -> u32 {
        32
    }

    fn gpio_configure(&mut self, _index: u32, _mode: PinMode) -> BoardResult<()> {
        Ok(())
    }

    fn gpio_write(&mut self, _index: u32, _level: Level) -> BoardResult<()> {
        Ok(())
    }

    fn gpio_read(&mut self, _index: u32) -> BoardResult<Level> {
        Ok(Level::Low)
    }

    fn gpio_release(&mut self, index: u32) {
        self.released.push(format!("gpio {index}"));
    }

    fn i2c_open(&mut self, _index: u32, _speed: Speed) -> BoardResult<()> {
        Ok(())
    }

    fn i2c_write(&mut self, _index: u32, _address: u16, _data: &[u8]) -> BoardResult<()> {
        Ok(())
    }

    fn i2c_read(&mut self, _index: u32, _address: u16, _buf: &mut [u8]) -> BoardResult<usize> {
        Err(ErrorCode::Nack)
    }

    fn i2c_write_read(
        &mut self,
        _index: u32,
        _address: u16,
        _data: &[u8],
        _buf: &mut [u8],
    ) -> BoardResult<usize> {
        Err(ErrorCode::Nack)
    }

    fn i2c_close(&mut self, index: u32) {
        self.released.push(format!("i2c {index}"));
    }

    fn spi_open(&mut self, _index: u32, _frequency_hz: u32, _mode: SpiMode) -> BoardResult<()> {
        Ok(())
    }

    fn spi_write(&mut self, _index: u32, _data: &[u8]) -> BoardResult<()> {
        Ok(())
    }

    fn spi_transfer(&mut self, _index: u32, _data: &[u8], _buf: &mut [u8]) -> BoardResult<usize> {
        Err(ErrorCode::Unsupported)
    }

    fn spi_close(&mut self, index: u32) {
        self.released.push(format!("spi {index}"));
    }

    fn now_us(&mut self) -> u64 {
        0
    }

    fn sleep_ms(&mut self, _ms: u32) {}

    fn sleep_us(&mut self, _us: u32) {}

    fn log(&mut self, _level: generated::log::Level, _message: &[u8]) {}

    fn trace(&mut self, line: &[u8]) {
        // Record trace lines in the same list so that one shows up
        // (release_all must not emit any).
        self.released
            .push(format!("trace {}", String::from_utf8_lossy(line)));
    }
}

/// A failed `call` fails the test. `wasmicon_core::Error` does not implement
/// `Debug` (the core uses no `core::fmt`, docs/decisions.md P-09), so `expect`
/// cannot be used directly. The output format matches `ports/host/tests/apps.rs`.
fn expect_ok(r: wasmicon_core::error::Result<()>, what: &str) {
    if let Err(e) = r {
        panic!("{what} failed: {} [{}]", e.reason(), e.kind().name());
    }
}

/// Looks up a host function number by import name.
fn host(module: &str, name: &str) -> u32 {
    u32::from(
        generated::resolve(module, name)
            .expect("not in the import table")
            .host_fn
            .index(),
    )
}

const GPIO: &str = "wasmicon:hal/gpio@0.1.0";
const I2C: &str = "wasmicon:hal/i2c@0.1.0";
const SPI: &str = "wasmicon:hal/spi@0.1.0";

/// Opens one gpio, one i2c and one spi handle. The out-pointers land in guest memory.
fn open_one_of_each(hal: &mut Hal<RecordingBoard>, mem: &mut [u8]) {
    let mut out = [0u64; 1];

    // gpio.pin.open(index = 5, mode = output, out-handle = 0)
    expect_ok(
        hal.call(host(GPIO, "[static]pin.open"), &[5, 1, 0], &mut out, mem),
        "pin.open",
    );
    assert_eq!(out[0], 0, "pin.open succeeds");

    // i2c.bus.open(index = 0, speed = standard, out-handle = 8)
    expect_ok(
        hal.call(host(I2C, "[static]bus.open"), &[0, 0, 8], &mut out, mem),
        "i2c bus.open",
    );
    assert_eq!(out[0], 0, "i2c bus.open succeeds");

    // spi.bus.open(index = 0, frequency-hz = 1 MHz, mode = 0, out-handle = 16)
    expect_ok(
        hal.call(
            host(SPI, "[static]bus.open"),
            &[0, 1_000_000, 0, 16],
            &mut out,
            mem,
        ),
        "spi bus.open",
    );
    assert_eq!(out[0], 0, "spi bus.open succeeds");
}

#[test]
fn closes_every_leaked_handle_in_a_fixed_order() {
    let mut mem = vec![0u8; 64 * 1024];
    let mut hal = Hal::new(RecordingBoard::default(), false);

    open_one_of_each(&mut hal, &mut mem);
    assert!(
        hal.board_mut().released.is_empty(),
        "opening alone releases nothing"
    );

    hal.release_all();
    assert_eq!(
        hal.board_mut().released,
        ["gpio 5", "i2c 0", "spi 0"],
        "releases in the order gpio, i2c, spi (identical across ports)"
    );
}

#[test]
fn is_idempotent() {
    let mut mem = vec![0u8; 64 * 1024];
    let mut hal = Hal::new(RecordingBoard::default(), false);

    open_one_of_each(&mut hal, &mut mem);
    hal.release_all();
    hal.board_mut().released.clear();

    // The second call does nothing: the same path as after the guest dropped
    // everything itself.
    hal.release_all();
    assert!(hal.board_mut().released.is_empty());
}

#[test]
fn emits_no_trace_lines() {
    let mut mem = vec![0u8; 64 * 1024];
    // trace_on = true. The recorded traces (docs/verification-report.md §6, §7)
    // must not change.
    let mut hal = Hal::new(RecordingBoard::default(), true);

    open_one_of_each(&mut hal, &mut mem);
    assert!(
        hal.board_mut()
            .released
            .iter()
            .any(|l| l.starts_with("trace")),
        "open does emit a trace (so this check is live)"
    );
    hal.board_mut().released.clear();

    hal.release_all();
    assert_eq!(
        hal.board_mut().released,
        ["gpio 5", "i2c 0", "spi 0"],
        "release_all calls the board directly and emits no trace line"
    );
}

#[test]
fn the_role_table_survives_release_all() {
    // The role table is not a handle, so release_all does not clear it (outside
    // the scope of docs/abi-spec.md §5.2). Locked in here as a reminder that a
    // loader reusing one `Hal` must clear it separately (the "rebuild the role
    // table per cycle" item in docs/TODO.md §5). Carried over, a number the next
    // app hard-codes that happens to match a role of the previous app would
    // print as `role:` and change the trace (docs/abi-spec.md §9).
    let mut mem = vec![0u8; 64 * 1024];
    let mut out = [0u64; 1];
    let limits = Limits {
        gpio_count: 32,
        reserved: &[],
        bus_pins: &[],
    };
    let roles = match from_table(&[("led", 2)], &limits) {
        Ok(m) => m,
        Err(e) => panic!("{}", e.reason()),
    };
    let mut hal = Hal::new(RecordingBoard::default(), true).with_roles(roles);

    // pin-by-role("led") hands out 2. role-ptr=0, role-len=3, out=16.
    mem[..3].copy_from_slice(b"led");
    expect_ok(
        hal.call(
            host("wasmicon:hal/board@0.1.0", "pin-by-role"),
            &[0, 3, 16],
            &mut out,
            &mut mem,
        ),
        "pin-by-role",
    );
    assert_eq!(out[0], 0, "led exists on RecordingBoard");

    hal.release_all();
    hal.board_mut().released.clear();

    // Even after the cleanup, GPIO 2 prints as role:led rather than a number.
    expect_ok(
        hal.call(
            host(GPIO, "[static]pin.open"),
            &[2, 1, 0],
            &mut out,
            &mut mem,
        ),
        "pin.open",
    );
    let trace: Vec<&String> = hal
        .board_mut()
        .released
        .iter()
        .filter(|l| l.starts_with("trace"))
        .collect();
    assert!(
        trace.iter().any(|l| l.contains("role:led")),
        "the role table survives, so role:led is printed: {trace:?}"
    );
}

#[test]
fn reopening_after_release_all_succeeds() {
    let mut mem = vec![0u8; 64 * 1024];
    let mut hal = Hal::new(RecordingBoard::default(), false);
    let mut out = [0u64; 1];

    open_one_of_each(&mut hal, &mut mem);

    // Before the cleanup this is a double open, so busy (docs/abi-spec.md §5.2).
    expect_ok(
        hal.call(
            host(I2C, "[static]bus.open"),
            &[0, 0, 8],
            &mut out,
            &mut mem,
        ),
        "double open call",
    );
    assert_eq!(
        out[0],
        u64::from(ErrorCode::Busy.status()),
        "busy before the cleanup"
    );

    // After the cleanup the next app can open it. The loader depends on this.
    hal.release_all();
    expect_ok(
        hal.call(
            host(I2C, "[static]bus.open"),
            &[0, 0, 8],
            &mut out,
            &mut mem,
        ),
        "i2c bus.open after the cleanup",
    );
    assert_eq!(out[0], 0, "succeeds after the cleanup");
}
