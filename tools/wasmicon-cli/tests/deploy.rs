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
    // Both regions in one UF2 file, so picotool is called once.
    assert_eq!(plan.files.len(), 1);
    let uf2 = &plan.files[0];
    assert!(
        uf2.extension().is_some_and(|e| e == "uf2"),
        "{}",
        uf2.display()
    );
    let len = std::fs::metadata(uf2)
        .expect("written next to the app")
        .len();
    // Each 256 bytes of flash take one 512-byte block.
    assert_eq!(len as usize, plan.flash_len() * 2);
    // Whole sectors, and far less than the 64 KiB slot.
    assert_eq!(plan.flash_len() % 4096, 0);
    assert!(plan.flash_len() < 64 * 1024, "{}", plan.flash_len());
    assert_eq!(
        plan.regions[0].bytes.len(),
        plan.image_len.div_ceil(4096) * 4096
    );
    assert_eq!(plan.regions[1].bytes.len(), 4096);
}

#[test]
fn an_espflash_board_gets_one_file_per_region() {
    // `espflash write-bin` takes one file at one offset.
    let dir = tmp("esp-files");
    let app = dir.join("blink.wasm");
    std::fs::write(&app, build("blink-rs")).expect("cannot write");

    let plan = prepared(&opts(app, Some("esp32s3")));
    assert_eq!(
        plan.files,
        [dir.join("blink.bin"), dir.join("blink.config.bin")]
    );
    for (f, r) in plan.files.iter().zip(&plan.regions) {
        assert_eq!(std::fs::read(f).expect("written"), r.bytes);
    }
    // The 1 MiB slot is no longer written whole.
    assert!(plan.flash_len() < 64 * 1024, "{}", plan.flash_len());
}

#[test]
fn a_small_slot_goes_to_espflash_in_one_call() {
    // The ESP32-C5's 64 KiB slot: filling the gap costs less than a second
    // connection (docs/verification-report.md §49).
    let dir = tmp("esp-merged");
    let app = dir.join("blink.wasm");
    std::fs::write(&app, build("blink-rs")).expect("cannot write");

    let plan = prepared(&opts(app, Some("esp32c5")));
    assert_eq!(plan.files, [dir.join("blink.bin")]);
    let one = std::fs::read(&plan.files[0]).expect("written");
    // From the app slot to the end of the config slot, the gap erased.
    assert_eq!(one.len(), 64 * 1024 + 4096);
    assert_eq!(plan.flash_len(), one.len());
    assert!(one[plan.image_len..64 * 1024].iter().all(|b| *b == 0xff));
    assert_eq!(&one[64 * 1024..], plan.regions[1].bytes.as_slice());
    assert!(!dir.join("blink.config.bin").exists());
}

#[test]
fn only_the_last_espflash_call_lets_the_app_run() {
    // The app must not start between the app and the config slot, and every
    // call connects with the default reset: skipping it left the ESP32-C5
    // unreachable until RESET (docs/verification-report.md §49).
    let writes = [
        (0x10_0000, "a.bin".to_string()),
        (0x20_0000, "a.config.bin".to_string()),
    ];
    let calls = deploy::espflash_calls(&writes, None, false, None);
    assert_eq!(
        calls,
        [
            vec!["write-bin", "--after", "no-reset", "0x100000", "a.bin"],
            vec!["write-bin", "0x200000", "a.config.bin"],
        ]
    );
    // `--no-run`: none of them does.
    let calls = deploy::espflash_calls(&writes, None, true, None);
    assert!(calls.iter().all(|c| c.contains(&"no-reset".to_string())));
    assert!(calls.iter().all(|c| !c.contains(&"--before".to_string())));
}

#[test]
fn picotool_loads_the_uf2_file_and_leaves_the_start_to_a_reboot() {
    // `-x` cannot start an app from a UF2 without an executable image, so
    // the board stays in BOOTSEL and `picotool reboot` starts it once.
    assert_eq!(
        deploy::picotool_load_uf2_args("app.uf2"),
        ["load", "app.uf2", "-t", "uf2", "-F"]
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

    // `picotool load -f` reboots a board it forced into BOOTSEL back into
    // the app, so it needs the same suppression (`-F`).
    let m = deploy::Mode::of(&mode_opts(true, false), Flasher::Picotool);
    assert!(m.keep_halted, "runs despite --no-run");
}

#[test]
fn picotool_starts_the_app_once_or_not_at_all() {
    // `-f` reboots a forced board back into the app and `-x` runs one that
    // was in BOOTSEL; `-F` leaves either in BOOTSEL. A separate
    // `picotool reboot` after `-f` would start the app twice, and a
    // `--monitor` capture would hold two runs.
    let run = deploy::picotool_load_args("app.bin", 0x1010_0000, false);
    assert_eq!(
        run,
        [
            "load",
            "app.bin",
            "-t",
            "bin",
            "-o",
            "0x10100000",
            "-f",
            "-x"
        ]
    );
    let halt = deploy::picotool_load_args("app.bin", 0x1010_0000, true);
    assert_eq!(
        halt,
        ["load", "app.bin", "-t", "bin", "-o", "0x10100000", "-F"]
    );
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
        !dir.join("blink.uf2").exists(),
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
            "--monitor-baud",
            "921600",
            "--port",
            "/dev/cu.usbmodem1"
        ]
    );
    assert!(deploy::espflash_monitor_args("esp32s3", None).contains(&"--no-addresses".to_string()));
}

#[test]
fn espflash_reads_at_the_firmwares_speed() {
    // espflash monitors at 115200 by default; UART0 runs at
    // `UART_BAUD`, so without this every byte is noise.
    let args = deploy::espflash_monitor_args("esp32s3", None);
    let at = args
        .iter()
        .position(|a| a == "--monitor-baud")
        .expect("--monitor-baud is passed");
    assert_eq!(args[at + 1], wasmicon_port::serial::UART_BAUD.to_string());
}
