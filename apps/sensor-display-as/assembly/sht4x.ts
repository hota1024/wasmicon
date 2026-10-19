// SHT4x (SHT40 / SHT41 / SHT45) readout. apps/README.md §1 is normative.

import { I2cBus, time } from "../../../bindings/assemblyscript/assembly/index";

/// I2C address. It depends on the part suffix (-AD1B is 0x44). Changing it
/// means changing three places: here, `ADDRESS` in `sensor-display-rs`, and
/// `SHT4X_ADDR` in `ports/host/tests/apps.rs` (the expected trace).
export const ADDRESS: u16 = 0x44;

/// Wait for the measurement to finish. The SHT4x high-precision measurement
/// takes at most 8.3 ms, so 15 ms (kept from the SHT3x days) leaves margin.
///
/// Change it together with the Rust version, always. `time` is excluded from
/// the trace (docs/abi-spec.md §9), so if the two languages disagree here,
/// the equality check does not notice.
const MEASURE_MS: u32 = 15;

/// CRC-8: polynomial 0x31, initial value 0xFF, no reflection.
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

/// Readout result. When `ok` is false, `crcFailed` tells the two failures
/// apart.
export class Reading {
  ok: bool = false;
  crcFailed: bool = false;
  rawT: u16 = 0;
  rawH: u16 = 0;
}

/// Single-shot measurement, high precision. SHT4x commands are one byte
/// (SHT3x commands are two).
const MEASURE = new Uint8Array(1);
const FRAME = new Uint8Array(6);

/// Takes one measurement and reads it back.
export function read(bus: I2cBus): Reading {
  const r = new Reading();
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

/// Temperature in centi-degrees Celsius, computed in fixed point.
export function tempCenti(rawT: u16): i32 {
  return -4500 + (17500 * <i32>rawT) / 65535;
}

/// Relative humidity in centi-percent.
///
/// SHT4x uses `-6 + 125 * raw / 65535`, which differs from the SHT3x
/// `100 * raw / 65535`. The raw formula can go below 0 and above 100, so it
/// is clamped as the datasheet says. Same order and same integer arithmetic
/// as the Rust version (apps/README.md §1).
export function humidityCenti(rawH: u16): i32 {
  const h = -600 + (12500 * <i32>rawH) / 65535;
  if (h < 0) return 0;
  if (h > 10000) return 10000;
  return h;
}
