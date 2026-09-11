//! M5Stack Tab5。
//!
//! 出典は Tab5 の PinMap (<https://docs.m5stack.com/en/core/Tab5>)。
//! 予約ピンは 3 ソース（公式 PinMap / ESPHome の実設定 / esp-bsp の
//! `m5stack_tab5.h`）が一致している。
//!
//! **Tab5 は GPIO の大半をオンボード周辺が使っている**（55 本中 31 本）。
//! 表は abi-spec §8、未確認の項目は docs/TODO.md §1.1。

use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::peripherals::Peripherals;
use esp_hal::usb::usb_serial_jtag::UsbSerialJtag;
use esp_hal::Blocking;

use wasmicon_core::generated::gpio::{Level, PinMode};

use super::{BoardDef, Serial};

/// ゲストに開放しない GPIO。**チップではなくボードの都合**で塞いでいる。
///
/// - `8..=15`: ESP32-C6 (Wi-Fi)。SDIO2 の D3..D0 / CMD / CK と IO2 / RESET。
///   C6 が双方向に駆動するので競合する
/// - `17`, `52`: M5-Bus の PB_IN / PB_OUT。名前から電源ボタン系と推定している
///   （**機能は未確認**）。落とすと実行ごと消えるので塞ぐ
/// - `20`, `21`, `34`: RS485 (SIT3088) の TX / RX / DIR。RX は SIT3088 が駆動する
/// - `22`: LCD のバックライト (LEDA)。落とすと画面が消える
/// - `23`: TP_INT。タッチコントローラが駆動する
/// - `26..=30`: 音声の I2S (ES8388 / ES7210)。コーデックが ASDOUT を駆動する
/// - `31`, `32`: **内部 I2C**。タッチ・コーデック・IMU・RTC・INA226 に加えて
///   PI4IOE5V6408 が 2 つぶら下がっており、この 2 つが LCD_RST / TP_RST /
///   CAM_RST と電源制御を握っている。素の GPIO として振ると表示も電源も飛ぶ
/// - `36`: CAM_MCLK
/// - `37`, `38`: トレース用の UART0
/// - `39..=44`: microSD。カードが DAT を駆動する
///
/// 開いている番号のうち 33 と 35 は P4 の strapping ピン（32..=38）。
#[cfg(not(feature = "led-backlight"))]
const RESERVED: &[(u32, u32)] = &[
    (8, 15),
    (17, 17),
    (20, 23),
    (26, 32),
    (34, 34),
    (36, 44),
    (52, 52),
];

/// `led-backlight` のときは 22 だけ開ける（下の `LED` を参照）。他は同じ。
#[cfg(feature = "led-backlight")]
const RESERVED: &[(u32, u32)] = &[
    (8, 15),
    (17, 17),
    (20, 21),
    (23, 23),
    (26, 32),
    (34, 34),
    (36, 44),
    (52, 52),
];

/// 役割名 → GPIO 番号（abi-spec §8 の表）。いずれも M5-Bus に出ている
/// 汎用 GPIO から取った（strapping ピンと PB_IN / PB_OUT は避けてある）。
///
/// `led` が外付けなのは、**Tab5 にユーザーが振れるオンボード LED が無い**ため
/// （esp-bsp の `m5stack_tab5.h` にも「Buttons and LEDs are not present」とある）。
/// **実機の配線は未確認**（docs/TODO.md §1.1）。
/// `led` 役割に充てる GPIO。既定は M5-Bus pin 2 に出ている汎用の G16（外付け）。
///
/// `led-backlight` feature を有効にすると **G22（LCD のバックライト LEDA）**に
/// 向ける。副作用として G22 を `reserved` から外す。
///
/// **現状このビルドでは何も光らない。** Tab5 のバックライトは G22 だけでは
/// 点かず、**PI4IOE5V6408（内部 I2C, 0x43）のピン 4 = `BSP_LCD_EN` で
/// LCD の電源を入れる**必要がある（esp-bsp の `bsp_feature_enable`
/// `BSP_FEATURE_LCD` がやっていること）。内部 I2C (G31/G32) は `reserved` で、
/// I2C 自体も未実装なので、**エキスパンダを叩けるようになるまでこの feature は
/// 目視確認には使えない**。2026-09-11 に実機で確認済み（docs/TODO.md §1.1.5）。
///
/// **トレースは変わらない。** `pin-by-role` で引いた番号は `wasmicon-port` の
/// `write_pin` が `role:led` に正規化するため（abi-spec §9）、どちらのビルドでも
/// `diff-traces.sh` の比較結果は同一になる。
///
/// 画像は出ない。パネルを初期化していないので点灯・消灯が見えるだけ。
/// LEDA の極性は未確認なので、反転して見えるかもしれない（点滅自体は見える）。
#[cfg(not(feature = "led-backlight"))]
const LED: u32 = 16;
#[cfg(feature = "led-backlight")]
const LED: u32 = 22;

const ROLES: &[(&str, u32)] = &[
    // 既定は M5-Bus pin 2、led-backlight のときは LCD のバックライト
    ("led", LED),
    // M5-Bus pin 22 / 23 / 8
    ("lcd-cs", 48),
    ("lcd-dc", 47),
    ("lcd-rst", 45),
];

pub const DEF: BoardDef = BoardDef {
    name: "esp32p4/tab5",
    roles: ROLES,
    reserved: RESERVED,
};

/// トレースの出力先。**USB-Serial-JTAG を使う**（USB-C 1 本で取れる）。
///
/// UART0 (G37/G38) にも出せるが、Tab5 ではそれが M5-Bus の 13/14 番ピンに
/// 出ているだけで USB には繋がっておらず、取り込みに USB シリアル変換と
/// 30 ピンコネクタへの配線が要る。USB-Serial-JTAG なら追加の部品が要らない。
///
/// **ホストが読んでいないと `write` はブロックする**（esp-hal の実装が
/// エンドポイントの空きをビジーウェイトする）。`espflash --monitor` なり
///端末なりを繋いでいないと、最初のバナー出力で止まったように見える。
pub struct TraceOut(UsbSerialJtag<'static, Blocking>);

/// ホストが読まないときに諦めるまでの時間。
///
/// **esp-hal の `UsbSerialJtag::write` はホストがドレインするまで無限に
/// ビジーウェイトする。** USB-Serial-JTAG はホストが CDC を開いていないと
/// エンドポイントが掃けないので、モニタを繋いでいない実機ではそこで止まる。
/// 2026-09-11 に Tab5 でこれを踏み、起動直後のバナー出力で停止していた
/// （保存 PC が `UsbSerialJtag` の待ちループを指していた）。
///
/// **測定器であるトレースを黙って捨てるのは避けたいが、装置ごと止まるのは
/// もっと悪い。** ここで諦めた場合は取りこぼしとして扱う（docs/TODO.md §1.1.5）。
const WRITE_TIMEOUT_US: u64 = 50_000;

impl Serial for TraceOut {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            let start = crate::chip::now_us();
            loop {
                if self.0.write_byte_nb(b).is_ok() {
                    break;
                }
                if crate::chip::now_us() - start > WRITE_TIMEOUT_US {
                    // ホストがいない。以降も掃けないので、この書き込みは捨てる。
                    return;
                }
                core::hint::spin_loop();
            }
        }
        let start = crate::chip::now_us();
        while self.0.flush_tx_nb().is_err() {
            if crate::chip::now_us() - start > WRITE_TIMEOUT_US {
                return;
            }
            core::hint::spin_loop();
        }
    }
}

/// ボードが握るハードウェア一式。
///
/// **内部 I2C (G31/G32) はここに出さない。** タッチ・コーデック・IMU・RTC・
/// INA226 に加えて PI4IOE5V6408 が 2 つ載っていて、そのうち 0x43 が LCD と
/// 電源を握っている。`open` の中で LCD の電源を入れるのにだけ使い、
/// ゲストにも `Board` にも渡さない（docs/TODO.md §1.1.5）。
pub struct Hw {
    pub serial: TraceOut,
    /// `power_on_lcd` が成功したか。実機で切り分けるために持ち回る
    /// （シリアルが後から繋がれても分かるよう、`main` が定期的に出す）。
    pub lcd_ok: bool,
    /// PORT.A (G54=SDA / G53=SCL)。外部ユニット用。ゲストの `i2c.bus` index 0。
    /// SDA/SCL の極性は未確認（docs/TODO.md §1.1）。
    pub i2c_porta: I2c<'static, Blocking>,
}

/// PI4IOE5V6408 のレジスタ（esp-bsp の
/// `esp_io_expander_pi4ioe5v6408.c` より）。
const PI4IO_REG_CHIP_RESET: u8 = 0x01;
const PI4IO_REG_IO_DIR: u8 = 0x03;
const PI4IO_REG_OUT_SET: u8 = 0x05;
const PI4IO_REG_OUT_H_IM: u8 = 0x07;
const PI4IO_REG_PULL_EN: u8 = 0x0B;
const PI4IO_REG_PULL_SEL: u8 = 0x0D;

/// LCD の電源を握るエキスパンダのアドレス。
/// `ESP_IO_EXPANDER_I2C_PI4IOE5V6408_ADDRESS_LOW`（ADDR ピン Low）。
const EXPANDER_LCD: u8 = 0x43;

/// そのエキスパンダ上で LCD_EN が繋がっているビット。
/// `IO_EXPANDER_PIN_NUM_4 = (1ULL << 4)` なのでマスクは 0x10。
const LCD_EN_BIT: u8 = 1 << 4;

/// バックライト (LEDA)。LCD_EN を立てた後にこれを駆動すると画面が光る。
const BACKLIGHT: u32 = 22;

type I2cResult = Result<(), esp_hal::i2c::master::Error>;

/// エキスパンダを既定状態へ戻す。ドライバの `reset()` と同じ順序・同じ値。
///
/// 既定値の意味（`dir_out_bit_zero = 0` より 1=出力、`OUT_H_IM` は 1=hi-Z）:
/// DIR=0xFF で全ビット出力、HIGHZ=0xFF で全て hi-Z なので、この時点では
/// 何も駆動していない。目的のビットだけ hi-Z を外して駆動する。
fn reset_expander(i2c: &mut I2c<'static, Blocking>) -> I2cResult {
    i2c.write(EXPANDER_LCD, &[PI4IO_REG_CHIP_RESET, 0xFF])?;
    let mut st = [0u8; 1];
    i2c.write_read(EXPANDER_LCD, &[PI4IO_REG_CHIP_RESET], &mut st)?;
    i2c.write(EXPANDER_LCD, &[PI4IO_REG_IO_DIR, 0xFF])?;
    i2c.write(EXPANDER_LCD, &[PI4IO_REG_OUT_H_IM, 0xFF])?;
    i2c.write(EXPANDER_LCD, &[PI4IO_REG_PULL_SEL, 0x00])?;
    i2c.write(EXPANDER_LCD, &[PI4IO_REG_PULL_EN, 0xFF])?;
    i2c.write(EXPANDER_LCD, &[PI4IO_REG_OUT_SET, 0x00])?;
    Ok(())
}

/// LCD の電源を入れる。
///
/// **G22（バックライト LEDA）だけでは画面は光らない。** 先にこのエキスパンダで
/// LCD_EN を立てる必要がある（esp-bsp の `bsp_feature_enable(BSP_FEATURE_LCD)`
/// と同じこと）。2026-09-11 に実機で「G22 だけでは光らない」ことを確認済み。
///
/// リセット直後なので読み戻さずに絶対値を書く。読みに依存しないぶん、
/// I2C の片方向（書き込み）だけでも成立する。
fn power_on_lcd(i2c: &mut I2c<'static, Blocking>) -> I2cResult {
    reset_expander(i2c)?;
    // hi-Z を外して駆動する（該当ビットを 0 に）。
    i2c.write(EXPANDER_LCD, &[PI4IO_REG_OUT_H_IM, !LCD_EN_BIT])?;
    // High にする。
    i2c.write(EXPANDER_LCD, &[PI4IO_REG_OUT_SET, LCD_EN_BIT])?;
    Ok(())
}

/// バックライトを明滅させる。**実機の診断用**。
///
/// USB-Serial-JTAG が読めない状態（docs/TODO.md §1.1.5）では、画面が唯一の
/// 可視な出力になる。成功と失敗で回数と間隔を変えることで、シリアル無しでも
/// 次の 3 つを 1 回の観測で区別できる:
///
/// - ゆっくり数回 → I2C 成功
/// - 速く多数回   → I2C 失敗（ただし G22 は見えている）
/// - 全く変化なし → G22 自体が見えない（LCD_EN が効いていないか配線）
///
/// LEDA の極性が未確認なので、点灯/消灯どちらが「オン」でも変化が見えるように
/// トグルしている。
fn blink_backlight(times: u32, period_us: u64) {
    // SAFETY: `EspBoard` はまだ作られていない。ここで作る `Gpio` は
    // この関数を出る前に捨てるので、排他の契約は保たれる。
    let mut gpio = unsafe { crate::chip::Gpio::steal() };
    if gpio.configure(BACKLIGHT, PinMode::Output).is_err() {
        return;
    }
    for i in 0..times {
        let level = if i % 2 == 0 { Level::High } else { Level::Low };
        let _ = gpio.write(BACKLIGHT, level);
        let start = crate::chip::now_us();
        while crate::chip::now_us() - start < period_us {
            core::hint::spin_loop();
        }
    }
    let _ = gpio.write(BACKLIGHT, Level::High);
}

pub fn open(p: Peripherals) -> Hw {
    let out = TraceOut(UsbSerialJtag::new(p.USB_DEVICE));

    // 以前ここで 2 秒待っていたが外した。`Serial::write` に上限を付けたので
    // ホスト待ちで止まることは無くなり、待つ理由が無い。
    // **タイマーに依存する待ちを起動経路の先頭に置くと、タイマーが動いて
    // いなかった場合にそこで全てが止まる**（切り分けの邪魔になる）。
    let mut serial = out;

    let cfg = I2cConfig::default();
    let mut i2c_internal = I2c::new(p.I2C0, cfg)
        .expect("I2C0 の初期化に失敗")
        .with_sda(p.GPIO31)
        .with_scl(p.GPIO32);
    let i2c_porta = I2c::new(p.I2C1, cfg)
        .expect("I2C1 の初期化に失敗")
        .with_sda(p.GPIO54)
        .with_scl(p.GPIO53);

    // 結果をここで出す。失敗しても続行する（画面が点かないだけで、
    // ランタイムの検証はトレースで行える）。
    // **シリアルより先に画面へ出す。** シリアルはホスト次第で詰まるので、
    // 可視な診断をその後ろに置くと、詰まったときに何も見えなくなる。
    let ok = power_on_lcd(&mut i2c_internal).is_ok();
    if ok {
        blink_backlight(6, 300_000); // ゆっくり 6 回 = I2C 成功
    } else {
        blink_backlight(20, 80_000); // 速く 20 回 = I2C 失敗
    }
    if ok {
        serial.write(b"lcd: power on\r\n");
    } else {
        serial.write(b"lcd: power on failed\r\n");
    }

    // 内部バスは `Hw` に入れない。今はここで LCD の電源を入れるだけで、
    // ゲストにも渡さないため。display を実装するときに持ち回る形へ変える。
    Hw {
        serial,
        lcd_ok: ok,
        i2c_porta,
    }
}

/// パニック経路から出力先を作り直す。
///
/// 通常の `TraceOut` はボードが持っていて panic handler からは届かないので、
/// USB_DEVICE を奪い直す。`esp_hal::init` の中で落ちた場合はまだ誰も
/// USB_DEVICE を取っていないし、後から落ちた場合も以降は停止するだけなので、
/// 二重に触っても競合しない。
///
/// # Safety
/// panic handler からのみ呼ぶこと。
pub unsafe fn steal_serial() -> TraceOut {
    // SAFETY: 上の契約により、これ以降 USB_DEVICE を使うのはこの一つだけ。
    let usb = unsafe { esp_hal::peripherals::USB_DEVICE::steal() };
    TraceOut(UsbSerialJtag::new(usb))
}
