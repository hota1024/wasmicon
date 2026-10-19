// Reads an SHT4x (SHT40) and shows the values on an ILI9341 (AssemblyScript).
//
// apps/README.md is normative for the drawing. This app must produce the
// same host-call sequence and the same pixels as sensor-display-rs.

import {
  I2cBus,
  SpiMode,
  Pin,
  PinMode,
  SpiBus,
  Speed,
  board,
  log,
} from "../../../bindings/assemblyscript/assembly/index";
import { Display, HEIGHT, MAX_TEXT, WIDTH } from "./ili9341";
import { humidityCenti, read, tempCenti } from "./sht4x";

/// Background.
const BG: u16 = 0x0841;
/// Header band.
const HEADER: u16 = 0x1a3f;
/// Card background.
const CARD: u16 = 0x2104;
/// Text of the values and the header.
const FG: u16 = 0xffff;
/// Labels and tick marks.
const MUTED: u16 = 0xa514;
/// Accent of the temperature card.
const TEMP_ACCENT: u16 = 0xfc00;
/// Accent of the humidity card.
const HUM_ACCENT: u16 = 0x07ff;
/// Gauge track.
const TRACK: u16 = 0x4208;

/// Card positions and size (screen layout in apps/README.md §2).
const TEMP_X: u16 = 4;
const HUM_X: u16 = 162;
const CARD_Y: u16 = 36;
const CARD_W: u16 = 154;
const CARD_H: u16 = 116;
/// Scale and y of the value text.
const VALUE_SCALE: u16 = 3;
const VALUE_Y: u16 = 84;

/// Gauge. barPx (0..=308) is the fill width in pixels.
const GAUGE_X: u16 = 6;
const GAUGE_Y: u16 = 178;
const GAUGE_H: u16 = 16;
/// Color zones of the gauge (end on the barPx axis, color):
/// -45..10 / ..30 / ..50 / ..175 °C.
const ZONE_END: StaticArray<u16> = [77, 105, 133, 308];
const ZONE_COLOR: StaticArray<u16> = [0x041f, 0x07e0, 0xffe0, 0xf800];
/// Tick marks (positions on the barPx axis): 0 / 25 / 50 / 100 °C.
const TICKS: StaticArray<u16> = [63, 98, 133, 203];

/// SPI clock.
const SPI_HZ: u32 = 24000000;

/// Holds the text being drawn.
const LINE = new Uint8Array(MAX_TEXT);

/// Entry point. The host calls it once, right after instantiation
/// (docs/abi-spec.md §3.3).
///
/// AssemblyScript has no `Drop`, so `run` has a single exit that calls
/// `close()` explicitly, in the order apps/README.md §4 requires: `i2c`,
/// `spi`, `rst`, `dc`, `cs`, skipping what was never opened. This matches
/// the Rust version's `Drop` (reverse declaration order).
export function run(): void {
  log.info("sensor-display start");

  const csIndex = board.pinByRole("lcd-cs");
  const dcIndex = board.pinByRole("lcd-dc");
  const rstIndex = board.pinByRole("lcd-rst");
  if (csIndex < 0 || dcIndex < 0 || rstIndex < 0) {
    log.error("role not found");
    return;
  }

  // Open the pins one at a time. Opening them all before checking would
  // call all three even after a failure, and the host-call sequence would
  // differ from the Rust version.
  let cs: Pin | null = null;
  let dc: Pin | null = null;
  let rst: Pin | null = null;
  let spi: SpiBus | null = null;
  let i2c: I2cBus | null = null;
  let err: string | null = null;

  cs = Pin.open(<u32>csIndex, PinMode.Output);
  if (cs == null) {
    err = "gpio open failed";
  } else {
    dc = Pin.open(<u32>dcIndex, PinMode.Output);
    if (dc == null) {
      err = "gpio open failed";
    } else {
      rst = Pin.open(<u32>rstIndex, PinMode.Output);
      if (rst == null) {
        err = "gpio open failed";
      } else {
        spi = SpiBus.open(0, SPI_HZ, SpiMode.Mode0);
        if (spi == null) {
          err = "spi open failed";
        } else {
          i2c = I2cBus.open(0, Speed.Standard);
          if (i2c == null) {
            err = "i2c open failed";
          } else {
            err = draw(spi, cs, dc, rst, i2c);
          }
        }
      }
    }
  }

  // The Rust version logs before dropping; keep that order.
  if (err != null) log.error(err);

  if (i2c != null) i2c.close();
  if (spi != null) spi.close();
  if (rst != null) rst.close();
  if (dc != null) dc.close();
  if (cs != null) cs.close();
}

/// Init, sensor read and drawing. Returns the error message on failure.
function draw(spi: SpiBus, cs: Pin, dc: Pin, rst: Pin, i2c: I2cBus): string | null {
  const display = new Display(spi, cs, dc);
  if (display.init(rst) != 0) return "display failed";
  if (drawFrame(display) != 0) return "display failed";

  const reading = read(i2c);
  if (!reading.ok) {
    return reading.crcFailed ? "sensor crc failed" : "sensor read failed";
  }

  const temp = tempCenti(reading.rawT);
  const humidity = humidityCenti(reading.rawH);
  if (drawValues(display, temp, humidity) != 0) return "display failed";
  return null;
}

/// Copies an ASCII string into LINE and returns its length.
function put(s: string): i32 {
  for (let i = 0; i < s.length; i++) LINE[i] = <u8>s.charCodeAt(i);
  return s.length;
}

/// Draws a string. 0 on success.
function text(d: Display, x: u16, y: u16, s: string, scale: u16, fg: u16, bg: u16): u32 {
  return d.drawText(x, y, LINE, put(s), scale, fg, bg);
}

/// The part drawn before the sensor is read: header, cards, gauge track and
/// tick marks. 0 on success.
function drawFrame(d: Display): u32 {
  let st = d.fillRect(0, 0, WIDTH, HEIGHT, BG);
  if (st != 0) return st;

  st = d.fillRect(0, 0, WIDTH, 28, HEADER);
  if (st != 0) return st;
  st = text(d, 8, 6, "WASMICON", 2, FG, HEADER);
  if (st != 0) return st;
  st = text(d, 272, 10, "SHT40", 1, FG, HEADER);
  if (st != 0) return st;

  st = drawCard(d, TEMP_X, "TEMP", "C", TEMP_ACCENT);
  if (st != 0) return st;
  st = drawCard(d, HUM_X, "HUMIDITY", "%", HUM_ACCENT);
  if (st != 0) return st;

  st = d.fillRect(4, 162, 312, 72, CARD);
  if (st != 0) return st;
  st = d.fillRect(GAUGE_X, GAUGE_Y, 308, GAUGE_H, TRACK);
  if (st != 0) return st;
  for (let i = 0; i < TICKS.length; i++) {
    st = d.fillRect(GAUGE_X + TICKS[i], 196, 1, 5, MUTED);
    if (st != 0) return st;
  }
  st = text(d, 6, 206, "-45", 1, MUTED, CARD);
  if (st != 0) return st;
  st = text(d, 65, 206, "0", 1, MUTED, CARD);
  if (st != 0) return st;
  st = text(d, 131, 206, "50", 1, MUTED, CARD);
  if (st != 0) return st;
  st = text(d, 197, 206, "100", 1, MUTED, CARD);
  if (st != 0) return st;
  return text(d, 282, 206, "175C", 1, MUTED, CARD);
}

/// One card: background, accent strip at the top, label, unit. 0 on success.
function drawCard(d: Display, x: u16, label: string, unit: string, accent: u16): u32 {
  let st = d.fillRect(x, CARD_Y, CARD_W, CARD_H, CARD);
  if (st != 0) return st;
  st = d.fillRect(x, CARD_Y, CARD_W, 4, accent);
  if (st != 0) return st;
  st = text(d, x + 8, 48, label, 1, MUTED, CARD);
  if (st != 0) return st;
  return text(d, x + CARD_W - 24, 46, unit, 2, accent, CARD);
}

/// The values read: the numbers on the cards and the gauge fill. 0 on success.
function drawValues(d: Display, temp: i32, humidity: i32): u32 {
  let st = drawValue(d, TEMP_X, formatValue(temp));
  if (st != 0) return st;
  st = drawValue(d, HUM_X, formatValue(humidity));
  if (st != 0) return st;

  // For each zone, fill only its overlap with [0, bar).
  const bar = <u16>barPx(temp);
  let start: u16 = 0;
  for (let i = 0; i < ZONE_END.length; i++) {
    const end = ZONE_END[i];
    if (bar > start) {
      const stop: u16 = bar < end ? bar : end;
      st = d.fillRect(GAUGE_X + start, GAUGE_Y, stop - start, GAUGE_H, ZONE_COLOR[i]);
      if (st != 0) return st;
    }
    start = end;
  }
  return 0;
}

/// Draws the value already formatted into LINE, centered horizontally in its
/// card. 0 on success.
function drawValue(d: Display, cardX: u16, n: i32): u32 {
  const w = <u16>n * 8 * VALUE_SCALE;
  const x: u16 = cardX + (CARD_W - w) / 2;
  return d.drawText(x, VALUE_Y, LINE, n, VALUE_SCALE, FG, CARD);
}

/// Length of the temperature bar. The only place in this app that uses f32
/// (apps/README.md §1).
///
/// The clamp is done in f32. Narrowing an out-of-range value to i32 would
/// behave differently in Rust (saturating conversion) and AssemblyScript
/// (trapping conversion).
function barPx(tempCentiValue: i32): i32 {
  const t: f32 = <f32>tempCentiValue / 100.0;
  let b: f32 = (t + 45.0) * 1.4;
  if (b < 0.0) b = 0.0;
  if (b > 308.0) b = 308.0;
  return <i32>b;
}

/// Formats a centi value into LINE as `23.44` / `-5.07`. Returns the number
/// of bytes written.
function formatValue(centi: i32): i32 {
  let n = 0;
  const neg = centi < 0;
  const v = neg ? -centi : centi;
  if (neg) LINE[n++] = 0x2d; // '-'

  const int = v / 100;
  const frac = v % 100;

  const digits = new Uint8Array(10);
  let d = 0;
  let x = int;
  do {
    digits[d++] = <u8>(0x30 + (x % 10));
    x = x / 10;
  } while (x != 0);
  while (d > 0) {
    LINE[n++] = digits[--d];
  }

  LINE[n++] = 0x2e; // '.'
  LINE[n++] = <u8>(0x30 + frac / 10);
  LINE[n++] = <u8>(0x30 + (frac % 10));
  return n;
}
