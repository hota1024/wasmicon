// Minimal ILI9341 driver. apps/README.md §2 is normative.
// Sends the same bytes as the Rust version (apps/sensor-display-rs/src/ili9341.rs).

import { Level, Pin, SpiBus, time } from "../../../bindings/assemblyscript/assembly/index";
import { GLYPHS, indexOf } from "./font";

export const WIDTH: u16 = 320;
export const HEIGHT: u16 = 240;

/// Longest text `drawText` sends at once (8 px x 40 = the screen width).
export const MAX_TEXT: i32 = 40;

/// One row of pixels (320 px x 2 bytes).
const ROW = new Uint8Array(<i32>WIDTH * 2);
/// Returned for an invalid argument: the status of ErrorCode.InvalidArgument
/// (discriminant + 1).
const ERR_INVALID: u32 = 1;

/// Returns whether `start + len` fits in `limit`. Widens to u32 before adding.
///
/// AssemblyScript narrows a u16 + u16 sum to u16 even inside a comparison (it
/// emits `i32.and 0xffff` after `i32.add`), so adding in u16 wraps and an
/// off-screen coordinate would then pass the bounds check. The inputs that
/// slip through have `start + len` in 65536..65856. There are two outcomes,
/// for different input ranges:
///
/// - `fillRect(65530, 0, 10, 1, c)`: `65530 + 10 == 4` passes, and `window`
///   sends an inverted rectangle (`x0 > x1`). The buffer is not exceeded
///   (`n == 20`).
/// - `fillRect(65216, 0, 640, 1, c)`: wraps to 320 and passes, and then
///   `n = w * 2 == 1280` exceeds the 640-byte `ROW`. Only this case is an
///   out-of-bounds write, and because `asconfig.json` sets `noAssert: true`
///   there is no bounds check: it silently corrupts linear memory (the Rust
///   side traps).
///
/// `drawText` computes the width and height in i32 and rejects anything
/// larger than the screen before narrowing to u16, so no wrapped value
/// reaches this function. Kept in the same shape as `in_bounds()` in the
/// Rust version (apps/sensor-display-rs/src/ili9341.rs).
function inBounds(start: u16, len: u16, limit: u16): bool {
  return <u32>start + <u32>len <= <u32>limit;
}

/// One command byte.
const CMD = new Uint8Array(1);
/// Command arguments (at most 4 bytes).
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

  /// Sends a command and its arguments within one CS assertion. 0 on success.
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

  /// Resets the panel and sends the init sequence. 0 on success.
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
    st = this.send(0x36, 1); // MADCTL = landscape
    if (st != 0) return st;
    return this.send(0x29, 0); // DISPON
  }

  /// Sets the rectangle that the next RAMWR writes to.
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

  /// Fills a rectangle with one color, sending it row by row. 0 on success.
  fillRect(x: u16, y: u16, w: u16, h: u16, color: u16): u32 {
    if (w == 0 || h == 0) return 0;
    // Do not draw off screen (apps/README.md §2). This also keeps the write
    // within ROW.
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

  /// Draws text in the monospace 8x8 font scaled by `scale`. 0 on success.
  ///
  /// Builds one scaled pixel row per font row and sends it `scale` times,
  /// so one screen row (ROW) is enough buffer.
  drawText(x: u16, y: u16, text: Uint8Array, len: i32, scale: u16, fg: u16, bg: u16): u32 {
    if (len == 0) return 0;
    // Reject a negative len. The Rust version takes `&[u8]`, so this hole
    // cannot be expressed there; it existed only on the AS side. With
    // `len == -8192`, `<u16>(n * 8) == 0` passes inBounds, `window` sends
    // `x1 = x - 1`, and `dataN` is called with a negative length (nothing
    // stops it under `noAssert: true`). The current callers only count up
    // from 0, so this is not reached today.
    if (len < 0) return ERR_INVALID;
    // Never truncate silently (apps/README.md §2).
    if (len > MAX_TEXT || scale == 0) return ERR_INVALID;
    const n = len;
    const s = <i32>scale;
    // Compare against the screen before narrowing to u16 (see inBounds).
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
