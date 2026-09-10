//! host ポートで手書きの Lチカゲストを動かし、トレースを確かめる。
//!
//! docs/handoff.md §5 Phase 2 の完了条件「ports/host でゲストが動き、mock GPIO の
//! トレースが出る」に対応する。Rust 版の `apps/blink-rs` は Phase 3 で作る。

use std::path::PathBuf;
use std::process::Command;

fn wat_to_wasm(name: &str) -> Vec<u8> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(name);
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("{name}.wasm"));
    let status = Command::new("wasm-tools")
        .arg("parse")
        .arg(&src)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("wasm-tools を起動できない");
    assert!(status.success(), "wasm-tools parse が失敗した");
    std::fs::read(&out).unwrap()
}

#[test]
fn blink_runs_and_traces() {
    let wasm = wat_to_wasm("blink.wat");

    // 対応機能セット内であることを確かめる（docs/handoff.md §5 Phase 3 と同じ検査）。
    let ok = Command::new("wasm-tools")
        .args([
            "validate",
            "--features=mvp,sign-extension,saturating-float-to-int,bulk-memory,multi-value,mutable-global",
        ])
        .arg(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/blink.wat.wasm"),
        )
        .status()
        .expect("wasm-tools を起動できない");
    assert!(ok.success(), "対応機能セット外の命令が入っている");

    // コアは core::fmt を使わないので Error に Debug が無い。理由を自分で出す。
    let trace = match wasmicon_host::run_wasm(&wasm, true) {
        Ok(out) => out.trace,
        Err(e) => panic!("実行に失敗: {} [{}]", e.reason(), e.kind().name()),
    };
    println!("{trace}");

    // ピンを 1 本開いてハンドル 1 を得る。
    assert!(
        trace.contains("> wasmicon:hal/gpio@0.1.0/[static]pin.open(2, 3)\n< 0 [1]"),
        "pin.open のトレースが違う:\n{trace}"
    );
    // time は abi-spec §9 によりトレースに含めない。
    assert!(!trace.contains("time@0.1.0"), "time がトレースに出ている");
    // 点滅は write と toggle が 3 回ずつ。
    assert_eq!(
        trace.matches("[method]pin.write(1, 1)").count(),
        3,
        "pin.write の回数が違う:\n{trace}"
    );
    assert_eq!(
        trace.matches("[method]pin.toggle(1)").count(),
        3,
        "pin.toggle の回数が違う:\n{trace}"
    );
    // 最後に drop。
    assert!(
        trace.contains("[resource-drop]pin(1)"),
        "drop のトレースが無い:\n{trace}"
    );
    // 全ての host call が成功している。
    assert!(
        !trace.contains("\n< 1"),
        "失敗した host call がある:\n{trace}"
    );
}

/// abi-spec §9 は 1 host call = 2 行（要求と結果）。ゲストが改行や `"` を
/// 混ぜてもこの形が崩れないこと。崩れると両ボードの diff が行単位でずれる。
#[test]
fn trace_escapes_guest_strings() {
    let wasm = wat_to_wasm("logesc.wat");
    let trace = match wasmicon_host::run_wasm(&wasm, true) {
        Ok(out) => out.trace,
        Err(e) => panic!("実行に失敗: {} [{}]", e.reason(), e.kind().name()),
    };
    println!("{trace}");

    let lines: Vec<&str> = trace.lines().collect();
    assert_eq!(lines.len(), 2, "1 host call は 2 行のはず:\n{trace}");
    assert_eq!(
        lines[0], r#"> wasmicon:hal/log@0.1.0/log(2, "a\nb\"c")"#,
        "エスケープされていない"
    );
    assert_eq!(lines[1], "<");
}
