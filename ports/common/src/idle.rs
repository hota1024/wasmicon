//! 走らせるアプリが無いときの待機（`docs/app-workflow.md` §3.3）。
//!
//! ファームにアプリは入っていないので、スロットが空・壊れているときは
//! 理由を出したあとここに来る。生存確認は **`led` 役のピンを `Board` 越しに
//! 直接振る**。`Hal` を通さないのでトレースは出ず、Wasm も通らない
//! （示すのは「ファームが生きている」までで、ランタイムが動くことは示さない）。

use wasmicon_core::generated::gpio::{Level, PinMode};

use crate::Board;

/// 点滅の半周期。1 Hz で点滅する。
const HALF_PERIOD_MS: u32 = 500;

/// `led` 役のピンを点滅させ続ける。戻らない。
///
/// `led` 役が無いボード、またはピンを出力にできないときは、点滅せずに
/// 待つだけになる（理由はすでにシリアルに出ている前提）。
pub fn heartbeat<B: Board>(board: &mut B) -> ! {
    let led = board
        .pin_by_role("led")
        .filter(|&pin| board.gpio_configure(pin, PinMode::Output).is_ok());
    loop {
        if let Some(pin) = led {
            let _ = board.gpio_write(pin, Level::High);
            board.sleep_ms(HALF_PERIOD_MS);
            let _ = board.gpio_write(pin, Level::Low);
        }
        board.sleep_ms(HALF_PERIOD_MS);
    }
}
