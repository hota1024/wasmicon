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
        flash_baud: None,
        idle: None,
        timeout: None,
    }
}

#[test]
fn a_runnable_app_gets_an_image_and_a_slot() {
    let dir = tmp("ok");
    let app = dir.join("blink.wasm");
    std::fs::write(&app, build("blink-rs")).expect("cannot write");

    let plan = prepared(&opts(app, Some("rp2350")));
    assert_eq!(plan.board.name, "rp2350");
    // The app slot at 1 MiB; `write_at` turns it into the address picotool
    // takes (XIP_BASE + offset).
    assert_eq!(plan.slot.offset, 0x10_0000);
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
        flash_baud: None,
        idle: None,
        timeout: None,
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
    let m = deploy::Mode::of(
        &mode_opts(true, false),
        Flasher::Espflash { chip: "esp32s3" },
    );
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
    let m = deploy::Mode::of(
        &mode_opts(false, true),
        Flasher::Espflash { chip: "esp32s3" },
    );
    assert!(m.hand_off && m.keep_halted && m.shared_port);

    // With separate ports, open the port ourselves and reset ourselves.
    let m = deploy::Mode::of(&mode_opts(false, true), Flasher::Picotool);
    assert!(!m.hand_off && !m.keep_halted && !m.shared_port);

    // With `--no-run` there is no capture (nothing runs, so nothing comes
    // out).
    let m = deploy::Mode::of(
        &mode_opts(true, true),
        Flasher::Espflash { chip: "esp32s3" },
    );
    assert!(!m.hand_off, "tries to capture although nothing runs");
    assert!(m.keep_halted);
}

#[test]
fn flash_baud_reaches_espflash_and_only_espflash() {
    use std::path::Path;
    use wasmicon_port::profile::Flasher;

    // A 4.2 MB assets image took 202 s at espflash's default 115200
    // (docs/verification-report.md §30).
    let args = deploy::espflash_write_args(
        0x10_0000,
        "app.bin",
        Some(Path::new("/dev/cu.usbmodem1")),
        true,
        Some(921_600),
    );
    assert_eq!(
        args,
        [
            "write-bin",
            "--port",
            "/dev/cu.usbmodem1",
            "--baud",
            "921600",
            "--after",
            "no-reset",
            "0x100000",
            "app.bin"
        ]
    );
    // Without it the command is what it was.
    assert_eq!(
        deploy::espflash_write_args(0x20_1000, "assets.bin", None, false, None),
        ["write-bin", "0x201000", "assets.bin"]
    );

    // picotool has no speed; accepting one silently would hide that.
    assert!(deploy::check_flash_baud(Some(921_600), Flasher::Picotool).is_err());
    assert!(deploy::check_flash_baud(None, Flasher::Picotool).is_ok());
    assert!(deploy::check_flash_baud(Some(921_600), Flasher::Espflash { chip: "esp32s3" }).is_ok());
}

#[test]
fn a_flashing_speed_for_picotool_is_refused_before_the_image_is_written() {
    let dir = tmp("flash-baud-pico");
    let app = dir.join("blink.wasm");
    std::fs::write(&app, build("blink-rs")).expect("cannot write");

    let mut o = opts(app, Some("rp2350"));
    o.flash_baud = Some(921_600);
    let msg = refused(&o, "a refusal of --flash-baud");
    assert!(msg.contains("picotool has no speed"), "{msg}");
    assert!(
        !dir.join("blink.bin").exists(),
        "the image was written anyway"
    );
}

#[test]
fn the_capture_is_not_rewritten_by_espflash() {
    use std::path::Path;

    // Without `--no-addresses`, espflash appends a symbol line after data
    // that looks like a code address, and `trace diff` then sees lines the
    // board never printed (docs/verification-report.md §32).
    let args = deploy::espflash_monitor_args("esp32s3", Some(Path::new("/dev/cu.usbmodem1")));
    assert_eq!(
        args,
        [
            "monitor",
            "-c",
            "esp32s3",
            "--non-interactive",
            "--no-addresses",
            "--port",
            "/dev/cu.usbmodem1"
        ]
    );
    assert!(deploy::espflash_monitor_args("esp32s3", None).contains(&"--no-addresses".to_string()));
}
