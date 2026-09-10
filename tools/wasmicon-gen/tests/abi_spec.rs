//! ジェネレータの導出結果が `docs/abi-spec.md` §7 の正規表と一致することを検査する。
//!
//! §7 が正なので、期待値は WIT からではなく仕様書から転記する。
//! ここが落ちたら、WIT か lowering 規則か仕様書のどれかがずれている。

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("リポジトリルートが見つからない")
}

/// abi-spec §7「v0.1 import 一覧（正規表）」からの転記。
/// sig 表記: `params:results`、`i`=i32 `I`=i64 `f`=f32 `F`=f64。
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
];

/// abi-spec §7 の error-code の表。ステータス値は discriminant + 1。
const SPEC_ERROR_CODE: &[&str] = &[
    "InvalidArgument",
    "InvalidHandle",
    "Busy",
    "Timeout",
    "Nack",
    "Io",
    "Unsupported",
    "OutOfMemory",
];

#[test]
fn import_table_matches_abi_spec_section_7() {
    let hal = wasmicon_gen::lower::load(&repo_root().join("wit")).expect("wit/ を読めない");

    let mut expected: Vec<String> = SPEC_SECTION_7
        .iter()
        .map(|(m, n, s)| format!("{m}\t{n}\t{s}"))
        .collect();
    expected.sort();

    let actual: Vec<String> = wasmicon_gen::sig_dump(&hal)
        .lines()
        .map(str::to_string)
        .collect();

    assert_eq!(expected, actual, "abi-spec §7 の表と一致しない");
}

#[test]
fn error_code_discriminants_match_abi_spec_section_7() {
    let hal = wasmicon_gen::lower::load(&repo_root().join("wit")).expect("wit/ を読めない");
    let types = hal
        .interfaces
        .iter()
        .find(|i| i.name == "types")
        .expect("interface types が無い");
    let ec = types
        .enums
        .iter()
        .find(|e| e.wit_name == "error-code")
        .expect("error-code が無い");

    let actual: Vec<&str> = ec.cases.iter().map(|c| c.ident.as_str()).collect();
    assert_eq!(
        SPEC_ERROR_CODE,
        actual.as_slice(),
        "error-code の並びが違う"
    );
}

#[test]
fn interfaces_follow_world_import_order() {
    let hal = wasmicon_gen::lower::load(&repo_root().join("wit")).expect("wit/ を読めない");
    let names: Vec<&str> = hal.interfaces.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(
        names,
        ["types", "gpio", "i2c", "spi", "time", "log"],
        "world app の import 宣言順と一致しない"
    );
}
