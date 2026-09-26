//! ILI9341 に絵を出すだけのデモ。センサーは要らない。
//!
//! `sensor-display-rs` は SHT40 を I2C で読むので、センサーを繋がずに
//! ディスプレイだけ動かすことができない（I2C が開けない時点で戻る）。
//! こちらは SPI と GPIO しか使わないので、LCD を 1 枚繋ぐだけで動く。
//!
//! 出すものはそれぞれ別の失敗の仕方を見るために選んである:
//!
//! - **カラーバー** — 色順（MADCTL の BGR ビット）。赤と青が入れ替わって
//!   見えたらそこ
//! - **外周 1 px の枠** — アドレス指定の端。`fill_rect` が画面ぴったり
//!   （x+w == 320, y+h == 240）を扱えているか
//! - **グレーのランプ** — RGB565 の詰め方。段が色付いて見えたらビット位置が
//!   ずれている
//! - **文字** — `draw_text` の経路
//! - **動く四角** — `time.sleep-ms` と繰り返しの描画
//!
//! ピン番号はハードコードせず `board.pin-by-role` で引くので、`lcd-cs` /
//! `lcd-dc` / `lcd-rst` を持つボードならどれでも同じ `.wasm` が動く
//! （abi-spec §8）。ただし**画面に出す文字だけは Pico 2 (RP2350) 向けに
//! 書いてある**ので、他のボードで使うなら `draw` の文字列を直す。
//! 配線は `apps/lcd-demo-rs/README.md`。

#![no_std]

mod font;
mod ili9341;

use wasmicon_hal::gpio::{Pin, PinMode};
use wasmicon_hal::spi::{Bus as SpiBus, Mode};
use wasmicon_hal::{board, log, time};

use ili9341::{Display, HEIGHT, WIDTH};

/// 背景。
const BLACK: u16 = 0x0000;
/// 文字と枠。
const WHITE: u16 = 0xFFFF;
/// 見出しの帯。
const BAND: u16 = 0x001F;

/// カラーバー。左から赤・橙・黄・緑・シアン・青・マゼンタ・白。
/// **赤と青が逆に出たら** MADCTL (`0x36`) の BGR ビットを疑う。
const BARS: [u16; 8] = [
    0xF800, 0xFD20, 0xFFE0, 0x07E0, 0x07FF, 0x001F, 0xF81F, 0xFFFF,
];

/// SPI のクロック。
///
/// `apps/README.md` §2 の 24 MHz より落としてある。こちらは 2 実装の一致を
/// 見るアプリではないので、ブリングアップで転びにくいほうを取る。
/// RP2350 (clk_peri 150 MHz) では 15 MHz に丸められる。
const SPI_HZ: u32 = 16_000_000;

/// 動く四角の 1 往復ぶんのコマ数。
const FRAMES: u16 = 16;

/// エントリポイント。ホストがインスタンス化直後に 1 回呼ぶ（abi-spec §3.3）。
///
/// ハンドルは `cs` → `dc` → `rst` → `spi` の順に宣言する。`Drop` は宣言の
/// 逆順に走るので、どの経路で抜けても `spi` → `rst` → `dc` → `cs` の順で
/// 解放される（`apps/README.md` §4 と同じ並び）。
#[unsafe(no_mangle)]
pub extern "C" fn run() {
    log::info("lcd-demo start");

    let (Ok(cs_i), Ok(dc_i), Ok(rst_i)) = (
        board::pin_by_role("lcd-cs"),
        board::pin_by_role("lcd-dc"),
        board::pin_by_role("lcd-rst"),
    ) else {
        log::error("role not found");
        return;
    };
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

    let display = Display::new(&spi, &cs, &dc);
    if display.init(&rst).is_err() || draw(&display).is_err() {
        log::error("display failed");
        return;
    }
    log::info("lcd-demo done");
}

/// 画面を組み立てる。失敗したら途中で止めて呼び出し元に返す。
fn draw(d: &Display<'_>) -> Result<(), wasmicon_hal::ErrorCode> {
    d.fill_rect(0, 0, WIDTH, HEIGHT, BLACK)?;

    // 見出しの帯。
    d.fill_rect(0, 0, WIDTH, 32, BAND)?;
    d.draw_text(8, 12, b"WASMICON", WHITE, BAND)?;
    d.draw_text(232, 12, b"RP2350", WHITE, BAND)?;

    // カラーバー。8 本 × 40 px でちょうど 320 px。
    let bar_w = WIDTH / BARS.len() as u16;
    let mut i = 0u16;
    while (i as usize) < BARS.len() {
        d.fill_rect(i * bar_w, 40, bar_w, 56, BARS[i as usize])?;
        i += 1;
    }
    d.draw_text(8, 104, b"RGB565 BARS", WHITE, BLACK)?;

    // グレーのランプ。16 段 × 20 px。R と B は 5 bit、G は 6 bit なので、
    // 同じ明るさにするには段の値を別々に伸ばす必要がある。
    let step_w = WIDTH / 16;
    let mut s = 0u16;
    while s < 16 {
        let r = (s * 31) / 15;
        let g = (s * 63) / 15;
        let b = (s * 31) / 15;
        d.fill_rect(s * step_w, 120, step_w, 32, (r << 11) | (g << 5) | b)?;
        s += 1;
    }
    d.draw_text(8, 160, b"GRAY RAMP", WHITE, BLACK)?;

    d.draw_text(8, 208, b"HELLO PICO 2", WHITE, BLACK)?;
    d.draw_text(8, 224, b"ILI9341 SPI", WHITE, BLACK)?;

    // 外周の枠。画面ぴったりの矩形を 4 つ置く。`fill_rect` は
    // `x + w > 320` を失敗にするので、ここが通れば端の扱いは合っている。
    d.fill_rect(0, 0, WIDTH, 1, WHITE)?;
    d.fill_rect(0, HEIGHT - 1, WIDTH, 1, WHITE)?;
    d.fill_rect(0, 0, 1, HEIGHT, WHITE)?;
    d.fill_rect(WIDTH - 1, 0, 1, HEIGHT, WHITE)?;

    sweep(d)
}

/// 16 px の四角を左右に 1 往復させる。
///
/// 毎コマ、前の位置を背景色で消してから次の位置を塗る。全画面の
/// フレームバッファは持てないので、動かすのはこの 2 つの矩形だけ。
fn sweep(d: &Display<'_>) -> Result<(), wasmicon_hal::ErrorCode> {
    /// 四角の一辺。
    const SIZE: u16 = 16;
    /// 走る帯の上端。カラーバーとテキストの間の空き。
    const TRACK_Y: u16 = 176;
    /// 左端と 1 コマの移動量。右端は `8 + 16 * 18 + 16 = 312` で枠に当たらない。
    const X0: u16 = 8;
    const STEP: u16 = 18;
    /// 四角の色（緑）。
    const BOX: u16 = 0x07E0;

    let mut prev = X0;
    d.fill_rect(prev, TRACK_Y, SIZE, SIZE, BOX)?;

    let mut n = 0u16;
    while n < FRAMES * 2 {
        // 前半は右へ、後半は左へ。往復して元の位置に戻る。
        let i = if n < FRAMES {
            n + 1
        } else {
            FRAMES * 2 - n - 1
        };
        let x = X0 + i * STEP;

        time::sleep_ms(40);
        d.fill_rect(prev, TRACK_Y, SIZE, SIZE, BLACK)?;
        d.fill_rect(x, TRACK_Y, SIZE, SIZE, BOX)?;
        prev = x;
        n += 1;
    }
    Ok(())
}

/// `no_std` なので自前で用意する（docs/handoff.md §3 #4）。
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
