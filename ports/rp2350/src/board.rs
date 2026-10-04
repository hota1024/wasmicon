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
//!
//! I2C は I2C0（DW_apb_i2c）を同じくレジスタ直叩きで使う。初期化と転送の
//! 手順は `rp235x-hal` の `i2c/controller.rs` に合わせてあるが、**`assert!` を
//! 使わずエラーコードを返す**（パニックハンドラは理由を出せない。
//! docs/TODO.md §1.4）。**実機では未検証**（docs/TODO.md §1.2）。

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

/// I2C0 に割り当てるピン（abi-spec §8）。FUNCSEL=3 が I2C0 に繋がる。
///
/// `SPI0_*` と同じ理由で `RESERVED` には入れていない。
const I2C0_SDA: usize = 4;
const I2C0_SCL: usize = 5;

/// I2C の 1 バイトあたりの待ち上限。
///
/// **仕様ではなく診断のための値**。クロックが止まる・相手が SCL を握り
/// 続けるといった状況で無言で固まらないようにするだけのもの。100 kHz で
/// 1 バイト（9 ビット）は 90 µs なので 25 ms は 250 倍以上の余裕があり、
/// 正常系で踏むことはない。踏んだら `timeout` を返す。
///
/// 反復回数ではなく TIMER0 の実時間で計るのは、回数だと `opt-level` で
/// 意味が変わるため。`ports/esp32s3` は `esp-hal` が同じ役割の
/// `Error::Timeout` を返すので、**2 つのポートはここで揃っている**。
const I2C_TIMEOUT_US: u64 = 25_000;

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

    fn i2c0() -> pac::I2C0 {
        // SAFETY: 同上。I2C0 はこのボードだけが触る。
        unsafe { pac::Peripherals::steal().I2C0 }
    }

    /// 現在のマイクロ秒。TIMER0 は 1 MHz なのでそのまま使える。
    ///
    /// TIMELR を読むと TIMEHR がラッチされるので、順序を守る。
    fn micros() -> u64 {
        let t = Self::timer();
        let lo = t.timelr().read().bits();
        let hi = t.timehr().read().bits();
        (u64::from(hi) << 32) | u64::from(lo)
    }

    /// `IC_TX_ABRT_SOURCE` を読み、立っていれば握ってエラーに変える。
    ///
    /// **このレジスタは `IC_CLR_TX_ABRT` を読むまでクリアされない**
    /// （`IC_CLR_TX_ABRT` 自体は常に 0 を読む）。クリアを忘れると TX FIFO が
    /// 再武装されず、次の転送が通らない。
    ///
    /// アボートのときハードウェアが STOP を自動で出すので、ここで STOP を
    /// 送る必要はない（`rp235x-hal` の `write_internal` のコメントと同じ）。
    fn i2c_take_abort(i2c: &pac::I2C0) -> BoardResult<()> {
        let src = i2c.ic_tx_abrt_source().read();
        if src.bits() == 0 {
            return Ok(());
        }
        // 読んでクリアする。
        let _ = i2c.ic_clr_tx_abrt().read();
        // アドレス / データが NACK されたのか、それ以外（調停負けなど）か。
        // host の mock は相手が居ないとき `nack` を返すので、そこに揃える。
        let nacked = src.abrt_7b_addr_noack().bit_is_set()
            || src.abrt_10addr1_noack().bit_is_set()
            || src.abrt_10addr2_noack().bit_is_set()
            || src.abrt_txdata_noack().bit_is_set()
            || src.abrt_gcall_noack().bit_is_set();
        Err(if nacked {
            ErrorCode::Nack
        } else {
            ErrorCode::Io
        })
    }

    /// STOP が出るまで待ってフラグを落とす。
    fn i2c_wait_stop(i2c: &pac::I2C0) -> BoardResult<()> {
        let deadline = Self::micros().saturating_add(I2C_TIMEOUT_US);
        while i2c.ic_raw_intr_stat().read().stop_det().is_inactive() {
            if Self::micros() > deadline {
                return Err(ErrorCode::Timeout);
            }
            core::hint::spin_loop();
        }
        let _ = i2c.ic_clr_stop_det().read();
        Ok(())
    }

    /// 転送相手のアドレスを設定する。
    ///
    /// `IC_TAR` は ENABLE=1 のままでは変えられないので一度落とす。
    /// **`IC_ENABLE_STATUS.IC_EN` が落ちきるのを待つ**（`rp235x-hal` は
    /// 待っていないが、待たないと `open` → `close` → `open` を跨いだときに
    /// 書き込みが無視されうる）。
    fn i2c_set_target(i2c: &pac::I2C0, address: u16) -> BoardResult<()> {
        // 7 bit アドレスのみ（`wit/i2c.wit`）。`ports/esp32s3` も `esp-hal` が
        // 範囲外を `AddressInvalid` で弾くので、2 つのポートは揃っている
        // （host の mock は検査しない。docs/TODO.md §2.1）。
        if address > 0x7f {
            return Err(ErrorCode::InvalidArgument);
        }
        i2c.ic_enable().write(|w| w.enable().disabled());
        let deadline = Self::micros().saturating_add(I2C_TIMEOUT_US);
        while i2c.ic_enable_status().read().ic_en().bit_is_set() {
            if Self::micros() > deadline {
                return Err(ErrorCode::Timeout);
            }
            core::hint::spin_loop();
        }
        i2c.ic_con()
            .modify(|_, w| w.ic_10bitaddr_master().addr_7bits());
        // SAFETY: 上で 0x7f 以下に絞っているので IC_TAR（10 bit）に収まる。
        i2c.ic_tar().write(|w| unsafe { w.ic_tar().bits(address) });
        i2c.ic_enable().write(|w| w.enable().enabled());
        Ok(())
    }

    /// `data` を送る。`stop` が false なら STOP を出さない
    /// （`write-read` の前半で使う。次の read が Repeated START になる）。
    ///
    /// **`data` は空でないこと**（呼び出し側が `invalid-argument` で弾く）。
    /// 空だと `data.len() - 1` が溢れる。
    fn i2c_send(i2c: &pac::I2C0, data: &[u8], stop: bool) -> BoardResult<()> {
        let last = data.len() - 1;
        for (i, &b) in data.iter().enumerate() {
            let deadline = Self::micros().saturating_add(I2C_TIMEOUT_US);
            while i2c.ic_status().read().tfnf().bit_is_clear() {
                // FIFO が空かないのは相手が止まっているとき。アボートが
                // 立っていればそれを理由として返す。
                Self::i2c_take_abort(i2c)?;
                if Self::micros() > deadline {
                    return Err(ErrorCode::Timeout);
                }
                core::hint::spin_loop();
            }
            i2c.ic_data_cmd().write(|w| {
                w.stop().bit(stop && i == last);
                // SAFETY: DAT は 8 bit。
                unsafe { w.dat().bits(b) }
            });
        }
        // シフトレジスタから出きるまで待ってからアボートを見る。
        let deadline = Self::micros().saturating_add(I2C_TIMEOUT_US);
        while i2c.ic_raw_intr_stat().read().tx_empty().is_inactive() {
            if Self::micros() > deadline {
                return Err(ErrorCode::Timeout);
            }
            core::hint::spin_loop();
        }
        Self::i2c_take_abort(i2c)?;
        if stop {
            Self::i2c_wait_stop(i2c)?;
        }
        Ok(())
    }

    /// `buf` を埋めるまで読む。読み出しは**1 バイトにつき READ コマンドを
    /// 1 つ `IC_DATA_CMD` に積む**必要がある（積まないとクロックが出ない）。
    ///
    /// **`buf` は空でないこと**（呼び出し側が `invalid-argument` で弾く）。
    /// 空だと `n - 1` が溢れる。
    fn i2c_recv(i2c: &pac::I2C0, buf: &mut [u8]) -> BoardResult<usize> {
        let n = buf.len();
        let last = n - 1;
        for (i, byte) in buf.iter_mut().enumerate() {
            let deadline = Self::micros().saturating_add(I2C_TIMEOUT_US);
            while i2c.ic_status().read().tfnf().bit_is_clear() {
                Self::i2c_take_abort(i2c)?;
                if Self::micros() > deadline {
                    return Err(ErrorCode::Timeout);
                }
                core::hint::spin_loop();
            }
            i2c.ic_data_cmd().write(|w| {
                w.stop().bit(i == last);
                w.cmd().read()
            });

            let deadline = Self::micros().saturating_add(I2C_TIMEOUT_US);
            while i2c.ic_rxflr().read().bits() == 0 {
                Self::i2c_take_abort(i2c)?;
                if Self::micros() > deadline {
                    return Err(ErrorCode::Timeout);
                }
                core::hint::spin_loop();
            }
            *byte = i2c.ic_data_cmd().read().dat().bits();
        }
        Self::i2c_wait_stop(i2c)?;
        Ok(n)
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

/// DW_apb_i2c の SCL カウンタを決める。`(hcnt, lcnt, spklen, sda_tx_hold)`。
///
/// 計算は `rp235x-hal` の `i2c/controller.rs`（さらに元は pico-sdk の
/// `hardware_i2c`）と同じ。**違うのは `assert!` ではなく `None` を返す点**
/// で、呼び出し側が `unsupported` に変える。パニックハンドラは理由を
/// 出せないので、範囲外はエラーで返さないと診断できない（TODO §1.4）。
///
/// 周期を 60% low / 40% high に割る。I2C はクロックを引き延ばされても
/// 構わないので、SPI の `spi_divisors` のような「要求値を超えない」丸めは
/// していない（`wit/i2c.wit` は `speed` の enum しか受け取らないので、
/// そもそも任意の周波数は来ない）。
///
/// `clk_peri` 150 MHz のときの 3 速度（いずれも誤差なしで出る）:
///
/// | speed | hcnt | lcnt | spklen | sda_hold | 実効 |
/// |---|---|---|---|---|---|
/// | `standard` | 600 | 900 | 56 | 46 | 100.00 kHz |
/// | `fast` | 150 | 225 | 14 | 46 | 400.00 kHz |
/// | `fast-plus` | 60 | 90 | 5 | 19 | 1000.00 kHz |
///
/// `None` を返すのは、1 MHz 超・0、`hcnt` / `lcnt` が 8..=0xffff の外
/// （`clk_peri` が 2 MHz 程度まで落ちると起きる）、`fast-plus` で
/// `clk_peri` が 32 MHz 未満、`sda_hold` が `lcnt - 2` を超える場合。
fn i2c_timing(clk_hz: u32, freq_hz: u32) -> Option<(u16, u16, u8, u16)> {
    if freq_hz == 0 || freq_hz > 1_000_000 {
        return None;
    }
    let clk = u64::from(clk_hz);
    let freq = u64::from(freq_hz);
    // 四捨五入してから 3:2 に割る。
    let period = (clk + freq / 2) / freq;
    let lcnt = period * 3 / 5;
    let hcnt = period - lcnt;
    if !(8..=0xffff).contains(&hcnt) || !(8..=0xffff).contains(&lcnt) {
        return None;
    }

    // I2C の仕様が要求する SDA のホールド時間（standard / fast は 300 ns、
    // fast-plus は 120 ns）をクロック数に直す。切り捨てを避けて +1 する。
    let sda_hold = if freq < 1_000_000 {
        (clk * 3) / 10_000_000 + 1
    } else {
        // fast-plus は clk_in > 32 MHz でないと必要なホールド時間が作れない。
        if clk < 32_000_000 {
            return None;
        }
        (clk * 3) / 25_000_000 + 1
    };
    // lcnt >= 8 は上で確かめてあるので引き算は溢れない。
    if sda_hold > lcnt - 2 {
        return None;
    }

    // スパイクフィルタの長さ。
    let spklen = if lcnt < 16 { 1 } else { lcnt / 16 };
    if spklen > 0xff {
        return None;
    }

    Some((hcnt as u16, lcnt as u16, spklen as u8, sda_hold as u16))
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

    // --- I2C。index 0 = I2C0 (SDA=GP4, SCL=GP5) ---

    fn i2c_open(&mut self, index: u32, speed: Speed) -> BoardResult<()> {
        // i2c1 もヘッダに出ているが、v0.1 で割り当てているのは i2c0 だけ
        // （abi-spec §8）。Hal 側で index < MAX_I2C は検査済み。
        // SPI と同じ判定にしてある（docs/TODO.md §2.1）。
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        let freq_hz = match speed {
            Speed::Standard => 100_000,
            Speed::Fast => 400_000,
            Speed::FastPlus => 1_000_000,
        };
        let Some((hcnt, lcnt, spklen, sda_hold)) = i2c_timing(self.peri_clock_hz, freq_hz) else {
            return Err(ErrorCode::Unsupported);
        };

        // SAFETY: Pico2Board::new の契約により、これらのペリフェラルは排他。
        let p = unsafe { pac::Peripherals::steal() };

        // 一度リセットを掛け直す。SPI0 と違い落としてから上げるのは、
        // open → close → open で前の設定が残らないようにするため。
        p.RESETS.reset().modify(|_, w| w.i2c0().set_bit());
        p.RESETS.reset().modify(|_, w| w.i2c0().clear_bit());
        while p.RESETS.reset_done().read().i2c0().bit_is_clear() {
            core::hint::spin_loop();
        }

        let i2c = p.I2C0;
        // 設定は ENABLE=0 のうちに行う。
        i2c.ic_enable().write(|w| w.enable().disabled());

        // マスターとして動かす。`ic_restart_en` は `write-read` の
        // Repeated START に要る。`rx_fifo_full_hld_ctrl` を立てると
        // 受信 FIFO が満杯のときにクロックを握ってくれるので取りこぼさない。
        i2c.ic_con().modify(|_, w| {
            w.speed().fast();
            w.master_mode().enabled();
            w.ic_slave_disable().slave_disabled();
            w.ic_restart_en().enabled();
            w.tx_empty_ctrl().enabled();
            w.rx_fifo_full_hld_ctrl().enabled();
            w
        });

        // SAFETY: i2c_timing が範囲内に収めた値。
        unsafe {
            i2c.ic_fs_scl_hcnt()
                .write(|w| w.ic_fs_scl_hcnt().bits(hcnt));
            i2c.ic_fs_scl_lcnt()
                .write(|w| w.ic_fs_scl_lcnt().bits(lcnt));
            i2c.ic_fs_spklen().write(|w| w.ic_fs_spklen().bits(spklen));
            i2c.ic_sda_hold()
                .modify(|_, w| w.ic_sda_tx_hold().bits(sda_hold));
            // 閾値は使わない（転送は IC_STATUS を見る同期ループ）。0 にしておく。
            i2c.ic_tx_tl().write(|w| w.tx_tl().bits(0));
            i2c.ic_rx_tl().write(|w| w.rx_tl().bits(0));
        }

        // SDA / SCL をパッドに出す。GPIO / SPI と同じく `write()` は
        // リセット値（ISO=1, PDE=1）から始まるので ISO を落とす。
        //
        // **内部プルアップを有効にする。** I2C は両線をプルアップで
        // high に保つバスで、RP2350 の内部プルは 50..80 kΩ と弱い。
        // `ports/esp32s3` では `esp-hal` の `connect_pin` が SDA/SCL に
        // 必ず `Pull::Up` を掛けるので（esp-hal 1.2.1 の
        // `i2c/master/low_level/mod.rs`）、**2 つのポートで同じバス条件に
        // なるよう揃えている**。pico-sdk の i2c の例も同じことをする。
        // ただしこれは弱い補助にすぎず、**外部 10 kΩ のプルアップが要る**
        // （短い配線の 100 kHz なら内部だけでも動くことがあるが、
        // `fast` / `fast-plus` では足りない）。
        for n in [I2C0_SDA, I2C0_SCL] {
            p.PADS_BANK0.gpio(n).write(|w| {
                w.ie().set_bit();
                w.od().clear_bit();
                w.pue().set_bit();
                w.pde().clear_bit();
                w.iso().clear_bit();
                w
            });
            p.IO_BANK0.gpio(n).gpio_ctrl().write(|w| w.funcsel().i2c());
        }

        i2c.ic_enable().write(|w| w.enable().enabled());
        Ok(())
    }

    fn i2c_write(&mut self, index: u32, address: u16, data: &[u8]) -> BoardResult<()> {
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        // 長さ 0 は Hal 側で invalid-argument にしている（abi-spec §8）。
        if data.is_empty() {
            return Err(ErrorCode::InvalidArgument);
        }
        let i2c = Self::i2c0();
        Self::i2c_set_target(&i2c, address)?;
        Self::i2c_send(&i2c, data, true)
    }

    fn i2c_read(&mut self, index: u32, address: u16, buf: &mut [u8]) -> BoardResult<usize> {
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        if buf.is_empty() {
            return Err(ErrorCode::InvalidArgument);
        }
        let i2c = Self::i2c0();
        Self::i2c_set_target(&i2c, address)?;
        Self::i2c_recv(&i2c, buf)
    }

    fn i2c_write_read(
        &mut self,
        index: u32,
        address: u16,
        data: &[u8],
        buf: &mut [u8],
    ) -> BoardResult<usize> {
        if index != 0 {
            return Err(ErrorCode::Unsupported);
        }
        if data.is_empty() || buf.is_empty() {
            return Err(ErrorCode::InvalidArgument);
        }
        let i2c = Self::i2c0();
        Self::i2c_set_target(&i2c, address)?;
        // 前半は STOP を出さない。次の read コマンドが Repeated START になる
        // （`ic_restart_en` を立ててある）。
        Self::i2c_send(&i2c, data, false)?;
        Self::i2c_recv(&i2c, buf)
    }

    fn i2c_close(&mut self, index: u32) {
        if index != 0 {
            return;
        }
        let i2c = Self::i2c0();
        i2c.ic_enable().write(|w| w.enable().disabled());
        // gpio_release / spi_close と同じく入力・プル無しに戻す
        // （FUNCSEL も SIO に戻るので、開いたままのバスが線を握らない）。
        for n in [I2C0_SDA, I2C0_SCL] {
            let _ = self.gpio_configure(n as u32, PinMode::Input);
        }
    }

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
        Self::micros()
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
