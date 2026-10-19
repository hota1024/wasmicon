//! Source of truth for the guest build flags (docs/app-workflow.md §4.4).
//!
//! The app contract lives in two files, `.cargo/config.toml` and
//! `asconfig.json`, and each exists three times: in `apps/`, in the
//! `wasmicon new` scaffolds and here. A mismatch still builds; it shows up
//! only on hardware ("call_indirect rejected by its table index", "over the
//! RP2040 two-page limit").
//!
//! So this module holds the file contents themselves, not a table of values,
//! and `wasmicon new` only writes them out. `tools/wasmicon-cli/tests/flags.rs`
//! compares the copies in `apps/` byte for byte (the same shape as
//! `wasmicon-gen --check`; the check rides on the `cargo test` job CI already
//! has, so no new job is needed).
//!
//! A one-character difference in a comment fails the test. Relaxing that would
//! bring back "the values agree but nobody knows which copy is the truth", so
//! it is strict on purpose.
//!
//! Not covered: `[profile.release]` (`opt-level = "z"` and the like) only
//! affects size. A mismatch there makes the app bigger but does not break it
//! silently. Only the two files docs/app-workflow.md §4.4 names are covered.

/// Initial size of linear memory in bytes. Must fit the tightest board
/// (`ports/rp2040` allows 2 pages). `tests/flags.rs` cross-checks it against
/// `profile::PROFILES`.
pub const INITIAL_MEMORY: u32 = 65_536;

/// Guest shadow stack in bytes.
pub const STACK_SIZE: u32 = 8_192;

/// One Wasm page.
pub const PAGE_SIZE: u32 = 65_536;

/// Initial memory on the AssemblyScript side, in pages.
///
/// If this does not equal `INITIAL_MEMORY` in pages, the Rust and AS versions
/// of the same app get different initial linear memory sizes. That is what
/// docs/app-workflow.md §4.4 is really about.
pub const AS_INITIAL_MEMORY_PAGES: u32 = INITIAL_MEMORY / PAGE_SIZE;

/// Contents of `.cargo/config.toml`. Byte-identical to `apps/.cargo/config.toml`.
pub const CARGO_CONFIG: &str = r#"# Guest build flags: the Wasmicon contract (docs/app-workflow.md §4.4).
#
# This file is identical to what `wasmicon new` writes out, and
# `tools/wasmicon-cli/tests/flags.rs` checks that it stays so. The source of
# truth is `CARGO_CONFIG` in `tools/wasmicon-cli/src/flags.rs`; editing this
# copy by hand inside the wasmicon repository fails that test (a one-character
# change in a comment is enough, and that is the point).
#
# Changing a value breaks apps silently on hardware: with reference-types
# enabled, call_indirect is rejected by its table index (docs/abi-spec.md §6.1),
# and a larger --initial-memory exceeds the RP2040 two-page limit.
#
# cargo looks for .cargo/config.toml upward from the directory it runs in, so
# these settings apply when cargo is run inside this directory.
[build]
target = "wasm32-unknown-unknown"

[target.wasm32-unknown-unknown]
rustflags = [
    # rustc 1.97 enables reference-types on wasm32 by default. It is outside
    # the supported feature set (docs/decisions.md D-07), so turn it off
    # explicitly. The other five (bulk-memory / multivalue / mutable-globals /
    # nontrapping-fptoint / sign-ext) are on by default and supported.
    "-Ctarget-feature=-reference-types",
    # One page (64 KiB) of linear memory, within the RP2040 two-page limit.
    "-Clink-arg=--initial-memory=65536",
    # Guest shadow stack.
    "-Clink-arg=-zstack-size=8192",
]
"#;

/// Contents of `asconfig.json`. Only `outFile` differs between projects.
///
/// `out_file` is relative to `asconfig.json` (for example
/// `build/blink_as.wasm`).
#[must_use]
pub fn asconfig(out_file: &str) -> String {
    ASCONFIG
        .replace("{OUT_FILE}", out_file)
        .replace("{PAGES}", &AS_INITIAL_MEMORY_PAGES.to_string())
}

/// Template for `asconfig`. `{OUT_FILE}` and `{PAGES}` are substituted.
///
/// JSON has no comments, so the file cannot say where its source of truth
/// is. Editing `apps/*-as/asconfig.json` by hand fails `tests/flags.rs`,
/// which is how the mismatch is noticed.
const ASCONFIG: &str = r#"{
  "targets": {
    "release": {
      "outFile": "{OUT_FILE}",
      "optimizeLevel": 3,
      "shrinkLevel": 2,
      "converge": false,
      "noAssert": true
    }
  },
  "options": {
    "runtime": "stub",
    "initialMemory": {PAGES},
    "exportStart": false,
    "disable": ["simd", "threads", "exception-handling", "reference-types"],
    "enable": ["sign-extension", "nontrapping-f2i", "bulk-memory", "mutable-globals"]
  }
}
"#;
