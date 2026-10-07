//! `wasmicon trace diff` — 2 つのシリアル出力からトレースを取り出して
//! 突き合わせる（`docs/handoff.md` §5 Phase 6 の (2)）。
//! `wasmicon trace replay` — 実機のトレースから I2C の応答を抜き出して、
//! `run --i2c-replay` が読む形にする（`verify/sht4x-replay.txt` の差し替え用）。
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

pub struct ReplayOptions {
    pub log: PathBuf,
    /// 書き出し先。無ければ標準出力。
    pub out: Option<PathBuf>,
}

/// 正規化したトレース。
///
/// 行は **`Vec<u8>` で持つ**。`String` に落とすと UTF-8 でないバイトが
/// どれも `U+FFFD` に潰れて、**違うノイズが混ざった 2 本を「一致」と
/// 言ってしまう**（シリアルがノイズを吐くことはこのコードの前提そのもので、
/// だから NUL を落としている）。表示のときだけ `from_utf8_lossy` する。
pub struct Trace {
    pub lines: Vec<Vec<u8>>,
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
    let lines = cleaned
        .split(|b| *b == b'\n')
        .filter(|l| matches!(l.first(), Some(b'>' | b'<')))
        .map(<[u8]>::to_vec)
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

fn at(t: &Trace, i: usize) -> String {
    t.lines.get(i).map_or_else(
        || "（ここで終わっている）".to_string(),
        |l| String::from_utf8_lossy(l).into_owned(),
    )
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

/// I2C の読み出しが受け取ったバイト列を、トレースの順に取り出す。
///
/// `i2c.read` / `i2c.write-read` の呼び出し行の次にある結果行の `data=0x…`
/// （abi-spec §9）を拾う。host は読み出し 1 回につき replay の 1 行を消費する
/// （`ports/host/src/hal.rs`）ので、同じ順に並べればそのまま replay になる。
/// 失敗した読み出し（status が 0 でない）は host でも消費しないので飛ばす。
///
/// # Errors
/// 32 バイトを超えて CRC に畳まれた読み出しがあるとき（元のバイト列が
/// トレースに残っていない）。
pub fn i2c_reads(t: &Trace) -> Result<Vec<Vec<u8>>> {
    let mut out = Vec::new();
    let mut lines = t.lines.iter();
    while let Some(line) = lines.next() {
        let is_read = line.starts_with(b"> wasmicon:hal/i2c@")
            && (contains(line, b"/[method]bus.read(")
                || contains(line, b"/[method]bus.write-read("));
        if !is_read {
            continue;
        }
        let Some(result) = lines.next() else { break };
        if !result.starts_with(b"< 0 ") {
            continue;
        }
        let Some(hex) = after(result, b"data=0x") else {
            bail!(
                "読み出しの結果に data= が無い: {}\n  \
                 data= を出す前のファームで取ったトレースではないか確認すること。",
                String::from_utf8_lossy(result)
            );
        };
        let hex: Vec<u8> = hex
            .iter()
            .copied()
            .take_while(u8::is_ascii_hexdigit)
            .collect();
        if hex.len() % 2 != 0 || after(result, b"..len=").is_some() {
            bail!(
                "読み出しのバイト列が完全に残っていない（32 バイト超は CRC に畳まれる）: {}",
                String::from_utf8_lossy(result)
            );
        }
        let bytes = hex
            .chunks(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap_or("zz"), 16))
            .collect::<Result<Vec<u8>, _>>()
            .context("data= を 16 進として読めない")?;
        out.push(bytes);
    }
    Ok(out)
}

/// `run --i2c-replay` が読む形（1 行 = 1 回の読み出し、16 進を空白区切り）。
#[must_use]
pub fn format_replay(reads: &[Vec<u8>], source: &str) -> String {
    let mut s = format!("# wasmicon trace replay で {source} から取り出した I2C の応答。\n");
    s.push_str("# 1 行 = 1 回の i2c 読み出しに返すバイト列（16 進、空白区切り）。\n");
    for r in reads {
        let line: Vec<String> = r.iter().map(|b| format!("{b:02x}")).collect();
        s.push_str(&line.join(" "));
        s.push('\n');
    }
    s
}

/// トレースから replay を書き出す。
///
/// # Errors
/// ファイルが読めない・書けない、トレース行が無い、読み出しが 1 件も無いとき。
pub fn replay(opts: &ReplayOptions) -> Result<bool> {
    let t = read(&opts.log)?;
    let reads = i2c_reads(&t)?;
    // 0 件を黙って空の replay にしない。host で走らせると「センサー無し」になり、
    // 取り込みの失敗が別の症状に化ける。
    if reads.is_empty() {
        bail!(
            "{} に成功した I2C の読み出しが 1 件も無い",
            opts.log.display()
        );
    }
    let text = format_replay(&reads, &short(&opts.log));
    match &opts.out {
        Some(p) => {
            std::fs::write(p, &text).with_context(|| format!("{} に書けない", p.display()))?;
            eprintln!("{} 件の読み出しを {} に書いた", reads.len(), p.display());
        }
        None => print!("{text}"),
    }
    Ok(true)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// `needle` の直後から行末まで。
fn after<'a>(hay: &'a [u8], needle: &[u8]) -> Option<&'a [u8]> {
    hay.windows(needle.len())
        .position(|w| w == needle)
        .map(|i| &hay[i + needle.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 比較しやすいように表示用へ落とす（テストの中だけ）。
    fn text(t: &Trace) -> Vec<String> {
        t.lines
            .iter()
            .map(|l| String::from_utf8_lossy(l).into_owned())
            .collect()
    }

    #[test]
    fn drops_banners_and_guest_logs() {
        let raw = b"wasmicon rp2040\n[wasm] blink start\n> a/b(1)\n< 0 [1]\n";
        assert_eq!(text(&normalize(raw)), ["> a/b(1)", "< 0 [1]"]);
    }

    #[test]
    fn line_noise_is_not_folded_into_one_character() {
        // UTF-8 でないバイトが違う 2 本を「一致」と言ってはいけない。
        // String に落とすとどちらも U+FFFD になって一致してしまう。
        let a = normalize(b"> a/b(1)\xff\n< 0 [1]\n");
        let b = normalize(b"> a/b(1)\xfe\n< 0 [1]\n");
        assert_eq!(a.lines.len(), 2);
        assert_eq!(first_divergence(&a, &b), Some(0), "1 行目で食い違う");
    }

    #[test]
    fn drops_cr_and_nul() {
        // 実機のシリアルは NUL を吐く（2026-09-26 に踏んだ）。
        let raw = b"wasmicon esp32s3\r\n\0\0[wasm] x\r\n\0> a/b(1)\r\n< 0 [1]\r\n";
        assert_eq!(text(&normalize(raw)), ["> a/b(1)", "< 0 [1]"]);
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

    #[test]
    fn extracts_i2c_reads_in_order() {
        let t = normalize(
            b"> wasmicon:hal/i2c@0.1.0/[method]bus.write(1, 68, 0xfd)\n< 0\n\
              > wasmicon:hal/i2c@0.1.0/[method]bus.read(1, 68, 6)\n< 0 [len=6 data=0x6421e074e970]\n\
              > wasmicon:hal/i2c@0.1.0/[method]bus.read(1, 68, 2)\n< 3\n\
              > wasmicon:hal/i2c@0.1.0/[method]bus.write-read(1, 68, 0x89, 2)\n< 0 [len=2 data=0xabcd]\n",
        );
        let reads = i2c_reads(&t).expect("読めるはず");
        // 失敗した読み出し（< 3）は host でも消費しないので飛ばす。
        assert_eq!(
            reads,
            [vec![0x64, 0x21, 0xe0, 0x74, 0xe9, 0x70], vec![0xab, 0xcd]]
        );
        assert_eq!(
            format_replay(&reads, "x.log")
                .lines()
                .filter(|l| !l.starts_with('#'))
                .collect::<Vec<_>>(),
            ["64 21 e0 74 e9 70", "ab cd"]
        );
    }

    #[test]
    fn a_trace_without_data_is_an_error() {
        // data= を出す前のファームのトレース。黙って空にしない。
        let t = normalize(b"> wasmicon:hal/i2c@0.1.0/[method]bus.read(1, 68, 6)\n< 0 [len=6]\n");
        assert!(i2c_reads(&t).is_err());
    }

    #[test]
    fn a_folded_read_is_an_error() {
        let t = normalize(
            b"> wasmicon:hal/i2c@0.1.0/[method]bus.read(1, 68, 40)\n\
              < 0 [len=40 data=0x00112233445566778899aabbccddeeff..len=40 crc32=01234567]\n",
        );
        assert!(i2c_reads(&t).is_err());
    }
}
