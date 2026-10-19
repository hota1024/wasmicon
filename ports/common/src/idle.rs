//! Parking when there is no app to run (docs/app-workflow.md §3.3).
//!
//! The firmware embeds no app. When the slot is empty or corrupt it prints the
//! reason on the serial port and comes here. No pin is driven: role names are
//! not firmware vocabulary, so there is no special "alive" LED pin either
//! (docs/app-workflow.md §3.9).

use crate::Board;

/// Parks forever. Never returns.
pub fn park<B: Board>(board: &mut B) -> ! {
    loop {
        board.sleep_ms(1000);
    }
}
