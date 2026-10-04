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
    let image = build(&wasm);

    // **拡張子は `.bin`。** `picotool load` は拡張子でファイル種別を判定する
    // ので、`.slot` のような独自の名前だと
    // 「does not have a recognized file type (extension)」で弾かれる
    // （2026-10-04 に Pico 2 W で踏んだ）。
    let out = opts.out.clone().unwrap_or_else(|| {
        let mut p = opts.path.clone();
        p.set_extension("bin");
        p
    });

    // ボードが分かるなら、スロットに収まるかを**書く前に**見る。
    // 入らないものを焼いても、実機で「slot truncated」が出るまで分からない。
    let board = match &opts.board {
        None => None,
        Some(name) => Some(resolve_board(name)?),
    };
    if let Some(p) = board {
        fits(p, image.len())?;
    }

    std::fs::write(&out, &image).with_context(|| format!("{} を書けない", out.display()))?;
    let crc = crc_of(&image);
    println!(
        "{}  {} B（wasm {} B + ヘッダ {} B、crc32 {crc:08x}）",
        out.display(),
        image.len(),
        wasm.len(),
        slot::HEADER_LEN
    );

    // 1 段は外のフラッシャに渡す（§3.5）。**ボードで道具も引数も違う。**
    if let Some(p) = board
        && let Some(sl) = p.slot
    {
        if p.name == "esp32s3" {
            // espflash は**フラッシュのオフセット**を取る。既定で
            // `--after hard-reset` までやるので、別にリセットは要らない。
            println!(
                "→ 書き込み: espflash write-bin --port <dev> {:#x} {}",
                sl.offset,
                out.display()
            );
            println!("  （ボタン操作は要らない。DTR/RTS でリセットされる）");
        } else {
            // **picotool の `-o` は絶対アドレス**（フラッシュのオフセットでは
            // ない）。オフセットを渡すと
            // 「invalid memory range 0x00100000-...」で弾かれる
            // （2026-10-04 に Pico 2 W で踏んだ）。XIP は 0x1000_0000 から。
            let addr = XIP_BASE + u64::from(sl.offset);
            println!("→ 書き込み: picotool load -o {addr:#x} {}", out.display());
            println!("  （BOOTSEL を押しながら USB を挿してから）");
        }
        println!("  `wasmicon deploy` なら 1 コマンドで済む");
    }
    Ok(true)
}

/// RP2040 / RP2350 の XIP の先頭。`picotool` に渡すアドレスの基準。
pub const XIP_BASE: u64 = 0x1000_0000;

/// スロット画像を組む（ヘッダ + wasm）。
#[must_use]
pub fn build(wasm: &[u8]) -> Vec<u8> {
    let mut image = Vec::with_capacity(slot::image_len(wasm.len()));
    image.extend_from_slice(&slot::header(wasm));
    image.extend_from_slice(wasm);
    image
}

/// 画像のヘッダに入っている CRC（§3.4 の 12..16）。
///
/// デバイス側のログと突き合わせられるよう、同じ値を出すために使う。
#[must_use]
pub fn crc_of(image: &[u8]) -> u32 {
    u32::from_le_bytes([image[12], image[13], image[14], image[15]])
}

/// `--board` を解決する。
///
/// # Errors
/// 知らない名前のとき。
pub fn resolve_board(name: &str) -> Result<&'static profile::Profile> {
    profile::by_name(name).with_context(|| {
        let known: Vec<&str> = profile::PROFILES.iter().map(|p| p.name).collect();
        format!("知らないボード {name}（あるのは {}）", known.join(" / "))
    })
}

/// そのボードのスロットに収まるか。
///
/// # Errors
/// 置き場所が決まっていない、または入らないとき。
pub fn fits(p: &profile::Profile, image_len: usize) -> Result<profile::Slot> {
    match p.slot {
        None => bail!(
            "{} のスロットの置き場所がまだ決まっていない（docs/TODO.md §5-2）",
            p.name
        ),
        Some(sl) if image_len > sl.len as usize => bail!(
            "{} のスロットに入らない（{image_len} B > {} B）",
            p.name,
            sl.len
        ),
        Some(sl) => Ok(sl),
    }
}
