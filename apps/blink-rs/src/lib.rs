//! Lチカ。Wasmicon の最小ゲスト。
//!
//! LED のピン番号は `board.pin-by-role` で引くので、同一の `.wasm` が
//! ESP32-S3 と RP2040 の両方で動く（abi-spec §8、docs/handoff.md §3 #2）。

#![no_std]

use wasmicon_hal::gpio::{Level, Pin, PinMode};
use wasmicon_hal::{board, log, time};

/// 点滅回数。
const BLINKS: u32 = 3;

/// 点灯・消灯の間隔（ミリ秒）。
const INTERVAL_MS: u32 = 1;

/// エントリポイント。ホストがインスタンス化直後に 1 回呼ぶ（abi-spec §3.3）。
#[unsafe(no_mangle)]
pub extern "C" fn run() {
    log::info("blink start");

    // ピン番号はボードごとに違うので、役割名で引く。
    let Ok(index) = board::pin_by_role("led") else {
        log::error("led の割り当てが無い");
        return;
    };
    let Ok(pin) = Pin::open(index, PinMode::Output) else {
        log::error("led のピンを開けない");
        return;
    };

    let mut i = 0;
    while i < BLINKS {
        if pin.write(Level::High).is_err() {
            log::error("write に失敗");
            return;
        }
        time::sleep_ms(INTERVAL_MS);
        if pin.toggle().is_err() {
            log::error("toggle に失敗");
            return;
        }
        time::sleep_ms(INTERVAL_MS);
        i += 1;
    }
    // pin はここで Drop され、[resource-drop]pin が呼ばれる。
}

/// `no_std` なので自前で用意する。
/// トラップに落とし、ホストにログを出させて停止させる（docs/handoff.md §3 #4）。
///
/// `cfg(not(test))` なのは、`cargo clippy --all-targets` がテストハーネスを
/// 組むときに std の panic handler と衝突するため。
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
