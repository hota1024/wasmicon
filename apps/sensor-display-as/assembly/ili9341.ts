// ILI9341 の最小ドライバ。apps/README.md §2 が正。
// Rust 版（apps/sensor-display-rs/src/ili9341.rs）と同じバイト列を送る。

import { Level, Pin, SpiBus, time } from "../../../bindings/assemblyscript/assembly/index";
import { GLYPHS, indexOf } from "./font";

export const WIDTH: u16 = 320;
export const HEIGHT: u16 = 240;

/// 文字列描画で一度に送れる最大文字数（8 px × 40 = 画面の幅）。
export const MAX_TEXT: i32 = 40;

/// 1 行分のピクセルバッファ（320 px × 2 バイト）。
const ROW = new Uint8Array(<i32>WIDTH * 2);
/// 引数が不正なときに返す。ErrorCode.InvalidArgument のステータス（discriminant+1）。
const ERR_INVALID: u32 = 1;

/// `start + len` が `limit` に収まるか。**u32 に広げてから足す。**
///
/// AssemblyScript は u16 同士の加算を比較の中でも u16 に丸める
/// （`i32.add` のあとに `i32.and 0xffff` を出す）ので、u16 のまま足すと
/// 折り返して画面外の座標が境界検査を通ってしまう。すり抜ける入力は
/// `start + len` が 65536..65856 のもので、**結果は 2 つあり入力の範囲が違う**:
///
/// - `fillRect(65530, 0, 10, 1, c)` → `65530 + 10 == 4` で通り、`window` が
///   `x0 > x1` の反転した矩形を送る。バッファは超えない（`n == 20`）
/// - `fillRect(65216, 0, 640, 1, c)` → 折り返して 320 で通り、さらに
///   `n = w * 2 == 1280` が 640 バイトの `ROW` を超える。**こちらだけが
///   範囲外書き込み**で、`asconfig.json` は `noAssert: true` なので
///   境界検査が無く、黙ってリニアメモリを壊す（Rust 側はトラップする）
///
/// `drawText` は幅と高さを i32 で計算し、画面を超えたら u16 に落とす前に
/// 弾くので、ここに折り返した値は来ない。
/// Rust 版の `in_bounds()` と同じ形に揃えてある
/// （`apps/sensor-display-rs/src/ili9341.rs`）。
function inBounds(start: u16, len: u16, limit: u16): bool {
  return <u32>start + <u32>len <= <u32>limit;
}

/// コマンド 1 バイト用。
const CMD = new Uint8Array(1);
/// 引数用（最大 4 バイト）。
const ARGS = new Uint8Array(4);

export class Display {
  constructor(public spi: SpiBus, public cs: Pin, public dc: Pin) {}

  private begin(): u32 {
    return this.cs.write(Level.Low);
  }

  private end(): u32 {
    return this.cs.write(Level.High);
  }

  private cmd(c: u8): u32 {
    let st = this.dc.write(Level.Low);
    if (st != 0) return st;
    CMD[0] = c;
    return this.spi.write(CMD);
  }

  private dataN(buf: Uint8Array, len: i32): u32 {
    const st = this.dc.write(Level.High);
    if (st != 0) return st;
    return this.spi.write(buf.subarray(0, len));
  }

  /// コマンドと引数を 1 回の CS で送る。0 なら成功。
  private send(c: u8, argLen: i32): u32 {
    let st = this.begin();
    if (st != 0) return st;
    st = this.cmd(c);
    if (st != 0) return st;
    if (argLen > 0) {
      st = this.dataN(ARGS, argLen);
      if (st != 0) return st;
    }
    return this.end();
  }

  /// リセットと初期化列。0 なら成功。
  init(rst: Pin): u32 {
    let st = this.cs.write(Level.High);
    if (st != 0) return st;
    st = rst.write(Level.Low);
    if (st != 0) return st;
    time.sleepMs(10);
    st = rst.write(Level.High);
    if (st != 0) return st;
    time.sleepMs(120);

    st = this.send(0x01, 0); // SWRESET
    if (st != 0) return st;
    time.sleepMs(120);
    st = this.send(0x11, 0); // SLPOUT
    if (st != 0) return st;
    time.sleepMs(120);
    ARGS[0] = 0x55;
    st = this.send(0x3a, 1); // PIXFMT = RGB565
    if (st != 0) return st;
    ARGS[0] = 0x28;
    st = this.send(0x36, 1); // MADCTL = 横向き
    if (st != 0) return st;
    return this.send(0x29, 0); // DISPON
  }

  /// 書き込み先の矩形を決める。
  private window(x: u16, y: u16, w: u16, h: u16): u32 {
    const x1: u16 = x + w - 1;
    const y1: u16 = y + h - 1;
    ARGS[0] = <u8>(x >> 8);
    ARGS[1] = <u8>x;
    ARGS[2] = <u8>(x1 >> 8);
    ARGS[3] = <u8>x1;
    const st = this.send(0x2a, 4);
    if (st != 0) return st;
    ARGS[0] = <u8>(y >> 8);
    ARGS[1] = <u8>y;
    ARGS[2] = <u8>(y1 >> 8);
    ARGS[3] = <u8>y1;
    return this.send(0x2b, 4);
  }

  /// 矩形を単色で塗る。行単位で送る。0 なら成功。
  fillRect(x: u16, y: u16, w: u16, h: u16, color: u16): u32 {
    if (w == 0 || h == 0) return 0;
    // 画面外は描かない（apps/README.md §2）。ROW の範囲外書き込みも防ぐ。
    if (!inBounds(x, w, WIDTH) || !inBounds(y, h, HEIGHT)) return ERR_INVALID;
    let st = this.window(x, y, w, h);
    if (st != 0) return st;

    const n = <i32>w * 2;
    for (let i = 0; i < n; i += 2) {
      ROW[i] = <u8>(color >> 8);
      ROW[i + 1] = <u8>color;
    }

    st = this.begin();
    if (st != 0) return st;
    st = this.cmd(0x2c); // RAMWR
    if (st != 0) return st;
    for (let r: u16 = 0; r < h; r++) {
      st = this.dataN(ROW, n);
      if (st != 0) return st;
    }
    return this.end();
  }

  /// 等幅 8×8 の文字を `scale` 倍に拡大して描く。0 なら成功。
  ///
  /// フォントの 1 行を拡大した 1 ピクセル行を組み立て、それを `scale` 回
  /// 送る。バッファは画面 1 行ぶん（ROW）で足りる。
  drawText(x: u16, y: u16, text: Uint8Array, len: i32, scale: u16, fg: u16, bg: u16): u32 {
    if (len == 0) return 0;
    // **負の len を弾く。** Rust 版は `&[u8]` を取るので構造的に表現できない
    // 穴で、AS 側だけに開いていた。`len == -8192` だと `<u16>(n * 8) == 0` に
    // なって inBounds を通り、`window` が `x1 = x - 1` を送ったうえで
    // `dataN` が負の長さで呼ばれる（`noAssert: true` なので止まらない）。
    // 今の呼び出し元は 0 から増やすだけなので到達しない。
    if (len < 0) return ERR_INVALID;
    // 黙って切り詰めない（apps/README.md §2）。
    if (len > MAX_TEXT || scale == 0) return ERR_INVALID;
    const n = len;
    const s = <i32>scale;
    // u16 に落とす前に画面と比べる（inBounds のコメント）。
    const wi = n * 8 * s;
    const hi = 8 * s;
    if (wi > <i32>WIDTH || hi > <i32>HEIGHT) return ERR_INVALID;
    const w = <u16>wi;
    const h = <u16>hi;
    if (!inBounds(x, w, WIDTH) || !inBounds(y, h, HEIGHT)) return ERR_INVALID;
    let st = this.window(x, y, w, h);
    if (st != 0) return st;

    st = this.begin();
    if (st != 0) return st;
    st = this.cmd(0x2c); // RAMWR
    if (st != 0) return st;
    const bytes = wi * 2;
    for (let glyphRow = 0; glyphRow < 8; glyphRow++) {
      let p = 0;
      for (let col = 0; col < n; col++) {
        const bits = GLYPHS[indexOf(text[col]) * 8 + glyphRow];
        for (let bit = 0; bit < 8; bit++) {
          const on = (<i32>bits & (0x80 >> bit)) != 0;
          const c: u16 = on ? fg : bg;
          for (let k = 0; k < s; k++) {
            ROW[p] = <u8>(c >> 8);
            ROW[p + 1] = <u8>c;
            p += 2;
          }
        }
      }
      for (let k = 0; k < s; k++) {
        st = this.dataN(ROW, bytes);
        if (st != 0) return st;
      }
    }
    return this.end();
  }
}
