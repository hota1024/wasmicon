//! SHT31 を読んで ILI9341 に表示する。
//!
//! 描画の詳細は `apps/README.md` が正。`sensor-display-as` と同じ
//! host call 列・同じピクセル出力を出さなければならない。
//!
//! ピン番号はハードコードせず `board.pin-by-role` で引くので、同一の `.wasm` が
//! ESP32-S3 と RP2040 の両方で動く（abi-spec §8）。

#![no_std]

mod font;
mod ili9341;
mod sht31;

use wasmicon_hal::gpio::{Pin, PinMode};
use wasmicon_hal::i2c::{Bus as I2cBus, Speed};
use wasmicon_hal::spi::{Bus as SpiBus, Mode};
use wasmicon_hal::{board, log};

use ili9341::Display;

/// 背景色（黒）。
const BG: u16 = 0x0000;
/// 文字色（白）。
const FG: u16 = 0xFFFF;
/// バーの色（赤）。
const BAR: u16 = 0xF800;

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
    if display.init(&rst).is_err()
        || display
            .fill_rect(0, 0, ili9341::WIDTH, ili9341::HEIGHT, BG)
            .is_err()
    {
        log::error("display failed");
        return;
    }

    let reading = match sht31::read(&i2c) {
        Ok(r) => r,
        Err(sht31::Error::Crc) => {
            log::error("sensor crc failed");
            return;
        }
        Err(sht31::Error::Io) => {
            log::error("sensor read failed");
            return;
        }
    };

    let temp = sht31::temp_centi(reading.raw_t);
    let humidity = sht31::humidity_centi(reading.raw_h);

    let mut buf = [0u8; ili9341::MAX_TEXT];
    let n = format_row(b'T', temp, b'C', &mut buf);
    if display.draw_text(8, 40, &buf[..n], FG, BG).is_err() {
        log::error("display failed");
        return;
    }
    let n = format_row(b'H', humidity, b'%', &mut buf);
    if display.draw_text(8, 60, &buf[..n], FG, BG).is_err() {
        log::error("display failed");
        return;
    }

    let bar = bar_px(temp);
    if bar > 0 && display.fill_rect(8, 80, bar as u16, 8, BAR).is_err() {
        log::error("display failed");
    }
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

/// `T 23.44C` の形に整形する。書けた長さを返す。
fn format_row(label: u8, centi: i32, unit: u8, out: &mut [u8]) -> usize {
    let mut n = 0;
    let put = |b: u8, out: &mut [u8], n: &mut usize| {
        if *n < out.len() {
            out[*n] = b;
            *n += 1;
        }
    };
    put(label, out, &mut n);
    put(b' ', out, &mut n);

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
    put(unit, out, &mut n);
    n
}

/// `no_std` なので自前で用意する（HANDOFF §3 #4）。
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
