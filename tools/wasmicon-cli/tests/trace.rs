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

/// スクリプトの判定。`true` = 一致。
fn script_says_match(a: &Path, b: &Path) -> bool {
    let root = repo_root();
    let out = Command::new("sh")
        .current_dir(&root)
        .arg("verify/diff-traces.sh")
        .arg(a)
        .arg(b)
        .output()
        .expect("diff-traces.sh を起動できない");
    out.status.success()
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
fn assert_agree(a: &Path, b: &Path, expect_match: bool, label: &str) {
    let script = script_says_match(a, b);
    let cli = cli_says_match(a, b).unwrap_or(false);
    assert_eq!(
        script, expect_match,
        "{label}: diff-traces.sh の判定が期待と違う"
    );
    assert_eq!(cli, expect_match, "{label}: CLI の判定が期待と違う");
}

struct Fixtures {
    dir: PathBuf,
}

impl Fixtures {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("wasmicon-trace-{name}"));
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
fn empty_traces_are_not_a_match() {
    // 空同士は文字列としては一致するが、それは「何も比較していない」。
    // ここは完了条件の判定に使うので、黙って成功を返すのが最悪の壊れ方になる。
    let f = Fixtures::new("empty");
    let a = f.write("e1.log", b"wasmicon rp2040\n[wasm] blink start\n");
    let b = f.write("e2.log", b"wasmicon esp32s3\n[wasm] blink start\n");

    assert!(!script_says_match(&a, &b), "スクリプトは失敗にする");
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
