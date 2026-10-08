//! `wasmicon` — アプリ作者が触る唯一の面（`docs/app-workflow.md` §4）。
//!
//! 0 段は `new` / `check` / `run` / `monitor` / `trace diff` / `doctor` が
//! 入り、1 段の `pack` / `deploy` も通っている。`size` は
//! `tools/measure-size.sh` のままにしてある（残作業は `docs/TODO.md` §5）。
//! 引数のパースは `wasmicon-gen` と同じ手書き + `USAGE` 定数。

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::ExitCode;

use wasmicon_cli::{
    check, deploy, doctor, manifest, monitor, new as new_cmd, pack, run as run_cmd, trace,
};

const USAGE: &str = "\
wasmicon — Wasmicon のアプリを検査・実行・配備する

使い方:
    wasmicon <コマンド> [オプション]

コマンド:
    new <dir>                アプリの雛形を出す
    check <app.wasm>         そのボードで走るかを検査する
    run <app.wasm>           host ポート（mock HAL）で走らせる
    pack <app.wasm>          スロット画像にする（ファームが読む形）
    deploy <app.wasm>        検査してボードのスロットに焼く
    monitor                  シリアルを開いてトレースを取り込む
    trace diff <a> <b>       2 つのシリアル出力のトレースを突き合わせる
    trace replay <log>       実機のトレースから I2C の応答を取り出す
    doctor                   道具が揃っているかを見る

new のオプション:
    --lang <rust|as>         言語（既定: rust）
    --board <name>           wasmicon.toml の [defaults] board に書く
    --hal <dir>              バインディングの置き場所（既定: 上に向かって探す）

check のオプション:
    --board <name>           検査するボード
                             rp2040 / rp2350 / esp32s3 / host
                             既定: wasmicon.toml の [defaults] board、
                             無ければ全ボード

pack のオプション:
    -o <file>                出力先（既定: <入力>.bin）
    --board <name>           スロットに収まるかを検査し、焼くコマンドを出す

deploy のオプション:
    --board <name>           送り先（既定: wasmicon.toml の [defaults] board）
    --no-run                 焼くだけでリセットしない
    --monitor                焼いてリセットし、そのままトレースを取り込む
    -o <file>                --monitor のときの取り込み先
    --port <dev>             シリアルの口（既定: /dev/cu.usb* から選ぶ）

monitor のオプション:
    --port <dev>             シリアルの口（既定: /dev/cu.usb* から選ぶ）
    --baud <n>               ボーレート（既定: 115200）
    -o <file>                標準出力とは別にファイルにも書く
    --idle <秒>              無音がこれだけ続いたら終わる（既定: 3。0 で無効）
    --timeout <秒>           全体の上限（既定: 無し）

trace replay のオプション:
    -o <file>                書き出し先（既定: 標準出力）。run --i2c-replay が読む形

run のオプション:
    --trace                  全 host call を abi-spec §9 の形式で出す
    --i2c-replay <file>      記録済みの I2C 応答（host にセンサーは無い）

共通:
    -h, --help               このヘルプ
    -V, --version            版を出す

`wasmicon.toml` があれば読む（カレントから上に探す）。
`[requirements] pin-roles` と `[board.<name>.roles]` を突き合わせ、deploy は
配線表をアプリと一緒にデバイスの設定スロットに書く（ファームは既定の表を持たない）。

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
        "new" => new_cmd::run(&parse_new(args)?),
        "check" => check::run(&parse_check(args)?),
        "run" => run_cmd::run(&parse_run(args)?),
        "pack" => pack::run(&parse_pack(args)?),
        "deploy" => deploy::run(&parse_deploy(args)?),
        "monitor" => monitor::run(&parse_monitor(args)?),
        "trace" => match parse_trace(args)? {
            TraceCmd::Diff(o) => trace::diff(&o),
            TraceCmd::Replay(o) => trace::replay(&o),
        },
        "doctor" => doctor::run(),
        other if other.starts_with('-') => {
            bail!("未知のオプション: {other}\n\n{USAGE}")
        }
        other => bail!("未知のコマンド: {other}\n\n{USAGE}"),
    }
}

fn parse_new(args: impl Iterator<Item = String>) -> Result<new_cmd::Options> {
    let mut dir: Option<PathBuf> = None;
    let mut lang = new_cmd::Lang::Rust;
    let mut board: Option<String> = None;
    let mut hal: Option<PathBuf> = None;

    let mut args = args.peekable();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--lang" => {
                lang = new_cmd::Lang::parse(&args.next().context("--lang に値が無い")?)?;
            }
            "--board" => board = Some(args.next().context("--board に値が無い")?),
            "--hal" => hal = Some(PathBuf::from(args.next().context("--hal に値が無い")?)),
            "-h" | "--help" => help(),
            other if other.starts_with('-') => bail!("未知のオプション: {other}\n\n{USAGE}"),
            other => {
                if dir.is_some() {
                    bail!("作れるのは 1 つだけ: {other}");
                }
                dir = Some(PathBuf::from(other));
            }
        }
    }

    let dir = dir.context("作る場所を渡すこと\n\n".to_string() + USAGE)?;
    Ok(new_cmd::Options {
        dir,
        lang,
        board,
        hal,
    })
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
    //
    // **読めない toml を黙って無視しない。** `deny_unknown_fields` なので
    // `i2c_replay` のような綴り違いで `load` が失敗する。そこを捨てると
    // **replay 無しで走って記録済みの 14,352 行と静かに食い違う** ——
    // `manifest.rs` の冒頭が「設定したのに効いていないことに気付けない」と
    // 書いている、まさにその形になる（`check` / `deploy` は `?` で上げて
    // いるのに、ここだけ落としていた）。
    let i2c_replay = match i2c_replay {
        Some(p) => Some(p),
        None => find_manifest()?.and_then(|m| m.default_i2c_replay),
    };
    Ok(run_cmd::Options {
        path,
        trace: trace_on,
        i2c_replay,
    })
}

fn parse_deploy(args: impl Iterator<Item = String>) -> Result<deploy::Options> {
    let mut path: Option<PathBuf> = None;
    let mut board: Option<String> = None;
    let mut no_run = false;
    let mut monitor = false;
    let mut out: Option<PathBuf> = None;
    let mut port: Option<PathBuf> = None;

    let mut args = args.peekable();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--board" => board = Some(args.next().context("--board に値が無い")?),
            "--no-run" => no_run = true,
            "--monitor" => monitor = true,
            "-o" => out = Some(PathBuf::from(args.next().context("-o に値が無い")?)),
            "--port" => port = Some(PathBuf::from(args.next().context("--port に値が無い")?)),
            "-h" | "--help" => help(),
            other if other.starts_with('-') => bail!("未知のオプション: {other}\n\n{USAGE}"),
            other => {
                if path.is_some() {
                    bail!("送れるのは 1 つだけ: {other}");
                }
                path = Some(PathBuf::from(other));
            }
        }
    }

    let path = path.context("送る .wasm を渡すこと\n\n".to_string() + USAGE)?;
    Ok(deploy::Options {
        path,
        board,
        manifest: find_manifest()?,
        no_run,
        monitor,
        out,
        port,
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
    // `--board` が無ければ toml の既定を使う（§4.7）。読めない toml は
    // 上げる（`parse_run` と同じ理由）。
    let board = match board {
        Some(b) => Some(b),
        None => find_manifest()?.and_then(|m| m.default_board),
    };
    Ok(pack::Options { path, out, board })
}

fn parse_monitor(args: impl Iterator<Item = String>) -> Result<monitor::Options> {
    let mut port: Option<PathBuf> = None;
    let mut baud = monitor::DEFAULT_BAUD;
    let mut out: Option<PathBuf> = None;
    let mut idle = Some(monitor::DEFAULT_IDLE);
    let mut timeout = None;

    let mut args = args.peekable();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" => port = Some(PathBuf::from(args.next().context("--port に値が無い")?)),
            "-o" => out = Some(PathBuf::from(args.next().context("-o に値が無い")?)),
            "--baud" => {
                let v = args.next().context("--baud に値が無い")?;
                baud = v
                    .parse()
                    .with_context(|| format!("--baud が数でない: {v}"))?;
            }
            "--idle" => {
                let v = args.next().context("--idle に値が無い")?;
                let secs: u64 = v
                    .parse()
                    .with_context(|| format!("--idle が数でない: {v}"))?;
                // 0 は「無音では終わらない」（Ctrl-C で止める）。
                idle = (secs > 0).then(|| std::time::Duration::from_secs(secs));
            }
            "--timeout" => {
                let v = args.next().context("--timeout に値が無い")?;
                let secs: u64 = v
                    .parse()
                    .with_context(|| format!("--timeout が数でない: {v}"))?;
                timeout = (secs > 0).then(|| std::time::Duration::from_secs(secs));
            }
            "-h" | "--help" => help(),
            other => bail!("未知のオプション: {other}\n\n{USAGE}"),
        }
    }

    Ok(monitor::Options {
        port,
        baud,
        out,
        idle,
        timeout,
    })
}

enum TraceCmd {
    Diff(trace::Options),
    Replay(trace::ReplayOptions),
}

fn parse_trace(mut args: impl Iterator<Item = String>) -> Result<TraceCmd> {
    let sub = args.next().context("trace の後に diff か replay が要る")?;
    // サブコマンドの判定より先に help を見る（`trace --help` が
    // 「diff だけ」で弾かれていた）。
    if sub == "-h" || sub == "--help" {
        help();
    }
    if sub != "diff" && sub != "replay" {
        bail!("trace のサブコマンドは diff と replay だけ: {sub}");
    }
    let mut files: Vec<PathBuf> = Vec::new();
    let mut out: Option<PathBuf> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => help(),
            "-o" if sub == "replay" => {
                out = Some(PathBuf::from(
                    args.next().context("-o の後にファイルが要る")?,
                ));
            }
            other if other.starts_with('-') => bail!("未知のオプション: {other}\n\n{USAGE}"),
            other => files.push(PathBuf::from(other)),
        }
    }
    if sub == "replay" {
        let [log] = files.as_slice() else {
            bail!(
                "取り出すログを 1 つ渡すこと（渡されたのは {} 個）",
                files.len()
            );
        };
        return Ok(TraceCmd::Replay(trace::ReplayOptions {
            log: log.clone(),
            out,
        }));
    }
    let [a, b] = files.as_slice() else {
        bail!(
            "突き合わせるログを 2 つ渡すこと（渡されたのは {} 個）",
            files.len()
        );
    };
    Ok(TraceCmd::Diff(trace::Options {
        a: a.clone(),
        b: b.clone(),
    }))
}

fn help() -> ! {
    print!("{USAGE}");
    std::process::exit(0)
}
