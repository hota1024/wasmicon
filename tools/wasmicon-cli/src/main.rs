//! `wasmicon` — アプリ作者が触る唯一の面（`docs/app-workflow.md` §4）。
//!
//! 0 段は `check` から。`run` / `monitor` / `trace diff` / `size` / `doctor` を
//! 続けて足す（§5）。引数のパースは `wasmicon-gen` と同じ手書き + `USAGE` 定数。

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::ExitCode;

use wasmicon_cli::check;

const USAGE: &str = "\
wasmicon — Wasmicon のアプリを検査・実行・配備する

使い方:
    wasmicon <コマンド> [オプション]

コマンド:
    check <app.wasm>    そのボードで走るかを検査する

check のオプション:
    --board <name>      検査するボード（既定: 全ボード）
                        rp2040 / rp2350 / esp32s3 / host

共通:
    -h, --help          このヘルプ
    -V, --version       版を出す

検査の中身は実ランタイムの decode / validate と、wit/ から生成した import 表。
host は全ボードより緩いので、`--board` でボードを指定したものだけが
「実機で走る」の根拠になる（docs/app-workflow.md §4.3）。
";

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("wasmicon: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// 検査が通れば `Ok(true)`。使い方の誤りは `Err`。
fn run() -> Result<bool> {
    let mut args = std::env::args().skip(1);
    let Some(cmd) = args.next() else {
        print!("{USAGE}");
        return Ok(true);
    };

    match cmd.as_str() {
        "-h" | "--help" => {
            print!("{USAGE}");
            Ok(true)
        }
        "-V" | "--version" => {
            println!("wasmicon {}", env!("CARGO_PKG_VERSION"));
            Ok(true)
        }
        "check" => check::run(&parse_check(args)?),
        other if other.starts_with('-') => {
            bail!("未知のオプション: {other}\n\n{USAGE}")
        }
        other => bail!("未知のコマンド: {other}\n\n{USAGE}"),
    }
}

fn parse_check(args: impl Iterator<Item = String>) -> Result<check::Options> {
    let mut path: Option<PathBuf> = None;
    let mut board: Option<String> = None;

    let mut args = args.peekable();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--board" => {
                board = Some(args.next().context("--board に値が無い")?);
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            other if other.starts_with('-') => bail!("未知のオプション: {other}\n\n{USAGE}"),
            other => {
                if path.is_some() {
                    bail!("検査できるのは 1 つだけ: {other}");
                }
                path = Some(PathBuf::from(other));
            }
        }
    }

    let path = path.context("検査する .wasm を渡すこと\n\n".to_string() + USAGE)?;
    Ok(check::Options { path, board })
}
