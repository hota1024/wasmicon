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

/// 失敗した host call が無いことを確かめる。
///
/// ステータスは discriminant + 1 なので、`< 1` だけを見ると
/// `invalid-argument` しか捕まえられない（`busy` は 3、`nack` は 5）。
fn assert_no_host_errors(trace: &str, label: &str) {
    for (i, line) in trace.lines().enumerate() {
        let Some(rest) = line.strip_prefix("< ") else {
            continue;
        };
        let code = rest.split(' ').next().unwrap_or("0");
        assert_eq!(
            code,
            "0",
            "{label}: {} 行目の host call が失敗している: {line}",
            i + 1
        );
    }
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

/// Phase 3 の完了条件: Rust 版と AS 版が同じトレースを出す。
///
/// abi-spec §9 により `time` はトレースに出ないので、**トレース全文の一致が
/// そのまま HANDOFF §2-10 の「time を除く全 host call と結果が一致」**になる。
#[test]
fn blink_rs_and_blink_as_agree() {
    let rs = run(&build_rust_app("blink-rs"), "blink-rs");
    let as_ = run(&build_as_app("blink-as", "blink_as.wasm"), "blink-as");
    assert_traces_equal(&rs, &as_);
}

/// トレースを行単位で突き合わせる。最初の食い違いだけ報告する。
fn assert_traces_equal(rs: &str, as_: &str) {
    let a: Vec<&str> = rs.lines().collect();
    let b: Vec<&str> = as_.lines().collect();
    assert!(!a.is_empty(), "トレースが空");
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(x, y, "{} 行目で食い違う（Rust 版 vs AS 版）", i + 1);
    }
    assert_eq!(a.len(), b.len(), "トレースの行数が違う");
}

/// 記録済みの SHT31 応答（`verify/sht31-replay.txt`）。
fn sht31_replay() -> Vec<Vec<u8>> {
    let path = repo_root().join("verify/sht31-replay.txt");
    wasmicon_host::load_i2c_replay(&path).expect("記録済み応答を読めない")
}

fn run_with_sensor(wasm: &[u8], label: &str) -> String {
    match wasmicon_host::run_wasm_with(wasm, true, sht31_replay()) {
        Ok(out) => out.trace,
        Err(e) => panic!("{label} の実行に失敗: {} [{}]", e.reason(), e.kind().name()),
    }
}

#[test]
fn sensor_display_rs_runs_on_host() {
    let wasm = build_rust_app("sensor-display-rs");
    assert_within_feature_set(&wasm, "sensor-display-rs");
    let trace = run_with_sensor(&wasm, "sensor-display-rs");

    // 役割名で 3 本引いている（abi-spec §8、§9 の正規化つき）。
    for role in ["lcd-cs", "lcd-dc", "lcd-rst"] {
        assert!(
            trace.contains(&format!("pin-by-role(\"{role}\")\n< 0 [role:{role}]")),
            "{role} を引いていない:\n{trace}"
        );
    }
    // SHT31 の単発計測コマンドを書いて 6 バイト読んでいる。
    assert!(
        trace.contains("[method]bus.write(1, 68, 0x2400)"),
        "SHT31 の計測コマンドが違う:\n{trace}"
    );
    assert!(
        trace.contains("[method]bus.read(1, 68, 6)\n< 0 [len=6]"),
        "SHT31 の読み出しが違う:\n{trace}"
    );
    // 背景は 240 行を 1 行ずつ送る（全画面フレームバッファを持たない）。
    assert!(
        trace
            .matches("wasmicon:hal/spi@0.1.0/[method]bus.write")
            .count()
            > 240,
        "背景の塗りつぶしが行単位で送られていない"
    );
    assert_no_host_errors(&trace, "sensor-display-rs");
}

#[test]
fn sensor_display_as_runs_on_host() {
    let wasm = build_as_app("sensor-display-as", "sensor_display_as.wasm");
    assert_within_feature_set(&wasm, "sensor-display-as");
    let trace = run_with_sensor(&wasm, "sensor-display-as");
    assert!(
        trace.contains("[method]bus.read(1, 68, 6)\n< 0 [len=6]"),
        "SHT31 の読み出しが違う:\n{trace}"
    );
    assert_no_host_errors(&trace, "sensor-display-as");
}

/// Phase 5 の完了条件の中核: Rust 版と AS 版が同じ描画をする。
///
/// `spi.write` のトレースは abi-spec §9 により data の CRC-32 なので、
/// これが全て一致すれば送っているピクセルが同一だと分かる。
/// 比較はトレース全文（= §2-10 の定義そのもの）。
#[test]
fn sensor_display_rs_and_as_agree() {
    let rs = run_with_sensor(&build_rust_app("sensor-display-rs"), "sensor-display-rs");
    let as_ = run_with_sensor(
        &build_as_app("sensor-display-as", "sensor_display_as.wasm"),
        "sensor-display-as",
    );
    assert!(
        rs.lines().count() > 1000,
        "トレースが短すぎる（{} 行）",
        rs.lines().count()
    );
    assert_traces_equal(&rs, &as_);
}

/// 失敗経路でも Rust 版と AS 版が一致すること。
///
/// Phase 5 の一致検査はハッピーパスしか通らないが、実機では
/// `ports/rp2040` / `ports/esp32s3` の SPI がまだ `unsupported` を返す。
/// そこで両言語が食い違うと、実機に持って行った瞬間に比較が意味を失う。
#[test]
fn sensor_display_agrees_when_spi_is_unsupported() {
    let opts = || wasmicon_host::Options {
        trace: true,
        i2c_replay: sht31_replay(),
        spi_unsupported: true,
    };
    let go = |wasm: &[u8], label: &str| -> String {
        match wasmicon_host::run_wasm_opts(wasm, opts()) {
            Ok(out) => out.trace,
            Err(e) => panic!("{label} の実行に失敗: {} [{}]", e.reason(), e.kind().name()),
        }
    };
    let rs = go(&build_rust_app("sensor-display-rs"), "sensor-display-rs");
    let as_ = go(
        &build_as_app("sensor-display-as", "sensor_display_as.wasm"),
        "sensor-display-as",
    );

    // SPI が開けないので、どちらも同じところで諦めるはず。
    assert!(
        rs.contains(r#"log(0, "spi open failed")"#),
        "Rust 版が spi open failed を出していない:\n{rs}"
    );
    assert!(
        rs.lines().count() < 40,
        "早期に諦めていない（{} 行）:\n{rs}",
        rs.lines().count()
    );
    assert_traces_equal(&rs, &as_);
}

/// センサーが応答しないときも一致すること（記録済み応答を渡さない）。
#[test]
fn sensor_display_agrees_when_sensor_is_silent() {
    let go = |wasm: &[u8], label: &str| -> String {
        match wasmicon_host::run_wasm_with(wasm, true, Vec::new()) {
            Ok(out) => out.trace,
            Err(e) => panic!("{label} の実行に失敗: {} [{}]", e.reason(), e.kind().name()),
        }
    };
    let rs = go(&build_rust_app("sensor-display-rs"), "sensor-display-rs");
    let as_ = go(
        &build_as_app("sensor-display-as", "sensor_display_as.wasm"),
        "sensor-display-as",
    );
    assert!(
        rs.contains(r#"log(0, "sensor read failed")"#),
        "Rust 版が sensor read failed を出していない"
    );
    assert_traces_equal(&rs, &as_);
}
