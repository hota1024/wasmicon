//! `wit/` を読み、`docs/abi-spec.md` の規則で生成物を組み立てるライブラリ。
//!
//! CLI は `src/main.rs`。テストからも同じ経路を使えるようにここに置く。

pub mod emit;
pub mod lower;
pub mod model;

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// 生成物 1 ファイル。
pub struct Output {
    pub path: PathBuf,
    pub contents: String,
}

/// リポジトリルートから 3 つの生成物を組み立てる。
pub fn outputs(root: &Path) -> Result<Vec<Output>> {
    let hal = lower::load(&root.join("wit"))?;
    Ok(vec![
        Output {
            path: root.join("runtime/src/generated.rs"),
            contents: rustfmt(&emit::runtime_rs(&hal))?,
        },
        Output {
            path: root.join("bindings/rust/src/generated.rs"),
            contents: rustfmt(&emit::bindings_rs(&hal))?,
        },
        Output {
            path: root.join("bindings/assemblyscript/assembly/generated.ts"),
            contents: emit::bindings_ts(&hal),
        },
    ])
}

/// `module\tname\tsig` を辞書順に並べたもの。`tools/check-sigs.sh` が
/// `tools/wit2sig.py` の出力と突き合わせる。
pub fn sig_dump(hal: &model::Hal) -> String {
    let mut lines: Vec<String> = hal
        .imports()
        .map(|(iface, f)| format!("{}\t{}\t{}", iface.module, f.name, f.sig()))
        .collect();
    lines.sort();
    lines.join("\n") + "\n"
}

/// 生成した Rust を rustfmt に通す。`cargo fmt --check` と生成物の diff ゼロ検査を
/// 同時に満たすために必須（rustfmt は rust-toolchain.toml で保証されている）。
pub fn rustfmt(src: &str) -> Result<String> {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "wasmicon-gen-{}-{:p}.rs",
        std::process::id(),
        src.as_ptr()
    ));
    std::fs::write(&path, src)?;
    let status = std::process::Command::new("rustfmt")
        .args(["--edition", "2024", "--quiet"])
        .arg(&path)
        .status()
        .context("rustfmt を起動できない（rust-toolchain.toml の components を確認）")?;
    if !status.success() {
        bail!("rustfmt が失敗した: {}", path.display());
    }
    let out = std::fs::read_to_string(&path)?;
    let _ = std::fs::remove_file(&path);
    Ok(out)
}
