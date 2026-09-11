//! M5Stack Tab5。
//!
//! 出典は Tab5 の PinMap (<https://docs.m5stack.com/en/core/Tab5>)。
//! 予約ピンは 3 ソース（公式 PinMap / ESPHome の実設定 / esp-bsp の
//! `m5stack_tab5.h`）が一致している。
//!
//! **Tab5 は GPIO の大半をオンボード周辺が使っている**（55 本中 31 本）。
//! 表は abi-spec §8、未確認の項目は docs/TODO.md §1.1。

use esp_hal::peripherals::Peripherals;
use esp_hal::uart::{Config as UartConfig, Uart};
use esp_hal::Blocking;

use super::BoardDef;

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
const RESERVED: &[(u32, u32)] = &[
    (8, 15),
    (17, 17),
    (20, 23),
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
const ROLES: &[(&str, u32)] = &[
    // M5-Bus pin 2
    ("led", 16),
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

/// UART0 (G37=TX, G38=RX) 115200 8N1。
///
/// **Tab5 では UART0 は M5-Bus の 13/14 番ピンに出ているだけ**で USB には
/// 繋がっていないので、取り込みには USB シリアル変換が要る
/// （USB-Serial-JTAG に移すかは未決。docs/TODO.md §1.1）。
pub fn open_serial(p: Peripherals) -> Uart<'static, Blocking> {
    Uart::new(p.UART0, UartConfig::default())
        .unwrap()
        .with_tx(p.GPIO37)
        .with_rx(p.GPIO38)
}
