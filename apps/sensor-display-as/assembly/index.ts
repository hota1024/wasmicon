// SHT31 を読んで ILI9341 に表示する（AssemblyScript）。
//
// 描画の詳細は apps/README.md が正。sensor-display-rs と同じ
// host call 列・同じピクセル出力を出さなければならない。

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
import { humidityCenti, read, tempCenti } from "./sht31";

/// 背景色（黒）。
const BG: u16 = 0x0000;
/// 文字色（白）。
const FG: u16 = 0xffff;
/// バーの色（赤）。
const BAR: u16 = 0xf800;

/// SPI のクロック。
const SPI_HZ: u32 = 24000000;

/// 整形した文字列を置く。
const LINE = new Uint8Array(MAX_TEXT);

/// エントリポイント。ホストがインスタンス化直後に 1 回呼ぶ（abi-spec §3.3）。
///
/// AssemblyScript には `Drop` が無いので、出口を 1 箇所にまとめて明示的に
/// `close()` を呼ぶ。順序は `apps/README.md` §4 が定める
/// `i2c` → `spi` → `rst` → `dc` → `cs`。開けていないものは飛ばす。
/// Rust 版の `Drop`（宣言の逆順）と同じ順になる。
export function run(): void {
  log.info("sensor-display start");

  const csIndex = board.pinByRole("lcd-cs");
  const dcIndex = board.pinByRole("lcd-dc");
  const rstIndex = board.pinByRole("lcd-rst");
  if (csIndex < 0 || dcIndex < 0 || rstIndex < 0) {
    log.error("role not found");
    return;
  }

  // 1 本ずつ開ける。まとめて開けると失敗しても全部呼んでしまい、
  // Rust 版と host call 列が食い違う。
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

  // Rust 版は log してから Drop する。順序を合わせる。
  if (err != null) log.error(err);

  if (i2c != null) i2c.close();
  if (spi != null) spi.close();
  if (rst != null) rst.close();
  if (dc != null) dc.close();
  if (cs != null) cs.close();
}

/// 初期化・センサー読み・描画。失敗したらメッセージを返す。
function draw(spi: SpiBus, cs: Pin, dc: Pin, rst: Pin, i2c: I2cBus): string | null {
  const display = new Display(spi, cs, dc);
  if (display.init(rst) != 0) return "display failed";
  if (display.fillRect(0, 0, WIDTH, HEIGHT, BG) != 0) return "display failed";

  const reading = read(i2c);
  if (!reading.ok) {
    return reading.crcFailed ? "sensor crc failed" : "sensor read failed";
  }

  const temp = tempCenti(reading.rawT);
  const humidity = humidityCenti(reading.rawH);

  let n = formatRow(0x54, temp, 0x43); // 'T' ... 'C'
  if (display.drawText(8, 40, LINE, n, FG, BG) != 0) return "display failed";
  n = formatRow(0x48, humidity, 0x25); // 'H' ... '%'
  if (display.drawText(8, 60, LINE, n, FG, BG) != 0) return "display failed";

  const bar = barPx(temp);
  if (bar > 0 && display.fillRect(8, 80, <u16>bar, 8, BAR) != 0) return "display failed";
  return null;
}

/// 温度バーの長さ。**このアプリで唯一 f32 を使う場所**（apps/README.md §1）。
///
/// クランプは f32 のまま行う。範囲外のまま i32 に落とすと、Rust の飽和変換と
/// AssemblyScript のトラップ変換で挙動が分かれる。
function barPx(tempCentiValue: i32): i32 {
  const t: f32 = <f32>tempCentiValue / 100.0;
  let b: f32 = (t + 45.0) * 1.4;
  if (b < 0.0) b = 0.0;
  if (b > 308.0) b = 308.0;
  return <i32>b;
}

/// `T 23.44C` の形に整形する。書けた長さを返す。
function formatRow(label: u8, centi: i32, unit: u8): i32 {
  let n = 0;
  LINE[n++] = label;
  LINE[n++] = 0x20; // ' '

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
  LINE[n++] = unit;
  return n;
}
