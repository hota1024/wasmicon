//! `wit/` を読み、`docs/abi-spec.md` の規則で生成物を組み立てるライブラリ。
//!
//! CLI は `src/main.rs`。テストからも同じ経路を使えるようにここに置く。

pub mod emit;
pub mod lower;
pub mod model;

use anyhow::{Context, Result, ensure};
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
///
/// rustfmt は設定ファイルを「整形対象のファイルが置かれた場所」から上に向かって探す。
/// 一時ディレクトリに置くとリポジトリの `rustfmt.toml` が効かず `cargo fmt` と
/// 食い違うので、`root/target/`（.gitignore 済み）の下で整形して探索させる。
/// `--config-path` にディレクトリを渡す方法は、設定ファイルが無いとエラーになるため使わない。
pub fn rustfmt(root: &Path, src: &str) -> Result<String> {
    let dir = root.join("target/wasmicon-gen");
    std::fs::create_dir_all(&dir).with_context(|| format!("{} を作れない", dir.display()))?;
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
        .context("rustfmt を起動できない（rust-toolchain.toml の components を確認）")?;
    ensure!(status.success(), "rustfmt が失敗した: {}", path.display());
    Ok(())
}
