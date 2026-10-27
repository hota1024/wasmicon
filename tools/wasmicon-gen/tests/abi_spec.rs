//! Checks that the generator's derivation matches the normative table of docs/abi-spec.md §7.
//!
//! The table is normative, so the expected values are copied from the spec, not from the WIT.
//! A failure here means the WIT, the lowering rules or the spec drifted apart.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root not found")
}

/// Copied from the v0.1 import table of docs/abi-spec.md §7.
/// Signature notation: `params:results`, `i` = i32, `I` = i64, `f` = f32, `F` = f64.
const SPEC_SECTION_7: &[(&str, &str, &str)] = &[
    // wasmicon:hal/gpio@0.1.0
    ("wasmicon:hal/gpio@0.1.0", "[static]pin.open", "iii:i"),
    ("wasmicon:hal/gpio@0.1.0", "[method]pin.set-mode", "ii:i"),
    ("wasmicon:hal/gpio@0.1.0", "[method]pin.read", "ii:i"),
    ("wasmicon:hal/gpio@0.1.0", "[method]pin.write", "ii:i"),
    ("wasmicon:hal/gpio@0.1.0", "[method]pin.toggle", "i:i"),
    ("wasmicon:hal/gpio@0.1.0", "[resource-drop]pin", "i:"),
    // wasmicon:hal/i2c@0.1.0
    ("wasmicon:hal/i2c@0.1.0", "[static]bus.open", "iii:i"),
    ("wasmicon:hal/i2c@0.1.0", "[method]bus.write", "iiii:i"),
    ("wasmicon:hal/i2c@0.1.0", "[method]bus.read", "iiiiii:i"),
    (
        "wasmicon:hal/i2c@0.1.0",
        "[method]bus.write-read",
        "iiiiiiii:i",
    ),
    ("wasmicon:hal/i2c@0.1.0", "[resource-drop]bus", "i:"),
    // wasmicon:hal/spi@0.1.0
    ("wasmicon:hal/spi@0.1.0", "[static]bus.open", "iiii:i"),
    ("wasmicon:hal/spi@0.1.0", "[method]bus.write", "iii:i"),
    ("wasmicon:hal/spi@0.1.0", "[method]bus.transfer", "iiiiii:i"),
    ("wasmicon:hal/spi@0.1.0", "[resource-drop]bus", "i:"),
    // wasmicon:hal/time@0.1.0
    ("wasmicon:hal/time@0.1.0", "now-us", ":I"),
    ("wasmicon:hal/time@0.1.0", "sleep-ms", "i:"),
    ("wasmicon:hal/time@0.1.0", "sleep-us", "i:"),
    // wasmicon:hal/log@0.1.0
    ("wasmicon:hal/log@0.1.0", "log", "iii:"),
    // wasmicon:hal/board@0.1.0
    ("wasmicon:hal/board@0.1.0", "pin-by-role", "iii:i"),
    // wasmicon:hal/assets@0.1.0
    ("wasmicon:hal/assets@0.1.0", "[static]blob.open", "iii:i"),
    ("wasmicon:hal/assets@0.1.0", "[method]blob.len", "ii:i"),
    ("wasmicon:hal/assets@0.1.0", "[method]blob.read", "iiiiii:i"),
    ("wasmicon:hal/assets@0.1.0", "[resource-drop]blob", "i:"),
];

/// Copied from the `wasmicon:device/display@0.1.0` table of docs/abi-spec.md §7.
const SPEC_SECTION_7_DEVICE: &[(&str, &str, &str)] = &[
    (
        "wasmicon:device/display@0.1.0",
        "[static]screen.open",
        "ii:i",
    ),
    (
        "wasmicon:device/display@0.1.0",
        "[method]screen.width",
        "i:i",
    ),
    (
        "wasmicon:device/display@0.1.0",
        "[method]screen.height",
        "i:i",
    ),
    (
        "wasmicon:device/display@0.1.0",
        "[method]screen.format",
        "i:i",
    ),
    (
        "wasmicon:device/display@0.1.0",
        "[method]screen.fill-rect",
        "iiiiii:i",
    ),
    (
        "wasmicon:device/display@0.1.0",
        "[method]screen.write-rect",
        "iiiiiii:i",
    ),
    (
        "wasmicon:device/display@0.1.0",
        "[method]screen.present",
        "i:i",
    ),
    (
        "wasmicon:device/display@0.1.0",
        "[resource-drop]screen",
        "i:",
    ),
];

/// The `error-code` table of docs/abi-spec.md §7. The status value is discriminant + 1.
const SPEC_ERROR_CODE: &[&str] = &[
    "InvalidArgument",
    "InvalidHandle",
    "Busy",
    "Timeout",
    "Nack",
    "Io",
    "Unsupported",
    "OutOfMemory",
    "NotFound",
];

#[test]
fn import_table_matches_abi_spec_section_7() {
    let hal = wasmicon_gen::lower::load(&repo_root().join("wit")).expect("cannot read wit/");

    let mut expected: Vec<String> = SPEC_SECTION_7
        .iter()
        .map(|(m, n, s)| format!("{m}\t{n}\t{s}"))
        .collect();
    expected.sort();

    let actual: Vec<String> = wasmicon_gen::sig_dump(&hal)
        .lines()
        .map(str::to_string)
        .collect();

    assert_eq!(
        expected, actual,
        "does not match the table of docs/abi-spec.md §7"
    );
}

#[test]
fn error_code_discriminants_match_abi_spec_section_7() {
    let hal = wasmicon_gen::lower::load(&repo_root().join("wit")).expect("cannot read wit/");
    let types = hal
        .interfaces
        .iter()
        .find(|i| i.name == "types")
        .expect("interface types is missing");
    let ec = types
        .enums
        .iter()
        .find(|e| e.wit_name == "error-code")
        .expect("error-code is missing");

    let actual: Vec<&str> = ec.cases.iter().map(|c| c.ident.as_str()).collect();
    assert_eq!(
        SPEC_ERROR_CODE,
        actual.as_slice(),
        "error-code cases are in the wrong order"
    );
}

#[test]
fn interfaces_follow_world_import_order() {
    let hal = wasmicon_gen::lower::load(&repo_root().join("wit")).expect("cannot read wit/");
    let names: Vec<&str> = hal.interfaces.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "types", "gpio", "i2c", "spi", "time", "log", "board", "assets"
        ],
        "does not follow the import order of world app"
    );
}

#[test]
fn device_import_table_matches_abi_spec_section_7() {
    let all =
        wasmicon_gen::lower::load_tree(&repo_root()).expect("cannot read wit/ and wit-device/");

    let mut expected: Vec<String> = SPEC_SECTION_7
        .iter()
        .chain(SPEC_SECTION_7_DEVICE)
        .map(|(m, n, s)| format!("{m}\t{n}\t{s}"))
        .collect();
    expected.sort();

    let actual: Vec<String> = wasmicon_gen::sig_dump(&all)
        .lines()
        .map(str::to_string)
        .collect();

    assert_eq!(
        expected, actual,
        "does not match the tables of docs/abi-spec.md §7"
    );
}

/// The device interfaces come after the HAL ones, so adding one leaves the HAL
/// part of the generated files and the host slot numbers unchanged.
#[test]
fn device_interfaces_follow_the_hal() {
    let hal = wasmicon_gen::lower::load(&repo_root().join("wit")).expect("cannot read wit/");
    let all =
        wasmicon_gen::lower::load_tree(&repo_root()).expect("cannot read wit/ and wit-device/");

    let hal_slots: Vec<&str> = hal.imports().map(|(_, f)| f.slot.as_str()).collect();
    let all_slots: Vec<&str> = all.imports().map(|(_, f)| f.slot.as_str()).collect();
    assert_eq!(&all_slots[..hal_slots.len()], hal_slots.as_slice());
    assert!(
        all.imports()
            .skip(hal_slots.len())
            .all(|(i, _)| i.module.starts_with("wasmicon:device/")),
        "a non-device interface follows the HAL"
    );
    assert_eq!(all.device_package.as_deref(), Some("wasmicon:device@0.1.0"));
}
