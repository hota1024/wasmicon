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
///
/// I2C は記録済みの応答を順に返す（`WASMICON_I2C_REPLAY`）。実機のセンサーが
/// 無くてもゲストを最後まで走らせられるようにするため。
pub struct HostBoard {
    pins: [PinState; NUM_GPIO],
    start: Instant,
    trace: String,
    /// 読み出しに順に返す応答。空なら `Nack`。
    i2c_replay: Vec<Vec<u8>>,
    replay_pos: usize,
    /// SPI を `unsupported` にする。実機ポートの現状（Phase 5 未実装）を模して
    /// 失敗経路をテストするためのもの。
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

    /// SPI を `unsupported` にする（失敗経路のテスト用）。
    #[must_use]
    pub fn with_spi_unsupported(mut self, yes: bool) -> Self {
        self.spi_unsupported = yes;
        self
    }

    /// 記録済みの I2C 応答を設定する。
    #[must_use]
    pub fn with_i2c_replay(mut self, replay: Vec<Vec<u8>>) -> Self {
        self.i2c_replay = replay;
        self
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
        // 記録済み応答があるなら、書き込みは受け付ける（計測コマンドなど）。
        if self.i2c_replay.is_empty() {
            return Err(ErrorCode::Nack);
        }
        Ok(())
    }

    fn i2c_read(&mut self, _index: u32, _address: u16, buf: &mut [u8]) -> BoardResult<usize> {
        let Some(resp) = self.i2c_replay.get(self.replay_pos) else {
            return Err(ErrorCode::Nack);
        };
        // 長さが足りないときは消費しない。再試行しても同じ失敗になるようにする。
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
