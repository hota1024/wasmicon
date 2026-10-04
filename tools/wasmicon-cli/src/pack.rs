//! `wasmicon pack` — `.wasm` をスロット画像にする（`docs/app-workflow.md` §3.4）。
//!
//! 形は `ports/common` の `slot` が持つ（**書く側と読む側で 2 回書かない**）。
//!
//! これは `deploy --persist` の素材で、1 段では**外のフラッシャに渡す**
//! （`espflash write-bin <offset>` / `picotool load -o <offset>`。§3.5）。
//! オフセットはボードごとの決めごとなので、ここでは画像を作るだけにする。
//!
//! **送る前に `check` を通す。** 壊れたものや、そのボードで走らないものを
//! スロットに書いても、実機で落ちるまで分からない。

use anyhow::{Context, Result};
use std::path::PathBuf;

use wasmicon_port::slot;

pub struct Options {
    pub path: PathBuf,
    /// 出力先。`None` なら `<入力>.slot`。
    pub out: Option<PathBuf>,
}

/// スロット画像を書く。
///
/// # Errors
/// 入力が読めない、出力が書けないとき。
pub fn run(opts: &Options) -> Result<bool> {
    let wasm =
        std::fs::read(&opts.path).with_context(|| format!("{} を読めない", opts.path.display()))?;

    let out = opts.out.clone().unwrap_or_else(|| {
        let mut p = opts.path.clone();
        p.set_extension("slot");
        p
    });

    let header = slot::header(&wasm);
    let mut image = Vec::with_capacity(slot::image_len(wasm.len()));
    image.extend_from_slice(&header);
    image.extend_from_slice(&wasm);

    std::fs::write(&out, &image).with_context(|| format!("{} を書けない", out.display()))?;

    // CRC はヘッダの 12..16（§3.4）。デバイス側のログと突き合わせられるよう
    // 同じ値を出す。
    let crc = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);
    println!(
        "{}  {} B（wasm {} B + ヘッダ {} B、crc32 {crc:08x}）",
        out.display(),
        image.len(),
        wasm.len(),
        slot::HEADER_LEN
    );
    Ok(true)
}
