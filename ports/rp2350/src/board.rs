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
//!
//! SPI は SPI0（PL022）をレジスタ直叩きで使う。`rp235x-hal` の `Spi` は
//! 型付きピンを要求するが、ボードは `Peripherals::steal()` で動くので
//! 所有権を渡せない。GPIO と同じ書き方に揃えてある。
//! GPIO と SPI は Pico 2 W 実機で確認済み（docs/verification-report.md §6）。
//! I2C はまだ `unsupported`。

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

/// SPI0 に割り当てるピン（abi-spec §8）。RP2350 ではこの 3 本の FUNCSEL=1 が
/// SPI0 に繋がる。CS はここに含めない。ゲストが `lcd-cs` の GPIO を直接振る
/// （`wit/spi.wit` の設計）。
///
/// `RESERVED` には入れていない。入れると GPIO の開放可否が host の mock や
/// 他のポートと食い違い、トレースの突き合わせ（handoff §5 Phase 6）が
/// ボードごとに別物になるため。SPI を開いたまま同じ番号を `gpio` で開けば
/// FUNCSEL が SIO に戻って SPI は黙って止まるが、それはゲスト側の誤りとする。
const SPI0_SCK: usize = 18;
const SPI0_MOSI: usize = 19;
const SPI0_MISO: usize = 16;

/// トレースとログを出す先。
pub trait Serial {
    fn write(&mut self, bytes: &[u8]);
}

/// Pico 2 のボード。
pub struct Pico2Board<S: Serial> {
    serial: S,
    /// `clk_peri` の周波数。SPI の分周器を決めるのに要る。
    /// 起動時に決まった実際の値を受け取る（150 MHz を決め打ちにしない）。
    peri_clock_hz: u32,
    /// オープンドレインとして開いているピン。
    ///
    /// RP2350 のパッドにもオープンドレイン制御は無い（`OD` は出力ディセーブル）
    /// ので、出力イネーブルで擬似する: low は OE=1 かつ OUT=0、high は OE=0 で
    /// ハイインピーダンス。
    open_drain: u32,
}

impl<S: Serial> Pico2Board<S> {
    /// # Safety
    /// SIO / IO_BANK0 / PADS_BANK0 / TIMER0 / SPI0、および RESETS の SPI0 ビットを
    /// このボードが排他的に使うこと。呼び出し側は同じペリフェラルを他で触らない
    /// 責任を負う。
    ///
    /// `peri_clock_hz` には `clocks.peripheral_clock.freq()` を渡す。
    #[must_use]
    pub unsafe fn new(serial: S, peri_clock_hz: u32) -> Self {
        Pico2Board {
            serial,
            peri_clock_hz,
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

    fn spi0() -> pac::SPI0 {
        // SAFETY: 同上。SPI0 はこのボードだけが触る。
        unsafe { pac::Peripherals::steal().SPI0 }
    }

    /// 送信が完全に終わるまで待ち、受信 FIFO を空にする。
    ///
    /// **`spi.write` / `spi.transfer` から戻る前に必ず通すこと。** ゲストは
    /// 戻った直後に DC や CS を動かす。最後のバイトがまだシフトレジスタに
    /// 残っている状態でそれをやると、ILI9341 はコマンドとデータを取り違える
    /// （レジスタの設定は正しいのに画面が壊れる、という形で出る）。
    fn spi_drain(spi: &pac::SPI0) {
        while spi.sspsr().read().bsy().bit_is_set() {
            core::hint::spin_loop();
        }
        while spi.sspsr().read().rne().bit_is_set() {
            let _ = spi.sspdr().read().bits();
        }
        // 読み捨てている間に立った受信オーバーランを落とす
        // （SSPICR は 1 を書いてクリアするレジスタ）。
        spi.sspicr().write(|w| w.roric().clear_bit_by_one());
    }
}

/// PL022 の分周器 `(cpsdvsr, scr)` を決める。出力は
/// `clk_peri / (cpsdvsr × (1 + scr))`。`cpsdvsr` は 2..=254 の偶数、
/// `scr` は 0..=255。
///
/// **要求値を超えない範囲で最も速い組み合わせ**を選ぶ。`wit/spi.wit` は
/// 「最も近い値に丸める」としているが、切り上げるとディスプレイの上限を
/// 越えうるので切り下げる方に倒している（例: `clk_peri` 150 MHz で
/// 24 MHz を頼むと 18.75 MHz になる）。rp2040 ポートに SPI を足すときも
/// 同じ規則にすること。
///
/// `cpsdvsr` を小さいほうから試し、`scr`（総分周 1..=256）で要求値以下に
/// 収まった最初の組を採る。総分周が 512 以下（`cpsdvsr` = 2 で表せる範囲）なら
/// 総分周は必ず偶数なので、これで「要求値以下で最大」になる。それより遅い
/// 要求では `cpsdvsr` の倍数の刻みしか試さないため、1 段遅い組を採ることが
/// ある（総分周 517 が要るとき 4×130 = 520 を採る。14×37 = 518 のほうが近い）。
/// ディスプレイの上限を越えない方向の誤差なので、そのままにしてある。
/// rp235x-hal の `set_baudrate` は先に
/// `cpsdvsr` を当て推量で決めるので、その境目（150 MHz で 100 kHz を
/// 頼むなど）では要求値を上回ることがある。ここでは総当たりにしてある。
///
/// 出せる範囲の外は端に丸める。上は `clk_peri / 2`、下は
/// `clk_peri / (254 × 256)`。
fn spi_divisors(clk_hz: u32, want_hz: u32) -> (u8, u8) {
    // 分母に来るので 0 は弾く（呼び出し側でも検査している）。
    let want = u64::from(want_hz.max(1));
    let clk = u64::from(clk_hz);

    let mut cpsdvsr: u32 = 2;
    while cpsdvsr <= 254 {
        let mut div: u32 = 1;
        while div <= 256 {
            if clk / (u64::from(cpsdvsr) * u64::from(div)) <= want {
                return (cpsdvsr as u8, (div - 1) as u8);
            }
            div += 1;
        }
        cpsdvsr += 2;
    }
    // 要求が下限より低い。最も遅い組み合わせに張り付ける。
    (254, 255)
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

    // --- SPI。index 0 = SPI0 (SCK=GP18, MOSI=GP19, MISO=GP16) ---

    fn spi_open(&mut self, index: u32, frequency_hz: u32, mode: SpiMode) -> BoardResult<()> {
        // spi1 もヘッダに出ているが、v0.1 で割り当てているのは spi0 だけ
        // （abi-spec §8）。Hal 側で index < MAX_SPI は検査済み。
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        if frequency_hz == 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        // SAFETY: Pico2Board::new の契約により、これらのペリフェラルは排他。
        let p = unsafe { pac::Peripherals::steal() };

        // SPI0 はリセットが掛かったまま起動する。解除して完了を待つ。
        // GPIO と違いここでしか解除していないので、忘れるとレジスタへの
        // 書き込みが素通りする。
        p.RESETS.reset().modify(|_, w| w.spi0().clear_bit());
        while p.RESETS.reset_done().read().spi0().bit_is_clear() {
            core::hint::spin_loop();
        }

        // SCK / MOSI / MISO をパッドに出す。GPIO と同じく `write()` は
        // リセット値（ISO=1, PDE=1）から始まるので、ISO を落とし忘れると
        // パッドが切り離されたままになる。
        for n in [SPI0_SCK, SPI0_MOSI, SPI0_MISO] {
            p.PADS_BANK0.gpio(n).write(|w| {
                // 出力側でも入力バッファは有効にしておく（PL022 は MISO を
                // 読むだけだが、rp235x-hal も 3 本まとめて立てている）。
                w.ie().set_bit();
                w.od().clear_bit();
                w.pue().clear_bit();
                w.pde().clear_bit();
                w.iso().clear_bit();
                w
            });
            p.IO_BANK0.gpio(n).gpio_ctrl().write(|w| w.funcsel().spi());
        }

        let (cpsdvsr, scr) = spi_divisors(self.peri_clock_hz, frequency_hz);
        let (spo, sph) = match mode {
            SpiMode::Mode0 => (false, false),
            SpiMode::Mode1 => (false, true),
            SpiMode::Mode2 => (true, false),
            SpiMode::Mode3 => (true, true),
        };

        let spi = p.SPI0;
        // 設定の前に必ず落とす（SSE=1 のまま CR0 を触ると挙動が未定義）。
        spi.sspcr1().write(|w| w);
        // SAFETY: cpsdvsr / scr は spi_divisors が範囲内に収めている。
        spi.sspcpsr()
            .write(|w| unsafe { w.cpsdvsr().bits(cpsdvsr) });
        spi.sspcr0().write(|w| {
            // SAFETY: 7 は DSS（4 bit）の範囲内。8 bit フレームを表す。
            unsafe { w.dss().bits(7) };
            w.frf().motorola();
            w.spo().bit(spo);
            w.sph().bit(sph);
            // SAFETY: scr は u8 なので SCR（8 bit）に収まる。
            unsafe { w.scr().bits(scr) };
            w
        });
        // マスターとして有効にする。ビット順は MSB first 固定（PL022 の
        // Motorola フォーマットはこれしかない。wit/spi.wit の前提と一致）。
        spi.sspcr1().write(|w| w.ms().clear_bit().sse().set_bit());

        // 前の設定で残った受信があれば捨てる。
        Self::spi_drain(&spi);
        Ok(())
    }

    fn spi_write(&mut self, index: u32, data: &[u8]) -> BoardResult<()> {
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        let spi = Self::spi0();
        for &b in data {
            while spi.sspsr().read().tnf().bit_is_clear() {
                core::hint::spin_loop();
            }
            // SAFETY: 8 bit フレームなので DATA（16 bit）に収まる。
            spi.sspdr()
                .write(|w| unsafe { w.data().bits(u16::from(b)) });
            // 受信は捨てるが、FIFO（8 段）を溢れさせないよう都度抜く。
            while spi.sspsr().read().rne().bit_is_set() {
                let _ = spi.sspdr().read().bits();
            }
        }
        Self::spi_drain(&spi);
        Ok(())
    }

    fn spi_transfer(&mut self, index: u32, data: &[u8], buf: &mut [u8]) -> BoardResult<usize> {
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        let spi = Self::spi0();
        let n = data.len().min(buf.len());
        // 全二重なので 1 バイト送って 1 バイト受ける、を繰り返す。FIFO に
        // 詰めてから読むほうが速いが、受信の取りこぼしを考えなくて済む
        // この形にしてある（v0.1 の転送は最大 128 バイト）。
        for i in 0..n {
            while spi.sspsr().read().tnf().bit_is_clear() {
                core::hint::spin_loop();
            }
            // SAFETY: 8 bit フレームなので DATA（16 bit）に収まる。
            spi.sspdr()
                .write(|w| unsafe { w.data().bits(u16::from(data[i])) });
            while spi.sspsr().read().rne().bit_is_clear() {
                core::hint::spin_loop();
            }
            buf[i] = spi.sspdr().read().data().bits() as u8;
        }
        Self::spi_drain(&spi);
        Ok(n)
    }

    fn spi_close(&mut self, index: u32) {
        if index != 0 {
            return;
        }
        let spi = Self::spi0();
        Self::spi_drain(&spi);
        spi.sspcr1().write(|w| w.sse().clear_bit());
        // gpio_release と同じく入力・プル無しに戻す（FUNCSEL も SIO に戻る）。
        for n in [SPI0_SCK, SPI0_MOSI, SPI0_MISO] {
            let _ = self.gpio_configure(n as u32, PinMode::Input);
        }
    }

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
