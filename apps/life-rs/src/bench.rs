//! 計測モード（`--features bench`）。
//!
//! 毎回**同じ種**から `GENERATIONS` 世代を回して仕事量を揃え、SPI のクロックを
//! `CLOCKS` の順に変えながら、1 回ごとに次の 1 行を UART のログに出す:
//!
//! ```text
//! bench spi=24000000 gens=200 total_us=… step_us=… draw_us=… cells=… us_per_cell=… fps10=…
//! ```
//!
//! - `step_us` = 世代の計算、`draw_us` = 描き直し（host call と SPI）の合計
//! - `fps10` = 10 倍した FPS（小数 1 桁を整数で）
//!
//! 全部のクロックを回すと `bench cycle done` を出して最初から繰り返す。
//! 実際に出るクロックはポートが丸める（RP2350 は要求値を超えない最大）。

use wasmicon_hal::gpio::Pin;
use wasmicon_hal::spi::{Bus as SpiBus, Mode};
use wasmicon_hal::{ErrorCode, log, time};

use crate::ili9341::{Display, HEIGHT, WIDTH};
use crate::life::{Board, Rng};
use crate::{BG, Cells, GENERATIONS, SEED, draw_diff};

/// 試す SPI のクロック（Hz）。
const CLOCKS: [u32; 4] = [16_000_000, 24_000_000, 32_000_000, 48_000_000];

/// 1 回分の結果。
struct Stats {
    total_us: u64,
    step_us: u64,
    draw_us: u64,
    cells: u64,
}

/// 計測を繰り返す。失敗したら理由を出して戻る。
pub fn run(cs: &Pin, dc: &Pin, rst: &Pin, cells: &Cells) {
    log::info("bench start");
    let mut first = true;
    loop {
        for hz in CLOCKS {
            let Ok(spi) = SpiBus::open(0, hz, Mode::Mode0) else {
                log::error("spi open failed");
                return;
            };
            let d = Display::new(&spi, cs, dc);
            // パネルの初期化は最初の 1 回だけ（SPI を開き直しても設定は残る）。
            if first {
                if d.init(rst).is_err() {
                    log::error("display failed");
                    return;
                }
                first = false;
            }
            match round(&d, cells) {
                Ok(s) => report(hz, &s),
                Err(_) => {
                    log::error("display failed");
                    return;
                }
            }
        }
        log::info("bench cycle done");
    }
}

/// 同じ種から `GENERATIONS` 世代を回して測る。
fn round(d: &Display, cells: &Cells) -> Result<Stats, ErrorCode> {
    let mut board = Board::EMPTY;
    board.randomize(&mut Rng::new(SEED));
    d.fill_rect(0, 0, WIDTH, HEIGHT, BG)?;
    draw_diff(d, cells, &Board::EMPTY, &board)?;

    let mut s = Stats {
        total_us: 0,
        step_us: 0,
        draw_us: 0,
        cells: 0,
    };
    let start = time::now_us();
    for _ in 0..GENERATIONS {
        let t0 = time::now_us();
        let next = board.step();
        let t1 = time::now_us();
        s.cells += u64::from(draw_diff(d, cells, &board, &next)?);
        let t2 = time::now_us();
        s.step_us += t1.saturating_sub(t0);
        s.draw_us += t2.saturating_sub(t1);
        board = next;
    }
    s.total_us = time::now_us().saturating_sub(start);
    Ok(s)
}

/// 1 行にしてログに出す。
fn report(hz: u32, s: &Stats) {
    let mut l = Line::new();
    l.s("bench spi=").n(u64::from(hz));
    l.s(" gens=").n(u64::from(GENERATIONS));
    l.s(" total_us=").n(s.total_us);
    l.s(" step_us=").n(s.step_us);
    l.s(" draw_us=").n(s.draw_us);
    l.s(" cells=").n(s.cells);
    l.s(" us_per_cell=")
        .n(s.draw_us.checked_div(s.cells).unwrap_or(0));
    l.s(" fps10=").n((u64::from(GENERATIONS) * 10_000_000)
        .checked_div(s.total_us)
        .unwrap_or(0));
    log::info(l.as_str());
}

/// 短い ASCII の行を組む（`core::fmt` を使わない）。
struct Line {
    buf: [u8; 160],
    n: usize,
}

impl Line {
    fn new() -> Self {
        Line {
            buf: [0; 160],
            n: 0,
        }
    }

    fn s(&mut self, t: &str) -> &mut Self {
        for &b in t.as_bytes() {
            if self.n < self.buf.len() {
                self.buf[self.n] = b;
                self.n += 1;
            }
        }
        self
    }

    fn n(&mut self, mut v: u64) -> &mut Self {
        let mut d = [0u8; 20];
        let mut i = d.len();
        loop {
            i -= 1;
            d[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        for &b in &d[i..] {
            if self.n < self.buf.len() {
                self.buf[self.n] = b;
                self.n += 1;
            }
        }
        self
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.n]).unwrap_or("?")
    }
}
