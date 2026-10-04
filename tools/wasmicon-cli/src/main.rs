//! `wasmicon` — アプリ作者が触る唯一の面（`docs/app-workflow.md` §4）。
//!
//! 0 段は `check` / `run` / `trace diff` / `doctor` まで入った。`monitor` と
//! `size` は未実装（§5。残作業は `docs/TODO.md` §5）。引数のパースは
//! `wasmicon-gen` と同じ手書き + `USAGE` 定数。

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::ExitCode;

use wasmicon_cli::{check, doctor, manifest, pack, run as run_cmd, trace};

const USAGE: &str = "\
wasmicon — Wasmicon のアプリを検査・実行・配備する

使い方:
    wasmicon <コマンド> [オプション]

コマンド:
    check <app.wasm>         そのボードで走るかを検査する
    run <app.wasm>           host ポート（mock HAL）で走らせる
    pack <app.wasm>          スロット画像にする（ファームが読む形）
    trace diff <a> <b>       2 つのシリアル出力のトレースを突き合わせる
    doctor                   道具が揃っているかを見る

check のオプション:
    --board <name>           検査するボード（既定: 全ボード）
                             rp2040 / rp2350 / esp32s3 / host

pack のオプション:
    -o <file>                出力先（既定: <入力>.slot）
    --board <name>           スロットに収まるかを検査し、焼くコマンドを出す

run のオプション:
    --trace                  全 host call を abi-spec §9 の形式で出す
    --i2c-replay <file>      記録済みの I2C 応答（host にセンサーは無い）

共通:
    -h, --help               このヘルプ
    -V, --version            版を出す

`wasmicon.toml` があれば読む（カレントから上に探す）。
`[requirements] pin-roles` を書くと、役割名の照合が参考から保証に変わる。

検査の中身は実ランタイムの decode / validate と、wit/ から生成した import 表。
host は全ボードより緩いので、`--board` でボードを指定したものだけが
「実機で走る」の根拠になる（docs/app-workflow.md §4.3）。
";

fn main() -> ExitCode {
    match dispatch() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("wasmicon: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// 判定が通れば `Ok(true)`。使い方の誤りは `Err`。
fn dispatch() -> Result<bool> {
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
        "run" => run_cmd::run(&parse_run(args)?),
        "pack" => pack::run(&parse_pack(args)?),
        "trace" => trace::diff(&parse_trace(args)?),
        "doctor" => doctor::run(),
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
            "--board" => board = Some(args.next().context("--board に値が無い")?),
            "-h" | "--help" => help(),
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
    // `wasmicon.toml` はカレントから上に探す（cargo と同じ）。無くてもよい。
    let manifest = find_manifest()?;
    // `--board` が無ければ toml の既定を使う。
    let board = board.or_else(|| manifest.as_ref().and_then(|m| m.default_board.clone()));
    Ok(check::Options {
        path,
        board,
        manifest,
    })
}

/// カレントから上に向かって `wasmicon.toml` を探す（§4.7）。
fn find_manifest() -> Result<Option<manifest::Manifest>> {
    let cwd = std::env::current_dir().context("カレントディレクトリが取れない")?;
    manifest::find(&cwd)
}

fn parse_run(args: impl Iterator<Item = String>) -> Result<run_cmd::Options> {
    let mut path: Option<PathBuf> = None;
    let mut trace_on = false;
    let mut i2c_replay: Option<PathBuf> = None;

    let mut args = args.peekable();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--trace" => trace_on = true,
            "--i2c-replay" => {
                let v = args.next().context("--i2c-replay に値が無い")?;
                i2c_replay = Some(PathBuf::from(v));
            }
            "-h" | "--help" => help(),
            other if other.starts_with('-') => bail!("未知のオプション: {other}\n\n{USAGE}"),
            other => {
                if path.is_some() {
                    bail!("走らせられるのは 1 つだけ: {other}");
                }
                path = Some(PathBuf::from(other));
            }
        }
    }

    let path = path.context("走らせる .wasm を渡すこと\n\n".to_string() + USAGE)?;
    // `--i2c-replay` が無ければ toml の既定（toml のある場所基準で解決済み）。
    let i2c_replay = i2c_replay.or_else(|| {
        find_manifest()
            .ok()
            .flatten()
            .and_then(|m| m.default_i2c_replay)
    });
    Ok(run_cmd::Options {
        path,
        trace: trace_on,
        i2c_replay,
    })
}

fn parse_pack(args: impl Iterator<Item = String>) -> Result<pack::Options> {
    let mut path: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut board: Option<String> = None;

    let mut args = args.peekable();
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" => out = Some(PathBuf::from(args.next().context("-o に値が無い")?)),
            "--board" => board = Some(args.next().context("--board に値が無い")?),
            "-h" | "--help" => help(),
            other if other.starts_with('-') => bail!("未知のオプション: {other}\n\n{USAGE}"),
            other => {
                if path.is_some() {
                    bail!("包めるのは 1 つだけ: {other}");
                }
                path = Some(PathBuf::from(other));
            }
        }
    }

    let path = path.context("包む .wasm を渡すこと\n\n".to_string() + USAGE)?;
    // `--board` が無ければ toml の既定を使う（§4.7）。
    let board = board.or_else(|| find_manifest().ok().flatten().and_then(|m| m.default_board));
    Ok(pack::Options { path, out, board })
}

fn parse_trace(mut args: impl Iterator<Item = String>) -> Result<trace::Options> {
    let sub = args.next().context("trace の後に diff が要る")?;
    // サブコマンドの判定より先に help を見る（`trace --help` が
    // 「diff だけ」で弾かれていた）。
    if sub == "-h" || sub == "--help" {
        help();
    }
    if sub != "diff" {
        bail!("trace のサブコマンドは diff だけ: {sub}");
    }
    let mut files: Vec<PathBuf> = Vec::new();
    for a in args {
        match a.as_str() {
            "-h" | "--help" => help(),
            other if other.starts_with('-') => bail!("未知のオプション: {other}\n\n{USAGE}"),
            other => files.push(PathBuf::from(other)),
        }
    }
    let [a, b] = files.as_slice() else {
        bail!(
            "突き合わせるログを 2 つ渡すこと（渡されたのは {} 個）",
            files.len()
        );
    };
    Ok(trace::Options {
        a: a.clone(),
        b: b.clone(),
    })
}

fn help() -> ! {
    print!("{USAGE}");
    std::process::exit(0)
}
