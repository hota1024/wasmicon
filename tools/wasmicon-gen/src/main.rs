//! `wit/` を読み、`docs/abi-spec.md` の規則で生成物を出力する（docs/handoff.md §5 Phase 1）。
//!
//! 出力:
//! - `runtime/src/generated.rs`
//! - `bindings/rust/src/generated.rs`
//! - `bindings/assemblyscript/assembly/generated.ts`
//!
//! サブセット（abi-spec §2）外の WIT 構文はエラーで拒否する。

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use wasmicon_gen::{lower, outputs, sig_dump};

const USAGE: &str = "\
wasmicon-gen — wit/ から runtime / bindings の生成物を出力する

使い方:
    cargo run -p wasmicon-gen -- [オプション]

オプション:
    --root <dir>   リポジトリのルート（既定: カレントディレクトリ）
    --check        書き込まず、生成物が最新かどうかだけ検査する（古ければ終了コード 1）
    --sigs         シグネチャ表を stdout に出す（tools/check-sigs.sh が使う）
    -h, --help     このヘルプ
";

fn main() -> Result<()> {
    let mut root = PathBuf::from(".");
    let mut check = false;
    let mut sigs = false;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--root" => root = PathBuf::from(args.next().context("--root に値が無い")?),
            "--check" => check = true,
            "--sigs" => sigs = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other => bail!("未知のオプション: {other}\n\n{USAGE}"),
        }
    }

    if sigs {
        let hal = lower::load(&root.join("wit"))?;
        print!("{}", sig_dump(&hal));
        return Ok(());
    }

    let outputs = outputs(&root)?;

    if check {
        let stale: Vec<&PathBuf> = outputs
            .iter()
            .filter(|o| std::fs::read_to_string(&o.path).ok().as_deref() != Some(&o.contents))
            .map(|o| &o.path)
            .collect();
        if !stale.is_empty() {
            for p in &stale {
                eprintln!("生成物が wit/ と一致しない: {}", p.display());
            }
            bail!(
                "`cargo run -p wasmicon-gen` で再生成すること（{} 件）",
                stale.len()
            );
        }
        println!("生成物は最新（{} ファイル）", outputs.len());
        return Ok(());
    }

    for o in &outputs {
        if let Some(dir) = o.path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("{} を作れない", dir.display()))?;
        }
        std::fs::write(&o.path, &o.contents)
            .with_context(|| format!("{} を書けない", o.path.display()))?;
        println!("生成: {}", o.path.display());
    }
    Ok(())
}
