//! 実機の切り分け用。**ゲストを走らせる前に、ポート層 (`Board`) を直接叩く。**
//!
//! `--features hw-probe` のときだけ `main` が呼ぶ。トレースが host と完全一致して
//! いても画面が出ないとき、「ポートのレジスタ設定がパッドまで届いているか」は
//! トレースからは分からない。ここが唯一それを確かめられる場所:
//!
//! - **ILI9341 の ID を読む**（`0xD3` / `0x04`）。`SDO`/`MISO` を `GPIO13` に
//!   繋いでいれば `0xD3` は `00 00 93 41` を返す。返ってくれば
//!   CS / DC / SCK / MOSI / MISO が全て生きていることになる。
//!   `00 00 00 00` や `ff ff ff ff` なら経路が死んでいる
//! - **DC を 1 Hz で振る**。テスターや LED で当たれば、GPIO がパッドまで
//!   届いているかを目で確認できる
//!
//! 切り分け専用。済んだら feature ごと落として構わない。

use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_core::generated::ErrorCode;
use wasmicon_port::fmt::Buf;
use wasmicon_port::Board;

use crate::board::{EspBoard, Serial};

/// 役割名で引ける番号だが、ここは `Board` を直接叩くので定数で持つ
/// （abi-spec §8 の ESP32-S3 の表と同じ）。
const CS: u32 = 10;
const DC: u32 = 14;
const RST: u32 = 15;

/// 読み出しは遅いほうが通りやすいので、デモより落としてある。
const SPI_HZ: u32 = 1_000_000;

/// DC を 1 Hz で振る回数。
const WIGGLE: u32 = 10;

/// 画面の大きさ（横向き）。
const WIDTH: u16 = 320;
const HEIGHT: u16 = 240;

/// 1 行分のバイト数。
const ROW_BYTES: usize = WIDTH as usize * 2;

fn say<S: Serial>(b: &mut EspBoard<S>, s: &str) {
    b.serial().write(s.as_bytes());
    b.serial().write(b"\r\n");
}

/// `<label>: ok` か `<label>: err=<n>` を出す。
fn say_result<S: Serial>(b: &mut EspBoard<S>, label: &str, r: Result<(), ErrorCode>) {
    let mut raw = [0u8; 64];
    let mut out = Buf::new(&mut raw);
    out.str(label);
    match r {
        Ok(()) => out.str(": ok"),
        Err(e) => {
            out.str(": err=");
            out.u32(e as u32);
        }
    }
    let n = out.as_bytes().len();
    let mut line = [0u8; 64];
    line[..n].copy_from_slice(out.as_bytes());
    b.serial().write(&line[..n]);
    b.serial().write(b"\r\n");
}

/// 読んだバイト列を 16 進で出す。
fn say_bytes<S: Serial>(b: &mut EspBoard<S>, label: &str, data: &[u8]) {
    let mut raw = [0u8; 96];
    let mut out = Buf::new(&mut raw);
    out.str(label);
    for &x in data {
        out.byte(b' ');
        out.hex(u32::from(x), 2);
    }
    let n = out.as_bytes().len();
    let mut line = [0u8; 96];
    line[..n].copy_from_slice(out.as_bytes());
    b.serial().write(&line[..n]);
    b.serial().write(b"\r\n");
}

/// CS を握って 1 コマンド（+ 引数）を送る。ゲストの `ili9341.rs` と同じ手順。
fn send<S: Serial>(b: &mut EspBoard<S>, cmd: u8, args: &[u8]) -> Result<(), ErrorCode> {
    b.gpio_write(CS, Level::Low)?;
    b.gpio_write(DC, Level::Low)?;
    b.spi_write(0, &[cmd])?;
    if !args.is_empty() {
        b.gpio_write(DC, Level::High)?;
        b.spi_write(0, args)?;
    }
    b.gpio_write(CS, Level::High)
}

/// コマンドを送って `n` バイト読む。ILI9341 は先頭 1 バイトがダミー。
fn read_id<S: Serial>(b: &mut EspBoard<S>, cmd: u8, n: usize) -> Result<[u8; 5], ErrorCode> {
    let mut rx = [0u8; 5];
    b.gpio_write(CS, Level::Low)?;
    b.gpio_write(DC, Level::Low)?;
    b.spi_write(0, &[cmd])?;
    b.gpio_write(DC, Level::High)?;
    let tx = [0u8; 5];
    b.spi_transfer(0, &tx[..n], &mut rx[..n])?;
    b.gpio_write(CS, Level::High)?;
    Ok(rx)
}

/// 画面全体を 1 色で塗る。**ゲストもインタプリタも通さずに SPI が生きているかを
/// 目で見るためのもの。** 赤くなればポート層の SPI とピンは全部生きている。
fn fill_screen<S: Serial>(b: &mut EspBoard<S>, color: u16) -> Result<(), ErrorCode> {
    let x1 = WIDTH - 1;
    let y1 = HEIGHT - 1;
    send(b, 0x2A, &[0, 0, (x1 >> 8) as u8, x1 as u8])?; // CASET
    send(b, 0x2B, &[0, 0, (y1 >> 8) as u8, y1 as u8])?; // PASET

    let mut row = [0u8; ROW_BYTES];
    let mut i = 0;
    while i < ROW_BYTES {
        row[i] = (color >> 8) as u8;
        row[i + 1] = color as u8;
        i += 2;
    }

    b.gpio_write(CS, Level::Low)?;
    b.gpio_write(DC, Level::Low)?;
    b.spi_write(0, &[0x2C])?; // RAMWR
    b.gpio_write(DC, Level::High)?;
    let mut r = 0;
    while r < HEIGHT {
        b.spi_write(0, &row)?;
        r += 1;
    }
    b.gpio_write(CS, Level::High)
}

// **MOSI をソフトだけで読み返す検査は成立しない。** MISO を MOSI と同じピンに
// 向けて全二重転送すれば読み返せるはずだが、ESP32-S3 では GPIO11 が FSPID
// そのものなので、`with_mosi` は GPIO マトリクスを使わず IO_MUX の直結機能を
// 選ぶ。そこへ `with_miso` を同じピンに向けると `set_alternate_function` が
// `mcu_sel` を GPIO 機能に書き換え、**直結していた MOSI 出力が切り離される**。
// 実際に試すと送ったバイトではなく `ff ff ff ff` が返る（浮いたパッドを
// 読んでいる）。MOSI がパッドに出ているかは外から当てるしかない。

/// 順に試す。失敗しても止めない（どこまで届くかを見たい）。
pub fn run<S: Serial>(b: &mut EspBoard<S>) {
    say(b, "probe: start (hw-probe)");
    say(
        b,
        "probe: cs=GPIO10 dc=GPIO14 rst=GPIO15 sck=GPIO12 mosi=GPIO11 miso=GPIO13",
    );

    let r = b.gpio_configure(CS, PinMode::Output);
    say_result(b, "probe: configure cs", r);
    let r = b.gpio_configure(DC, PinMode::Output);
    say_result(b, "probe: configure dc", r);
    let r = b.gpio_configure(RST, PinMode::Output);
    say_result(b, "probe: configure rst", r);

    let _ = b.gpio_write(CS, Level::High);
    let _ = b.gpio_write(RST, Level::Low);
    b.sleep_ms(10);
    let _ = b.gpio_write(RST, Level::High);
    b.sleep_ms(120);
    say(b, "probe: reset pulsed");

    let r = b.spi_open(0, SPI_HZ, SpiMode::Mode0);
    say_result(b, "probe: spi_open(0, 1MHz, mode0)", r);

    // 初期化列（ゲストと同じ。apps/README.md §2）。
    let mut init = send(b, 0x01, &[]); // SWRESET
    b.sleep_ms(120);
    init = init.and_then(|()| send(b, 0x11, &[])); // SLPOUT
    b.sleep_ms(120);
    init = init.and_then(|()| send(b, 0x3A, &[0x55])); // PIXFMT = RGB565
    init = init.and_then(|()| send(b, 0x36, &[0x28])); // MADCTL = 横向き
    init = init.and_then(|()| send(b, 0x29, &[])); // DISPON
    say_result(b, "probe: init sequence", init);

    // ここが本題。MISO が繋がっていれば 0xD3 は 00 00 93 41 を返す。
    match read_id(b, 0xD3, 5) {
        Ok(rx) => say_bytes(b, "probe: 0xD3 (want 00 00 93 41) ->", &rx[..5]),
        Err(e) => say_result(b, "probe: 0xD3", Err(e)),
    }
    match read_id(b, 0x04, 4) {
        Ok(rx) => say_bytes(b, "probe: 0x04 RDDID             ->", &rx[..4]),
        Err(e) => say_result(b, "probe: 0x04", Err(e)),
    }

    // 全画面を赤で塗る。ここで赤くなればポート層の SPI とピンは全部生きている。
    // 同時に SCK / MOSI に 150 KB ぶんのクロックが出るので、そちらに
    // USB シリアル変換の RXD を当てていれば大量のバイトが観測できる。
    say(b, "probe: filling screen red (port layer only)");
    let r = fill_screen(b, 0xF800);
    say_result(b, "probe: fill_screen", r);

    // GPIO がパッドまで届いているかをテスター / LED で見るための 1 Hz。
    say(b, "probe: wiggling DC (GPIO14) at 1Hz for 10s");
    let mut i = 0;
    while i < WIGGLE {
        let _ = b.gpio_write(DC, Level::High);
        b.sleep_ms(500);
        let _ = b.gpio_write(DC, Level::Low);
        b.sleep_ms(500);
        i += 1;
    }

    b.spi_close(0);
    say(b, "probe: done, handing over to the guest");
}
