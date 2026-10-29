//! The in-package PSRAM: brought up once and handed out as one slice.
//!
//! The plain build brings it up only for the Tab5 panel's frame buffer
//! (`tab5::Pending::new`). The `psram` build brings it up at boot and puts
//! the arena at its start; the panel gets what follows (`main`).
//!
//! On v1.0 silicon this must come before DSI: esp-hal's PSRAM init rewrites
//! every external LDO and leaves the one that powers the D-PHY off
//! (docs/verification-report.md §28).

use esp_hal::peripherals::PSRAM;
use esp_hal::psram::{Psram, PsramConfig, PsramMode, PsramSize, PsramTimingParams};

/// Maps all of PSRAM and returns it, or `None` when nothing was mapped
/// (esp-hal maps nothing when it cannot initialise PSRAM).
pub fn up(peri: PSRAM<'static>) -> Option<&'static mut [u8]> {
    let psram = Psram::new(
        peri,
        PsramConfig {
            mode: PsramMode::Hex,
            size: PsramSize::AutoDetect,
            // esp-hal clocks PSRAM from a 500 MHz MPLL, which 200 MHz does
            // not divide (docs/verification-report.md §28).
            timing: PsramTimingParams::MHZ_125,
        },
    );
    let (ptr, len) = psram.raw_parts();
    // Never dropped, so the mapping stays whatever `Drop` a later esp-hal
    // gives `Psram`. It has none today, which is what the lint points out.
    #[allow(clippy::forget_non_drop)]
    core::mem::forget(psram);
    if ptr.is_null() || len == 0 {
        return None;
    }
    // SAFETY: `Psram::new` mapped `len` bytes of PSRAM at `ptr` into the
    // data bus, and the `Psram` that owns the mapping is forgotten above, so
    // the mapping lasts for good. `peri` is the only PSRAM
    // peripheral and is consumed here, so this runs at most once, and the
    // firmware has no allocator: the slice is the only reference to the range.
    Some(unsafe { core::slice::from_raw_parts_mut(ptr, len) })
}
