//! Assets: reads two assets the developer ships next to the app
//! (docs/app-workflow.md §3.10).
//!
//! `hello.txt` is read whole and logged as text. `pattern.bin` is read in
//! chunks and checked against the formula that generated it, so the trace and
//! the log together show that every byte arrives intact on every board.

#![no_std]

use wasmicon_hal::assets::Blob;
use wasmicon_hal::log;

/// Bytes of `pattern.bin` read per host call. Smaller than the asset, so the
/// reads cross a 4 KiB boundary of the slot.
const CHUNK: usize = 1024;

/// `pattern.bin` is `PATTERN_LEN` bytes of `(i * 7) as u8`
/// (`apps/assets-rs/assets/`).
const PATTERN_LEN: u32 = 4100;

/// Entry point. The host calls it once, right after instantiation
/// (docs/abi-spec.md §3.3).
#[unsafe(no_mangle)]
pub extern "C" fn run() {
    log::info("assets start");

    let Ok(hello) = Blob::open("hello.txt") else {
        log::error("no asset hello.txt");
        return;
    };
    let mut text = [0u8; 64];
    let Ok(len) = hello.len() else {
        log::error("len failed");
        return;
    };
    let len = len as usize;
    if len > text.len() || hello.read(0, &mut text[..len]).is_err() {
        log::error("cannot read hello.txt");
        return;
    }
    match core::str::from_utf8(&text[..len]) {
        Ok(s) => log::info(s),
        Err(_) => log::error("hello.txt is not UTF-8"),
    }
    // Released here, so pattern.bin gets the same handle.
    drop(hello);

    let Ok(pattern) = Blob::open("pattern.bin") else {
        log::error("no asset pattern.bin");
        return;
    };
    if pattern.len() != Ok(PATTERN_LEN) {
        log::error("pattern.bin has the wrong length");
        return;
    }
    let mut buf = [0u8; CHUNK];
    let mut at: u32 = 0;
    let mut ok = true;
    while at < PATTERN_LEN {
        let n = (PATTERN_LEN - at).min(CHUNK as u32) as usize;
        if pattern.read(at, &mut buf[..n]).is_err() {
            log::error("cannot read pattern.bin");
            return;
        }
        for (i, b) in buf[..n].iter().enumerate() {
            if *b != ((at as usize + i) * 7) as u8 {
                ok = false;
            }
        }
        at += n as u32;
    }
    log::info(if ok { "pattern ok" } else { "pattern mismatch" });
    // `pattern` is dropped here, which calls [resource-drop]blob.
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
