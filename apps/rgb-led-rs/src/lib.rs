//! Lights the on-board RGB LED (WS2812B) of the ESP32-C5-WIFI6-KIT.
//!
//! The LED is on GPIO27, and the esp32c5 port maps `spi.bus` index 1 to a
//! bus that drives only the SPI2 MOSI on GPIO27 (docs/abi-spec.md §8). A
//! WS2812 encodes each bit as the length of a High pulse (0 = short,
//! 1 = long), so each bit is expanded to 3 SPI bits:
//!
//! ```text
//! WS2812 0 -> SPI 100 (High about 0.42 us, Low about 0.85 us)
//! WS2812 1 -> SPI 110 (High about 0.85 us, Low about 0.42 us)
//! ```
//!
//! At about 2.4 MHz (about 0.42 us per bit) this meets the WS2812B datasheet
//! timing, T0H 0.4 us / T1H 0.8 us (both +/-0.15 us). Toggling a gpio
//! through host calls costs about 10 us per call, far too slow for these
//! pulses.
//!
//! Shows red, green, blue and white in turn, then cycles the hue through a
//! rainbow, then turns the LED off.

#![no_std]

use wasmicon_hal::spi::{Bus, Mode};
use wasmicon_hal::{log, time};

/// SPI bus wired to the RGB LED (chosen by the esp32c5 port,
/// docs/abi-spec.md §8).
const LED_BUS: u32 = 1;

/// SPI clock. The port rounds down to the fastest value it can produce
/// (2.35-2.4 MHz).
const SPI_HZ: u32 = 2_400_000;

/// Brightness cap (0..=255). A WS2812B at full power is painful to look at.
const BRIGHTNESS: u8 = 32;

/// Steps per hue cycle, and the time per step.
const HUE_STEPS: u32 = 96;
const STEP_MS: u32 = 20;

/// Number of hue cycles.
const ROUNDS: u32 = 5;

/// Zero bytes sent first. A Low of 50 us or longer resets the WS2812 so it
/// accepts the next color; 16 bytes = 128 bits, about 54 us.
const RESET_BYTES: usize = 16;

/// One color (GRB, 24 bits) expanded to 72 SPI bits.
const COLOR_BYTES: usize = 9;

#[unsafe(no_mangle)]
pub extern "C" fn run() {
    log::info("rgb-led start");

    let Ok(bus) = Bus::open(LED_BUS, SPI_HZ, Mode::Mode0) else {
        log::error("cannot open spi.bus index 1 (the RGB LED exists only on esp32c5)");
        return;
    };

    for (r, g, b) in [
        (BRIGHTNESS, 0, 0),
        (0, BRIGHTNESS, 0),
        (0, 0, BRIGHTNESS),
        (BRIGHTNESS, BRIGHTNESS, BRIGHTNESS),
    ] {
        if show(&bus, r, g, b).is_err() {
            return;
        }
        time::sleep_ms(500);
    }

    let mut round = 0;
    while round < ROUNDS {
        let mut step = 0;
        while step < HUE_STEPS {
            let (r, g, b) = hue(step);
            if show(&bus, r, g, b).is_err() {
                return;
            }
            time::sleep_ms(STEP_MS);
            step += 1;
        }
        round += 1;
    }

    // Turn it off. A WS2812 keeps the last color it received, so without
    // this it stays lit.
    let _ = show(&bus, 0, 0, 0);
    log::info("rgb-led done");
    // `bus` is dropped here, which calls [resource-drop]bus.
}

/// Sends one color.
fn show(bus: &Bus, r: u8, g: u8, b: u8) -> Result<(), ()> {
    let mut frame = [0u8; RESET_BYTES + COLOR_BYTES];
    encode([g, r, b], &mut frame[RESET_BYTES..]);
    bus.write(&frame)
        .map_err(|_| log::error("spi.write failed"))
}

/// Expands 3 GRB bytes, MSB first, with one WS2812 bit becoming 3 SPI bits.
fn encode(grb: [u8; 3], out: &mut [u8]) {
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut o = 0;
    for byte in grb {
        let mut i = 8;
        while i > 0 {
            i -= 1;
            let pattern = if (byte >> i) & 1 == 1 { 0b110 } else { 0b100 };
            acc = (acc << 3) | pattern;
            bits += 3;
            while bits >= 8 {
                bits -= 8;
                out[o] = (acc >> bits) as u8;
                o += 1;
            }
            // Drop the high bits already emitted so the accumulator cannot
            // overflow.
            acc &= (1 << bits) - 1;
        }
    }
}

/// Converts a hue (0..HUE_STEPS) to RGB, with brightness up to `BRIGHTNESS`.
fn hue(step: u32) -> (u8, u8, u8) {
    // Six segments; within each, one channel ramps 0 -> max or max -> 0.
    let seg_len = HUE_STEPS / 6;
    let seg = step / seg_len;
    let pos = step % seg_len;
    let max = u32::from(BRIGHTNESS);
    let up = (max * pos / seg_len) as u8;
    let down = (max - max * pos / seg_len) as u8;
    let m = BRIGHTNESS;
    match seg {
        0 => (m, up, 0),
        1 => (down, m, 0),
        2 => (0, m, up),
        3 => (0, down, m),
        4 => (up, 0, m),
        _ => (m, 0, down),
    }
}

/// Required because the crate is `no_std`. Traps (docs/decisions.md A-04).
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_bit_is_100_and_a_one_bit_is_110() {
        // 0x80 0x00 0x00 -> 1 0000000 0...: 110 100 100 100 ...
        let mut out = [0u8; COLOR_BYTES];
        encode([0x80, 0, 0], &mut out);
        // 110100100100100100100100 = 0xD2 0x49 0x24; the rest repeats 100.
        assert_eq!(out, [0xD2, 0x49, 0x24, 0x92, 0x49, 0x24, 0x92, 0x49, 0x24]);
        let mut all = [0u8; COLOR_BYTES];
        encode([0xff, 0xff, 0xff], &mut all);
        assert_eq!(all, [0xDB, 0x6D, 0xB6, 0xDB, 0x6D, 0xB6, 0xDB, 0x6D, 0xB6]);
    }

    #[test]
    fn the_hue_wheel_stays_within_brightness_and_starts_red() {
        assert_eq!(hue(0), (BRIGHTNESS, 0, 0));
        for s in 0..HUE_STEPS {
            let (r, g, b) = hue(s);
            assert!(r <= BRIGHTNESS && g <= BRIGHTNESS && b <= BRIGHTNESS);
        }
    }
}
