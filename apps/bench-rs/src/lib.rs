//! インタプリタと host call の速さを測るベンチマーク。
//!
//! **同じ `.wasm` を host でも実機でも走らせ**、同じ物差しで比べる
//! （`wasmicon run` / `wasmicon deploy --monitor`）。結果は 1 項目 1 行で
//! ログに出す:
//!
//! ```text
//! bench name=loop iters=1000000 us=…
//! ```
//!
//! 計算の項目は `core::hint::black_box` で結果を使い、最適化で消えないようにする。
//! host call の項目は呼び出し 1 回あたりの時間を見るためのもので、`time.now-us`
//! （いちばん軽い）、`gpio.write`、`spi.write`（1 バイトと 98 バイト）を回す。
//! 時間を測る `time.now-us` 自体の呼び出し（項目ごとに 2 回）は誤差として無視する。
//!
//! 走り終えると `bench done` を出して `run` から戻る。

#![no_std]

// life-rs の盤面の計算をそのまま使う（写しを作らない）。使わないメソッドもある。
#[allow(dead_code)]
#[path = "../../life-rs/src/life.rs"]
mod life;

use core::hint::black_box;

use wasmicon_hal::gpio::{Level, Pin, PinMode};
use wasmicon_hal::spi::{Bus as SpiBus, Mode};
use wasmicon_hal::{board, log, time};

/// 整数ループ（命令の振り分けの速さ）。
const LOOP_ITERS: u32 = 1_000_000;
/// 再帰の fib（関数呼び出しと分岐）。
const FIB_N: u32 = 24;
/// エラトステネスのふるい（メモリの読み書き）。配列はゲストのスタック
/// （8 KiB）に置くので、それより十分小さくする。
const SIEVE_LEN: usize = 2048;
const SIEVE_ROUNDS: u32 = 40;
/// ライフゲームの世代の計算（life-rs と同じコード）。
const LIFE_GENS: u32 = 200;
/// host call の回数。
const CALLS: u32 = 1000;

#[unsafe(no_mangle)]
pub extern "C" fn run() {
    log::info("bench start");

    measure("loop", LOOP_ITERS, || {
        let mut x: u32 = 1;
        for i in 0..black_box(LOOP_ITERS) {
            x = x.wrapping_mul(1_664_525).wrapping_add(i) ^ (x >> 7);
        }
        black_box(x);
    });

    measure("fib", FIB_N, || {
        black_box(fib(black_box(FIB_N)));
    });

    measure("sieve", SIEVE_ROUNDS, || {
        let mut flags = [true; SIEVE_LEN];
        let mut count = 0u32;
        for _ in 0..black_box(SIEVE_ROUNDS) {
            flags.fill(true);
            count = 0;
            for i in 2..SIEVE_LEN {
                if flags[i] {
                    count += 1;
                    let mut j = i * 2;
                    while j < SIEVE_LEN {
                        flags[j] = false;
                        j += i;
                    }
                }
            }
        }
        black_box(count);
    });

    measure("life", LIFE_GENS, || {
        let mut b = life::Board::EMPTY;
        b.randomize(&mut life::Rng::new(0x5EED_1F3E));
        for _ in 0..black_box(LIFE_GENS) {
            b = b.step();
        }
        black_box(b.row(0));
    });

    measure("now-us", CALLS, || {
        for _ in 0..black_box(CALLS) {
            black_box(time::now_us());
        }
    });

    match board::pin_by_role("led").map(|i| Pin::open(i, PinMode::Output)) {
        Ok(Ok(pin)) => measure("gpio-write", CALLS, || {
            for i in 0..black_box(CALLS) {
                let level = if i & 1 == 0 { Level::High } else { Level::Low };
                let _ = pin.write(level);
            }
        }),
        _ => log::error("bench gpio-write skipped (no led role)"),
    }

    match SpiBus::open(0, 16_000_000, Mode::Mode0) {
        Ok(spi) => {
            measure("spi-write-1", CALLS, || {
                for _ in 0..black_box(CALLS) {
                    let _ = spi.write(&[0x00]);
                }
            });
            let block = [0x5Au8; 98];
            measure("spi-write-98", CALLS, || {
                for _ in 0..black_box(CALLS) {
                    let _ = spi.write(black_box(&block));
                }
            });
        }
        Err(_) => log::error("bench spi skipped (open failed)"),
    }

    log::info("bench done");
}

fn fib(n: u32) -> u32 {
    if n < 2 {
        n
    } else {
        fib(n - 1).wrapping_add(fib(n - 2))
    }
}

/// `f` を 1 回走らせて時間を測り、1 行出す。
fn measure(name: &str, iters: u32, f: impl FnOnce()) {
    let t0 = time::now_us();
    f();
    let us = time::now_us().saturating_sub(t0);
    let mut l = Line::new();
    l.s("bench name=")
        .s(name)
        .s(" iters=")
        .n(u64::from(iters))
        .s(" us=")
        .n(us);
    log::info(l.as_str());
}

/// 短い ASCII の行を組む（`core::fmt` を使わない）。
struct Line {
    buf: [u8; 96],
    n: usize,
}

impl Line {
    fn new() -> Self {
        Line { buf: [0; 96], n: 0 }
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

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
