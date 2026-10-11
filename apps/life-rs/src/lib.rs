//! ILI9341 にライフゲーム（B3/S23）を描き続けるデモ。センサーは要らない。
//!
//! - 盤面は 40×29 セル（1 セル 8×8 px、描くのは 7×7 で 1 px の隙間を格子にする）。
//!   端は上下左右で繋がる。画面の上 8 px は FPS の表示（`FPS_EVERY` 世代ごとに更新）
//! - FPS は `time.now-us` で測る。`time` はトレースに出ないが、**表示する数字は
//!   実時間で変わる**ので、FPS の文字を描く SPI の中身はボードと実行ごとに違う
//! - 固定の種の xorshift32 で盤面を埋め、`GENERATIONS` 世代ごとに種を変えて
//!   やり直す（盤面の進み方は種で決まる）
//! - 世代のあいだは待たない（計算と描画の速さで回す）
//! - 全画面フレームバッファは持たない（RP2040 に載らない）。盤面はビットで持ち、
//!   **前の世代と違うセルだけ**描き直す
//! - **`run` から戻らない。** 実機ではずっと動き続け、`wasmicon run` も終わらない
//!   （Ctrl-C で止める）
//!
//! ピンは `board.pin-by-role` で `lcd-cs` / `lcd-dc` / `lcd-rst` を引く
//! （番号は `wasmicon.toml` の配線表）。配線は `apps/lcd-demo-rs/README.md` と同じ。
//!
//! トレースを出すファームでは、1 世代に数百の host call を UART に流すので
//! 動きが遅い。滑らかに見たいときは `trace` を切ったファームで走らせる。

#![no_std]

#[cfg(feature = "bench")]
mod bench;
mod font;
mod ili9341;
mod life;

use wasmicon_hal::gpio::{Pin, PinMode};
use wasmicon_hal::spi::{Bus as SpiBus, Mode};
use wasmicon_hal::{ErrorCode, board, log, time};

use ili9341::{Display, HEIGHT, WIDTH};
use life::{Board, ROWS, Rng};

/// 背景（セルの隙間もこの色）。
const BG: u16 = 0x0841;
/// 生きているセル。
const LIVE: u16 = 0x3FE8;
/// FPS の文字。
const TEXT: u16 = 0xA514;

/// 盤面の上に取る表示の帯の高さ（px）。
const TOP: u16 = 8;
/// FPS を測って描き直す間隔（世代）。毎世代描くと、文字を描く時間で FPS が下がる。
const FPS_EVERY: u32 = 20;

/// 1 セルの描画の大きさ（px）。8 px の升目に 7 px 描き、1 px を隙間にする。
const CELL: u16 = 8;
const DOT: u16 = 7;

/// 1 セルの画素のバイト数（7×7 px × 2 バイト）。
const DOT_BYTES: usize = DOT as usize * DOT as usize * 2;

/// 1 回の種で回す世代数。
const GENERATIONS: u32 = 200;
/// SPI のクロック。lcd-demo と同じく、ブリングアップで転びにくい値。
const SPI_HZ: u32 = 16_000_000;

/// 最初の種。回ごとに足していく。
const SEED: u32 = 0x5EED_1F3E;

/// エントリポイント。ホストがインスタンス化直後に 1 回呼ぶ（abi-spec §3.3）。
#[unsafe(no_mangle)]
pub extern "C" fn run() {
    log::info("life start");

    let (Ok(cs_i), Ok(dc_i), Ok(rst_i)) = (
        board::pin_by_role("lcd-cs"),
        board::pin_by_role("lcd-dc"),
        board::pin_by_role("lcd-rst"),
    ) else {
        log::error("role not found");
        return;
    };
    let (Ok(cs), Ok(dc), Ok(rst)) = (
        Pin::open(cs_i, PinMode::Output),
        Pin::open(dc_i, PinMode::Output),
        Pin::open(rst_i, PinMode::Output),
    ) else {
        log::error("gpio open failed");
        return;
    };
    // セルの画素は 2 種類しか無いので、最初に 1 回だけ作って使い回す。
    let cells = Cells {
        live: dot(LIVE),
        dead: dot(BG),
    };

    // 計測モードは SPI のクロックを変えながら開き直すので、ここで分かれる。
    #[cfg(feature = "bench")]
    {
        bench::run(&cs, &dc, &rst, &cells);
        return;
    }

    #[allow(unreachable_code)]
    let Ok(spi) = SpiBus::open(0, SPI_HZ, Mode::Mode0) else {
        log::error("spi open failed");
        return;
    };

    let display = Display::new(&spi, &cs, &dc);
    if display.init(&rst).is_err() {
        log::error("display failed");
        return;
    }
    if play(&display, &cells).is_err() {
        log::error("display failed");
    }
}

/// 生きたセルと死んだセルの画素。
struct Cells {
    live: [u8; DOT_BYTES],
    dead: [u8; DOT_BYTES],
}

/// 1 セルぶんの画素を単色で作る。
fn dot(color: u16) -> [u8; DOT_BYTES] {
    let mut px = [0u8; DOT_BYTES];
    for p in px.chunks_exact_mut(2) {
        p[0] = (color >> 8) as u8;
        p[1] = color as u8;
    }
    px
}

/// 種を変えながら回し続ける。描画が失敗したときだけ戻る。
fn play(d: &Display, cells: &Cells) -> Result<(), ErrorCode> {
    let mut round: u32 = 0;
    loop {
        let mut rng = Rng::new(SEED.wrapping_add(round.wrapping_mul(0x9E37_79B9)));
        let mut board = Board::EMPTY;
        board.randomize(&mut rng);
        log::info("life new round");

        d.fill_rect(0, 0, WIDTH, HEIGHT, BG)?;
        draw_diff(d, cells, &Board::EMPTY, &board)?;

        let mut since = time::now_us();
        for generation in 1..=GENERATIONS {
            let next = board.step();
            draw_diff(d, cells, &board, &next)?;
            board = next;
            if generation % FPS_EVERY == 0 {
                let now = time::now_us();
                draw_fps(d, FPS_EVERY, now.saturating_sub(since))?;
                since = now;
            }
        }
        round = round.wrapping_add(1);
    }
}

/// `frames` 世代に `elapsed_us` かかったときの FPS を、小数 1 桁で上の帯に描く。
fn draw_fps(d: &Display, frames: u32, elapsed_us: u64) -> Result<(), ErrorCode> {
    let tenths = (u64::from(frames) * 10_000_000)
        .checked_div(elapsed_us)
        .unwrap_or(0);
    let mut text = *b"FPS     .  ";
    // 整数部は右詰め 4 桁（9999.9 まで）。
    let int = tenths / 10;
    let mut v = int;
    let mut i = 7;
    loop {
        text[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 || i == 4 {
            break;
        }
        i -= 1;
    }
    text[9] = b'0' + (tenths % 10) as u8;
    d.draw_text(4, 0, &text[..10], TEXT, BG)
}

/// `old` から `new` で変わったセルだけ描き直す。描いたセルの数を返す。
fn draw_diff(d: &Display, cells: &Cells, old: &Board, new: &Board) -> Result<u32, ErrorCode> {
    let mut drawn = 0;
    for y in 0..ROWS {
        let mut changed = old.row(y) ^ new.row(y);
        while changed != 0 {
            let x = changed.trailing_zeros() as usize;
            changed &= changed - 1;
            let px = if new.get(x, y) {
                &cells.live
            } else {
                &cells.dead
            };
            d.blit(x as u16 * CELL, TOP + y as u16 * CELL, DOT, DOT, px)?;
            drawn += 1;
        }
    }
    Ok(drawn)
}

/// `no_std` なので自前で用意する。
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
