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

use esp_hal::i2c::master::Config as I2cConfig;
use esp_hal::spi::master::Config as SpiConfig;
use esp_hal::spi::Mode as HalSpiMode;
use esp_hal::time::Rate;

use crate::boards::{I2cBus, Serial, SpiBus, DEF};
use crate::chip::{self, Gpio};

/// ESP32-P4 のボード。どのボードかは feature で選ぶ（`crate::boards`）。
pub struct EspBoard<S: Serial> {
    serial: S,
    gpio: Gpio,
    /// ゲストに開放するバス。Tab5 では PORT.A（外部ユニット用）。
    /// **内部 I2C はここに入れない。** 電源とタッチとコーデックが同じ線に
    /// いるので、ゲストに触らせない（abi-spec §8、docs/TODO.md §1.1.5）。
    i2c: I2cBus,
    /// ゲストに開放する SPI バス。Tab5 では M5-Bus の SPI2。
    /// CS / DC / RST は役割名で引く GPIO なので、ここには含めない。
    spi: SpiBus,
}

impl<S: Serial> EspBoard<S> {
    /// # Safety
    /// GPIO と IO_MUX をこのボードが排他的に使うこと。
    #[must_use]
    pub unsafe fn new(serial: S, i2c: I2cBus, spi: SpiBus) -> Self {
        // SAFETY: 呼び出し側の契約をそのまま chip::Gpio に渡す。
        let gpio = unsafe { Gpio::steal() };
        EspBoard {
            serial,
            gpio,
            i2c,
            spi,
        }
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

    // --- I2C: index 0 = ボードが公開する外部バス（Tab5 は PORT.A）---

    fn i2c_open(&mut self, index: u32, speed: Speed) -> BoardResult<()> {
        // index 0 だけ。ボードが持つバスは 1 本（abi-spec §8）。
        if index != 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        // SPI と同じく `open` の指定をそのまま反映する。黙って既定の 100 kHz で
        // 動かすと、`fast` / `fast-plus` を要求したゲストが成功を受け取りながら
        // 別の速度で通信することになり、理由の分からない結果になる。
        let cfg = I2cConfig::default().with_frequency(match speed {
            Speed::Standard => Rate::from_khz(100),
            Speed::Fast => Rate::from_khz(400),
            Speed::FastPlus => Rate::from_khz(1000),
        });
        // 分周器で作れない周波数は `ConfigError` になる。
        self.i2c
            .apply_config(&cfg)
            .map_err(|_| ErrorCode::InvalidArgument)
    }

    fn i2c_write(&mut self, index: u32, address: u16, data: &[u8]) -> BoardResult<()> {
        if index != 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        self.i2c.write(addr7(address)?, data).map_err(map_i2c_err)
    }

    fn i2c_read(&mut self, index: u32, address: u16, buf: &mut [u8]) -> BoardResult<usize> {
        if index != 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        self.i2c
            .read(addr7(address)?, buf)
            .map(|()| buf.len())
            .map_err(map_i2c_err)
    }

    fn i2c_write_read(
        &mut self,
        index: u32,
        address: u16,
        data: &[u8],
        buf: &mut [u8],
    ) -> BoardResult<usize> {
        if index != 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        self.i2c
            .write_read(addr7(address)?, data, buf)
            .map(|()| buf.len())
            .map_err(map_i2c_err)
    }

    fn i2c_close(&mut self, _index: u32) {}

    // --- SPI: index 0 = ボードが公開する外部バス（Tab5 は M5-Bus の SPI2）---

    fn spi_open(&mut self, index: u32, frequency_hz: u32, mode: SpiMode) -> BoardResult<()> {
        // index 0 だけ。ボードが持つバスは 1 本（abi-spec §8）。
        if index != 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        // I2C と違い、SPI は周波数とモードを `open` で指定できる（abi-spec §7）。
        let cfg = SpiConfig::default()
            .with_frequency(Rate::from_hz(frequency_hz))
            .with_mode(match mode {
                SpiMode::Mode0 => HalSpiMode::_0,
                SpiMode::Mode1 => HalSpiMode::_1,
                SpiMode::Mode2 => HalSpiMode::_2,
                SpiMode::Mode3 => HalSpiMode::_3,
            });
        // 分周器で作れない周波数は `ConfigError` になる。
        self.spi
            .apply_config(&cfg)
            .map_err(|_| ErrorCode::InvalidArgument)
    }

    fn spi_write(&mut self, index: u32, data: &[u8]) -> BoardResult<()> {
        if index != 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        self.spi.write(data).map_err(map_spi_err)
    }

    fn spi_transfer(&mut self, index: u32, data: &[u8], buf: &mut [u8]) -> BoardResult<usize> {
        if index != 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        // esp-hal の `transfer` は**その場で入れ替える**（送った分だけ受け取る）。
        // WIT は送信元と受信先が別なので、一度 `buf` へ写してから渡す。
        // 長さが違うときは短い方に合わせる（`ports/host` と同じ規則）。
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        self.spi.transfer(&mut buf[..n]).map_err(map_spi_err)?;
        Ok(n)
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

/// WIT の `u16` アドレスを 7bit に落とす。10bit アドレスは扱わない。
fn addr7(address: u16) -> BoardResult<u8> {
    u8::try_from(address)
        .ok()
        .filter(|a| *a < 0x80)
        .ok_or(ErrorCode::InvalidArgument)
}

/// esp-hal の SPI エラーを abi-spec §7 のコードへ落とす。
///
/// SPI は I2C と違って ACK が無いので、失敗はバスの使い方か容量の問題に限られる。
fn map_spi_err(e: esp_hal::spi::Error) -> ErrorCode {
    use esp_hal::spi::Error as E;
    match e {
        E::FifoSizeExeeded | E::MaxDmaTransferSizeExceeded => ErrorCode::OutOfMemory,
        E::Unsupported => ErrorCode::Unsupported,
        _ => ErrorCode::Io,
    }
}

/// esp-hal の I2C エラーを abi-spec §7 のコードへ落とす。
fn map_i2c_err(e: esp_hal::i2c::master::Error) -> ErrorCode {
    use esp_hal::i2c::master::Error as E;
    match e {
        E::AcknowledgeCheckFailed(_) => ErrorCode::Nack,
        E::Timeout => ErrorCode::Timeout,
        E::ZeroLengthInvalid => ErrorCode::InvalidArgument,
        E::FifoExceeded => ErrorCode::OutOfMemory,
        _ => ErrorCode::Io,
    }
}
