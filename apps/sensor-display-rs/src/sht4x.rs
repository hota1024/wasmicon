//! SHT4x (SHT40 / SHT41 / SHT45) readout. apps/README.md §1 is normative.

use wasmicon_hal::i2c::Bus;
use wasmicon_hal::time;

/// I2C address. It depends on the part suffix (-AD1B is 0x44). Changing it
/// means changing three places: here, `ADDRESS` in `sensor-display-as`, and
/// `SHT4X_ADDR` in `ports/host/tests/apps.rs` (the expected trace).
pub const ADDRESS: u16 = 0x44;

/// Single-shot measurement, high precision. SHT4x commands are one byte
/// (SHT3x commands are two).
const MEASURE: [u8; 1] = [0xFD];

/// Wait for the measurement to finish. The SHT4x high-precision measurement
/// takes at most 8.3 ms, so 15 ms (kept from the SHT3x days) leaves margin.
///
/// Change it together with the AS version, always. `time` is excluded from
/// the trace (docs/abi-spec.md §9), so if the two languages disagree here,
/// neither `sensor_display_rs_and_as_agree` nor `diff-traces.sh` notices.
/// Too short a wait reads the value before conversion on hardware (CRC
/// mismatch or a stale value), and that does not reproduce on the host
/// either.
const MEASURE_MS: u32 = 15;

/// CRC-8: polynomial 0x31, initial value 0xFF, no reflection.
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

/// Raw measurement.
pub struct Reading {
    pub raw_t: u16,
    pub raw_h: u16,
}

/// Readout failure.
pub enum Error {
    /// The bus transfer failed.
    Io,
    /// The CRC did not match.
    Crc,
}

/// Takes one measurement and reads it back.
///
/// # Errors
/// `Io` if a transfer fails, `Crc` if a checksum does not match.
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

/// Temperature in centi-degrees Celsius, computed in fixed point.
#[must_use]
pub fn temp_centi(raw_t: u16) -> i32 {
    -4500 + (17500 * i32::from(raw_t)) / 65535
}

/// Relative humidity in centi-percent.
///
/// SHT4x uses `-6 + 125 * raw / 65535`, which differs from the SHT3x
/// `100 * raw / 65535`. The raw formula can go below 0 and above 100, so it
/// is clamped as the datasheet says. `12500 * 65535 = 819_187_500` fits in
/// i32.
///
/// clippy suggests `i32::clamp`. The value is the same either way because
/// this is integer arithmetic; there is no determinism concern like the f32
/// clamp in `bar_px`. The AS version is written as two `if`s, so the shape
/// is kept the same for side-by-side reading; copies of the same logic have
/// drifted apart before (docs/TODO.md §2.1).
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

    /// A copy of `verify/sht4x-replay.txt` (a response recorded from a real
    /// SHT40). When that file is replaced, update this too: the expected
    /// values below are derived from `FIXTURE`, so changing only one side
    /// fails.
    const FIXTURE: [u8; 6] = [0x67, 0x54, 0xc0, 0xa4, 0xd8, 0x40];

    /// Raw values of `FIXTURE`, assembled the same way as `read`.
    fn fixture_raw() -> (u16, u16) {
        (
            (u16::from(FIXTURE[0]) << 8) | u16::from(FIXTURE[1]),
            (u16::from(FIXTURE[3]) << 8) | u16::from(FIXTURE[4]),
        )
    }

    /// The response layout and the CRC-8 are the same on SHT3x and SHT4x.
    /// Switching sensors did not change this part.
    #[test]
    fn fixture_crc_is_valid() {
        assert_eq!(crc8(&FIXTURE[0..2]), FIXTURE[2]);
        assert_eq!(crc8(&FIXTURE[3..5]), FIXTURE[5]);
    }

    /// The temperature formula is the same on SHT3x and SHT4x
    /// (`-45 + 175 * raw / 65535`). Locked in because a change here also
    /// changes the f32 input of `bar_px`.
    #[test]
    fn temperature_matches_fixture() {
        assert_eq!(temp_centi(fixture_raw().0), 2563); // 25.63 °C
        assert_eq!(temp_centi(0), -4500); // lower end, -45.00 °C
        assert_eq!(temp_centi(65535), 13000); // upper end, 130.00 °C
    }

    /// The humidity formula changed with SHT4x (the SHT3x `100 * raw / 65535`
    /// would give 6439).
    #[test]
    fn humidity_uses_the_sht4x_formula() {
        // The SHT3x formula would give 6439 for this value.
        assert_eq!(humidity_centi(fixture_raw().1), 7449); // 74.49 %
    }

    /// The raw formula can go below 0 and above 100, so it is clamped as the
    /// datasheet says.
    #[test]
    fn humidity_is_clamped_to_the_datasheet_range() {
        assert_eq!(humidity_centi(0), 0); // raw formula: -600
        assert_eq!(humidity_centi(3145), 0); // raw formula: -1
        assert_eq!(humidity_centi(65535), 10000); // raw formula: 11900
        // Just above the clamp boundary passes through (not flattened to 0).
        assert_eq!(humidity_centi(3151), 1);
    }
}
