//! Writes the generated files from `wit/` and `wit-device/` by the rules of `docs/abi-spec.md`.
//!
//! Outputs:
//! - `runtime/src/generated.rs`
//! - `bindings/rust/src/generated.rs`
//! - `bindings/assemblyscript/assembly/generated.ts`
//! - `bindings/c/generated.h`
//!
//! WIT syntax outside the subset of docs/abi-spec.md §2 is rejected with an error.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use wasmicon_gen::{lower, outputs, sig_dump};

const USAGE: &str = "\
wasmicon-gen - writes the generated files of runtime/ and bindings/ from wit/ and wit-device/

Usage:
    cargo run -p wasmicon-gen -- [options]

Options:
    --root <dir>   repository root (default: the current directory)
    --check        write nothing; exit 1 if a generated file is out of date
    --sigs         print the signature table to stdout (used by tools/check-sigs.sh)
    -h, --help     show this help
";

fn main() -> Result<()> {
    let mut root = PathBuf::from(".");
    let mut check = false;
    let mut sigs = false;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--root" => root = PathBuf::from(args.next().context("`--root` needs a value")?),
            "--check" => check = true,
            "--sigs" => sigs = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other => bail!("unknown option `{other}`\n\n{USAGE}"),
        }
    }

    if sigs {
        let hal = lower::load_tree(&root)?;
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
                eprintln!("out of date: {}", p.display());
            }
            bail!(
                "{} generated file(s) out of date; run `cargo run -p wasmicon-gen`",
                stale.len()
            );
        }
        println!("generated files are up to date ({} files)", outputs.len());
        return Ok(());
    }

    for o in &outputs {
        if let Some(dir) = o.path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create `{}`", dir.display()))?;
        }
        std::fs::write(&o.path, &o.contents)
            .with_context(|| format!("cannot write `{}`", o.path.display()))?;
        println!("wrote {}", o.path.display());
    }
    Ok(())
}
