//! SHT4x（SHT40 / SHT41 / SHT45）の読み出し。`apps/README.md` §1 が正。

use wasmicon_hal::i2c::Bus;
use wasmicon_hal::time;

/// I2C アドレス。**サフィックスで変わる**（-AD1B が 0x44）ので、
/// 変えるなら 3 箇所: ここ、`sensor-display-as` の `ADDRESS`、
/// `ports/host/tests/apps.rs` の `SHT4X_ADDR`（トレースの期待値）。
pub const ADDRESS: u16 = 0x44;

/// 単発計測（高精度）。SHT4x のコマンドは **1 バイト**（SHT3x は 2 バイト）。
const MEASURE: [u8; 1] = [0xFD];

/// 計測が終わるまでの待ち時間。SHT4x の高精度は最大 8.3 ms なので余裕がある。
/// SHT3x のときと同じ 15 ms のまま据え置いている。
///
/// **変えるなら AS 版と必ず同時に変える。** `time` は abi-spec §9 でトレース
/// 対象外なので、ここが 2 言語で食い違っても `sensor_display_rs_and_as_agree`
/// も `diff-traces.sh` も気付かない。短くしすぎると実機で変換前の値を読んで
/// CRC 不一致や古い値になるが、それも host では再現しない。
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
///
/// SHT4x は `-6 + 125·raw/65535` で、**SHT3x の `100·raw/65535` とは違う**。
/// 素の式は 0 未満・100 超に振れるのでデータシートどおりクランプする。
/// `12500 * 65535 = 819_187_500` は i32 に収まる。
///
/// clippy は `i32::clamp` を勧める。**整数演算なのでどちらでも値は同じ**で、
/// `bar_px` の f32 クランプのような決定性の問題は無い。AS 版が `if` 2 本で
/// 書かれているので、並べて読めるように構造だけ揃えてある（§2.1 の教訓で
/// 2 つのゲストが食い違ったことがある）。
#[allow(clippy::manual_clamp)]
#[must_use]
pub fn humidity_centi(raw_h: u16) -> i32 {
    let h = -600 + (12500 * i32::from(raw_h)) / 65535;
    if h < 0 {
        0
    } else if h > 10000 {
        10000
    } else {
        h
    }
}

#[cfg(test)]
mod tests {
    use super::{crc8, humidity_centi, temp_centi};

    /// `verify/sht4x-replay.txt`（実機の SHT40 から記録した応答）の**写し**。
    /// あのファイルを差し替えるときは、ここも一緒に直す
    /// （下の期待値は `FIXTURE` から導出しているので、片方だけ直すと落ちる）。
    const FIXTURE: [u8; 6] = [0x67, 0x54, 0xc0, 0xa4, 0xd8, 0x40];

    /// `FIXTURE` の生の測定値。`read` と同じ組み立て方をする。
    fn fixture_raw() -> (u16, u16) {
        (
            (u16::from(FIXTURE[0]) << 8) | u16::from(FIXTURE[1]),
            (u16::from(FIXTURE[3]) << 8) | u16::from(FIXTURE[4]),
        )
    }

    /// 応答の形式と CRC-8 は SHT3x と SHT4x で同一。
    /// センサーを替えてもここは変わっていない。
    #[test]
    fn fixture_crc_is_valid() {
        assert_eq!(crc8(&FIXTURE[0..2]), FIXTURE[2]);
        assert_eq!(crc8(&FIXTURE[3..5]), FIXTURE[5]);
    }

    /// 温度の式は SHT3x と SHT4x で同一（`-45 + 175·raw/65535`）。
    /// **この値が動くと `bar_px` の f32 の題材も変わる**ので釘を打っておく。
    #[test]
    fn temperature_matches_fixture() {
        assert_eq!(temp_centi(fixture_raw().0), 2563); // 25.63 °C
        assert_eq!(temp_centi(0), -4500); // 下端 -45.00 °C
        assert_eq!(temp_centi(65535), 13000); // 上端 130.00 °C
    }

    /// 湿度は SHT4x で式が変わった（SHT3x は `100·raw/65535` で 6439 になる）。
    #[test]
    fn humidity_uses_the_sht4x_formula() {
        // SHT3x の式なら 6439 になる値。
        assert_eq!(humidity_centi(fixture_raw().1), 7449); // 74.49 %
    }

    /// 素の式は 0 未満・100 超に振れるので、データシートどおりクランプする。
    #[test]
    fn humidity_is_clamped_to_the_datasheet_range() {
        assert_eq!(humidity_centi(0), 0); // 素の式では -600
        assert_eq!(humidity_centi(3145), 0); // 素の式では -1
        assert_eq!(humidity_centi(65535), 10000); // 素の式では 11900
        // クランプの境目のすぐ上は通す（一律 0 にしていない）。
        assert_eq!(humidity_centi(3151), 1);
    }
}
