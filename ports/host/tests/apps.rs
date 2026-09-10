//! `apps/` のゲストを host ポートで動かし、トレースを突き合わせる。
//!
//! HANDOFF §5 Phase 3 の完了条件:
//! 「blink-rs と blink-as の Wasm が対応機能セット内で、同じ GPIO トレースを出す」

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("リポジトリルートが見つからない")
}

/// guest workspace でアプリをビルドして `.wasm` を返す。
fn build_rust_app(pkg: &str) -> Vec<u8> {
    let root = repo_root();
    // cargo test はテストプロセスに RUSTUP_TOOLCHAIN を渡す。これが立っていると
    // rustup は toolchain override ファイルを一切見ないので、apps/rust-toolchain.toml の
    // targets（wasm32-unknown-unknown の自動導入）が効かない。外してから起動する。
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

/// 対応機能セット（HANDOFF §2-7）に収まっているか。
fn assert_within_feature_set(wasm: &[u8], label: &str) {
    let tmp = std::env::temp_dir().join(format!("wasmicon-{label}.wasm"));
    std::fs::write(&tmp, wasm).unwrap();
    let ok = Command::new("wasm-tools")
        .args([
            "validate",
            "--features=mvp,sign-extension,saturating-float-to-int,bulk-memory,multi-value,mutable-global",
        ])
        .arg(&tmp)
        .status()
        .expect("wasm-tools を起動できない");
    assert!(ok.success(), "{label} が対応機能セット外の命令を含む");
}

fn run(wasm: &[u8], label: &str) -> String {
    match wasmicon_host::run_wasm(wasm, true) {
        Ok(out) => out.trace,
        Err(e) => panic!("{label} の実行に失敗: {} [{}]", e.reason(), e.kind().name()),
    }
}

/// Lチカが期待どおりの host call 列を出すか。
fn assert_blink_trace(trace: &str, label: &str) {
    // 役割名で引いたピン番号はボード依存なので、トレースでは役割名になる（abi-spec §9）。
    assert!(
        trace.contains("> wasmicon:hal/board@0.1.0/pin-by-role(\"led\")\n< 0 [role:led]"),
        "{label}: pin-by-role のトレースが違う:\n{trace}"
    );
    assert!(
        trace.contains("> wasmicon:hal/gpio@0.1.0/[static]pin.open(role:led, 3)\n< 0 [1]"),
        "{label}: pin.open のトレースが違う:\n{trace}"
    );
    assert_eq!(
        trace.matches("[method]pin.write(1, 1)").count(),
        3,
        "{label}: pin.write の回数が違う:\n{trace}"
    );
    assert_eq!(
        trace.matches("[method]pin.toggle(1)").count(),
        3,
        "{label}: pin.toggle の回数が違う:\n{trace}"
    );
    assert!(
        trace.contains("[resource-drop]pin(1)"),
        "{label}: drop のトレースが無い:\n{trace}"
    );
    // time はトレースに含めない（abi-spec §9）。
    assert!(!trace.contains("time@0.1.0"), "{label}: time が出ている");
    // 全ての host call が成功している。
    assert!(
        !trace.contains("\n< 1"),
        "{label}: 失敗した host call がある:\n{trace}"
    );
}

/// AssemblyScript のアプリを asc でビルドして `.wasm` を返す。
fn build_as_app(dir: &str, out: &str) -> Vec<u8> {
    // asc は同じ outFile に書くので、テストが並列に走ると書きかけを読んでしまう。
    // ビルドと読み出しをまとめて直列化する。
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
    let wasm = app.join("build").join(out);
    std::fs::read(&wasm).unwrap_or_else(|e| panic!("{} を読めない: {e}", wasm.display()))
}

#[test]
fn blink_rs_runs_on_host() {
    let wasm = build_rust_app("blink-rs");
    assert_within_feature_set(&wasm, "blink-rs");
    let trace = run(&wasm, "blink-rs");
    println!("{trace}");
    assert_blink_trace(&trace, "blink-rs");
}

#[test]
fn blink_as_runs_on_host() {
    let wasm = build_as_app("blink-as", "blink_as.wasm");
    assert_within_feature_set(&wasm, "blink-as");
    let trace = run(&wasm, "blink-as");
    println!("{trace}");
    assert_blink_trace(&trace, "blink-as");
}

/// Phase 3 の完了条件: Rust 版と AS 版が同じ GPIO トレースを出す。
#[test]
fn blink_rs_and_blink_as_agree() {
    let rs = run(&build_rust_app("blink-rs"), "blink-rs");
    let as_ = run(&build_as_app("blink-as", "blink_as.wasm"), "blink-as");

    // 比較の主眼は gpio と board の列。HANDOFF §2-10 は「全 host call 列と
    // 結果が一致」なので、要求の行だけでなく直後の結果の行（< ...）も含める。
    let pick = |t: &str| -> Vec<String> {
        let lines: Vec<&str> = t.lines().collect();
        let mut out = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            if l.contains("gpio@0.1.0") || l.contains("board@0.1.0") {
                out.push((*l).to_string());
                if let Some(next) = lines.get(i + 1)
                    && next.starts_with('<')
                {
                    out.push((*next).to_string());
                }
            }
        }
        out
    };
    assert_eq!(
        pick(&rs),
        pick(&as_),
        "Rust 版と AS 版で host call 列が違う\n--- rust ---\n{rs}\n--- as ---\n{as_}"
    );
    assert!(!pick(&rs).is_empty(), "トレースが空");
}
