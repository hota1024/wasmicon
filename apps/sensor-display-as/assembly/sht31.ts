// SHT31 / SHT30 の読み出し。apps/README.md §1 が正。

import { I2cBus, time } from "../../../bindings/assemblyscript/assembly/index";

/// I2C アドレス。
export const ADDRESS: u16 = 0x44;

/// 計測が終わるまでの待ち時間。
const MEASURE_MS: u32 = 15;

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

const MEASURE = new Uint8Array(2);
const FRAME = new Uint8Array(6);

/// 1 回測って読む。
export function read(bus: I2cBus): Reading {
  const r = new Reading();
  MEASURE[0] = 0x24;
  MEASURE[1] = 0x00;
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
export function humidityCenti(rawH: u16): i32 {
  return (10000 * <i32>rawH) / 65535;
}
