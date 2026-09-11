//! `wasmicon_port::Board` の実装。
//!
//! **ボード固有の値をここに書かない。** チップのレジスタ操作は `crate::chip`、
//! ピンの割り当ては `crate::boards` にある。ここはその 2 つを繋ぐだけなので、
//! ボードが増えてもこのファイルは変わらない。

use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::i2c::Speed;
use wasmicon_core::generated::log::Level as LogLevel;
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_core::generated::ErrorCode;
use wasmicon_port::{Board, BoardResult};

use crate::boards::{Serial, DEF};
use crate::chip::{self, Gpio};

/// ESP32-P4 のボード。どのボードかは feature で選ぶ（`crate::boards`）。
pub struct EspBoard<S: Serial> {
    serial: S,
    gpio: Gpio,
}

impl<S: Serial> EspBoard<S> {
    /// # Safety
    /// GPIO と IO_MUX をこのボードが排他的に使うこと。
    #[must_use]
    pub unsafe fn new(serial: S) -> Self {
        // SAFETY: 呼び出し側の契約をそのまま chip::Gpio に渡す。
        let gpio = unsafe { Gpio::steal() };
        EspBoard { serial, gpio }
    }

    /// シリアルへの参照。失敗の理由を出すのに使う。
    pub fn serial(&mut self) -> &mut S {
        &mut self.serial
    }
}

impl<S: Serial> Board for EspBoard<S> {
    fn pin_by_role(&self, role: &str) -> Option<u32> {
        DEF.pin_by_role(role)
    }

    fn gpio_count(&self) -> u32 {
        chip::NUM_GPIO
    }

    fn gpio_reserved(&self, index: u32) -> bool {
        DEF.is_reserved(index)
    }

    fn gpio_configure(&mut self, index: u32, mode: PinMode) -> BoardResult<()> {
        self.gpio.configure(index, mode)
    }

    fn gpio_write(&mut self, index: u32, level: Level) -> BoardResult<()> {
        self.gpio.write(index, level)
    }

    fn gpio_read(&mut self, index: u32) -> BoardResult<Level> {
        self.gpio.read(index)
    }

    fn gpio_release(&mut self, index: u32) {
        // abi-spec §5.2: 入力・プル無しに戻す。
        let _ = self.gpio.configure(index, PinMode::Input);
    }

    // --- I2C / SPI は Phase 5 で実装する ---

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

    // --- 時間 ---

    fn now_us(&mut self) -> u64 {
        chip::now_us()
    }

    fn sleep_ms(&mut self, ms: u32) {
        self.sleep_us(ms.saturating_mul(1000));
    }

    fn sleep_us(&mut self, us: u32) {
        let target = chip::now_us().saturating_add(u64::from(us));
        while chip::now_us() < target {
            core::hint::spin_loop();
        }
    }

    // --- 出力 ---

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
