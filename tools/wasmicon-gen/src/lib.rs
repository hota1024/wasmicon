//! Reads `wit/` and builds the generated files by the rules of `docs/abi-spec.md`.
//!
//! The CLI is `src/main.rs`; the logic lives here so that tests use the same path.

pub mod emit;
pub mod lower;
pub mod model;

use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};

/// One generated file.
pub struct Output {
    pub path: PathBuf,
    pub contents: String,
}

/// Builds the four generated files from the repository root.
pub fn outputs(root: &Path) -> Result<Vec<Output>> {
    let hal = lower::load_tree(root)?;
    Ok(vec![
        Output {
            path: root.join("runtime/src/generated.rs"),
            contents: rustfmt(root, &emit::runtime_rs(&hal))?,
        },
        Output {
            path: root.join("bindings/rust/src/generated.rs"),
            contents: rustfmt(root, &emit::bindings_rs(&hal))?,
        },
        Output {
            path: root.join("bindings/assemblyscript/assembly/generated.ts"),
            contents: emit::bindings_ts(&hal),
        },
        Output {
            path: root.join("bindings/c/generated.h"),
            contents: emit::bindings_c(&hal),
        },
    ])
}

/// `module\tname\tsig` lines in sorted order. `tools/check-sigs.sh` cross-checks
/// them against the output of `tools/wit2sig.py`.
pub fn sig_dump(hal: &model::Hal) -> String {
    let mut lines: Vec<String> = hal
        .imports()
        .map(|(iface, f)| format!("{}\t{}\t{}", iface.module, f.name, f.sig()))
        .collect();
    lines.sort();
    lines.join("\n") + "\n"
}

/// Runs rustfmt over generated Rust. Required so that `cargo fmt --check` and the
/// zero-diff check of the generated files hold at the same time (rust-toolchain.toml
/// guarantees that rustfmt is installed).
///
/// rustfmt looks for its config file upward from the directory of the file it formats.
/// A file in a temporary directory would miss the repository's `rustfmt.toml` and disagree
/// with `cargo fmt`, so the file is formatted under `root/target/` (gitignored) instead.
/// Passing a directory to `--config-path` is not used because it errors when no config
/// file exists.
pub fn rustfmt(root: &Path, src: &str) -> Result<String> {
    let dir = root.join("target/wasmicon-gen");
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create `{}`", dir.display()))?;
    let path = dir.join(format!("fmt-{}-{:p}.rs", std::process::id(), src.as_ptr()));
    std::fs::write(&path, src)?;

    let formatted = run_rustfmt(&path).and_then(|()| Ok(std::fs::read_to_string(&path)?));
    let _ = std::fs::remove_file(&path);
    formatted
}

fn run_rustfmt(path: &Path) -> Result<()> {
    let status = std::process::Command::new("rustfmt")
        .args(["--edition", "2024", "--quiet"])
        .arg(path)
        .status()
        .context("cannot run rustfmt (check the components in rust-toolchain.toml)")?;
    ensure!(status.success(), "rustfmt failed on `{}`", path.display());
    Ok(())
}
