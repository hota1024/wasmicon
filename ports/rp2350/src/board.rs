//! Raspberry Pi Pico 2 / Pico 2 W のボード実装。
//!
//! GPIO は番号で動的に触る必要があるので、型付きピンではなく SIO / IO_BANK0 /
//! PADS_BANK0 のレジスタを直接叩く。HAL の共通部分は `wasmicon-port` にある。
//!
//! RP2040 版 (`ports/rp2040`) とほぼ同じだが、RP2350 では次が違う:
//!
//! - パッドに **アイソレーションラッチ (`ISO`) があり、リセット値が 1**。
//!   これを落とさないとパッドは切り離されたままで、GPIO は静かに死ぬ。
//!   PADS_BANK0 への `write()` はリセット値から始まるので、書くたびに
//!   明示的に `iso().clear_bit()` する必要がある
//! - FUNCSEL が型付きの enum なので `funcsel().sio()` で書ける（`unsafe` 不要）
//! - タイマは `TIMER0`（RP2350 には TIMER0 / TIMER1 の 2 つある）

use rp235x_hal::pac;
use wasmicon_core::generated::ErrorCode;
use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::i2c::Speed;
use wasmicon_core::generated::log::Level as LogLevel;
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_port::{Board, BoardResult};

/// RP2350A（Pico 2 / Pico 2 W の QFN-60）の GPIO は GP0..GP29。
/// QFN-80 の RP2350B は GP0..GP47 だが、対象ボードには載っていない。
const NUM_GPIO: u32 = 30;

/// 役割名 → GPIO 番号（abi-spec §8 の表）。
///
/// Pico 2 / Pico 2 W はヘッダのピン配置が Pico / Pico WH と同じなので、
/// RP2040 ポートと同じ番号にしてある。
///
/// `led` が外付けなのは、Pico 2 W のオンボード LED が CYW43439 側にあって
/// RP2350 の GPIO では駆動できないため（Pico 2 は GP25 だが、両方で同じ
/// 配線にするため外付けに揃える）。**実機の配線は未確認**（docs/TODO.md §1.1）。
const ROLES: &[(&str, u32)] = &[("led", 15), ("lcd-cs", 17), ("lcd-dc", 20), ("lcd-rst", 21)];

/// ゲストに開放しない GPIO。
///
/// GP0/GP1 はトレース用の UART0。開けられるとトレースが途切れる。
/// GP23/24/25/29 は Pico 2 W では CYW43439（無線とオンボード LED）に、
/// Pico 2 では SMPS の PS / VBUS 検出 / オンボード LED / VSYS 監視に
/// 繋がっている。どちらでも同じ物が動くよう、両方まとめて閉じておく。
const RESERVED: &[u32] = &[0, 1, 23, 24, 25, 29];

/// トレースとログを出す先。
pub trait Serial {
    fn write(&mut self, bytes: &[u8]);
}

/// Pico 2 のボード。
pub struct Pico2Board<S: Serial> {
    serial: S,
    /// オープンドレインとして開いているピン。
    ///
    /// RP2350 のパッドにもオープンドレイン制御は無い（`OD` は出力ディセーブル）
    /// ので、出力イネーブルで擬似する: low は OE=1 かつ OUT=0、high は OE=0 で
    /// ハイインピーダンス。
    open_drain: u32,
}

impl<S: Serial> Pico2Board<S> {
    /// # Safety
    /// SIO / IO_BANK0 / PADS_BANK0 / TIMER0 をこのボードが排他的に使うこと。
    /// 呼び出し側は同じペリフェラルを他で触らない責任を負う。
    #[must_use]
    pub unsafe fn new(serial: S) -> Self {
        Pico2Board {
            serial,
            open_drain: 0,
        }
    }

    /// シリアルへの参照。失敗の理由を出すのに使う。
    pub fn serial(&mut self) -> &mut S {
        &mut self.serial
    }

    fn sio() -> pac::SIO {
        // SAFETY: Pico2Board::new の契約により、SIO はこのボードだけが触る。
        unsafe { pac::Peripherals::steal().SIO }
    }

    fn timer() -> pac::TIMER0 {
        // SAFETY: 同上。TIMER0 は読み出しのみ。
        unsafe { pac::Peripherals::steal().TIMER0 }
    }
}

impl<S: Serial> Board for Pico2Board<S> {
    fn pin_by_role(&self, role: &str) -> Option<u32> {
        ROLES.iter().find(|(r, _)| *r == role).map(|(_, i)| *i)
    }

    fn gpio_count(&self) -> u32 {
        NUM_GPIO
    }

    fn gpio_reserved(&self, index: u32) -> bool {
        RESERVED.contains(&index)
    }

    fn gpio_configure(&mut self, index: u32, mode: PinMode) -> BoardResult<()> {
        if index >= NUM_GPIO {
            return Err(ErrorCode::InvalidArgument);
        }
        let n = index as usize;
        // SAFETY: Pico2Board::new の契約により、これらのペリフェラルは排他。
        let p = unsafe { pac::Peripherals::steal() };

        // パッドの設定。入力バッファとプル、出力ディセーブル、そして
        // アイソレーションの解除。`write()` はリセット値（ISO=1, PDE=1）から
        // 始まるので、どのフィールドも書き残さない。
        p.PADS_BANK0.gpio(n).write(|w| {
            w.ie().set_bit();
            w.od().clear_bit();
            match mode {
                PinMode::InputPullUp => {
                    w.pue().set_bit();
                    w.pde().clear_bit();
                }
                PinMode::InputPullDown => {
                    w.pue().clear_bit();
                    w.pde().set_bit();
                }
                _ => {
                    w.pue().clear_bit();
                    w.pde().clear_bit();
                }
            }
            // 最後にパッドを繋ぐ。落とし忘れると GPIO は無反応のままになる。
            w.iso().clear_bit();
            w
        });

        // ソフトウェア制御にする。
        p.IO_BANK0.gpio(n).gpio_ctrl().write(|w| w.funcsel().sio());

        let mask = 1u32 << index;
        match mode {
            PinMode::OutputOpenDrain => {
                // ハイインピーダンスから始める（外部プルアップに任せる）。
                self.open_drain |= mask;
                p.SIO.gpio_out_clr().write(|w| unsafe { w.bits(mask) });
                p.SIO.gpio_oe_clr().write(|w| unsafe { w.bits(mask) });
            }
            PinMode::Output => {
                self.open_drain &= !mask;
                p.SIO.gpio_oe_set().write(|w| unsafe { w.bits(mask) });
            }
            _ => {
                self.open_drain &= !mask;
                p.SIO.gpio_oe_clr().write(|w| unsafe { w.bits(mask) });
            }
        }
        Ok(())
    }

    fn gpio_write(&mut self, index: u32, level: Level) -> BoardResult<()> {
        if index >= NUM_GPIO {
            return Err(ErrorCode::InvalidArgument);
        }
        let sio = Self::sio();
        let mask = 1u32 << index;
        if self.open_drain & mask != 0 {
            // オープンドレイン: low は駆動、high はハイインピーダンス。
            match level {
                Level::Low => {
                    sio.gpio_out_clr().write(|w| unsafe { w.bits(mask) });
                    sio.gpio_oe_set().write(|w| unsafe { w.bits(mask) });
                }
                Level::High => sio.gpio_oe_clr().write(|w| unsafe { w.bits(mask) }),
            }
            return Ok(());
        }
        // 出力に設定されていなければ書けない。
        if sio.gpio_oe().read().bits() & mask == 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        match level {
            Level::High => sio.gpio_out_set().write(|w| unsafe { w.bits(mask) }),
            Level::Low => sio.gpio_out_clr().write(|w| unsafe { w.bits(mask) }),
        }
        Ok(())
    }

    fn gpio_read(&mut self, index: u32) -> BoardResult<Level> {
        if index >= NUM_GPIO {
            return Err(ErrorCode::InvalidArgument);
        }
        let sio = Self::sio();
        let mask = 1u32 << index;
        // 出力モードなら出力値、入力モードなら入力値（WIT の read の定義）。
        let bits = if self.open_drain & mask != 0 {
            // オープンドレインは線の実際の状態を読む。
            sio.gpio_in().read().bits()
        } else if sio.gpio_oe().read().bits() & mask != 0 {
            sio.gpio_out().read().bits()
        } else {
            sio.gpio_in().read().bits()
        };
        Ok(if bits & mask != 0 {
            Level::High
        } else {
            Level::Low
        })
    }

    fn gpio_release(&mut self, index: u32) {
        if index >= NUM_GPIO {
            return;
        }
        // abi-spec §5.2: 入力・プル無しに戻す。
        self.open_drain &= !(1u32 << index);
        let _ = self.gpio_configure(index, PinMode::Input);
    }

    // --- I2C / SPI は Phase 5 で実装する（docs/TODO.md §1.2） ---

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

    // --- 時間。TIMER0 は 1 MHz なのでそのままマイクロ秒。 ---

    fn now_us(&mut self) -> u64 {
        let t = Self::timer();
        // TIMELR を読むと TIMEHR がラッチされる。順序を守る。
        let lo = t.timelr().read().bits();
        let hi = t.timehr().read().bits();
        (u64::from(hi) << 32) | u64::from(lo)
    }

    fn sleep_ms(&mut self, ms: u32) {
        self.sleep_us(ms.saturating_mul(1000));
    }

    fn sleep_us(&mut self, us: u32) {
        let start = self.now_us();
        let target = start.saturating_add(u64::from(us));
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
