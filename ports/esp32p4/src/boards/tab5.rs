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
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::uart::{Config as UartConfig, Uart};
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
/// 向ける。副作用として G22 を `reserved` から外す。
///
/// **G22 だけでは光らない。** Tab5 のバックライトは
/// **PI4IOE5V6408（内部 I2C, 0x43）のピン 4 = `BSP_LCD_EN` で LCD の電源を
/// 入れる**必要がある（esp-bsp の `bsp_feature_enable` `BSP_FEATURE_LCD` が
/// やっていること）。2026-09-11 に実機で確認済み（docs/TODO.md §1.1.5）。
/// 電源投入は下の `power_on_lcd` が `open` の中で行うので、この feature 単体で
/// 目視できる。内部 I2C (G31/G32) は `reserved` のままでゲストには渡さない。
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

/// トレースの出力先。**UART0 (G37=TX, G38=RX) 115200 8N1。**
///
/// Tab5 では UART0 は M5-Bus の 13/14 番ピンに出ているだけで USB には
/// 繋がっていない。取り込みには 3.3V の USB シリアル変換が要る。
///
/// **一度 USB-Serial-JTAG に変更したが戻した**（2026-09-11）。部品が要らない
/// 利点はあったが、この個体では出力を取れず、しかも失敗の原因を切り分ける
/// 手段が無くなった。USB-OTG も試したが列挙されなかった。経緯は
/// docs/TODO.md §1.1.5。UART なら RP2040 / ESP32-S3 と経路が揃う。
///
/// UART は受け手がいなくても送信が詰まらないので、`UsbSerialJtag` と違って
/// ホスト未接続で止まることはない。
pub struct TraceOut(Uart<'static, Blocking>);

impl Serial for TraceOut {
    fn write(&mut self, bytes: &[u8]) {
        let _ = self.0.write(bytes);
        let _ = self.0.flush();
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
    /// PORT.A (G54=SDA / G53=SCL)。外部ユニット用。ゲストの `i2c.bus` index 0。
    /// SDA/SCL の極性は未確認（docs/TODO.md §1.1）。
    pub i2c_porta: I2c<'static, Blocking>,
    /// M5-Bus の SPI2 (SCK=G5 / MOSI=G18 / MISO=G19)。ゲストの `spi.bus` index 0。
    /// abi-spec §8 の表のとおり。CS / DC / RST は役割名で引く GPIO
    /// （`lcd-cs` = G48 / `lcd-dc` = G47 / `lcd-rst` = G45）で、
    /// **ゲストが自分で叩く**。ここでは握らない。
    ///
    /// **バスのピンは `reserved` に入れていない。** PORT.A の I2C と同じ扱いで、
    /// 外部向けのピンは開けたままにしてある。ゲストが同じ番号を素の GPIO として
    /// 開くと競合するが、それは I2C でも同じ（docs/TODO.md §1.2）。
    pub spi_bus: Spi<'static, Blocking>,
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

pub fn open(p: Peripherals) -> Hw {
    // 以前ここでホストの接続待ちに 2 秒入れていたが外した。UART は受け手が
    // いなくても送信が詰まらないので待つ理由が無い。**タイマーに依存する待ちを
    // 起動経路の先頭に置くと、タイマーが動いていなかった場合にそこで全てが
    // 止まる**（切り分けの邪魔になる）。
    let mut serial = TraceOut(
        Uart::new(p.UART0, UartConfig::default())
            .expect("UART0 の初期化に失敗")
            .with_tx(p.GPIO37)
            .with_rx(p.GPIO38),
    );

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
    serial.write(if ok {
        b"lcd: power on\r\n"
    } else {
        b"lcd: power on failed\r\n"
    });

    // M5-Bus の SPI2。周波数とモードは `spi.bus.open` で上書きされるので、
    // ここは既定のまま作るだけでよい（`apply_config` で差し替える）。
    let spi_bus = Spi::new(p.SPI2, SpiConfig::default())
        .expect("SPI2 の初期化に失敗")
        .with_sck(p.GPIO5)
        .with_mosi(p.GPIO18)
        .with_miso(p.GPIO19);

    // 内部バスは `Hw` に入れない。今はここで LCD の電源を入れるだけで、
    // ゲストにも渡さないため。display を実装するときに持ち回る形へ変える。
    Hw {
        serial,
        i2c_porta,
        spi_bus,
    }
}

// パニック経路の出力はここに持たない。**`chip::early_write` を使う。**
// 以前は UART を奪い直して組み立てていたが、`esp_hal::init` の中で落ちると
// panic handler 自身がクロック未設定のまま UART 生成へ入って止まり、panic が
// 一切観測できなかった（2026-09-21、Tab5）。詳細は docs/TODO.md §1.1.5。
