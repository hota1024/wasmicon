//! Locks in that our interpreter and wasmtime produce the same trace.
//!
//! The guests are the four apps the trace-equality tests use: blink and sensor-display,
//! Rust and AS versions, plus assets-rs for `wasmicon:hal/assets@0.1.0`. This verifies
//! the interpreter against an external reference implementation, independently of any
//! hardware (docs/verification-report.md §2).

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root not found")
}

/// Builds a Rust app in the guest workspace.
fn build_rust_app(pkg: &str) -> Vec<u8> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let root = repo_root();
    // The RUSTUP_TOOLCHAIN that cargo test passes down would disable the toolchain override.
    let status = Command::new("cargo")
        .current_dir(root.join("apps"))
        .env_remove("RUSTUP_TOOLCHAIN")
        .args(["build", "--release", "-p", pkg])
        .status()
        .expect("cannot start cargo");
    assert!(status.success(), "build of {pkg} failed");
    let wasm = root
        .join("apps/target/wasm32-unknown-unknown/release")
        .join(format!("{}.wasm", pkg.replace('-', "_")));
    std::fs::read(&wasm).unwrap_or_else(|e| panic!("cannot read {}: {e}", wasm.display()))
}

/// Builds an AssemblyScript app with asc.
fn build_as_app(dir: &str, out: &str) -> Vec<u8> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let root = repo_root();
    let app = root.join("apps").join(dir);
    assert!(
        root.join("node_modules/assemblyscript").exists(),
        "AssemblyScript is not installed; run `npm install` at the repository root"
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
        .expect("cannot start npx");
    assert!(status.success(), "build of {dir} failed");
    std::fs::read(app.join("build").join(out)).expect("cannot read the build output")
}

fn sht4x_replay() -> Vec<Vec<u8>> {
    wasmicon_host::load_i2c_replay(&repo_root().join("verify/sht4x-replay.txt"))
        .expect("cannot read the recorded responses")
}

/// The two assets of `apps/assets-rs`, as `wasmicon run` loads them.
fn assets_rs_assets() -> Vec<(String, Vec<u8>)> {
    let dir = repo_root().join("apps/assets-rs/assets");
    ["hello.txt", "pattern.bin"]
        .iter()
        .map(|name| {
            let body = std::fs::read(dir.join(name))
                .unwrap_or_else(|e| panic!("cannot read the fixture {name}: {e}"));
            ((*name).to_string(), body)
        })
        .collect()
}

/// Asserts that the two engines produce identical traces with the same options.
fn assert_agrees_with(wasm: &[u8], opts: wasmicon_host::Options, label: &str) {
    let c = wasmicon_differential::compare_with(wasm, opts)
        .unwrap_or_else(|e| panic!("{label}: run failed: {e:#}"));
    if let Some((line, ours, theirs)) = c.first_difference() {
        panic!("{label}: engines differ at line {line}\n  wasmicon: {ours}\n  wasmtime: {theirs}");
    }
    assert!(c.agrees(), "{label}: traces mismatch");
    assert!(
        !c.wasmicon.is_empty(),
        "{label}: trace is empty (nothing was verified)"
    );
}

/// Asserts that the two engines produce identical traces.
fn assert_agrees(wasm: &[u8], replay: Vec<Vec<u8>>, label: &str) {
    assert_agrees_with(
        wasm,
        wasmicon_host::Options {
            i2c_replay: replay,
            ..Default::default()
        },
        label,
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
        sht4x_replay(),
        "sensor-display-rs",
    );
}

#[test]
fn sensor_display_as_agrees_with_wasmtime() {
    assert_agrees(
        &build_as_app("sensor-display-as", "sensor_display_as.wasm"),
        sht4x_replay(),
        "sensor-display-as",
    );
}

#[test]
fn assets_rs_agrees_with_wasmtime() {
    assert_agrees_with(
        &build_rust_app("assets-rs"),
        wasmicon_host::Options {
            assets: assets_rs_assets(),
            ..Default::default()
        },
        "assets-rs",
    );
}

#[test]
fn assets_as_agrees_with_wasmtime() {
    assert_agrees_with(
        &build_as_app("assets-as", "assets_as.wasm"),
        wasmicon_host::Options {
            assets: assets_rs_assets(),
            ..Default::default()
        },
        "assets-as",
    );
}
