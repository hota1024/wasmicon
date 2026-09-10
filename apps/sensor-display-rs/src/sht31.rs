//! SHT31 / SHT30 の読み出し。`apps/README.md` §1 が正。

use wasmicon_hal::i2c::Bus;
use wasmicon_hal::time;

/// I2C アドレス。
pub const ADDRESS: u16 = 0x44;

/// 単発計測（高精度・クロックストレッチなし）。
const MEASURE: [u8; 2] = [0x24, 0x00];

/// 計測が終わるまでの待ち時間。
const MEASURE_MS: u32 = 15;

/// CRC-8。多項式 0x31、初期値 0xFF、反転なし。
#[must_use]
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0xFF;
    for &b in data {
        crc ^= b;
        let mut i = 0;
        while i < 8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x31
            } else {
                crc << 1
            };
            i += 1;
        }
    }
    crc
}

/// 生の測定値。
pub struct Reading {
    pub raw_t: u16,
    pub raw_h: u16,
}

/// 読み出しの失敗。
pub enum Error {
    /// 通信できない。
    Io,
    /// CRC が合わない。
    Crc,
}

/// 1 回測って読む。
///
/// # Errors
/// 通信に失敗したら `Io`、CRC が合わなければ `Crc`。
pub fn read(bus: &Bus) -> Result<Reading, Error> {
    if bus.write(ADDRESS, &MEASURE).is_err() {
        return Err(Error::Io);
    }
    time::sleep_ms(MEASURE_MS);

    let mut buf = [0u8; 6];
    match bus.read(ADDRESS, &mut buf) {
        Ok(6) => {}
        _ => return Err(Error::Io),
    }
    if crc8(&buf[0..2]) != buf[2] || crc8(&buf[3..5]) != buf[5] {
        return Err(Error::Crc);
    }
    Ok(Reading {
        raw_t: (u16::from(buf[0]) << 8) | u16::from(buf[1]),
        raw_h: (u16::from(buf[3]) << 8) | u16::from(buf[4]),
    })
}

/// 温度（摂氏 ×100）。固定小数で計算する。
#[must_use]
pub fn temp_centi(raw_t: u16) -> i32 {
    -4500 + (17500 * i32::from(raw_t)) / 65535
}

/// 相対湿度（％ ×100）。
#[must_use]
pub fn humidity_centi(raw_h: u16) -> i32 {
    (10000 * i32::from(raw_h)) / 65535
}
