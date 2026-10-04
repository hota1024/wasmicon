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

use anyhow::{Context, Result, bail};
use std::path::PathBuf;

use wasmicon_port::profile;
use wasmicon_port::slot;

pub struct Options {
    pub path: PathBuf,
    /// 出力先。`None` なら `<入力>.slot`。
    pub out: Option<PathBuf>,
    /// `--board`。渡すと**スロットに収まるかを検査**し、書き込むコマンドを出す。
    pub board: Option<String>,
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

    // ボードが分かるなら、スロットに収まるかを**書く前に**見る。
    // 入らないものを焼いても、実機で「slot truncated」が出るまで分からない。
    let board = match &opts.board {
        None => None,
        Some(name) => Some(profile::by_name(name).with_context(|| {
            let known: Vec<&str> = profile::PROFILES.iter().map(|p| p.name).collect();
            format!("知らないボード {name}（あるのは {}）", known.join(" / "))
        })?),
    };
    if let Some(p) = board {
        match p.slot {
            None => bail!(
                "{} のスロットの置き場所がまだ決まっていない（docs/TODO.md §5-2）",
                p.name
            ),
            Some(sl) if image.len() > sl.len as usize => bail!(
                "{} のスロットに入らない（{} B > {} B）",
                p.name,
                image.len(),
                sl.len
            ),
            Some(_) => {}
        }
    }

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

    // 1 段は外のフラッシャに渡す（§3.5）。オフセットはプロファイルが持つ。
    if let Some(p) = board
        && let Some(sl) = p.slot
    {
        println!(
            "→ 書き込み: picotool load -o {:#x} {}",
            sl.offset,
            out.display()
        );
        println!("  （BOOTSEL を押しながら USB を挿してから）");
    }
    Ok(true)
}
