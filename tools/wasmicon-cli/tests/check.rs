//! Locks in the `wasmicon check` verdicts on the real apps in `apps/`.
//!
//! Two points:
//!
//! - `blink-rs` passes on every board (it uses only gpio / time / log /
//!   board, so it runs even on rp2040, which has no SPI / I2C).
//! - `sensor-display-rs` fails only on rp2040 (I2C and SPI not implemented).
//!   That is why `check --board` exists, and a host run cannot show it.
//!
//! The guests are built the same way as in `ports/host/tests/apps.rs`.

use std::path::{Path, PathBuf};
use std::process::Command;

use wasmicon_cli::check::{self, Facts};
use wasmicon_port::profile::{self, Interfaces, Profile};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root not found")
}

/// Builds an app in the guest workspace and returns the `.wasm`.
fn build(pkg: &str) -> Vec<u8> {
    // Serializes the tests within this process.
    //
    // Races across processes are not prevented. `ports/host/tests/apps.rs`
    // builds the same `.wasm` into the same target, so a runner that runs
    // test binaries in parallel (nextest and the like) can read a file while
    // the other linker is still writing it. cargo's lock serializes builds
    // but not reads. Extracting a shared helper is docs/TODO.md §5.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let root = repo_root();
    // cargo test passes RUSTUP_TOOLCHAIN to the test process. While it is
    // set, rustup ignores the toolchain override, so the `targets` of
    // apps/rust-toolchain.toml (automatic wasm32 install) do not apply.
    // Remove it before starting.
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

fn facts_of(wasm: &[u8]) -> Facts {
    check::facts(wasm).unwrap_or_else(|e| panic!("cannot decode: {e:#}"))
}

#[test]
fn blink_rs_passes_on_every_board() {
    let wasm = build("blink-rs");
    let f = facts_of(&wasm);

    assert!(
        matches!(f.run_export, Some(Ok(()))) && f.has_memory_export,
        "run: func() and memory are present"
    );
    assert_eq!(f.import_failures(), 0, "every import matches the table");
    // blink touches only gpio / time / board.
    assert!(!f.per_iface.contains_key("i2c"));
    assert!(!f.per_iface.contains_key("spi"));

    for p in profile::PROFILES {
        let v = check::judge(&wasm, p, &f, None, &[], None);
        assert!(
            v.is_ok(),
            "fails on {} ({} failures, unimplemented {:?})",
            p.name,
            v.fails(),
            v.unimplemented
        );
    }
}

#[test]
fn sensor_display_fails_only_on_rp2040() {
    let wasm = build("sensor-display-rs");
    let f = facts_of(&wasm);

    assert_eq!(f.import_failures(), 0);
    assert!(f.per_iface.contains_key("i2c"), "reads the SHT4x over I2C");
    assert!(
        f.per_iface.contains_key("spi"),
        "draws on the ILI9341 over SPI"
    );

    let rp2040 = check::judge(&wasm, &profile::RP2040, &f, None, &[], None);
    assert!(
        !rp2040.is_ok(),
        "rp2040 fails because I2C / SPI are not implemented"
    );
    let mut unimpl = rp2040.unimplemented.clone();
    unimpl.sort();
    assert_eq!(unimpl, ["i2c", "spi"], "these two are the only reasons");
    assert!(
        rp2040.validate.is_ok(),
        "an initial 1 page fits the RP2040 2-page limit"
    );

    for p in [&profile::RP2350, &profile::ESP32S3, &profile::HOST] {
        let v = check::judge(&wasm, p, &f, None, &[], None);
        assert!(v.is_ok(), "passes on {} ({} failures)", p.name, v.fails());
    }
}

#[test]
fn memory_limit_is_per_board() {
    // "Passes on the host, fails on hardware", observed
    // (docs/app-workflow.md §4.3). A .wasm that asks for 4 pages passes on
    // host and rp2350 and fails on rp2040.
    let wasm = wat_memory_pages(4);
    let f = facts_of(&wasm);

    assert_eq!(f.mem_pages, Some(4));
    assert!(
        check::judge(&wasm, &profile::HOST, &f, None, &[], None)
            .validate
            .is_ok(),
        "host allows up to 65536 pages"
    );
    assert!(
        check::judge(&wasm, &profile::RP2350, &f, None, &[], None)
            .validate
            .is_ok(),
        "rp2350 allows up to 4 pages"
    );
    assert!(
        check::judge(&wasm, &profile::RP2040, &f, None, &[], None)
            .validate
            .is_err(),
        "rp2040 allows up to 2 pages, so validate fails"
    );
}

#[test]
fn the_psram_variant_takes_what_the_plain_esp32s3_refuses() {
    // Same circuit board, two firmwares (docs/app-workflow.md §4.3). An app
    // that has to run on both stays within the plain board's 4 pages.
    let wasm = wat_memory_pages(64);
    let f = facts_of(&wasm);
    assert!(
        check::judge(&wasm, &profile::ESP32S3, &f, None, &[], None)
            .validate
            .is_err(),
        "esp32s3 allows up to 4 pages"
    );
    let psram = check::judge(&wasm, &profile::ESP32S3_PSRAM, &f, None, &[], None);
    assert!(psram.is_ok(), "esp32s3-psram fits 64 pages");

    // The wiring comes from the board's table for both.
    assert_eq!(profile::ESP32S3_PSRAM.wiring, profile::ESP32S3.name);
}

#[test]
fn the_p4_psram_variant_fits_what_the_plain_p4_refuses() {
    // The same split on the ESP32-P4: 4 pages on the plain firmware, 240 with
    // the arena in PSRAM.
    let wasm = wat_memory_pages(240);
    let f = facts_of(&wasm);
    assert!(
        check::judge(&wasm, &profile::ESP32P4, &f, None, &[], None)
            .validate
            .is_err(),
        "esp32p4 allows up to 4 pages"
    );
    assert!(
        check::judge(&wasm, &profile::ESP32P4_PSRAM, &f, None, &[], None).is_ok(),
        "esp32p4-psram fits 240 pages"
    );
    let wasm = wat_memory_pages(241);
    let f = facts_of(&wasm);
    assert!(
        check::judge(&wasm, &profile::ESP32P4_PSRAM, &f, None, &[], None)
            .validate
            .is_err(),
        "one page over the limit"
    );
    assert_eq!(profile::ESP32P4_PSRAM.wiring, profile::ESP32P4.name);
}

#[test]
fn the_psram_variant_validates_deeply_nested_blocks() {
    // A large C `switch` nests one block per case. 200 levels are over the
    // MCU limit of 64 and within the 256 of esp32s3-psram. The check runs
    // the validator with the profile's own scratch size, so this also shows
    // that 16 KiB holds the control stack (on a 64-bit host, where it is
    // larger than on the board).
    let wasm = wat_nested_blocks(200);
    let f = facts_of(&wasm);
    assert!(
        check::judge(&wasm, &profile::ESP32S3, &f, None, &[], None)
            .validate
            .is_err(),
        "esp32s3 allows a control depth of 64"
    );
    assert!(
        check::judge(&wasm, &profile::ESP32S3_PSRAM, &f, None, &[], None).is_ok(),
        "esp32s3-psram allows a control depth of 256"
    );
    // The full 256 levels (the function body is one of them) fit as well.
    let wasm = wat_nested_blocks(255);
    let f = facts_of(&wasm);
    assert!(check::judge(&wasm, &profile::ESP32S3_PSRAM, &f, None, &[], None).is_ok());
    let wasm = wat_nested_blocks(256);
    let f = facts_of(&wasm);
    assert!(
        check::judge(&wasm, &profile::ESP32S3_PSRAM, &f, None, &[], None)
            .validate
            .is_err(),
        "one level over the limit"
    );
}

#[test]
fn the_module_section_reports_the_control_depth_a_board_must_allow() {
    // Measured under the host's limits, so a board that refuses the module
    // can say how deep it has to go (its own validator stops at its limit).
    let f = facts_of(&wat_nested_blocks(200));
    let needs = f.needs.expect("the host validates it");
    assert_eq!(needs.max_control_depth, 201, "the body's own block counts");
    assert_eq!(
        needs.over(&profile::ESP32S3.config),
        vec![("max_control_depth", 201, 64)]
    );
    assert!(needs.over(&profile::ESP32S3_PSRAM.config).is_empty());
}

#[test]
fn the_arena_is_part_of_the_judgement() {
    // validate alone is not enough. Linear memory takes the rest of the
    // arena, so even within max_memory_pages the min_pages * 64 KiB may not
    // fit in what was left. On hardware, instantiate fails there.
    //
    // A module asking for 2 pages (128 KiB) meets a board whose arena is
    // only 100 KiB. The Config stays RP2040's (2-page limit), so validate
    // passes and only instantiate fails.
    let wasm = wat_memory_pages(2);
    let tight = Profile {
        name: "tight",
        wiring: "tight",
        config: profile::RP2040.config,
        interfaces: Interfaces::ALL,
        roles: &[],
        gpio_count: 30,
        reserved: &[],
        bus_pins: &[],
        arena: 100 * 1024,
        scratch: 8 * 1024,
        slot: None,
        role_slot: None,
        assets: None,
    };
    let (stage, err) = check::instantiate_with(&wasm, &tight).expect_err("fails: not enough arena");
    assert_eq!(
        stage.label(),
        "instantiate",
        "reports instantiate, not validate ({err})"
    );
    assert!(
        err.contains("arena") || err.contains("Arena") || err.contains("out of"),
        "the message says the arena ran out: {err}"
    );

    // The same module passes with RP2040's real size (160 KiB).
    assert!(
        check::instantiate_with(&wasm, &profile::RP2040).is_ok(),
        "2 pages fit RP2040's real arena"
    );
}

#[test]
fn every_board_can_instantiate_its_advertised_maximum() {
    // Checks on every board that the advertised maximum page count really
    // fits. ESP32-S3 is the tightest (4 pages = 256 KiB in a 300 KiB arena).
    // Its DRAM is nearly used up (docs/TODO.md §1.4), so shrinking the arena
    // shows up here as `max_memory_pages: 4` becoming a lie.
    for p in profile::PROFILES {
        if p.name == "host" {
            continue; // host advertises 65536 pages but has a 256-page arena
        }
        let pages = p.config.max_memory_pages;
        let wasm = wat_memory_pages(pages);
        assert!(
            check::instantiate_with(&wasm, p).is_ok(),
            "{}: the advertised {pages} pages do not fit the {} B arena",
            p.name,
            p.arena
        );
    }
}

#[test]
fn a_board_that_copies_the_app_into_the_arena_has_less_room() {
    // ESP32-S3 copies the slot into RAM (XIP at an arbitrary offset cannot
    // be relied on). The copy goes into the same arena as linear memory, so
    // the bigger the app, the less headroom instantiate has. Without
    // modeling that, `check` passes where hardware fails, which is exactly
    // what this function exists to prevent.
    //
    // Take the maximum page count and add a custom section bigger than the
    // headroom.
    let pages = profile::ESP32S3.config.max_memory_pages;
    let padded = with_custom_section(&wat_memory_pages(pages), 40 * 1024);

    // On a board that does not copy (XIP), the app's size costs no arena,
    // so the same module passes. If this failed, the failure below would
    // prove "decode copies the bytes into the arena" instead of "the copy
    // costs arena".
    assert!(
        check::instantiate_with(&padded, &profile::RP2350).is_ok(),
        "the app's size costs arena on an XIP board"
    );

    // On a copying board it fails.
    assert!(
        check::instantiate_with(&padded, &profile::ESP32S3).is_err(),
        "the copy is not modeled (headroom overestimated by the app's size)"
    );

    // With the same page count, a small app passes (showing that size was
    // the reason).
    assert!(
        check::instantiate_with(&wat_memory_pages(pages), &profile::ESP32S3).is_ok(),
        "even a small app fails"
    );
}

/// Appends a custom section to make the `.wasm` bigger.
///
/// decode skips a name-only custom section, so this changes the file size
/// without changing the module's meaning.
fn with_custom_section(wasm: &[u8], extra: usize) -> Vec<u8> {
    let mut w = wasm.to_vec();
    // Name "pad" (length 3) + padding.
    let payload = 1 + 3 + extra;
    w.push(0x00); // section id = 0 (custom)
    let mut len = payload;
    loop {
        // Unsigned LEB128.
        let mut byte = u8::try_from(len & 0x7f).expect("7 bits");
        len >>= 7;
        if len != 0 {
            byte |= 0x80;
        }
        w.push(byte);
        if len == 0 {
            break;
        }
    }
    w.push(0x03);
    w.extend_from_slice(b"pad");
    w.resize(w.len() + extra, 0);
    w
}

#[test]
fn the_run_export_must_take_and_return_nothing() {
    // docs/abi-spec.md §3.3 says `run: func()`. A `run` with a parameter
    // fails at run time with `wrong arity`, so checking that the export
    // exists is not enough.
    let ok = wat_memory_pages(1);
    let f = facts_of(&ok);
    assert!(matches!(f.run_export, Some(Ok(()))), "() -> () passes");
    assert_eq!(f.failures(), 0);

    let bad = wat_run_with_param();
    let f = facts_of(&bad);
    assert!(
        matches!(f.run_export, Some(Err(_))),
        "a run with a parameter is rejected"
    );
    assert_eq!(f.failures(), 1, "the wrong run type counts as one failure");
}

#[test]
fn declared_roles_must_be_in_the_boards_wiring() {
    // Hardware boards have no default table (docs/app-workflow.md §3.9). A
    // declared role missing from the board's wiring table (`[board.<name>.roles]`
    // in wasmicon.toml) fails before anything is sent, instead of becoming
    // `unsupported` on hardware.
    let wasm = wat_memory_pages(1);
    let f = facts_of(&wasm);
    let declared = vec!["status-led".to_string(), "lcd-cs".to_string()];

    // Without a declaration, roles never fail the check.
    let without = check::judge(&wasm, &profile::RP2350, &f, None, &[], None);
    assert!(without.is_ok(), "no declaration, no role failure");

    // When the wiring table has only one of the two, the other one fails.
    let provided = vec!["status-led".to_string()];
    let with = check::judge(
        &wasm,
        &profile::RP2350,
        &f,
        Some(&declared),
        &provided,
        None,
    );
    assert_eq!(with.missing_declared_roles, ["lcd-cs"]);
    assert!(!with.is_ok(), "a declared role that is not provided fails");
    assert_eq!(with.fails(), 1);

    // A hardware board hands out nothing without a manifest.
    assert!(check::provided_roles(&profile::RP2350, None).is_empty());
    // The host hands out the mock's table.
    let host = check::provided_roles(&profile::HOST, None);
    assert!(host.contains(&"lcd-cs".to_string()), "{host:?}");
}

/// A module whose `run` is `(i32) -> ()`, violating docs/abi-spec.md §3.3.
fn wat_run_with_param() -> Vec<u8> {
    let mut w = Vec::new();
    w.extend_from_slice(b"\0asm");
    w.extend_from_slice(&1u32.to_le_bytes());
    // type: (i32) -> ()
    w.extend_from_slice(&[0x01, 0x05, 0x01, 0x60, 0x01, 0x7f, 0x00]);
    w.extend_from_slice(&[0x03, 0x02, 0x01, 0x00]);
    w.extend_from_slice(&[0x05, 0x03, 0x01, 0x00, 0x01]);
    let exports: &[u8] = &[
        0x02, 0x03, b'r', b'u', b'n', 0x00, 0x00, 0x06, b'm', b'e', b'm', b'o', b'r', b'y', 0x02,
        0x00,
    ];
    w.push(0x07);
    w.push(u8::try_from(exports.len()).expect("export section too large"));
    w.extend_from_slice(exports);
    w.extend_from_slice(&[0x0a, 0x04, 0x01, 0x02, 0x00, 0x0b]);
    w
}

/// Builds `(module (memory n) (export "memory" ...) (func (export "run")))`
/// by hand: the smallest `.wasm` without pulling in a wat assembler.
fn wat_memory_pages(pages: u32) -> Vec<u8> {
    let mut w = Vec::new();
    w.extend_from_slice(b"\0asm");
    w.extend_from_slice(&1u32.to_le_bytes());

    // type: () -> ()
    w.extend_from_slice(&[0x01, 0x04, 0x01, 0x60, 0x00, 0x00]);
    // func: one, of type 0
    w.extend_from_slice(&[0x03, 0x02, 0x01, 0x00]);
    // memory: min = pages, no max
    let mut limits = vec![0x01, 0x00]; // one memory, no max
    leb128(&mut limits, pages as usize);
    w.push(0x05);
    leb128(&mut w, limits.len());
    w.extend_from_slice(&limits);
    // export: "run" = func 0, "memory" = mem 0
    let exports: &[u8] = &[
        0x02, // 2 entries
        0x03, b'r', b'u', b'n', 0x00, 0x00, //
        0x06, b'm', b'e', b'm', b'o', b'r', b'y', 0x02, 0x00,
    ];
    w.push(0x07);
    w.push(u8::try_from(exports.len()).expect("export section too large"));
    w.extend_from_slice(exports);
    // code: an empty body
    w.extend_from_slice(&[0x0a, 0x04, 0x01, 0x02, 0x00, 0x0b]);
    w
}

/// The module of `wat_memory_pages(1)` with `run` holding `depth` nested
/// empty `block`s, the shape a large C `switch` lowers to.
fn wat_nested_blocks(depth: usize) -> Vec<u8> {
    let mut w = wat_memory_pages(1);
    // Drop the empty code section and write one with the nested body.
    w.truncate(w.len() - 6);
    let mut body = vec![0x00]; // no locals
    for _ in 0..depth {
        body.extend_from_slice(&[0x02, 0x40]); // block (empty type)
    }
    body.extend(std::iter::repeat_n(0x0b, depth + 1)); // the blocks' and the body's `end`
    let mut code = vec![0x01]; // one body
    leb128(&mut code, body.len());
    code.extend_from_slice(&body);
    w.push(0x0a);
    leb128(&mut w, code.len());
    w.extend_from_slice(&code);
    w
}

fn leb128(out: &mut Vec<u8>, mut v: usize) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// A display app (docs/device-layer.md §6): rp2040 has no display at all, a
/// board with one needs its `[board.<name>.display]` entry, and the host's
/// mock is built in.
#[test]
fn display_demo_needs_a_display_entry_on_a_board() {
    use wasmicon_port::roles::DisplayConfig;

    let wasm = build("display-demo-rs");
    let f = facts_of(&wasm);
    let lcd = DisplayConfig::Ili9341 {
        spi: 0,
        cs: 17,
        dc: 20,
        rst: 21,
        rotation: 90,
    };

    let rp2040 = check::judge(&wasm, &profile::RP2040, &f, None, &[], Some(lcd));
    assert_eq!(rp2040.unimplemented, ["display"]);
    assert!(rp2040.validate.is_ok(), "only the display row fails");
    assert_eq!(rp2040.fails(), 1);

    let without = check::judge(&wasm, &profile::RP2350, &f, None, &[], None);
    assert!(matches!(without.display, Some(Err(_))));
    assert_eq!(without.fails(), 1);

    let with = check::judge(&wasm, &profile::RP2350, &f, None, &[], Some(lcd));
    assert!(with.is_ok(), "{} failures", with.fails());

    // The Tab5 panel's driver is in the ESP32-P4 firmware only: elsewhere
    // naming it fails here instead of `open` returning `unsupported`.
    let tab5 = check::judge(
        &wasm,
        &profile::ESP32P4,
        &f,
        None,
        &[],
        Some(DisplayConfig::Tab5),
    );
    assert!(tab5.is_ok(), "{} failures", tab5.fails());
    let tab5_s3 = check::judge(
        &wasm,
        &profile::ESP32S3,
        &f,
        None,
        &[],
        Some(DisplayConfig::Tab5),
    );
    assert!(matches!(tab5_s3.display, Some(Err(_))));
    assert_eq!(tab5_s3.fails(), 1);

    let host = check::judge(&wasm, &profile::HOST, &f, None, &[], None);
    assert!(host.is_ok(), "the host mock needs no entry");
}
