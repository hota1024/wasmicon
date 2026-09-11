//! M5Stack Tab5。
//!
//! 出典は Tab5 の PinMap (<https://docs.m5stack.com/en/core/Tab5>)。
//! 予約ピンは 3 ソース（公式 PinMap / ESPHome の実設定 / esp-bsp の
//! `m5stack_tab5.h`）が一致している。
//!
//! **Tab5 は GPIO の大半をオンボード周辺が使っている**（55 本中 31 本）。
//! 表は abi-spec §8、未確認の項目は docs/TODO.md §1.1。

use esp_hal::peripherals::Peripherals;
use esp_hal::usb::usb_serial_jtag::UsbSerialJtag;
use esp_hal::Blocking;

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
/// 向ける。部品も配線もなしに blink が目視できるようになるので、実機で最初に
/// 「動いているか」を見るときに使う。副作用として G22 を `reserved` から外す。
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

impl Serial for TraceOut {
    fn write(&mut self, bytes: &[u8]) {
        let _ = self.0.write(bytes);
        let _ = self.0.flush_tx();
    }
}

pub fn open_serial(p: Peripherals) -> TraceOut {
    let out = TraceOut(UsbSerialJtag::new(p.USB_DEVICE));

    // **ここで待たないと最初の 1 行が出ない。**
    // `esp_hal::init` が周辺を一度落とすため、USB-Serial-JTAG はアプリ起動時に
    // 再列挙される。ホストが列挙を終える前に書くと、`write` がドレインされずに
    // 止まったまま（`UsbSerialJtag::write` はホスト待ちでビジーウェイトする）
    // ホスト側の開き直しで DTR が動き、`rst:0x17 CHIP_USB_UART_RESET` で
    // リセットが繰り返される。
    let start = crate::chip::now_us();
    while crate::chip::now_us() - start < 2_000_000 {
        core::hint::spin_loop();
    }
    out
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
