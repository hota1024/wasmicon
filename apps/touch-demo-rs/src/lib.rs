//! Draws where the touch panel is touched, through `wasmicon:device/display`
//! and `wasmicon:device/touch` (docs/device-layer.md §4, §11).
//!
//! The host owns both the panel and the touch controller, so this app has no
//! driver, no pins and no bus. Touch coordinates are display pixels after the
//! display's rotation, so the dots land under the finger on any display.
//!
//! - A bar along the top: CLEAR (gray, left) clears the canvas, DONE (red,
//!   right) ends the app.
//! - Below it, each touched point leaves a dot; the Nth point of a multi-touch
//!   read gets the Nth color.
//!
//! On the host the mock touch panel replays a script, one line per read
//! (`wasmicon run --touch-script`), so a run repeats exactly.

#![no_std]

use wasmicon_hal::display::{PixelFormat, Screen};
use wasmicon_hal::touch::Panel;
use wasmicon_hal::{ErrorCode, log, time};

type Result<T> = core::result::Result<T, ErrorCode>;

const CANVAS: u16 = 0x0000;
const BAR: u16 = 0x2104;
const CLEAR: u16 = 0x8410;
const DONE: u16 = 0xF800;
/// Dot colors, one per point of a read.
const INK: [u16; 5] = [0xFFFF, 0x07E0, 0x07FF, 0xFFE0, 0xF81F];

/// Height of the top bar in pixels.
const BAR_H: u32 = 40;
/// Width of the CLEAR and DONE buttons.
const BUTTON_W: u32 = 80;
/// Side of a dot.
const DOT: u32 = 6;
/// Points read at a time: the most a capacitive controller reports.
const POINTS: usize = INK.len();
/// Pause between reads.
const POLL_MS: u32 = 10;
/// Reads before the app gives up waiting for DONE, about five minutes.
const MAX_POLLS: u32 = 30_000;

/// Entry point. The host calls it once, right after instantiation
/// (docs/abi-spec.md §3.3).
#[unsafe(no_mangle)]
pub extern "C" fn run() {
    log::info("touch-demo start");
    let Ok(screen) = Screen::open(0) else {
        log::error("no display");
        return;
    };
    if screen.format() != Some(PixelFormat::Rgb565) {
        log::error("unknown pixel format");
        return;
    }
    let Ok(panel) = Panel::open(0) else {
        log::error("no touch panel");
        return;
    };
    match paint(&screen, &panel) {
        Ok(true) => log::info("touch-demo done"),
        Ok(false) => log::info("touch-demo timed out"),
        Err(_) => log::error("touch-demo failed"),
    }
}

/// What a touched point does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hit {
    Clear,
    Done,
    /// On the bar between the buttons.
    Bar,
    Canvas,
}

fn hit(w: u32, x: u32, y: u32) -> Hit {
    if y >= BAR_H {
        Hit::Canvas
    } else if x < BUTTON_W {
        Hit::Clear
    } else if x + BUTTON_W >= w {
        Hit::Done
    } else {
        Hit::Bar
    }
}

/// Runs until DONE is touched (`Ok(true)`) or `MAX_POLLS` reads pass.
fn paint(screen: &Screen, panel: &Panel) -> Result<bool> {
    let (w, h) = (screen.width(), screen.height());
    if w < 2 * BUTTON_W + 1 || h <= BAR_H + DOT {
        log::error("display too small");
        return Err(ErrorCode::Unsupported);
    }
    draw_bar(screen, w)?;
    screen.fill_rect(0, BAR_H, w, h - BAR_H, u32::from(CANVAS))?;
    screen.present()?;

    // A button acts when it is first touched, not on every read it stays
    // touched.
    let mut was_down = false;
    let mut buf = [0u8; 4 * POINTS];
    for _ in 0..MAX_POLLS {
        let n = panel.read(&mut buf)?;
        let mut drew = false;
        for (i, p) in buf.chunks_exact(4).take(n).enumerate() {
            let x = u32::from(u16::from_be_bytes([p[0], p[1]]));
            let y = u32::from(u16::from_be_bytes([p[2], p[3]]));
            match hit(w, x, y) {
                Hit::Done if !was_down => return Ok(true),
                Hit::Clear if !was_down => {
                    screen.fill_rect(0, BAR_H, w, h - BAR_H, u32::from(CANVAS))?;
                    drew = true;
                }
                Hit::Canvas => {
                    // Keep the dot on the canvas at the right and bottom edges.
                    let dx = x.min(w - DOT);
                    let dy = y.min(h - DOT);
                    screen.fill_rect(dx, dy, DOT, DOT, u32::from(INK[i]))?;
                    drew = true;
                }
                _ => {}
            }
        }
        was_down = n > 0;
        if drew {
            screen.present()?;
        }
        time::sleep_ms(POLL_MS);
    }
    Ok(false)
}

fn draw_bar(screen: &Screen, w: u32) -> Result<()> {
    screen.fill_rect(0, 0, w, BAR_H, u32::from(BAR))?;
    screen.fill_rect(0, 0, BUTTON_W, BAR_H, u32::from(CLEAR))?;
    screen.fill_rect(w - BUTTON_W, 0, BUTTON_W, BAR_H, u32::from(DONE))
}

/// Required because the crate is `no_std` (docs/decisions.md A-04).
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

#[cfg(test)]
mod tests {
    use super::{Hit, hit};

    #[test]
    fn buttons_sit_at_both_ends_of_the_bar() {
        assert!(hit(320, 0, 0) == Hit::Clear);
        assert!(hit(320, 79, 39) == Hit::Clear);
        assert!(hit(320, 80, 0) == Hit::Bar);
        assert!(hit(320, 239, 0) == Hit::Bar);
        assert!(hit(320, 240, 0) == Hit::Done);
        assert!(hit(320, 319, 39) == Hit::Done);
        assert!(hit(320, 0, 40) == Hit::Canvas);
    }
}
