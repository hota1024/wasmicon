// SHT40（SHT4x）の読み出し。apps/README.md §1 が正。
//
// sensor-display-rs の sht4x.rs と同じ host call 列・同じ値を出す。
// 片方だけ変えると sensor_display_rs_and_as_agree が落ちる。

import { I2cBus, time } from "../../../bindings/assemblyscript/assembly/index";

/// I2C アドレス。
export const ADDRESS: u16 = 0x44;

/// 計測が終わるまでの待ち時間。データシートの高精度計測は最大 8.3 ms。
const MEASURE_MS: u32 = 10;

/// CRC-8。多項式 0x31、初期値 0xFF、反転なし。
export function crc8(data: Uint8Array, from: i32, len: i32): u8 {
  let crc: u8 = 0xff;
  for (let i = 0; i < len; i++) {
    crc ^= data[from + i];
    for (let b = 0; b < 8; b++) {
      crc = (crc & 0x80) != 0 ? <u8>((crc << 1) ^ 0x31) : <u8>(crc << 1);
    }
  }
  return crc;
}

/// 読み出しの結果。`ok` が false のときは `crcFailed` で理由を分ける。
export class Reading {
  ok: bool = false;
  crcFailed: bool = false;
  rawT: u16 = 0;
  rawH: u16 = 0;
}

const MEASURE = new Uint8Array(1);
const FRAME = new Uint8Array(6);

/// 1 回測って読む。
export function read(bus: I2cBus): Reading {
  const r = new Reading();
  // SHT4x は 1 バイトコマンド（高精度計測）。
  MEASURE[0] = 0xfd;
  if (bus.write(ADDRESS, MEASURE) != 0) return r;
  time.sleepMs(MEASURE_MS);

  if (bus.read(ADDRESS, FRAME) != 6) return r;
  if (crc8(FRAME, 0, 2) != FRAME[2] || crc8(FRAME, 3, 2) != FRAME[5]) {
    r.crcFailed = true;
    return r;
  }
  r.ok = true;
  r.rawT = <u16>((<u16>FRAME[0] << 8) | <u16>FRAME[1]);
  r.rawH = <u16>((<u16>FRAME[3] << 8) | <u16>FRAME[4]);
  return r;
}

/// 温度（摂氏 ×100）。固定小数で計算する。
export function tempCenti(rawT: u16): i32 {
  return -4500 + (17500 * <i32>rawT) / 65535;
}

/// 相対湿度（％ ×100）。
///
/// SHT4x の式は raw=0 で -600、raw=65535 で 11900 を返すので、データシートの
/// とおり 0..10000 に収める。min / max を使わず if で書くのは Rust 版と
/// 同じ命令列にするため（apps/README.md §1）。
export function humidityCenti(rawH: u16): i32 {
  let v = -600 + (12500 * <i32>rawH) / 65535;
  if (v < 0) v = 0;
  if (v > 10000) v = 10000;
  return v;
}
