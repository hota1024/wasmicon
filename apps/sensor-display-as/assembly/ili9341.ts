// ILI9341 の最小ドライバ。apps/README.md §2 が正。
// Rust 版（apps/sensor-display-rs/src/ili9341.rs）と同じバイト列を送る。

import { Level, Pin, SpiBus, time } from "../../../bindings/assemblyscript/assembly/index";
import { GLYPHS, indexOf } from "./font";

export const WIDTH: u16 = 320;
export const HEIGHT: u16 = 240;

/// 文字列描画で一度に送れる最大文字数。
export const MAX_TEXT: i32 = 12;

/// 1 行分のピクセルバッファ（320 px × 2 バイト）。
const ROW = new Uint8Array(<i32>WIDTH * 2);
/// 文字列描画のバッファ（MAX_TEXT×8 px × 8 行 × 2 バイト）。
const TEXT = new Uint8Array(MAX_TEXT * 8 * 8 * 2);
/// コマンド 1 バイト用。
const CMD = new Uint8Array(1);
/// 引数用（最大 4 バイト）。
const ARGS = new Uint8Array(4);

export class Display {
  constructor(public spi: SpiBus, public cs: Pin, public dc: Pin) {}

  private begin(): void {
    this.cs.write(Level.Low);
  }

  private end(): void {
    this.cs.write(Level.High);
  }

  private cmd(c: u8): void {
    this.dc.write(Level.Low);
    CMD[0] = c;
    this.spi.write(CMD);
  }

  private dataN(buf: Uint8Array, len: i32): void {
    this.dc.write(Level.High);
    this.spi.write(buf.subarray(0, len));
  }

  /// コマンドと引数を 1 回の CS で送る。
  private send(c: u8, argLen: i32): void {
    this.begin();
    this.cmd(c);
    if (argLen > 0) this.dataN(ARGS, argLen);
    this.end();
  }

  /// リセットと初期化列。
  init(rst: Pin): void {
    this.cs.write(Level.High);
    rst.write(Level.Low);
    time.sleepMs(10);
    rst.write(Level.High);
    time.sleepMs(120);

    this.send(0x01, 0); // SWRESET
    time.sleepMs(120);
    this.send(0x11, 0); // SLPOUT
    time.sleepMs(120);
    ARGS[0] = 0x55;
    this.send(0x3a, 1); // PIXFMT = RGB565
    ARGS[0] = 0x28;
    this.send(0x36, 1); // MADCTL = 横向き
    this.send(0x29, 0); // DISPON
  }

  /// 書き込み先の矩形を決める。
  private window(x: u16, y: u16, w: u16, h: u16): void {
    const x1: u16 = x + w - 1;
    const y1: u16 = y + h - 1;
    ARGS[0] = <u8>(x >> 8);
    ARGS[1] = <u8>x;
    ARGS[2] = <u8>(x1 >> 8);
    ARGS[3] = <u8>x1;
    this.send(0x2a, 4);
    ARGS[0] = <u8>(y >> 8);
    ARGS[1] = <u8>y;
    ARGS[2] = <u8>(y1 >> 8);
    ARGS[3] = <u8>y1;
    this.send(0x2b, 4);
  }

  /// 矩形を単色で塗る。行単位で送る。
  fillRect(x: u16, y: u16, w: u16, h: u16, color: u16): void {
    if (w == 0 || h == 0) return;
    this.window(x, y, w, h);

    const n = <i32>w * 2;
    for (let i = 0; i < n; i += 2) {
      ROW[i] = <u8>(color >> 8);
      ROW[i + 1] = <u8>color;
    }

    this.begin();
    this.cmd(0x2c); // RAMWR
    for (let r: u16 = 0; r < h; r++) {
      this.dataN(ROW, n);
    }
    this.end();
  }

  /// 等幅 8×8 で文字列を描く。
  drawText(x: u16, y: u16, text: Uint8Array, len: i32, fg: u16, bg: u16): void {
    const n = len > MAX_TEXT ? MAX_TEXT : len;
    if (n == 0) return;
    const w = <u16>(n * 8);
    this.window(x, y, w, 8);

    const stride = n * 8 * 2;
    for (let row = 0; row < 8; row++) {
      for (let col = 0; col < n; col++) {
        const bits = GLYPHS[indexOf(text[col]) * 8 + row];
        for (let bit = 0; bit < 8; bit++) {
          const on = (<i32>bits & (0x80 >> bit)) != 0;
          const c: u16 = on ? fg : bg;
          const p = row * stride + (col * 8 + bit) * 2;
          TEXT[p] = <u8>(c >> 8);
          TEXT[p + 1] = <u8>c;
        }
      }
    }

    this.begin();
    this.cmd(0x2c); // RAMWR
    this.dataN(TEXT, stride * 8);
    this.end();
  }
}
