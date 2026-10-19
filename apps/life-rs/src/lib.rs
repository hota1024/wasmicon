//! Demo that keeps drawing the game of life (B3/S23) on an ILI9341. No sensor
//! needed.
//!
//! - The board is 40x29 cells (8x8 px per cell; 7x7 is drawn, leaving a 1 px
//!   grid). The edges wrap around. The top 8 px of the screen show the FPS,
//!   updated every `FPS_EVERY` generations.
//! - FPS is measured with `time.now-us`. `time` is not traced, but the number
//!   shown depends on real time, so the SPI bytes of the FPS text differ per
//!   board and per run.
//! - The board is seeded by an xorshift32 with a fixed seed; every
//!   `GENERATIONS` generations it restarts with the next seed (the seed fully
//!   determines how the board evolves).
//! - No delay between generations; it runs as fast as compute and drawing
//!   allow.
//! - No full-screen frame buffer (it does not fit on RP2040). The board is
//!   stored as bits, and only the cells that changed since the previous
//!   generation are redrawn.
//! - `run` never returns. On hardware it runs forever, and `wasmicon run`
//!   does not end either (stop it with Ctrl-C).
//!
//! The pins come from `board.pin-by-role` for `lcd-cs` / `lcd-dc` / `lcd-rst`
//! (numbers in the wiring table of `wasmicon.toml`). Wiring is the same as
//! `apps/lcd-demo-rs/README.md`.
//!
//! A tracing firmware pushes hundreds of host calls per generation through
//! the UART, so the animation is slow. For a smooth picture, run a firmware
//! built without `trace`.

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

/// Background (the gaps between cells too).
const BG: u16 = 0x0841;
/// Live cell.
const LIVE: u16 = 0x3FE8;
/// FPS text.
const TEXT: u16 = 0xA514;

/// Height in px of the status band above the board.
const TOP: u16 = 8;
/// Generations between FPS measurements and redraws. Drawing it every
/// generation would lower the FPS by the time spent on the text.
const FPS_EVERY: u32 = 20;

/// Cell geometry in px: 7 px drawn in an 8 px grid, leaving 1 px as the gap.
const CELL: u16 = 8;
const DOT: u16 = 7;

/// Bytes of pixels per cell (7x7 px x 2 bytes).
const DOT_BYTES: usize = DOT as usize * DOT as usize * 2;

/// Generations per seed.
const GENERATIONS: u32 = 200;
/// SPI clock. As in lcd-demo, a value that is less likely to fail during
/// bring-up.
const SPI_HZ: u32 = 16_000_000;

/// First seed; each round adds to it.
const SEED: u32 = 0x5EED_1F3E;

/// Entry point. The host calls it once, right after instantiation
/// (docs/abi-spec.md §3.3).
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
    // There are only two kinds of cell pixels, so build them once and reuse
    // them.
    let cells = Cells {
        live: dot(LIVE),
        dead: dot(BG),
    };

    // Bench mode reopens SPI at several clocks, so it branches off here.
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

/// Pixels of a live cell and of a dead cell.
struct Cells {
    live: [u8; DOT_BYTES],
    dead: [u8; DOT_BYTES],
}

/// Builds the pixels of one cell in a single color.
fn dot(color: u16) -> [u8; DOT_BYTES] {
    let mut px = [0u8; DOT_BYTES];
    for p in px.chunks_exact_mut(2) {
        p[0] = (color >> 8) as u8;
        p[1] = color as u8;
    }
    px
}

/// Runs forever, changing the seed each round. Returns only when drawing
/// fails.
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

/// Draws the FPS for `frames` generations that took `elapsed_us`, with one
/// decimal, in the top band.
fn draw_fps(d: &Display, frames: u32, elapsed_us: u64) -> Result<(), ErrorCode> {
    let tenths = (u64::from(frames) * 10_000_000)
        .checked_div(elapsed_us)
        .unwrap_or(0);
    let mut text = *b"FPS     .  ";
    // Integer part right-aligned in 4 digits (up to 9999.9).
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

/// Redraws only the cells that changed from `old` to `new`. Returns how many
/// cells were drawn.
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

/// Required because the crate is `no_std` (docs/decisions.md A-04).
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
