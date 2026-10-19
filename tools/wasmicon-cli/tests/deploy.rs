//! Locks in the pre-flash verdict of `wasmicon deploy`.
//!
//! Calling the flasher needs hardware, so only `prepare` (check and image
//! preparation) is covered. "Never flash what does not run" is the point of
//! this stage, and it is decided here.

use std::path::{Path, PathBuf};
use std::process::Command;

use wasmicon_cli::deploy;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root not found")
}

fn tmp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("deploy-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the work directory");
    dir
}

/// Builds an app in the guest workspace and returns the `.wasm`.
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
        .expect("cannot start cargo");
    assert!(status.success(), "building {pkg} failed");
    let wasm = root
        .join("apps/target/wasm32-unknown-unknown/release")
        .join(format!("{}.wasm", pkg.replace('-', "_")));
    std::fs::read(&wasm).unwrap_or_else(|e| panic!("cannot read {}: {e}", wasm.display()))
}

/// Asserts that `prepare` passes and returns the plan.
///
/// `Plan` does not implement `Debug` (`Profile` / `Slot` on the port side do
/// not), so `expect` cannot be used.
fn prepared(o: &deploy::Options) -> deploy::Plan {
    match deploy::prepare(o) {
        Ok(p) => p,
        Err(e) => panic!("expected to pass but failed: {e:#}"),
    }
}

/// Asserts that `prepare` fails and returns the reason.
fn refused(o: &deploy::Options, what: &str) -> String {
    match deploy::prepare(o) {
        Err(e) => format!("{e:#}"),
        Ok(_) => panic!("expected {what}, but it passed"),
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
    std::fs::write(&app, build("blink-rs")).expect("cannot write");

    let plan = prepared(&opts(app, Some("rp2350")));
    assert_eq!(plan.board.name, "rp2350");
    // picotool takes an address, not an offset.
    assert_eq!(plan.addr, 0x1010_0000);
    assert_eq!(plan.image_len, plan.wasm_len + 16);
    assert!(plan.image.is_file(), "the image is written next to the app");
    assert!(
        plan.image.extension().is_some_and(|e| e == "bin"),
        "picotool decides the file type by extension, so .bin \
         (docs/verification-report.md §9)"
    );
}

#[test]
fn an_app_that_cannot_run_on_the_board_is_not_flashed() {
    // The point of this stage. sensor-display uses I2C and SPI, so it does
    // not run on rp2040 (the port returns unsupported). Flashed anyway, it
    // would fail only on hardware.
    let dir = tmp("check-fails");
    let app = dir.join("sensor.wasm");
    std::fs::write(&app, build("sensor-display-rs")).expect("cannot write");

    let msg = refused(&opts(app.clone(), Some("rp2040")), "a refusal to flash");
    assert!(msg.contains("check failed"), "{msg}");
    assert!(msg.contains("rp2040"), "{msg}");
    assert!(
        !dir.join("sensor.bin").exists(),
        "no image either (it stops before flashing)"
    );

    // The same app passes on rp2350.
    prepared(&opts(app, Some("rp2350")));
}

#[test]
fn a_board_without_a_slot_is_refused() {
    // The host has no flash. The three ports have slots.
    let dir = tmp("no-slot");
    let app = dir.join("blink.wasm");
    std::fs::write(&app, build("blink-rs")).expect("cannot write");

    let msg = refused(&opts(app, Some("host")), "no slot location");
    assert!(msg.contains("not decided"), "{msg}");
}

#[test]
fn the_board_must_be_named_somewhere() {
    // With no way to know where to write, nothing is guessed.
    let dir = tmp("no-board");
    let app = dir.join("blink.wasm");
    std::fs::write(&app, build("blink-rs")).expect("cannot write");

    let msg = refused(&opts(app, None), "no target board");
    assert!(msg.contains("--board"), "{msg}");
    assert!(msg.contains("wasmicon.toml"), "{msg}");
}

/// The smallest `Options` for looking at `Mode`. The path is not used.
fn mode_opts(no_run: bool, monitor: bool) -> deploy::Options {
    deploy::Options {
        path: PathBuf::from("x.wasm"),
        board: None,
        manifest: None,
        no_run,
        monitor,
        out: None,
        port: None,
    }
}

#[test]
fn no_run_keeps_an_espflash_board_halted() {
    use wasmicon_port::profile::Flasher;

    // The point: `espflash write-bin` does `--after hard-reset` by default,
    // so without suppression the app runs right after flashing despite
    // `--no-run`, and "written; runs on the next reset" would be a lie.
    // Nobody wants that before attaching a probe or on a board wired to a
    // motor.
    let m = deploy::Mode::of(&mode_opts(true, false), Flasher::Espflash);
    assert!(m.keep_halted, "resets despite --no-run");
    assert!(!m.hand_off);

    // picotool resets in a separate call, so nothing needs suppressing.
    let m = deploy::Mode::of(&mode_opts(true, false), Flasher::Picotool);
    assert!(!m.keep_halted);
}

#[test]
fn monitor_hands_the_port_over_only_when_it_is_shared() {
    use wasmicon_port::profile::Flasher;

    // A board with a shared port is not run by the flash step; the reset is
    // left to espflash.
    let m = deploy::Mode::of(&mode_opts(false, true), Flasher::Espflash);
    assert!(m.hand_off && m.keep_halted && m.shared_port);

    // With separate ports, open the port ourselves and reset ourselves.
    let m = deploy::Mode::of(&mode_opts(false, true), Flasher::Picotool);
    assert!(!m.hand_off && !m.keep_halted && !m.shared_port);

    // With `--no-run` there is no capture (nothing runs, so nothing comes
    // out).
    let m = deploy::Mode::of(&mode_opts(true, true), Flasher::Espflash);
    assert!(!m.hand_off, "tries to capture although nothing runs");
    assert!(m.keep_halted);
}
