//! SHT4x（SHT40）を読んで ILI9341 に表示する。
//!
//! 描画の詳細は `apps/README.md` が正。`sensor-display-as` と同じ
//! host call 列・同じピクセル出力を出さなければならない。
//!
//! ピン番号はハードコードせず `board.pin-by-role` で引くので、同一の `.wasm` が
//! ESP32-S3 と RP2040 の両方で動く（abi-spec §8）。

#![no_std]

mod font;
mod ili9341;
mod sht4x;

use wasmicon_hal::gpio::{Pin, PinMode};
use wasmicon_hal::i2c::{Bus as I2cBus, Speed};
use wasmicon_hal::spi::{Bus as SpiBus, Mode};
use wasmicon_hal::{ErrorCode, board, log};

use ili9341::Display;

/// 背景。
const BG: u16 = 0x0841;
/// 見出しの帯。
const HEADER: u16 = 0x1A3F;
/// カードの地。
const CARD: u16 = 0x2104;
/// 数値と見出しの文字。
const FG: u16 = 0xFFFF;
/// ラベルと目盛り。
const MUTED: u16 = 0xA514;
/// 温度カードの差し色。
const TEMP_ACCENT: u16 = 0xFC00;
/// 湿度カードの差し色。
const HUM_ACCENT: u16 = 0x07FF;
/// ゲージの溝。
const TRACK: u16 = 0x4208;

/// カードの位置と大きさ（`apps/README.md` §2 の画面レイアウト）。
const TEMP_X: u16 = 4;
const HUM_X: u16 = 162;
const CARD_Y: u16 = 36;
const CARD_W: u16 = 154;
const CARD_H: u16 = 116;
/// 数値の倍率と y。
const VALUE_SCALE: u16 = 3;
const VALUE_Y: u16 = 84;

/// ゲージ。`bar_px` の 0..=308 がそのまま幅になる。
const GAUGE_X: u16 = 6;
const GAUGE_Y: u16 = 178;
const GAUGE_H: u16 = 16;
/// ゲージの色の区間（`bar_px` 上の終端と色）。-45..10 / ..30 / ..50 / ..175 °C。
const ZONES: [(u16, u16); 4] = [(77, 0x041F), (105, 0x07E0), (133, 0xFFE0), (308, 0xF800)];
/// 目盛り（`bar_px` 上の位置）。0 / 25 / 50 / 100 °C。
const TICKS: [u16; 4] = [63, 98, 133, 203];

/// SPI のクロック。
const SPI_HZ: u32 = 24_000_000;

/// エントリポイント。ホストがインスタンス化直後に 1 回呼ぶ（abi-spec §3.3）。
///
/// ハンドルは全て 1 つのスコープに置き、宣言順を `cs` → `dc` → `rst` → `spi` →
/// `i2c` にしてある。`Drop` は宣言の逆順に走るので、どの経路で抜けても
/// `apps/README.md` §4 が定める解放順（i2c → spi → rst → dc → cs）になる。
#[unsafe(no_mangle)]
pub extern "C" fn run() {
    log::info("sensor-display start");

    let (Ok(cs_i), Ok(dc_i), Ok(rst_i)) = (
        board::pin_by_role("lcd-cs"),
        board::pin_by_role("lcd-dc"),
        board::pin_by_role("lcd-rst"),
    ) else {
        log::error("role not found");
        return;
    };
    // 1 本ずつ開ける。まとめて開けると失敗しても全部呼んでしまい、
    // AssemblyScript 版と host call 列が食い違う。
    let Ok(cs) = Pin::open(cs_i, PinMode::Output) else {
        log::error("gpio open failed");
        return;
    };
    let Ok(dc) = Pin::open(dc_i, PinMode::Output) else {
        log::error("gpio open failed");
        return;
    };
    let Ok(rst) = Pin::open(rst_i, PinMode::Output) else {
        log::error("gpio open failed");
        return;
    };
    let Ok(spi) = SpiBus::open(0, SPI_HZ, Mode::Mode0) else {
        log::error("spi open failed");
        return;
    };
    let Ok(i2c) = I2cBus::open(0, Speed::Standard) else {
        log::error("i2c open failed");
        return;
    };

    let display = Display::new(&spi, &cs, &dc);
    if display.init(&rst).is_err() || draw_frame(&display).is_err() {
        log::error("display failed");
        return;
    }

    let reading = match sht4x::read(&i2c) {
        Ok(r) => r,
        Err(sht4x::Error::Crc) => {
            log::error("sensor crc failed");
            return;
        }
        Err(sht4x::Error::Io) => {
            log::error("sensor read failed");
            return;
        }
    };

    let temp = sht4x::temp_centi(reading.raw_t);
    let humidity = sht4x::humidity_centi(reading.raw_h);
    if draw_values(&display, temp, humidity).is_err() {
        log::error("display failed");
    }
}

/// センサーを読む前に描ける部分。見出し・カード・ゲージの溝と目盛り。
fn draw_frame(d: &Display) -> Result<(), ErrorCode> {
    d.fill_rect(0, 0, ili9341::WIDTH, ili9341::HEIGHT, BG)?;

    d.fill_rect(0, 0, ili9341::WIDTH, 28, HEADER)?;
    d.draw_text(8, 6, b"WASMICON", 2, FG, HEADER)?;
    d.draw_text(272, 10, b"SHT40", 1, FG, HEADER)?;

    draw_card(d, TEMP_X, b"TEMP", b"C", TEMP_ACCENT)?;
    draw_card(d, HUM_X, b"HUMIDITY", b"%", HUM_ACCENT)?;

    d.fill_rect(4, 162, 312, 72, CARD)?;
    d.fill_rect(GAUGE_X, GAUGE_Y, 308, GAUGE_H, TRACK)?;
    for t in TICKS {
        d.fill_rect(GAUGE_X + t, 196, 1, 5, MUTED)?;
    }
    d.draw_text(6, 206, b"-45", 1, MUTED, CARD)?;
    d.draw_text(65, 206, b"0", 1, MUTED, CARD)?;
    d.draw_text(131, 206, b"50", 1, MUTED, CARD)?;
    d.draw_text(197, 206, b"100", 1, MUTED, CARD)?;
    d.draw_text(282, 206, b"175C", 1, MUTED, CARD)
}

/// カード 1 枚。地・上端の差し色・ラベル・単位。
fn draw_card(d: &Display, x: u16, label: &[u8], unit: &[u8], accent: u16) -> Result<(), ErrorCode> {
    d.fill_rect(x, CARD_Y, CARD_W, CARD_H, CARD)?;
    d.fill_rect(x, CARD_Y, CARD_W, 4, accent)?;
    d.draw_text(x + 8, 48, label, 1, MUTED, CARD)?;
    d.draw_text(x + CARD_W - 24, 46, unit, 2, accent, CARD)
}

/// 読んだ値。カードの数値とゲージの塗り。
fn draw_values(d: &Display, temp: i32, humidity: i32) -> Result<(), ErrorCode> {
    let mut buf = [0u8; ili9341::MAX_TEXT];
    let n = format_value(temp, &mut buf);
    draw_value(d, TEMP_X, &buf[..n])?;
    let n = format_value(humidity, &mut buf);
    draw_value(d, HUM_X, &buf[..n])?;

    // 区間ごとに、塗る範囲 [0, bar) と重なる分だけ塗る。
    let bar = bar_px(temp) as u16;
    let mut start = 0;
    for (end, color) in ZONES {
        if bar > start {
            let stop = if bar < end { bar } else { end };
            d.fill_rect(GAUGE_X + start, GAUGE_Y, stop - start, GAUGE_H, color)?;
        }
        start = end;
    }
    Ok(())
}

/// 数値をカードの横中央に描く。
fn draw_value(d: &Display, card_x: u16, text: &[u8]) -> Result<(), ErrorCode> {
    let w = text.len() as u16 * 8 * VALUE_SCALE;
    let x = card_x + (CARD_W - w) / 2;
    d.draw_text(x, VALUE_Y, text, VALUE_SCALE, FG, CARD)
}

/// 温度バーの長さ。**このアプリで唯一 f32 を使う場所**（`apps/README.md` §1）。
///
/// クランプは f32 のまま行う。範囲外のまま `i32` に落とすと、Rust の飽和変換と
/// AssemblyScript のトラップ変換で挙動が分かれる。
///
/// clippy は `f32::clamp` を勧めるが、AssemblyScript 版と**同じ命令列**にしたい。
/// `clamp` は `f32.min` / `f32.max` に落ちることがあり、AS 側の比較 + 分岐とは
/// 別の命令になる。決定性検証の題材なので構造を揃えておく。
#[allow(clippy::manual_clamp)]
fn bar_px(temp_centi: i32) -> i32 {
    let t = temp_centi as f32 / 100.0;
    let mut b = (t + 45.0) * 1.4;
    if b < 0.0 {
        b = 0.0;
    }
    if b > 308.0 {
        b = 308.0;
    }
    b as i32
}

/// `23.44` / `-5.07` の形に整形する。書けた長さを返す。
fn format_value(centi: i32, out: &mut [u8]) -> usize {
    let mut n = 0;
    let put = |b: u8, out: &mut [u8], n: &mut usize| {
        if *n < out.len() {
            out[*n] = b;
            *n += 1;
        }
    };

    let neg = centi < 0;
    let v = if neg { -centi } else { centi };
    if neg {
        put(b'-', out, &mut n);
    }

    let int = v / 100;
    let frac = v % 100;
    // 整数部。0 埋めはしない。
    let mut digits = [0u8; 10];
    let mut d = 0;
    let mut x = int;
    loop {
        digits[d] = b'0' + (x % 10) as u8;
        d += 1;
        x /= 10;
        if x == 0 {
            break;
        }
    }
    while d > 0 {
        d -= 1;
        put(digits[d], out, &mut n);
    }

    put(b'.', out, &mut n);
    put(b'0' + (frac / 10) as u8, out, &mut n);
    put(b'0' + (frac % 10) as u8, out, &mut n);
    n
}

/// `no_std` なので自前で用意する（docs/handoff.md §3 #4）。
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
