//! Runs the guests in `apps/` on the host port and cross-checks their traces.
//!
//! Locks in that every app's Wasm stays within the supported feature set
//! (docs/decisions.md D-07) and that the Rust, AssemblyScript and C versions
//! of one app produce identical traces (docs/decisions.md D-10).

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root not found")
}

/// Builds an app in the guest workspace and returns the `.wasm`.
fn build_rust_app(pkg: &str) -> Vec<u8> {
    // Several tests run cargo against the same target directory. cargo's own
    // lock serializes the builds, but another test can read a half-written
    // .wasm in between. Serialize the build and the read together.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let root = repo_root();
    // cargo test passes RUSTUP_TOOLCHAIN to the test process. With it set,
    // rustup ignores every toolchain override file, so the targets in
    // apps/rust-toolchain.toml (automatic wasm32-unknown-unknown install) have
    // no effect. Remove it before launching.
    let status = Command::new("cargo")
        .current_dir(root.join("apps"))
        .env_remove("RUSTUP_TOOLCHAIN")
        .args(["build", "--release", "-p", pkg])
        .status()
        .expect("cannot run cargo");
    assert!(status.success(), "build of {pkg} failed");
    let wasm = root
        .join("apps/target/wasm32-unknown-unknown/release")
        .join(format!("{}.wasm", pkg.replace('-', "_")));
    std::fs::read(&wasm).unwrap_or_else(|e| panic!("cannot read {}: {e}", wasm.display()))
}

/// Checks that the Wasm stays within the supported feature set (docs/decisions.md D-07).
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
        .expect("cannot run wasm-tools");
    assert!(
        ok.success(),
        "{label} contains an instruction outside the supported feature set"
    );
}

/// Checks that no host call failed.
///
/// The status is discriminant + 1, so looking only for `< 1` would catch
/// `invalid-argument` alone (`busy` is 3, `nack` is 5).
fn assert_no_host_errors(trace: &str, label: &str) {
    for (i, line) in trace.lines().enumerate() {
        let Some(rest) = line.strip_prefix("< ") else {
            continue;
        };
        let code = rest.split(' ').next().unwrap_or("0");
        assert_eq!(
            code,
            "0",
            "{label}: host call at line {} failed: {line}",
            i + 1
        );
    }
}

fn run(wasm: &[u8], label: &str) -> String {
    match wasmicon_host::run_wasm(wasm, true) {
        Ok(out) => out.trace,
        Err(e) => panic!(
            "{label} execution failed: {} [{}]",
            e.reason(),
            e.kind().name()
        ),
    }
}

/// Checks that blink emits the expected host call sequence.
fn assert_blink_trace(trace: &str, label: &str) {
    // A pin number looked up by role is board-specific, so the trace shows the
    // role name (docs/abi-spec.md §9).
    assert!(
        trace.contains("> wasmicon:hal/board@0.1.0/pin-by-role(\"led\")\n< 0 [role:led]"),
        "{label}: wrong pin-by-role trace:\n{trace}"
    );
    assert!(
        trace.contains("> wasmicon:hal/gpio@0.1.0/[static]pin.open(role:led, 3)\n< 0 [1]"),
        "{label}: wrong pin.open trace:\n{trace}"
    );
    assert_eq!(
        trace.matches("[method]pin.write(1, 1)").count(),
        3,
        "{label}: wrong pin.write count:\n{trace}"
    );
    assert_eq!(
        trace.matches("[method]pin.toggle(1)").count(),
        3,
        "{label}: wrong pin.toggle count:\n{trace}"
    );
    assert!(
        trace.contains("[resource-drop]pin(1)"),
        "{label}: missing drop trace:\n{trace}"
    );
    // time is not traced (docs/abi-spec.md §9).
    assert!(
        !trace.contains("time@0.1.0"),
        "{label}: time appears in the trace"
    );
    // Every host call succeeded.
    assert!(
        !trace.contains("\n< 1"),
        "{label}: a host call failed:\n{trace}"
    );
}

/// Builds an AssemblyScript app with asc and returns the `.wasm`.
fn build_as_app(dir: &str, out: &str) -> Vec<u8> {
    // asc writes the same outFile, so parallel tests would read a half-written
    // file. Serialize the build and the read together.
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
        .expect("cannot run npx");
    assert!(status.success(), "build of {dir} failed");
    let wasm = app.join("build").join(out);
    std::fs::read(&wasm).unwrap_or_else(|e| panic!("cannot read {}: {e}", wasm.display()))
}

/// Builds a C app with `bindings/c/build.sh` and returns the `.wasm`.
///
/// The sources are `apps/<dir>/src/*.c`. `WASMICON_CLANG` overrides the clang used.
fn build_c_app(dir: &str, out: &str) -> Vec<u8> {
    // Same output path, so serialize the build and the read together (as in build_as_app).
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let root = repo_root();
    let app = root.join("apps").join(dir);
    let mut srcs: Vec<PathBuf> = std::fs::read_dir(app.join("src"))
        .unwrap_or_else(|e| panic!("cannot read {dir}/src: {e}"))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "c"))
        .collect();
    srcs.sort();
    let build = app.join("build");
    std::fs::create_dir_all(&build).unwrap();
    let wasm = build.join(out);
    let status = Command::new("sh")
        .arg(root.join("bindings/c/build.sh"))
        .arg(&wasm)
        .args(&srcs)
        .status()
        .expect("cannot run sh");
    assert!(
        status.success(),
        "build of {dir} failed (needs a clang with wasm32 support and wasm-ld; see bindings/c/build.sh)"
    );
    std::fs::read(&wasm).unwrap_or_else(|e| panic!("cannot read {}: {e}", wasm.display()))
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

/// The Rust and AS versions produce the same trace.
///
/// `time` is not traced (docs/abi-spec.md §9), so equality of the whole trace
/// is exactly "every host call and result except time matches"
/// (docs/decisions.md D-10).
#[test]
fn blink_rs_and_blink_as_agree() {
    let rs = run(&build_rust_app("blink-rs"), "blink-rs");
    let as_ = run(&build_as_app("blink-as", "blink_as.wasm"), "blink-as");
    assert_traces_equal(&rs, &as_);
}

#[test]
fn blink_c_runs_on_host() {
    let wasm = build_c_app("blink-c", "blink_c.wasm");
    assert_within_feature_set(&wasm, "blink-c");
    let trace = run(&wasm, "blink-c");
    println!("{trace}");
    assert_blink_trace(&trace, "blink-c");
}

/// The C version produces the same trace as the Rust version (checks `bindings/c`).
#[test]
fn blink_rs_and_blink_c_agree() {
    let rs = run(&build_rust_app("blink-rs"), "blink-rs");
    let c = run(&build_c_app("blink-c", "blink_c.wasm"), "blink-c");
    assert_traces_equal(&rs, &c);
}

/// Cross-checks two traces line by line and reports only the first mismatch.
fn assert_traces_equal(rs: &str, as_: &str) {
    let a: Vec<&str> = rs.lines().collect();
    let b: Vec<&str> = as_.lines().collect();
    assert!(!a.is_empty(), "trace is empty");
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(
            x,
            y,
            "mismatch at line {} (Rust version vs the other)",
            i + 1
        );
    }
    assert_eq!(a.len(), b.len(), "trace line counts differ");
}

/// Size of the host trace that the hardware records were cross-checked
/// against (docs/verification-report.md §6, §7).
///
/// README.md, docs/TODO.md and docs/verification-report.md quote these two
/// numbers as they are. The point is to fail here when the reference side
/// moves; when it does, fix those three documents along with the numbers.
const LCD_DEMO_TRACE_LINES: usize = 14_352;
const LCD_DEMO_SPI_WRITES: usize = 3_272;

/// `lcd-demo-rs` (the SPI-and-GPIO-only demo) runs to completion on the host.
///
/// The hardware records (docs/verification-report.md §6, §7) were
/// cross-checked against this trace. Without running it in CI, a silent
/// change of the reference side would go unnoticed.
#[test]
fn lcd_demo_rs_runs_on_host() {
    let wasm = build_rust_app("lcd-demo-rs");
    assert_within_feature_set(&wasm, "lcd-demo-rs");
    let trace = run(&wasm, "lcd-demo-rs");

    // Three pins are looked up by role (docs/abi-spec.md §8, §9).
    for role in ["lcd-cs", "lcd-dc", "lcd-rst"] {
        assert!(
            trace.contains(&format!("pin-by-role(\"{role}\")\n< 0 [role:{role}]")),
            "{role} was not looked up:\n{trace}"
        );
    }
    // The app exists to avoid needing the sensor, so it never touches I2C.
    assert!(!trace.contains("i2c@0.1.0"), "touches i2c:\n{trace}");
    // The background goes out as 240 rows, one at a time (no full-screen framebuffer).
    assert!(
        trace
            .matches("wasmicon:hal/spi@0.1.0/[method]bus.write")
            .count()
            > 240,
        "not sent row by row:\n{trace}"
    );
    // The reference itself. Must equal the numbers the documents quote.
    assert_eq!(
        trace.lines().count(),
        LCD_DEMO_TRACE_LINES,
        "trace line count differs from the hardware record; the numbers in README.md, \
         docs/TODO.md and docs/verification-report.md need updating too"
    );
    assert_eq!(
        trace
            .matches("wasmicon:hal/spi@0.1.0/[method]bus.write")
            .count(),
        LCD_DEMO_SPI_WRITES,
        "spi.write count differs from the hardware record (the number of CRC-32s cross-checked)"
    );
    // Runs to the end. Giving up midway would not print done (Level::Info = 2).
    assert!(
        trace.contains(r#"log(2, "lcd-demo done")"#),
        "did not run to the end:\n{trace}"
    );
    // The release order is spi, rst, dc, cs per apps/README.md §4.
    let at = |needle: &str| {
        trace
            .find(needle)
            .unwrap_or_else(|| panic!("missing {needle}:\n{trace}"))
    };
    let order = [
        "wasmicon:hal/spi@0.1.0/[resource-drop]bus(1)",
        "wasmicon:hal/gpio@0.1.0/[resource-drop]pin(3)",
        "wasmicon:hal/gpio@0.1.0/[resource-drop]pin(2)",
        "wasmicon:hal/gpio@0.1.0/[resource-drop]pin(1)",
    ];
    for pair in order.windows(2) {
        assert!(
            at(pair[0]) < at(pair[1]),
            "release order differs from apps/README.md §4 ({} comes after {}):\n{trace}",
            pair[0],
            pair[1]
        );
    }
    assert_no_host_errors(&trace, "lcd-demo-rs");
}

/// I2C address of the SHT4x (apps/README.md §1). Traces print it in decimal.
///
/// It depends on the part suffix (-AD1B is 0x44). Change this along with the
/// guest's `ADDRESS`. It is collected here so the address appears in only
/// three places: the Rust guest, the AS guest and this constant.
const SHT4X_ADDR: u16 = 0x44;

/// Recorded SHT4x responses (`verify/sht4x-replay.txt`).
fn sht4x_replay() -> Vec<Vec<u8>> {
    let path = repo_root().join("verify/sht4x-replay.txt");
    wasmicon_host::load_i2c_replay(&path).expect("cannot read the recorded responses")
}

fn run_with_sensor(wasm: &[u8], label: &str) -> String {
    match wasmicon_host::run_wasm_with(wasm, true, sht4x_replay()) {
        Ok(out) => out.trace,
        Err(e) => panic!(
            "{label} execution failed: {} [{}]",
            e.reason(),
            e.kind().name()
        ),
    }
}

#[test]
fn sensor_display_rs_runs_on_host() {
    let wasm = build_rust_app("sensor-display-rs");
    assert_within_feature_set(&wasm, "sensor-display-rs");
    let trace = run_with_sensor(&wasm, "sensor-display-rs");

    // Three pins are looked up by role (docs/abi-spec.md §8, §9).
    for role in ["lcd-cs", "lcd-dc", "lcd-rst"] {
        assert!(
            trace.contains(&format!("pin-by-role(\"{role}\")\n< 0 [role:{role}]")),
            "{role} was not looked up:\n{trace}"
        );
    }
    // Writes the SHT4x single-shot measurement command (high precision, 1 byte)
    // and reads 6 bytes.
    assert!(
        trace.contains(&format!("[method]bus.write(1, {SHT4X_ADDR}, 0xfd)")),
        "wrong SHT4x measurement command:\n{trace}"
    );
    assert!(
        trace.contains(&format!(
            "[method]bus.read(1, {SHT4X_ADDR}, 6)\n< 0 [len=6 data=0x6754c0a4d840]"
        )),
        "wrong SHT4x read:\n{trace}"
    );
    // The background goes out as 240 rows, one at a time (no full-screen framebuffer).
    assert!(
        trace
            .matches("wasmicon:hal/spi@0.1.0/[method]bus.write")
            .count()
            > 240,
        "background fill is not sent row by row"
    );
    assert_no_host_errors(&trace, "sensor-display-rs");
}

#[test]
fn sensor_display_as_runs_on_host() {
    let wasm = build_as_app("sensor-display-as", "sensor_display_as.wasm");
    assert_within_feature_set(&wasm, "sensor-display-as");
    let trace = run_with_sensor(&wasm, "sensor-display-as");
    // Mirrors the Rust version (`sensor_display_rs_runs_on_host`), so a
    // regression of one version alone to the 2-byte SHT3x command is caught here.
    assert!(
        trace.contains(&format!("[method]bus.write(1, {SHT4X_ADDR}, 0xfd)")),
        "wrong SHT4x measurement command:\n{trace}"
    );
    assert!(
        trace.contains(&format!(
            "[method]bus.read(1, {SHT4X_ADDR}, 6)\n< 0 [len=6 data=0x6754c0a4d840]"
        )),
        "wrong SHT4x read:\n{trace}"
    );
    assert_no_host_errors(&trace, "sensor-display-as");
}

/// The Rust and AS versions draw the same picture.
///
/// The `spi.write` trace is the CRC-32 of the data (docs/abi-spec.md §9), so
/// when all of them match the pixels sent are identical. The comparison is
/// the whole trace, which is the docs/decisions.md D-10 definition itself.
#[test]
fn sensor_display_rs_and_as_agree() {
    let rs = run_with_sensor(&build_rust_app("sensor-display-rs"), "sensor-display-rs");
    let as_ = run_with_sensor(
        &build_as_app("sensor-display-as", "sensor_display_as.wasm"),
        "sensor-display-as",
    );
    assert!(
        rs.lines().count() > 1000,
        "trace too short ({} lines)",
        rs.lines().count()
    );
    assert_traces_equal(&rs, &as_);
}

/// The Rust and AS versions agree at the humidity clamp.
///
/// The one record in `verify/sht4x-replay.txt` (raw_h = 0xa4d8 -> 74.49%) is
/// far from both ends of the clamp, so `sensor_display_rs_and_as_agree` takes
/// neither clamp branch: dropping the clamp in one language would still pass
/// CI. Synthetic responses that hit the bounds are checked separately
/// (closing the asymmetry in docs/TODO.md §2.2).
///
/// A response with a wrong CRC-8 makes the guest bail out with `sensor crc
/// failed` before reaching the clamp, so bytes 3 and 6 are computed with poly
/// 0x31 / init 0xFF. The temperature is fixed at 23.44 deg C (raw_t = 0x6421)
/// and only the humidity varies.
#[test]
fn sensor_display_agrees_at_humidity_clamp_bounds() {
    let cases: [(&str, [u8; 6]); 3] = [
        // Raw formula gives -600. Lower clamp.
        ("raw_h=0", [0x64, 0x21, 0xe0, 0x00, 0x00, 0x81]),
        // Raw formula gives -1. Just inside the lower bound (sign handling differs easily).
        ("raw_h=3145", [0x64, 0x21, 0xe0, 0x0c, 0x49, 0x80]),
        // Raw formula gives 11900. Upper clamp.
        ("raw_h=65535", [0x64, 0x21, 0xe0, 0xff, 0xff, 0xac]),
    ];
    let rs_wasm = build_rust_app("sensor-display-rs");
    let as_wasm = build_as_app("sensor-display-as", "sensor_display_as.wasm");
    for (label, frame) in cases {
        let go = |wasm: &[u8], lang: &str| -> String {
            match wasmicon_host::run_wasm_with(wasm, true, vec![frame.to_vec()]) {
                Ok(out) => out.trace,
                Err(e) => panic!(
                    "{lang} / {label} execution failed: {} [{}]",
                    e.reason(),
                    e.kind().name()
                ),
            }
        };
        let rs = go(&rs_wasm, "sensor-display-rs");
        let as_ = go(&as_wasm, "sensor-display-as");
        // A broken synthetic response bails out at the CRC and matches without
        // ever reaching the clamp.
        assert!(
            !rs.contains(r#"log(0, "sensor crc failed")"#),
            "{label}: wrong CRC in the synthetic response (a test bug):\n{rs}"
        );
        assert_traces_equal(&rs, &as_);
    }
}

/// The Rust and AS versions agree across temperatures.
///
/// The replay's 25.63 deg C is a positive 5-character value whose gauge stops
/// partway through the second segment. It never hits a negative temperature
/// (6 characters, centered), the edge where no gauge is drawn or a fill that
/// spans segments, so those are checked separately. The humidity is fixed at
/// 51.08% (raw_h = 0x74e9).
#[test]
fn sensor_display_agrees_across_temperatures() {
    let cases: [(&str, [u8; 6]); 5] = [
        // -45.00 deg C. bar_px == 0, no gauge drawn.
        ("raw_t=0", [0x00, 0x00, 0x81, 0x74, 0xe9, 0x70]),
        // -40.01 deg C. Partway through the first segment.
        ("raw_t=1872", [0x07, 0x50, 0x51, 0x74, 0xe9, 0x70]),
        // -1.25 deg C. Negative with a one-digit integer part.
        ("raw_t=16384", [0x40, 0x00, 0x08, 0x74, 0xe9, 0x70]),
        // 60.00 deg C. Spans all four segments.
        ("raw_t=39321", [0x99, 0x99, 0xbe, 0x74, 0xe9, 0x70]),
        // 130.00 deg C. The SHT4x maximum, 6 characters.
        ("raw_t=65535", [0xff, 0xff, 0xac, 0x74, 0xe9, 0x70]),
    ];
    let rs_wasm = build_rust_app("sensor-display-rs");
    let as_wasm = build_as_app("sensor-display-as", "sensor_display_as.wasm");
    for (label, frame) in cases {
        let go = |wasm: &[u8], lang: &str| -> String {
            match wasmicon_host::run_wasm_with(wasm, true, vec![frame.to_vec()]) {
                Ok(out) => out.trace,
                Err(e) => panic!(
                    "{lang} / {label} execution failed: {} [{}]",
                    e.reason(),
                    e.kind().name()
                ),
            }
        };
        let rs = go(&rs_wasm, "sensor-display-rs");
        let as_ = go(&as_wasm, "sensor-display-as");
        assert!(
            !rs.contains(r#"log(0, "sensor crc failed")"#),
            "{label}: wrong CRC in the synthetic response (a test bug):\n{rs}"
        );
        // An off-screen rectangle is rejected inside the guest and never becomes
        // a host call, so assert_no_host_errors cannot see it. Check the log.
        assert!(
            !rs.contains(r#"log(0, "display failed")"#),
            "{label}: drawing does not fit the screen:\n{rs}"
        );
        assert_no_host_errors(&rs, label);
        assert_traces_equal(&rs, &as_);
    }
}

/// The Rust and AS versions agree on the failure path too.
///
/// The equality tests above cover only the happy path, but `ports/rp2040`
/// still returns `unsupported` for SPI (docs/TODO.md §1.2). If the two
/// languages diverged there, the comparison would lose its meaning the moment
/// the app reached that board.
#[test]
fn sensor_display_agrees_when_spi_is_unsupported() {
    let opts = || wasmicon_host::Options {
        trace: true,
        i2c_replay: sht4x_replay(),
        spi_unsupported: true,
        assets: Vec::new(),
    };
    let go = |wasm: &[u8], label: &str| -> String {
        match wasmicon_host::run_wasm_opts(wasm, opts()) {
            Ok(out) => out.trace,
            Err(e) => panic!(
                "{label} execution failed: {} [{}]",
                e.reason(),
                e.kind().name()
            ),
        }
    };
    let rs = go(&build_rust_app("sensor-display-rs"), "sensor-display-rs");
    let as_ = go(
        &build_as_app("sensor-display-as", "sensor_display_as.wasm"),
        "sensor-display-as",
    );

    // SPI cannot be opened, so both should give up at the same point.
    assert!(
        rs.contains(r#"log(0, "spi open failed")"#),
        "Rust version did not print spi open failed:\n{rs}"
    );
    assert!(
        rs.lines().count() < 40,
        "did not give up early ({} lines):\n{rs}",
        rs.lines().count()
    );
    assert_traces_equal(&rs, &as_);
}

/// The versions agree when the sensor does not respond (no recorded responses).
#[test]
fn sensor_display_agrees_when_sensor_is_silent() {
    let go = |wasm: &[u8], label: &str| -> String {
        match wasmicon_host::run_wasm_with(wasm, true, Vec::new()) {
            Ok(out) => out.trace,
            Err(e) => panic!(
                "{label} execution failed: {} [{}]",
                e.reason(),
                e.kind().name()
            ),
        }
    };
    let rs = go(&build_rust_app("sensor-display-rs"), "sensor-display-rs");
    let as_ = go(
        &build_as_app("sensor-display-as", "sensor_display_as.wasm"),
        "sensor-display-as",
    );
    assert!(
        rs.contains(r#"log(0, "sensor read failed")"#),
        "Rust version did not print sensor read failed"
    );
    assert_traces_equal(&rs, &as_);
}

/// The two assets of `apps/assets-rs`, as `wasmicon run` loads them from its
/// `wasmicon.toml`.
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

/// assets-rs reads a text and a 4,100-byte pattern through
/// `wasmicon:hal/assets@0.1.0` and checks every byte (docs/app-workflow.md §3.10).
#[test]
fn assets_rs_runs_on_host() {
    let wasm = build_rust_app("assets-rs");
    assert_within_feature_set(&wasm, "assets-rs");
    let opts = wasmicon_host::Options {
        trace: true,
        assets: assets_rs_assets(),
        ..Default::default()
    };
    let trace = match wasmicon_host::run_wasm_opts(&wasm, opts) {
        Ok(out) => out.trace,
        Err(e) => panic!(
            "assets-rs execution failed: {} [{}]",
            e.reason(),
            e.kind().name()
        ),
    };
    println!("{trace}");
    assert_no_host_errors(&trace, "assets-rs");

    // hello.txt: opened, measured, read whole and logged as text.
    assert!(
        trace.contains("> wasmicon:hal/assets@0.1.0/[static]blob.open(\"hello.txt\")\n< 0 [1]"),
        "{trace}"
    );
    assert!(trace.contains("[method]blob.len(1)\n< 0 [27]"), "{trace}");
    assert!(
        trace.contains("[method]blob.read(1, 0, 27)\n< 0 [len=27 data=0x"),
        "{trace}"
    );
    assert!(
        trace.contains(r#"log(2, "Hello from the assets slot.")"#),
        "{trace}"
    );
    // pattern.bin takes the same handle once hello.txt is dropped, and is
    // read in five 1,024-byte chunks (the last one 4 bytes), crossing the
    // 4 KiB boundary of the slot.
    assert!(
        trace.contains("[static]blob.open(\"pattern.bin\")\n< 0 [1]"),
        "{trace}"
    );
    assert_eq!(
        trace.matches("[method]blob.read(").count(),
        1 + 5,
        "{trace}"
    );
    assert!(
        trace.contains("[method]blob.read(1, 4096, 4)\n< 0 [len=4 data=0x"),
        "{trace}"
    );
    assert!(trace.contains(r#"log(2, "pattern ok")"#), "{trace}");
    assert_eq!(trace.matches("[resource-drop]blob(1)").count(), 2);
}

/// Without the assets the app reports the missing one and returns; it must not trap.
#[test]
fn assets_rs_without_the_assets_logs_and_exits() {
    let wasm = build_rust_app("assets-rs");
    let trace = run(&wasm, "assets-rs");
    println!("{trace}");
    assert!(
        trace.contains("[static]blob.open(\"hello.txt\")\n< 9\n"),
        "not-found: {trace}"
    );
    assert!(trace.contains(r#"log(0, "no asset hello.txt")"#), "{trace}");
    assert!(
        !trace.contains("pattern.bin"),
        "stops at the first missing asset: {trace}"
    );
}
