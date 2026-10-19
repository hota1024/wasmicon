//! ESP32-C5-WIFI6-KIT の基板の RGB LED（WS2812B）を光らせる。
//!
//! RGB LED は GPIO27 に繋がっていて、esp32c5 のポートは `spi.bus` の
//! index 1 を「SPI2 の MOSI だけを GPIO27 に出すバス」にしている
//! （abi-spec §8）。WS2812 は 1 ビットを High の長さで表す（0 = 短い、
//! 1 = 長い）ので、**1 ビットを SPI の 3 ビットに展開して**送る:
//!
//! ```text
//! WS2812 の 0 → SPI 100（High 約 0.42 µs、Low 約 0.85 µs）
//! WS2812 の 1 → SPI 110（High 約 0.85 µs、Low 約 0.42 µs）
//! ```
//!
//! SPI を約 2.4 MHz（1 ビット約 0.42 µs）で回すと、WS2812B のデータシートの
//! T0H 0.4 µs / T1H 0.8 µs（どちらも ±0.15 µs）に収まる。gpio を host call で
//! 振るのでは 1 回 10 µs かかるので、この幅は作れない。
//!
//! 赤・緑・青・白を順に出したあと、色相を回して虹色にし、最後に消す。

#![no_std]

use wasmicon_hal::spi::{Bus, Mode};
use wasmicon_hal::{log, time};

/// RGB LED に繋がった SPI バス（esp32c5 のポートが決める。abi-spec §8）。
const LED_BUS: u32 = 1;

/// SPI のクロック。ポートは要求以下で最も速い値に丸める（2.35〜2.4 MHz）。
const SPI_HZ: u32 = 2_400_000;

/// 明るさの上限（0..=255）。WS2812B は全開だと目に痛いので抑える。
const BRIGHTNESS: u8 = 32;

/// 虹色の 1 周の段数と、1 段の時間。
const HUE_STEPS: u32 = 96;
const STEP_MS: u32 = 20;

/// 虹色を何周するか。
const ROUNDS: u32 = 5;

/// 先頭に置く 0 のバイト数。WS2812 は Low が 50 µs 以上続くとリセット
/// （次の色の受け付け）になる。16 バイト = 128 ビット ≒ 54 µs。
const RESET_BYTES: usize = 16;

/// 1 色（GRB 24 ビット）を SPI の 72 ビットに展開した長さ。
const COLOR_BYTES: usize = 9;

#[unsafe(no_mangle)]
pub extern "C" fn run() {
    log::info("rgb-led start");

    let Ok(bus) = Bus::open(LED_BUS, SPI_HZ, Mode::Mode0) else {
        log::error("spi.bus index 1 を開けない（RGB LED は esp32c5 だけ）");
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

    // 消す。WS2812 は最後に受けた色を保つので、送らないと点いたままになる。
    let _ = show(&bus, 0, 0, 0);
    log::info("rgb-led done");
    // bus はここで Drop され、[resource-drop]bus が呼ばれる。
}

/// 1 色を送る。
fn show(bus: &Bus, r: u8, g: u8, b: u8) -> Result<(), ()> {
    let mut frame = [0u8; RESET_BYTES + COLOR_BYTES];
    encode([g, r, b], &mut frame[RESET_BYTES..]);
    bus.write(&frame)
        .map_err(|_| log::error("spi.write に失敗"))
}

/// GRB の 3 バイトを、WS2812 の 1 ビット = SPI の 3 ビットに展開する（MSB から）。
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
            // 出し終えた上位ビットを捨てる（あふれさせない）。
            acc &= (1 << bits) - 1;
        }
    }
}

/// 色相（0..HUE_STEPS）を RGB にする。明るさは `BRIGHTNESS` まで。
fn hue(step: u32) -> (u8, u8, u8) {
    // 6 区間に分け、区間の中で 1 色を 0 → max か max → 0 に動かす。
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

/// `no_std` なので自前で用意する。トラップに落とす（docs/handoff.md §3 #4）。
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
        // 0x80 0x00 0x00 → 1 0000000 0...: 110 100 100 100 ...
        let mut out = [0u8; COLOR_BYTES];
        encode([0x80, 0, 0], &mut out);
        // 110100100100100100100100 = 0xD2 0x49 0x24、残りは 100 の繰り返し。
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
