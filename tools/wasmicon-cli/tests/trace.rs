//! `wasmicon trace diff` の判定を `verify/diff-traces.sh` と突き合わせる。
//!
//! 同じ正規化を 2 つ持つことになるので、**片方だけ直したら落ちる**ように
//! しておく。`tools/check-sigs.sh` が `wit2sig.py` と `wasmicon-gen` を
//! 突き合わせているのと同じ形。
//!
//! スクリプト側の `--self-test` が守っている性質（バナーと CR の吸収、
//! 実際の差分の検出、空トレース同士を一致と言わない、NUL が混ざっても
//! 全行取れる）を、同じ入力で Rust 側にも要求する。

use std::path::{Path, PathBuf};
use std::process::Command;

use wasmicon_cli::trace;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("リポジトリルートが見つからない")
}

/// スクリプトの判定。
struct ScriptVerdict {
    matched: bool,
    /// 一致したときにスクリプトが報告する行数。
    lines: Option<usize>,
}

fn script_verdict(a: &Path, b: &Path) -> ScriptVerdict {
    let root = repo_root();
    let out = Command::new("sh")
        .current_dir(&root)
        .arg("verify/diff-traces.sh")
        .arg(a)
        .arg(b)
        .output()
        .expect("diff-traces.sh を起動できない");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    // `一致: N 行（...）` から N を取る。
    let lines = stdout
        .split_once("一致: ")
        .and_then(|(_, rest)| rest.split_once(" 行"))
        .and_then(|(n, _)| n.trim().parse().ok());
    ScriptVerdict {
        matched: out.status.success(),
        lines,
    }
}

/// Rust 側の判定。`Ok(true)` = 一致、`Err` = トレース行が取れない。
fn cli_says_match(a: &Path, b: &Path) -> Result<bool, String> {
    trace::diff(&trace::Options {
        a: a.to_path_buf(),
        b: b.to_path_buf(),
    })
    .map_err(|e| format!("{e:#}"))
}

/// 両者の判定が一致することを確かめる。
///
/// **判定だけでなく行数も見る。** 真偽だけだと、正規化が食い違って
/// 取れた行数が違っても「どちらも一致」で通ってしまう。
fn assert_agree(a: &Path, b: &Path, expect_match: bool, label: &str) {
    let script = script_verdict(a, b);
    let cli = cli_says_match(a, b);

    assert_eq!(
        script.matched, expect_match,
        "{label}: diff-traces.sh の判定が期待と違う"
    );
    match cli {
        Ok(m) => assert_eq!(m, expect_match, "{label}: CLI の判定が期待と違う"),
        Err(e) => panic!("{label}: CLI が失敗した（期待は {expect_match}）: {e}"),
    }

    if expect_match {
        let bytes = std::fs::read(a).expect("読めない");
        let n = trace::normalize(&bytes).lines.len();
        assert_eq!(
            Some(n),
            script.lines,
            "{label}: 取り出した行数が食い違う（CLI {n} 行、スクリプト {:?} 行）",
            script.lines
        );
    }
}

struct Fixtures {
    dir: PathBuf,
}

impl Fixtures {
    /// `CARGO_TARGET_TMPDIR` の下に作る。
    ///
    /// **共有の `/tmp` に固定名で置くと、worktree を並べて `cargo test` を
    /// 回したときに同じ `a.log` を書き合う**（この repo は複数 worktree で
    /// 作業する）。target の下なら worktree ごとに別で、`cargo clean` で消える。
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("trace-{name}"));
        // 前回の残りは消してから作る。
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("作業ディレクトリを作れない");
        Fixtures { dir }
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.dir.join(name);
        std::fs::write(&p, bytes).expect("書けない");
        p
    }
}

/// スクリプトの self-test と同じトレース本体。
const TRACE: &str = "> wasmicon:hal/log@0.1.0/log(2, \"blink start\")\n\
                     <\n\
                     > wasmicon:hal/gpio@0.1.0/[static]pin.open(role:led, 3)\n\
                     < 0 [1]\n";

#[test]
fn banners_and_cr_are_absorbed() {
    let f = Fixtures::new("banner");
    let a = f.write(
        "a.log",
        format!("wasmicon rp2040\n[wasm] blink start\n{TRACE}").as_bytes(),
    );
    // 同じトレースだがバナーとログ行が違い、改行も CRLF。
    let b_text = format!("wasmicon esp32s3\r\n[wasm] blink start\r\n{TRACE}").replace('\n', "\r\n");
    let b = f.write("b.log", b_text.as_bytes());

    assert_agree(&a, &b, true, "バナーと CR");
}

#[test]
fn a_real_difference_is_detected() {
    let f = Fixtures::new("diff");
    let a = f.write("a.log", format!("wasmicon rp2040\n{TRACE}").as_bytes());
    let c = f.write(
        "c.log",
        format!("wasmicon rp2040\n{TRACE}")
            .replace("role:led", "role:lcd-cs")
            .as_bytes(),
    );

    assert_agree(&a, &c, false, "役割名が違う");
}

#[test]
fn nul_bytes_do_not_eat_lines() {
    // 2026-09-26 に実機で踏んだ形。NUL を落とさないと grep がバイナリ判定して
    // 1 行しか返さない（スクリプト側の self-test が守っているのと同じ性質）。
    let f = Fixtures::new("nul");
    let a = f.write("a.log", format!("wasmicon rp2040\n{TRACE}").as_bytes());

    let mut nul = Vec::new();
    nul.extend_from_slice(b"wasmicon esp32s3\r\n\0\0[wasm] blink start\r\n");
    nul.extend_from_slice(b"> wasmicon:hal/log@0.1.0/log(2, \"blink start\")\r\n<\r\n");
    nul.extend_from_slice(b"\0> wasmicon:hal/gpio@0.1.0/[static]pin.open(role:led, 3)\r\n");
    nul.extend_from_slice(b"< 0 [1]\r\n");
    let b = f.write("nul.log", &nul);

    assert_agree(&a, &b, true, "NUL が混ざったログ");
}

#[test]
fn line_noise_is_compared_byte_for_byte() {
    // UTF-8 でないバイトが違う 2 本は不一致。String に落とすと
    // どちらも U+FFFD になって一致してしまう（CLI 側）。ロケールが
    // UTF-8 のままだと macOS の tr が途中で止まる（スクリプト側）。
    // **両方を直したので、ここで両者が揃う。**
    let f = Fixtures::new("noise");
    let mut a_bytes = Vec::new();
    a_bytes.extend_from_slice(b"wasmicon rp2040\n> a/b(1)\xff\n< 0 [1]\n");
    let mut b_bytes = Vec::new();
    b_bytes.extend_from_slice(b"wasmicon esp32s3\n> a/b(1)\xfe\n< 0 [1]\n");
    let a = f.write("noise-a.log", &a_bytes);
    let b = f.write("noise-b.log", &b_bytes);

    assert_agree(&a, &b, false, "非 UTF-8 のノイズが違う");
}

#[test]
fn empty_traces_are_not_a_match() {
    // 空同士は文字列としては一致するが、それは「何も比較していない」。
    // ここは完了条件の判定に使うので、黙って成功を返すのが最悪の壊れ方になる。
    let f = Fixtures::new("empty");
    let a = f.write("e1.log", b"wasmicon rp2040\n[wasm] blink start\n");
    let b = f.write("e2.log", b"wasmicon esp32s3\n[wasm] blink start\n");

    assert!(!script_verdict(&a, &b).matched, "スクリプトは失敗にする");
    assert!(
        cli_says_match(&a, &b).is_err(),
        "CLI も失敗にする（一致と言わない）"
    );
}

#[test]
fn line_counts_exclude_banners() {
    let f = Fixtures::new("count");
    let a = f.write(
        "a.log",
        format!("wasmicon rp2040\n[wasm] blink start\n{TRACE}").as_bytes(),
    );
    let bytes = std::fs::read(&a).expect("読めない");
    // バナーとログ行を除いた 4 行。
    assert_eq!(trace::normalize(&bytes).lines.len(), 4);
}
