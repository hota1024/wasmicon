//! Locks in that the guest build flags exist in exactly one place
//! (docs/app-workflow.md §4.4).
//!
//! The same shape as `wasmicon-gen --check`: what the source of truth
//! (`flags.rs`) writes out is compared byte for byte with the files in the
//! repository. A one-character difference in a comment fails.
//!
//! Without this, fixing `apps/` while the `wasmicon new` scaffold stays old
//! breaks silently (the build passes; it shows up only on hardware).
//!
//! The check rides on `cargo test`, so CI needs no extra job
//! (`- run: cargo test` in `.github/workflows/ci.yml`).

use std::path::{Path, PathBuf};

use wasmicon_cli::flags;
use wasmicon_port::profile;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root not found")
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

#[test]
fn apps_cargo_config_is_the_canonical_one() {
    assert_eq!(
        read("apps/.cargo/config.toml"),
        flags::CARGO_CONFIG,
        "apps/.cargo/config.toml differs from CARGO_CONFIG in \
         tools/wasmicon-cli/src/flags.rs. flags.rs is the source of truth \
         (wasmicon new writes the same bytes). To change apps/, change flags.rs"
    );
}

#[test]
fn every_asconfig_is_the_canonical_one() {
    let apps = repo_root().join("apps");
    let mut seen = 0;
    for entry in std::fs::read_dir(&apps).expect("cannot read apps/") {
        let dir = entry.expect("cannot read an entry of apps/").path();
        let conf = dir.join("asconfig.json");
        if !conf.is_file() {
            continue;
        }
        let name = dir
            .file_name()
            .and_then(|s| s.to_str())
            .expect("directory name")
            .to_string();
        // Only `outFile` differs between projects. It is derived from the
        // directory name, so a name / content mismatch fails here too.
        let want = flags::asconfig(&format!("build/{}.wasm", name.replace('-', "_")));
        let got = std::fs::read_to_string(&conf).expect("cannot read asconfig.json");
        assert_eq!(
            got, want,
            "{}/asconfig.json differs from the flags.rs template",
            name
        );
        seen += 1;
    }
    // Zero files is not a silent success (the worst case is "matched" after
    // a change in apps/ layout stopped the files from being found).
    assert!(seen >= 2, "only {seen} asconfig.json found");
}

#[test]
fn the_two_languages_ask_for_the_same_initial_memory() {
    // This is what docs/app-workflow.md §4.4 is really about. If the Rust
    // and AS versions differ in initial memory, the same app meets the
    // board limits differently.
    assert_eq!(
        flags::AS_INITIAL_MEMORY_PAGES * flags::PAGE_SIZE,
        flags::INITIAL_MEMORY,
        "the AS page count and the Rust byte count disagree"
    );
    // The numbers inside the strings match the constants (so one cannot be
    // changed without the other).
    assert!(
        flags::CARGO_CONFIG.contains(&format!("--initial-memory={}", flags::INITIAL_MEMORY)),
        "--initial-memory in CARGO_CONFIG differs from INITIAL_MEMORY"
    );
    assert!(
        flags::CARGO_CONFIG.contains(&format!("-zstack-size={}", flags::STACK_SIZE)),
        "-zstack-size in CARGO_CONFIG differs from STACK_SIZE"
    );
    assert!(
        flags::asconfig("x.wasm").contains(&format!(
            "\"initialMemory\": {}",
            flags::AS_INITIAL_MEMORY_PAGES
        )),
        "initialMemory in asconfig differs from the constant"
    );
}

#[test]
fn the_initial_memory_fits_the_tightest_board() {
    // Fits the tightest board (`ports/rp2040` allows 2 pages). Raising it
    // makes instantiate fail there (docs/TODO.md §5).
    // Do not filter on `slot.is_some()`: `slot: None` means "slot location
    // not decided yet" (ESP32-S3 was in that state at first), so a new port
    // in that state would silently drop out of the limit check while the
    // test stays green. Filter on "not host" instead (as `tests/profiles.rs`
    // does).
    let tightest = profile::PROFILES
        .iter()
        .filter(|p| p.name != "host") // the host limit is permissive
        .map(|p| p.config.max_memory_pages)
        .min()
        .expect("no profiles");
    let pages = flags::INITIAL_MEMORY / flags::PAGE_SIZE;
    assert!(
        pages <= tightest,
        "an initial {pages} pages exceeds the tightest board's {tightest} pages"
    );
}

#[test]
fn reference_types_stays_off() {
    // With it on, `call_indirect` is rejected by its table index
    // (docs/abi-spec.md §6.1). The feature is on by default, so check that
    // it is still turned off explicitly.
    assert!(
        flags::CARGO_CONFIG.contains("-Ctarget-feature=-reference-types"),
        "reference-types is not turned off"
    );
    assert!(
        flags::asconfig("x.wasm").contains("\"reference-types\""),
        "reference-types is not in the asconfig disable list"
    );
}
