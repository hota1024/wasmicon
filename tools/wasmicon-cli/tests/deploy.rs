//! `wasmicon deploy` の**焼く前の判定**を固定する。
//!
//! フラッシャの呼び出しは実機が要るので、`prepare`（検査と画像の用意）だけを
//! 見る。**走らないものを焼かない**ことがこの段の肝で、それはここで決まる。

use std::path::{Path, PathBuf};
use std::process::Command;

use wasmicon_cli::deploy;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("リポジトリルートが見つからない")
}

fn tmp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("deploy-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("作業ディレクトリを作れない");
    dir
}

/// guest workspace でアプリをビルドして `.wasm` を返す。
fn build(pkg: &str) -> Vec<u8> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let root = repo_root();
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

/// 通ることを確かめて計画を返す。
///
/// `Plan` は `Debug` を実装しない（`Profile` / `Slot` がポート側で
/// 持っていない）ので、`expect` は使えない。
fn prepared(o: &deploy::Options) -> deploy::Plan {
    match deploy::prepare(o) {
        Ok(p) => p,
        Err(e) => panic!("通ると思ったが落ちた: {e:#}"),
    }
}

/// 落ちることを確かめて理由を返す。
fn refused(o: &deploy::Options, what: &str) -> String {
    match deploy::prepare(o) {
        Err(e) => format!("{e:#}"),
        Ok(_) => panic!("{what} を期待したが通ってしまった"),
    }
}

fn opts(path: PathBuf, board: Option<&str>) -> deploy::Options {
    deploy::Options {
        path,
        board: board.map(str::to_string),
        manifest: None,
        no_run: true,
        monitor: false,
        out: None,
        port: None,
    }
}

#[test]
fn a_runnable_app_gets_an_image_and_an_address() {
    let dir = tmp("ok");
    let app = dir.join("blink.wasm");
    std::fs::write(&app, build("blink-rs")).expect("書けない");

    let plan = prepared(&opts(app, Some("rp2350")));
    assert_eq!(plan.board.name, "rp2350");
    // picotool に渡すのは**アドレス**（オフセットではない）。
    assert_eq!(plan.addr, 0x1010_0000);
    assert_eq!(plan.image_len, plan.wasm_len + 16);
    assert!(plan.image.is_file(), "画像をアプリの隣に書く");
    assert!(
        plan.image.extension().is_some_and(|e| e == "bin"),
        "picotool は拡張子で種別を判定するので .bin（§9）"
    );
}

#[test]
fn an_app_that_cannot_run_on_the_board_is_not_flashed() {
    // **これがこの段の肝。** sensor-display は I2C と SPI を使うので
    // rp2040 では動かない（ポートが unsupported を返す）。焼いてしまうと
    // 実機で落ちるまで分からない。
    let dir = tmp("check-fails");
    let app = dir.join("sensor.wasm");
    std::fs::write(&app, build("sensor-display-rs")).expect("書けない");

    let msg = refused(&opts(app.clone(), Some("rp2040")), "焼かない");
    assert!(msg.contains("check が落ちた"), "{msg}");
    assert!(msg.contains("rp2040"), "{msg}");
    assert!(
        !dir.join("sensor.bin").exists(),
        "画像も書かない（焼く前に止まる）"
    );

    // 同じアプリは rp2350 では通る。
    prepared(&opts(app, Some("rp2350")));
}

#[test]
fn a_board_without_a_slot_is_refused() {
    // host にフラッシュは無い。3 ポートはスロットを持っている。
    let dir = tmp("no-slot");
    let app = dir.join("blink.wasm");
    std::fs::write(&app, build("blink-rs")).expect("書けない");

    let msg = refused(&opts(app, Some("host")), "置き場所が無い");
    assert!(msg.contains("決まっていない"), "{msg}");
}

#[test]
fn the_board_must_be_named_somewhere() {
    // どこに書くか決まらないので、推測しない。
    let dir = tmp("no-board");
    let app = dir.join("blink.wasm");
    std::fs::write(&app, build("blink-rs")).expect("書けない");

    let msg = refused(&opts(app, None), "ボードが決まらない");
    assert!(msg.contains("--board"), "{msg}");
    assert!(msg.contains("wasmicon.toml"), "{msg}");
}
