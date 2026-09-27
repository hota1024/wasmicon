//! ESP32-S3 (DevKitC-1) のボード実装。
//!
//! GPIO は番号で動的に触る必要があるので、型付きピンではなく GPIO / IO_MUX の
//! レジスタを直接叩く。HAL の共通部分は `wasmicon-port` にある。
//!
//! SPI は SPI2（FSPI）を `esp-hal` の `spi::master` ドライバで使う。GPIO と違って
//! レジスタ直叩きにしていないのは、GSPI のクロック分周・GPIO マトリクスの
//! 配線・FIFO の扱いを自前で持つ理由が無いため。ドライバが要求する
//! 「所有された」ペリフェラルとピンは `steal()` で作る（ボードは
//! `EspBoard::new` の契約でこれらを排他的に持つ）。
//!
//! `ports/rp2350` の SPI と揃えた点（docs/TODO.md §1.2）:
//!
//! - **周波数は要求値を超えない最大**。`esp-hal` の `Config::with_frequency` が
//!   そう書いてある（"The closest available frequency that does not exceed
//!   `frequency` is used"）。APB 80 MHz のとき 16 MHz の要求は 80/5 でちょうど
//!   16 MHz（RP2350 は clk_peri 150 MHz から 15 MHz に丸める）
//! - **送信が終わるまで戻らない**。`Spi::write` は FIFO 64 バイトごとに
//!   `flush()`（`CMD.USR` が落ちるまで待つ）を通す。PL022 の `BSY` 待ちと
//!   同じ役割で、これが無いと最後のバイトがシフト中に DC / CS が動いて
//!   ILI9341 がコマンドとデータを取り違える
//! - **失敗条件**。`index != 0` は `unsupported`、`frequency-hz == 0` は
//!   `invalid-argument`。下限（APB/1024 = 78.125 kHz）未満の要求は
//!   `esp-hal` が弾くので `unsupported` にしている。RP2350 は最も遅い
//!   組み合わせに張り付けるので、**この 1 点だけ揃っていない**
//!   （docs/TODO.md §2.1）
//!
//! I2C はまだ `unsupported`。

use esp_hal::gpio::AnyPin;
use esp_hal::peripherals::{GPIO, IO_MUX, SPI2};
use esp_hal::spi::master::{Config as HalSpiConfig, Spi};
use esp_hal::spi::Mode as HalSpiMode;
use esp_hal::time::Rate;
use esp_hal::Blocking;
use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::i2c::Speed;
use wasmicon_core::generated::log::Level as LogLevel;
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_core::generated::ErrorCode;
use wasmicon_port::{Board, BoardResult};

/// ESP32-S3 の GPIO は 0..48（21..25 は存在しない番号がある）。
const NUM_GPIO: u32 = 49;

/// IO_MUX の MCU_SEL に入れる「GPIO 機能」。ESP32-S3 では 1
/// （`esp-metadata-generated` の `gpio.gpio_function`）。
const MCU_SEL_GPIO: u8 = 1;

/// IO_MUX の FUN_DRV。2 = 20 mA で、ESP32-S3 のパッドのリセット値と同じ。
const FUN_DRV_DEFAULT: u8 = 2;

/// GPIO マトリクスの「GPIO 出力」信号（`GPIO_FUNCn_OUT_SEL`）。
///
/// **ESP32-S3 では 256。128 ではない。** チップごとに違う値なので決め打ちで
/// 写してはいけない（`esp-metadata-generated` の `OutputSignal::GPIO`）:
/// ESP32 / S2 / S3 は 256、C3 / C6 など RISC-V 勢は信号マップが 128 本ぶん
/// 短いので 128。**S3 で 128 を書くと `I2S0O_SD1` がパッドに繋がるので、
/// `GPIO_OUT` / `GPIO_ENABLE` は正しく読めるのにピンが一切動かない。**
/// 2026-09-26 に実機で踏んだ: host call のトレースは host と 14,352 行
/// 完全一致するのに ILI9341 が真白のまま、という形で出た
/// （`docs/verification-report.md` §7）。
const SIG_GPIO_OUT: u16 = 256;

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

/// SPI2（FSPI）に割り当てるピン（abi-spec §8）。CS はここに含めない。
/// ゲストが `lcd-cs` の GPIO を直接振る（`wit/spi.wit` の設計）。
///
/// `reserved` には入れていない。入れると GPIO の開放可否が host の mock や
/// 他のポートと食い違い、トレースの突き合わせ（handoff §5 Phase 6）が
/// ボードごとに別物になるため。SPI を開いたまま同じ番号を `gpio` で開けば
/// GPIO マトリクスの配線が上書きされて SPI は黙って止まるが、それは
/// ゲスト側の誤りとする（`ports/rp2350` と同じ扱い）。
const SPI2_SCK: u8 = 12;
const SPI2_MOSI: u8 = 11;
const SPI2_MISO: u8 = 13;

/// トレースとログを出す先。
pub trait Serial {
    fn write(&mut self, bytes: &[u8]);
}

/// DevKitC-1 のボード。
pub struct EspBoard<S: Serial> {
    serial: S,
    /// 開いている SPI2 のドライバ。`spi_close` で落とすと `Drop` が
    /// ピンの配線とペリフェラルのクロックを戻す。
    spi: Option<Spi<'static, Blocking>>,
}

impl<S: Serial> EspBoard<S> {
    /// # Safety
    /// GPIO / IO_MUX / SPI2、および SPI2 に割り当てた GPIO
    /// (`SPI2_SCK` / `SPI2_MOSI` / `SPI2_MISO`) をこのボードが排他的に
    /// 使うこと。
    #[must_use]
    pub unsafe fn new(serial: S) -> Self {
        EspBoard { serial, spi: None }
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
        //
        // `write()` なので、指定しない欄はリセット値に戻る。プルを開き直しで
        // 確実に落とすためにこうしているが、**ドライブ強度はリセット値に
        // 任せず明示する**（SVD のリセット値に依存させない）。2 = 20 mA で、
        // これは ESP32-S3 のパッドの既定でもある。
        m.gpio(n).write(|w| {
            // SAFETY: MCU_SEL_GPIO (1) は MCU_SEL（3 bit）、FUN_DRV_DEFAULT (2) は
            // FUN_DRV（2 bit）の範囲内。
            unsafe { w.mcu_sel().bits(MCU_SEL_GPIO) };
            unsafe { w.fun_drv().bits(FUN_DRV_DEFAULT) };
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
        //
        // **これで LCD の `RESET` が浮く。** 2026-09-26 に実機で確認: デモが
        // 描き終わってハンドルが解放されると、`lcd-rst` が入力に戻って線が
        // 浮き、モジュール側にプルアップが無いためパネルがリセットして画面が
        // 白に戻る。**ポート固有ではない**（§5.2 の帰結で、`ports/rp2350` でも
        // 同じ条件が揃えば起きる）。線のアイドルレベルは外部回路が決めるべき
        // もので、`RESET` に 10 kΩ 程度のプルアップを入れれば解決する
        // （docs/verification-report.md §7、apps/lcd-demo-rs/README.md）。
        let _ = self.gpio_configure(index, PinMode::Input);
    }

    // --- I2C は Phase 5 で実装する（docs/TODO.md §1.2） ---

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

    // --- SPI。index 0 = SPI2 (SCK=GPIO12, MOSI=GPIO11, MISO=GPIO13) ---

    fn spi_open(&mut self, index: u32, frequency_hz: u32, mode: SpiMode) -> BoardResult<()> {
        // SPI3 もあるが、v0.1 で割り当てているのは SPI2 だけ（abi-spec §8）。
        // Hal 側で index < MAX_SPI は検査済み。
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        if frequency_hz == 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        // Hal 側が二重 open を Busy で弾くので普通はここに残っていないが、
        // 残っていたら先に落とす（`steal` したペリフェラルを二重に持たない）。
        self.spi = None;

        let config = HalSpiConfig::default()
            .with_frequency(Rate::from_hz(frequency_hz))
            .with_mode(match mode {
                SpiMode::Mode0 => HalSpiMode::_0,
                SpiMode::Mode1 => HalSpiMode::_1,
                SpiMode::Mode2 => HalSpiMode::_2,
                SpiMode::Mode3 => HalSpiMode::_3,
            });

        // SAFETY: EspBoard::new の契約により、SPI2 と SPI2_* のピンはこのボード
        // だけが触る。self.spi を先に None にしてあるので、同じものを指す
        // ドライバは同時に 2 つ存在しない。
        let (spi2, sck, mosi, miso) = unsafe {
            (
                SPI2::steal(),
                AnyPin::steal(SPI2_SCK),
                AnyPin::steal(SPI2_MOSI),
                AnyPin::steal(SPI2_MISO),
            )
        };
        // 失敗するのは要求周波数が出せる範囲の外のときだけ
        // （上は APB、下は APB/1024）。
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
        // 64 バイト（FIFO）ごとに区切られるが、CS はゲストが握ったままなので
        // ILI9341 から見れば 1 回の転送のまま。区切りごとに flush が入る。
        spi.write(data).map_err(|_| ErrorCode::Io)
    }

    fn spi_transfer(&mut self, index: u32, data: &[u8], buf: &mut [u8]) -> BoardResult<usize> {
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        let spi = self.spi.as_mut().ok_or(ErrorCode::InvalidHandle)?;
        let n = data.len().min(buf.len());
        // ドライバの全二重転送はその場置き換えなので、送る側を写してから渡す。
        buf[..n].copy_from_slice(&data[..n]);
        spi.transfer(&mut buf[..n]).map_err(|_| ErrorCode::Io)?;
        Ok(n)
    }

    fn spi_close(&mut self, index: u32) {
        if index != 0 {
            return;
        }
        // Drop がピンの配線とペリフェラルのクロックを戻す。
        self.spi = None;
        // gpio_release と同じく入力・プル無しに戻す。ドライバの Drop は
        // GPIO マトリクスの信号を外すだけで、パッドの向きやプルは触らない。
        for n in [SPI2_SCK, SPI2_MOSI, SPI2_MISO] {
            let _ = self.gpio_configure(u32::from(n), PinMode::Input);
        }
    }

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
