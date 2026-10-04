//! `wasmicon trace diff` — 2 つのシリアル出力からトレースを取り出して
//! 突き合わせる（`docs/handoff.md` §5 Phase 6 の (2)）。
//!
//! 正規化の規則は `verify/diff-traces.sh` と**同一**。あちらは CI が
//! `--self-test` で回し続けるので、**両者が同じ判定を出すことを
//! `tests/trace.rs` が突き合わせる**（`tools/check-sigs.sh` が
//! `wit2sig.py` と突き合わせているのと同じ形）。片方だけ直すと落ちる。
//!
//! 規則は 2 つだけ:
//!
//! - **CR と NUL を落とす。** 実機のシリアルはリセットや電源投入の瞬間に
//!   ライン・ノイズで NUL を吐く。落とさないと 2026-09-26 に踏んだ形
//!   （`grep` がバイナリ判定して 1 行しか返さない）になる
//! - **`>` か `<` で始まる行だけ残す。** バナーやゲストの `[wasm] ...` は
//!   ボードごとに違うので捨てる。捨ててよいのは、同じ内容が `log` の
//!   host call としてトレースに出ているため
//!
//! **トレース行が 1 行も無ければ失敗にする。** 空同士は文字列として一致して
//! しまうが、それは「一致した」ではなく「何も比較していない」。ここは完了条件の
//! 判定に使うので、黙って成功を返すのが最悪の壊れ方になる。

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

pub struct Options {
    pub a: PathBuf,
    pub b: PathBuf,
}

/// 正規化したトレース。
pub struct Trace {
    pub lines: Vec<String>,
}

/// シリアル出力からトレース行だけを取り出す。
///
/// `verify/diff-traces.sh` の `normalize()` と同じ: CR と NUL を落として
/// `>` / `<` で始まる行を残す。
#[must_use]
pub fn normalize(bytes: &[u8]) -> Trace {
    let cleaned: Vec<u8> = bytes
        .iter()
        .copied()
        .filter(|b| *b != b'\r' && *b != 0)
        .collect();
    let text = String::from_utf8_lossy(&cleaned);
    let lines = text
        .lines()
        .filter(|l| l.starts_with('>') || l.starts_with('<'))
        .map(str::to_string)
        .collect();
    Trace { lines }
}

/// 最初に食い違う位置（0 始まり）。同じなら `None`。
#[must_use]
pub fn first_divergence(a: &Trace, b: &Trace) -> Option<usize> {
    for (i, (x, y)) in a.lines.iter().zip(&b.lines).enumerate() {
        if x != y {
            return Some(i);
        }
    }
    if a.lines.len() == b.lines.len() {
        None
    } else {
        Some(a.lines.len().min(b.lines.len()))
    }
}

/// 突き合わせる。一致すれば `true`。
///
/// # Errors
/// ファイルが読めないか、どちらかからトレース行が 1 行も取れないとき。
pub fn diff(opts: &Options) -> Result<bool> {
    let a = read(&opts.a)?;
    let b = read(&opts.b)?;

    match first_divergence(&a, &b) {
        None => {
            println!(
                "一致: {} 行（{} と {}）",
                a.lines.len(),
                opts.a.display(),
                opts.b.display()
            );
            Ok(true)
        }
        Some(i) => {
            eprintln!(
                "不一致: {} は {} 行、{} は {} 行",
                opts.a.display(),
                a.lines.len(),
                opts.b.display(),
                b.lines.len()
            );
            // トレースは追記しかされないので、最初の食い違いが原因に最も近い。
            eprintln!("  最初の食い違い: {} 行目", i + 1);
            eprintln!("    {}  {}", short(&opts.a), at(&a, i));
            eprintln!("    {}  {}", short(&opts.b), at(&b, i));
            Ok(false)
        }
    }
}

/// 詳細行はファイル名だけにする（絶対パスは見出しに出ている）。
fn short(p: &Path) -> String {
    p.file_name()
        .unwrap_or(p.as_os_str())
        .to_string_lossy()
        .into_owned()
}

fn at(t: &Trace, i: usize) -> &str {
    t.lines
        .get(i)
        .map_or("（ここで終わっている）", String::as_str)
}

/// 読んで正規化する。トレース行が 0 行なら失敗。
fn read(path: &Path) -> Result<Trace> {
    let bytes = std::fs::read(path).with_context(|| format!("{} を読めない", path.display()))?;
    let t = normalize(&bytes);
    if t.lines.is_empty() {
        bail!(
            "{} からトレース行を 1 行も取り出せない。\n  \
             abi-spec §9 の '> ' / '< ' で始まる行が必要。取り込みが途中で切れて\n  \
             いないか、行頭にタイムスタンプが付いていないか確認すること。",
            path.display()
        );
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_banners_and_guest_logs() {
        let raw = b"wasmicon rp2040\n[wasm] blink start\n> a/b(1)\n< 0 [1]\n";
        let t = normalize(raw);
        assert_eq!(t.lines, ["> a/b(1)", "< 0 [1]"]);
    }

    #[test]
    fn drops_cr_and_nul() {
        // 実機のシリアルは NUL を吐く（2026-09-26 に踏んだ）。
        let raw = b"wasmicon esp32s3\r\n\0\0[wasm] x\r\n\0> a/b(1)\r\n< 0 [1]\r\n";
        let t = normalize(raw);
        assert_eq!(t.lines, ["> a/b(1)", "< 0 [1]"]);
    }

    #[test]
    fn finds_the_first_divergence() {
        let a = normalize(b"> x\n< 0\n> y\n");
        let b = normalize(b"> x\n< 0\n> z\n");
        assert_eq!(first_divergence(&a, &b), Some(2));
        assert_eq!(first_divergence(&a, &a), None);
    }

    #[test]
    fn a_shorter_trace_diverges_at_its_end() {
        let a = normalize(b"> x\n< 0\n> y\n");
        let b = normalize(b"> x\n< 0\n");
        assert_eq!(first_divergence(&a, &b), Some(2));
    }
}
