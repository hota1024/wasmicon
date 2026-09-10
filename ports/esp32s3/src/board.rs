//! ESP32-S3 (DevKitC-1) のボード実装。
//!
//! GPIO は番号で動的に触る必要があるので、型付きピンではなく GPIO / IO_MUX の
//! レジスタを直接叩く。HAL の共通部分は `wasmicon-port` にある。

use esp_hal::peripherals::{GPIO, IO_MUX};
use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::i2c::Speed;
use wasmicon_core::generated::log::Level as LogLevel;
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_core::generated::ErrorCode;
use wasmicon_port::{Board, BoardResult};

/// ESP32-S3 の GPIO は 0..48（21..25 は存在しない番号がある）。
const NUM_GPIO: u32 = 49;

/// IO_MUX の MCU_SEL に入れる「GPIO 機能」。ESP32-S3 では 1。
const MCU_SEL_GPIO: u8 = 1;

/// GPIO マトリクスの「GPIO 出力」信号。ESP32-S3 では 128。
const SIG_GPIO_OUT: u16 = 128;

/// ゲストに開放しない GPIO。
///
/// - 22..=25: ESP32-S3 には存在しない欠番
/// - 26..=32: SPI フラッシュと PSRAM。XIP 実行中に触るとファームウェアごと落ちる
/// - 43, 44: トレース用の UART0（DevKitC-1 では USB シリアルに直結）
fn reserved(index: u32) -> bool {
    matches!(index, 22..=32 | 43 | 44)
}

/// 役割名 → GPIO 番号（abi-spec §8 の表）。
///
/// `led` が外付けなのは、DevKitC-1 のオンボード LED が WS2812 で
/// 素の GPIO では駆動できないため。**実機の配線は未確認**（docs/handoff.md §8）。
const ROLES: &[(&str, u32)] = &[("led", 2), ("lcd-cs", 10), ("lcd-dc", 14), ("lcd-rst", 15)];

/// トレースとログを出す先。
pub trait Serial {
    fn write(&mut self, bytes: &[u8]);
}

/// DevKitC-1 のボード。
pub struct EspBoard<S: Serial> {
    serial: S,
}

impl<S: Serial> EspBoard<S> {
    /// # Safety
    /// GPIO と IO_MUX をこのボードが排他的に使うこと。
    #[must_use]
    pub unsafe fn new(serial: S) -> Self {
        EspBoard { serial }
    }

    /// シリアルへの参照。失敗の理由を出すのに使う。
    pub fn serial(&mut self) -> &mut S {
        &mut self.serial
    }
}

/// 32 本ずつに分かれたレジスタのどちらを触るかを決める。
fn bank(index: u32) -> (bool, u32) {
    if index < 32 {
        (false, 1u32 << index)
    } else {
        (true, 1u32 << (index - 32))
    }
}

impl<S: Serial> Board for EspBoard<S> {
    fn pin_by_role(&self, role: &str) -> Option<u32> {
        ROLES.iter().find(|(r, _)| *r == role).map(|(_, i)| *i)
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
        // SAFETY: EspBoard::new の契約により GPIO / IO_MUX はこのボードだけが触る。
        let (gpio, io_mux) = unsafe { (GPIO::steal(), IO_MUX::steal()) };
        let g = gpio.register_block();
        let m = io_mux.register_block();

        // IO_MUX: GPIO 機能にして、入力バッファとプルを設定する。
        m.gpio(n).write(|w| {
            unsafe { w.mcu_sel().bits(MCU_SEL_GPIO) };
            w.fun_ie()
                .bit(!matches!(mode, PinMode::Output | PinMode::OutputOpenDrain));
            w.fun_wpu().bit(matches!(mode, PinMode::InputPullUp));
            w.fun_wpd().bit(matches!(mode, PinMode::InputPullDown));
            w
        });

        // GPIO マトリクス: 出力は GPIO_OUT 信号に繋ぐ。
        g.func_out_sel_cfg(n)
            .write(|w| unsafe { w.out_sel().bits(SIG_GPIO_OUT) });

        // オープンドレインは PIN_PAD_DRIVER で切り替える。
        g.pin(n)
            .modify(|_, w| w.pad_driver().bit(matches!(mode, PinMode::OutputOpenDrain)));

        let (high, mask) = bank(index);
        match mode {
            PinMode::Output | PinMode::OutputOpenDrain => {
                if high {
                    g.enable1_w1ts().write(|w| unsafe { w.bits(mask) });
                } else {
                    g.enable_w1ts().write(|w| unsafe { w.bits(mask) });
                }
            }
            _ => {
                if high {
                    g.enable1_w1tc().write(|w| unsafe { w.bits(mask) });
                } else {
                    g.enable_w1tc().write(|w| unsafe { w.bits(mask) });
                }
            }
        }
        Ok(())
    }

    fn gpio_write(&mut self, index: u32, level: Level) -> BoardResult<()> {
        if index >= NUM_GPIO {
            return Err(ErrorCode::InvalidArgument);
        }
        // SAFETY: 同上。
        let gpio = unsafe { GPIO::steal() };
        let g = gpio.register_block();
        let (high, mask) = bank(index);
        let enabled = if high {
            g.enable1().read().bits() & mask
        } else {
            g.enable().read().bits() & mask
        };
        if enabled == 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        match (level, high) {
            (Level::High, false) => g.out_w1ts().write(|w| unsafe { w.bits(mask) }),
            (Level::Low, false) => g.out_w1tc().write(|w| unsafe { w.bits(mask) }),
            (Level::High, true) => g.out1_w1ts().write(|w| unsafe { w.bits(mask) }),
            (Level::Low, true) => g.out1_w1tc().write(|w| unsafe { w.bits(mask) }),
        };
        Ok(())
    }

    fn gpio_read(&mut self, index: u32) -> BoardResult<Level> {
        if index >= NUM_GPIO {
            return Err(ErrorCode::InvalidArgument);
        }
        // SAFETY: 同上。
        let gpio = unsafe { GPIO::steal() };
        let g = gpio.register_block();
        let (high, mask) = bank(index);
        let (enable, out, input) = if high {
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
        // 出力モードなら出力値、入力モードなら入力値（WIT の read の定義）。
        let bits = if enable & mask != 0 { out } else { input };
        Ok(if bits & mask != 0 {
            Level::High
        } else {
            Level::Low
        })
    }

    fn gpio_release(&mut self, index: u32) {
        // abi-spec §5.2: 入力・プル無しに戻す。
        let _ = self.gpio_configure(index, PinMode::Input);
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
