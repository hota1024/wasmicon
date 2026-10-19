//! Blink: the smallest Wasmicon guest.
//!
//! The LED pin number comes from `board.pin-by-role`, so the same `.wasm`
//! runs on both ESP32-S3 and RP2040 (docs/abi-spec.md §8,
//! docs/app-workflow.md §3.9).

#![no_std]

use wasmicon_hal::gpio::{Level, Pin, PinMode};
use wasmicon_hal::{board, log, time};

/// Number of blinks.
const BLINKS: u32 = 3;

/// On and off interval in milliseconds.
const INTERVAL_MS: u32 = 1;

/// Entry point. The host calls it once, right after instantiation
/// (docs/abi-spec.md §3.3).
#[unsafe(no_mangle)]
pub extern "C" fn run() {
    log::info("blink start");

    // The pin number differs per board, so look it up by role name.
    let Ok(index) = board::pin_by_role("led") else {
        log::error("no pin assigned to role led");
        return;
    };
    let Ok(pin) = Pin::open(index, PinMode::Output) else {
        log::error("cannot open led pin");
        return;
    };

    let mut i = 0;
    while i < BLINKS {
        if pin.write(Level::High).is_err() {
            log::error("write failed");
            return;
        }
        time::sleep_ms(INTERVAL_MS);
        if pin.toggle().is_err() {
            log::error("toggle failed");
            return;
        }
        time::sleep_ms(INTERVAL_MS);
        i += 1;
    }
    // `pin` is dropped here, which calls [resource-drop]pin.
}

/// Required because the crate is `no_std`. Traps so that the host logs the
/// reason and halts (docs/decisions.md A-04).
///
/// `cfg(not(test))` because `cargo clippy --all-targets` builds the test
/// harness, whose std panic handler would clash with this one.
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
