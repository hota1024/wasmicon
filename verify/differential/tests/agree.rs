//! 自作インタプリタと wasmtime が同じトレースを出すことを確かめる。
//!
//! docs/handoff.md §5 Phase 6 の (3)。ゲストは `apps/` の 4 つ全部。
//! 実機とは無関係に、インタプリタの正しさを外部の参照実装で検証する。

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("リポジトリルートが見つからない")
}

/// guest workspace で Rust のアプリをビルドする。
fn build_rust_app(pkg: &str) -> Vec<u8> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let root = repo_root();
    // cargo test が渡す RUSTUP_TOOLCHAIN は toolchain override を無効にする。
    let status = Command::new("cargo")
        .current_dir(root.join("apps"))
        .env_remove("RUSTUP_TOOLCHAIN")
        .args(["build", "--release", "-p", pkg])
        .status()
        .expect("cargo を起動できない");
    assert!(status.success(), "{pkg} のビルドに失敗した");
    let wasm = root
        .join("apps/target/wasm32-unknown-unknown/release")
        .join(format!("{}.wasm", pkg.replace('-', "_")));
    std::fs::read(&wasm).unwrap_or_else(|e| panic!("{} を読めない: {e}", wasm.display()))
}

/// AssemblyScript のアプリを asc でビルドする。
fn build_as_app(dir: &str, out: &str) -> Vec<u8> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let root = repo_root();
    let app = root.join("apps").join(dir);
    assert!(
        root.join("node_modules/assemblyscript").exists(),
        "AssemblyScript が入っていない。リポジトリルートで `npm install` を実行すること"
    );
    let status = Command::new("npx")
        .current_dir(&app)
        .args([
            "asc",
            "assembly/index.ts",
            "--config",
            "asconfig.json",
            "--target",
            "release",
        ])
        .status()
        .expect("npx を起動できない");
    assert!(status.success(), "{dir} のビルドに失敗した");
    std::fs::read(app.join("build").join(out)).expect("ビルド結果を読めない")
}

fn sht31_replay() -> Vec<Vec<u8>> {
    wasmicon_host::load_i2c_replay(&repo_root().join("verify/sht31-replay.txt"))
        .expect("記録済み応答を読めない")
}

/// 2 つのエンジンでトレースが一致することを確かめる。
fn assert_agrees(wasm: &[u8], replay: Vec<Vec<u8>>, label: &str) {
    let c = wasmicon_differential::compare(wasm, replay)
        .unwrap_or_else(|e| panic!("{label}: 実行に失敗: {e:#}"));
    if let Some((line, ours, theirs)) = c.first_difference() {
        panic!(
            "{label}: {line} 行目でエンジンが食い違う\n  wasmicon: {ours}\n  wasmtime: {theirs}"
        );
    }
    assert!(c.agrees(), "{label}: トレースが一致しない");
    assert!(
        !c.wasmicon.is_empty(),
        "{label}: トレースが空（何も検証していない）"
    );
}

#[test]
fn blink_rs_agrees_with_wasmtime() {
    assert_agrees(&build_rust_app("blink-rs"), Vec::new(), "blink-rs");
}

#[test]
fn blink_as_agrees_with_wasmtime() {
    assert_agrees(
        &build_as_app("blink-as", "blink_as.wasm"),
        Vec::new(),
        "blink-as",
    );
}

#[test]
fn sensor_display_rs_agrees_with_wasmtime() {
    assert_agrees(
        &build_rust_app("sensor-display-rs"),
        sht31_replay(),
        "sensor-display-rs",
    );
}

#[test]
fn sensor_display_as_agrees_with_wasmtime() {
    assert_agrees(
        &build_as_app("sensor-display-as", "sensor_display_as.wasm"),
        sht31_replay(),
        "sensor-display-as",
    );
}
