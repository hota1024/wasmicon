//! Mock board for the PC.
//!
//! The shared part of the HAL (import resolution, handle table, status, trace
//! formatting) is in `wasmicon-port`. This file implements only the
//! board-specific operations. Keeping the same shape as the board ports
//! guarantees that the trace format matches (docs/abi-spec.md §9).

use std::time::Instant;

use wasmicon_core::generated::ErrorCode;
use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::i2c::Speed;
use wasmicon_core::generated::log::Level as LogLevel;
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_port::{Board, BoardResult};

/// Number of GPIOs on the mock board.
const NUM_GPIO: usize = 48;

/// Role name -> GPIO number table of the mock board. Unlike a board, the host
/// has no flash, so the table lives here (`profile::HOST.roles`,
/// docs/app-workflow.md §3.9).
const ROLES: &[(&str, u32)] = wasmicon_port::profile::HOST.roles;

// Check at compile time that the numbers can be opened on this mock, as the
// board ports do (moving the table to the profile separated it from NUM_GPIO;
// this ties them back together). The mock has no reserved pins.
const _: () = wasmicon_port::profile::assert_roles_openable(ROLES, NUM_GPIO as u32, &[]);

#[derive(Clone, Copy, Default)]
struct PinState {
    configured: bool,
    mode: u32,
    level: u32,
}

/// Mock board on the PC. No peripherals are attached.
///
/// I2C replays recorded responses in order (`WASMICON_I2C_REPLAY`) so that a
/// guest can run to completion without the real sensor.
pub struct HostBoard {
    pins: [PinState; NUM_GPIO],
    start: Instant,
    trace: String,
    /// Responses returned to reads, in order. Empty means `Nack`.
    i2c_replay: Vec<Vec<u8>>,
    replay_pos: usize,
    /// Makes SPI return `unsupported`, as a port without SPI does (rp2040), to
    /// test the failure path.
    spi_unsupported: bool,
}

impl HostBoard {
    #[must_use]
    pub fn new() -> Self {
        HostBoard {
            pins: [PinState::default(); NUM_GPIO],
            start: Instant::now(),
            trace: String::new(),
            i2c_replay: Vec::new(),
            replay_pos: 0,
            spi_unsupported: false,
        }
    }

    /// Makes SPI return `unsupported` (for testing the failure path).
    #[must_use]
    pub fn with_spi_unsupported(mut self, yes: bool) -> Self {
        self.spi_unsupported = yes;
        self
    }

    /// Sets the recorded I2C responses.
    #[must_use]
    pub fn with_i2c_replay(mut self, replay: Vec<Vec<u8>>) -> Self {
        self.i2c_replay = replay;
        self
    }

    /// The trace collected so far.
    #[must_use]
    pub fn trace_output(&self) -> &str {
        &self.trace
    }
}

impl Default for HostBoard {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether the mode is an output (`Output` or `OutputOpenDrain`).
fn is_output(mode: u32) -> bool {
    mode >= PinMode::Output as u32
}

/// The mock's wiring table, for `Hal::with_roles`.
///
/// # Panics
/// When `profile::HOST.roles` fails the check (the same condition as the
/// compile-time `assert_roles_openable`, so it does not happen in practice).
#[must_use]
pub fn host_roles() -> wasmicon_port::roles::RoleMap {
    let limits = wasmicon_port::roles::Limits::of(&wasmicon_port::profile::HOST);
    match wasmicon_port::roles::from_table(ROLES, &limits) {
        Ok(map) => map,
        Err(e) => panic!("profile::HOST.roles is invalid: {}", e.reason()),
    }
}

impl Board for HostBoard {
    fn gpio_count(&self) -> u32 {
        NUM_GPIO as u32
    }

    fn gpio_configure(&mut self, index: u32, mode: PinMode) -> BoardResult<()> {
        let p = self
            .pins
            .get_mut(index as usize)
            .ok_or(ErrorCode::InvalidArgument)?;
        p.configured = true;
        p.mode = mode as u32;
        Ok(())
    }

    fn gpio_write(&mut self, index: u32, level: Level) -> BoardResult<()> {
        let p = self
            .pins
            .get_mut(index as usize)
            .ok_or(ErrorCode::InvalidArgument)?;
        if !is_output(p.mode) {
            return Err(ErrorCode::InvalidArgument);
        }
        p.level = level as u32;
        Ok(())
    }

    fn gpio_read(&mut self, index: u32) -> BoardResult<Level> {
        let p = self
            .pins
            .get(index as usize)
            .ok_or(ErrorCode::InvalidArgument)?;
        Level::from_u32(p.level).ok_or(ErrorCode::Io)
    }

    fn gpio_release(&mut self, index: u32) {
        if let Some(p) = self.pins.get_mut(index as usize) {
            *p = PinState::default();
        }
    }

    // --- I2C / SPI: no device is attached to the mock ---

    fn i2c_open(&mut self, _index: u32, _speed: Speed) -> BoardResult<()> {
        Ok(())
    }

    fn i2c_write(&mut self, _index: u32, _address: u16, _data: &[u8]) -> BoardResult<()> {
        // With recorded responses, writes are accepted (measurement commands and so on).
        if self.i2c_replay.is_empty() {
            return Err(ErrorCode::Nack);
        }
        Ok(())
    }

    fn i2c_read(&mut self, _index: u32, _address: u16, buf: &mut [u8]) -> BoardResult<usize> {
        let Some(resp) = self.i2c_replay.get(self.replay_pos) else {
            return Err(ErrorCode::Nack);
        };
        // A short response is not consumed, so a retry fails the same way.
        if resp.len() < buf.len() {
            return Err(ErrorCode::Io);
        }
        self.replay_pos += 1;
        let n = buf.len();
        buf.copy_from_slice(&resp[..n]);
        Ok(n)
    }

    fn i2c_write_read(
        &mut self,
        index: u32,
        address: u16,
        _data: &[u8],
        buf: &mut [u8],
    ) -> BoardResult<usize> {
        self.i2c_read(index, address, buf)
    }

    fn i2c_close(&mut self, _index: u32) {}

    fn spi_open(&mut self, _index: u32, _frequency_hz: u32, _mode: SpiMode) -> BoardResult<()> {
        if self.spi_unsupported {
            return Err(ErrorCode::Unsupported);
        }
        Ok(())
    }

    fn spi_write(&mut self, _index: u32, _data: &[u8]) -> BoardResult<()> {
        if self.spi_unsupported {
            return Err(ErrorCode::Unsupported);
        }
        Ok(())
    }

    fn spi_transfer(&mut self, _index: u32, data: &[u8], buf: &mut [u8]) -> BoardResult<usize> {
        // MISO reads 0.
        let n = data.len().min(buf.len());
        buf[..n].fill(0);
        Ok(n)
    }

    fn spi_close(&mut self, _index: u32) {}

    // --- time ---

    fn now_us(&mut self) -> u64 {
        self.start.elapsed().as_micros() as u64
    }

    fn sleep_ms(&mut self, ms: u32) {
        std::thread::sleep(std::time::Duration::from_millis(u64::from(ms)));
    }

    fn sleep_us(&mut self, us: u32) {
        std::thread::sleep(std::time::Duration::from_micros(u64::from(us)));
    }

    // --- output ---

    fn log(&mut self, _level: LogLevel, message: &[u8]) {
        // docs/abi-spec.md §4.2: UTF-8 is not validated.
        println!("[wasm] {}", String::from_utf8_lossy(message));
    }

    fn trace(&mut self, line: &[u8]) {
        self.trace.push_str(&String::from_utf8_lossy(line));
        self.trace.push('\n');
    }
}
