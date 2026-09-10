//! PC 用の mock ボード。
//!
//! HAL の共通部分（import の解決、ハンドル表、ステータス、トレース整形）は
//! `wasmicon-port` にある。ここはボード固有の操作だけを実装する。
//! 実機のポートと同じ形にしておくことで、トレースの書式が確実に揃う（abi-spec §9）。

use std::time::Instant;

use wasmicon_core::generated::ErrorCode;
use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::i2c::Speed;
use wasmicon_core::generated::log::Level as LogLevel;
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_port::{Board, BoardResult};

/// mock ボードの GPIO 本数。
const NUM_GPIO: usize = 48;

/// mock ボードの役割名 → GPIO 番号（abi-spec §8）。
const ROLES: &[(&str, u32)] = &[("led", 2), ("lcd-cs", 10), ("lcd-dc", 11), ("lcd-rst", 12)];

#[derive(Clone, Copy, Default)]
struct PinState {
    configured: bool,
    mode: u32,
    level: u32,
}

/// PC 上の mock ボード。ペリフェラルは繋がっていない。
pub struct HostBoard {
    pins: [PinState; NUM_GPIO],
    start: Instant,
    trace: String,
}

impl HostBoard {
    #[must_use]
    pub fn new() -> Self {
        HostBoard {
            pins: [PinState::default(); NUM_GPIO],
            start: Instant::now(),
            trace: String::new(),
        }
    }

    /// 溜めたトレース。
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

/// 出力モードか（`Output` と `OutputOpenDrain`）。
fn is_output(mode: u32) -> bool {
    mode >= PinMode::Output as u32
}

impl Board for HostBoard {
    fn pin_by_role(&self, role: &str) -> Option<u32> {
        ROLES.iter().find(|(r, _)| *r == role).map(|(_, i)| *i)
    }

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

    // --- I2C / SPI: mock にはデバイスが繋がっていない ---

    fn i2c_open(&mut self, _index: u32, _speed: Speed) -> BoardResult<()> {
        Ok(())
    }

    fn i2c_write(&mut self, _index: u32, _address: u16, _data: &[u8]) -> BoardResult<()> {
        Err(ErrorCode::Nack)
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

    fn i2c_close(&mut self, _index: u32) {}

    fn spi_open(&mut self, _index: u32, _frequency_hz: u32, _mode: SpiMode) -> BoardResult<()> {
        Ok(())
    }

    fn spi_write(&mut self, _index: u32, _data: &[u8]) -> BoardResult<()> {
        Ok(())
    }

    fn spi_transfer(&mut self, _index: u32, data: &[u8], buf: &mut [u8]) -> BoardResult<usize> {
        // MISO は 0 を返す。
        let n = data.len().min(buf.len());
        buf[..n].fill(0);
        Ok(n)
    }

    fn spi_close(&mut self, _index: u32) {}

    // --- 時間 ---

    fn now_us(&mut self) -> u64 {
        self.start.elapsed().as_micros() as u64
    }

    fn sleep_ms(&mut self, ms: u32) {
        std::thread::sleep(std::time::Duration::from_millis(u64::from(ms)));
    }

    fn sleep_us(&mut self, us: u32) {
        std::thread::sleep(std::time::Duration::from_micros(u64::from(us)));
    }

    // --- 出力 ---

    fn log(&mut self, _level: LogLevel, message: &[u8]) {
        // abi-spec §4.2: UTF-8 の検証はしない。
        println!("[wasm] {}", String::from_utf8_lossy(message));
    }

    fn trace(&mut self, line: &[u8]) {
        self.trace.push_str(&String::from_utf8_lossy(line));
        self.trace.push('\n');
    }
}
