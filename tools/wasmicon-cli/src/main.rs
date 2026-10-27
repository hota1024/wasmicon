//! `wasmicon`: the one tool app authors touch (docs/app-workflow.md §4).
//!
//! Stage 0 (`new` / `check` / `run` / `monitor` / `trace diff` / `doctor`) and
//! stage 1 (`pack` / `deploy` / `assets pack` / `assets write`) of
//! docs/app-workflow.md §5 are implemented. `size` stays
//! `tools/measure-size.sh` (remaining work: docs/TODO.md §5).
//! Arguments are parsed by hand with a `USAGE` constant, as in `wasmicon-gen`.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::ExitCode;

use wasmicon_cli::{
    assets, check, deploy, doctor, manifest, monitor, new as new_cmd, pack, run as run_cmd, trace,
};

const USAGE: &str = "\
wasmicon - check, run and deploy Wasmicon apps

Usage:
    wasmicon <command> [options]

Commands:
    new <dir>                write an app scaffold
    check <app.wasm>         check that the app runs on a board
    run <app.wasm>           run on the host port (mock HAL)
    pack <app.wasm>          build the slot image the firmware reads
    deploy <app.wasm>        check, then flash to the board's app slot
    assets pack              build the assets slot image from [assets] in wasmicon.toml
    assets write             build it and flash it to the board's assets slot
    monitor                  open the serial port and capture the trace
    trace diff <a> <b>       compare the traces in two serial captures
    trace replay <log>       extract the I2C responses from a hardware trace
    doctor                   report which tools are installed

Options for new:
    --lang <rust|as>         language (default: rust)
    --board <name>           written to [defaults] board in wasmicon.toml
    --hal <dir>              where the bindings are (default: search upward)

Options for check:
    --board <name>           board to check against
                             rp2040 / rp2350 / esp32s3 / esp32s3-psram /
                             esp32c5 / esp32p4 / host
                             default: [defaults] board in wasmicon.toml,
                             otherwise every board

Options for pack:
    -o <file>                output path (default: <input>.bin)
    --board <name>           check that the image fits the slot and print the flash command

Options for deploy:
    --board <name>           target board (default: [defaults] board in wasmicon.toml)
    --no-run                 flash only; do not reset
    --monitor                flash, reset and capture the trace
    -o <file>                where --monitor writes the capture
    --port <dev>             serial port (default: pick from /dev/cu.usb*)

Options for assets pack:
    -o <file>                output path (default: assets.bin next to wasmicon.toml)
    --board <name>           check that the image fits the assets slot and print the flash command

Options for assets write:
    --board <name>           target board (default: [defaults] board in wasmicon.toml)
    --port <dev>             serial port (default: pick from /dev/cu.usb*)

Options for monitor:
    --port <dev>             serial port (default: pick from /dev/cu.usb*)
    --baud <n>               baud rate (default: 115200)
    -o <file>                also write to this file besides stdout
    --idle <secs>            stop after this much silence (default: 3; 0 disables)
    --timeout <secs>         overall limit (default: none)

Options for trace replay:
    -o <file>                output path (default: stdout), in the form run --i2c-replay reads

Options for run:
    --trace                  print every host call in the docs/abi-spec.md §9 format
    --i2c-replay <file>      recorded I2C responses (the host has no sensor)
    --asset <name>=<file>    serve <file> as the asset <name> (repeatable; on top of [assets])
    --display-out <file>     write the mock display's picture (after the last present) as a PNG
    --display-size <W>x<H>   size of the mock display (default 320x240; the Tab5 panel is 720x1280)

Common:
    -h, --help               this help
    -V, --version            print the version

`wasmicon.toml` is read when present (searched upward from the current directory).
`[requirements] pin-roles` is cross-checked against `[board.<name>.roles]`, and
deploy writes the wiring table to the device's config slot together with the app
(the firmware has no default table). `[assets]` names the read-only data the app
opens: run serves the files, assets write flashes them (deploy leaves them alone).

The check uses the real runtime's decode / validate and the import table generated
from wit/. The host is more permissive than every board, so only a check with
`--board <name>` shows that the app runs on hardware (docs/app-workflow.md §4.3).
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

/// `Ok(true)` when the command passes. A usage error is `Err`.
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
        "assets" => match parse_assets(args)? {
            AssetsCmd::Pack(o) => assets::pack(&o),
            AssetsCmd::Write(o) => assets::write(&o),
        },
        "doctor" => doctor::run(),
        other if other.starts_with('-') => {
            bail!("unknown option `{other}`\n\n{USAGE}")
        }
        other => bail!("unknown command `{other}`\n\n{USAGE}"),
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
                lang = new_cmd::Lang::parse(&args.next().context("--lang needs a value")?)?;
            }
            "--board" => board = Some(args.next().context("--board needs a value")?),
            "--hal" => hal = Some(PathBuf::from(args.next().context("--hal needs a value")?)),
            "-h" | "--help" => help(),
            other if other.starts_with('-') => bail!("unknown option `{other}`\n\n{USAGE}"),
            other => {
                if dir.is_some() {
                    bail!("only one directory can be created: `{other}`");
                }
                dir = Some(PathBuf::from(other));
            }
        }
    }

    let dir = dir.context("pass the directory to create\n\n".to_string() + USAGE)?;
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
            "--board" => board = Some(args.next().context("--board needs a value")?),
            "-h" | "--help" => help(),
            other if other.starts_with('-') => bail!("unknown option `{other}`\n\n{USAGE}"),
            other => {
                if path.is_some() {
                    bail!("only one .wasm can be checked: `{other}`");
                }
                path = Some(PathBuf::from(other));
            }
        }
    }

    let path = path.context("pass the .wasm to check\n\n".to_string() + USAGE)?;
    // `wasmicon.toml` is searched upward from the current directory (like
    // cargo). It is optional.
    let manifest = find_manifest()?;
    // Without `--board`, use the default from the manifest.
    let board = board.or_else(|| manifest.as_ref().and_then(|m| m.default_board.clone()));
    Ok(check::Options {
        path,
        board,
        manifest,
    })
}

/// Searches upward from the current directory for `wasmicon.toml`
/// (docs/app-workflow.md §4.7).
fn find_manifest() -> Result<Option<manifest::Manifest>> {
    let cwd = std::env::current_dir().context("cannot determine the current directory")?;
    manifest::find(&cwd)
}

fn parse_run(args: impl Iterator<Item = String>) -> Result<run_cmd::Options> {
    let mut path: Option<PathBuf> = None;
    let mut trace_on = false;
    let mut i2c_replay: Option<PathBuf> = None;
    let mut overrides: Vec<(String, PathBuf)> = Vec::new();
    let mut display_out: Option<PathBuf> = None;
    let mut display_size: Option<(u32, u32)> = None;

    let mut args = args.peekable();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--trace" => trace_on = true,
            "--i2c-replay" => {
                let v = args.next().context("--i2c-replay needs a value")?;
                i2c_replay = Some(PathBuf::from(v));
            }
            "--display-out" => {
                let v = args.next().context("--display-out needs a file")?;
                display_out = Some(PathBuf::from(v));
            }
            "--display-size" => {
                let v = args.next().context("--display-size needs <W>x<H>")?;
                display_size = Some(parse_display_size(&v)?);
            }
            "--asset" => {
                let v = args.next().context("--asset needs <name>=<file>")?;
                let Some((name, file)) = v.split_once('=') else {
                    bail!("--asset takes <name>=<file>, got `{v}`");
                };
                if !wasmicon_port::assets::valid_name(name.as_bytes()) {
                    bail!(
                        "`{name}` is not a valid asset name (1 to {} of a-z / 0-9 / - / _ / ., \
                         starting with a letter or a digit)",
                        wasmicon_port::assets::MAX_NAME
                    );
                }
                overrides.push((name.to_string(), PathBuf::from(file)));
            }
            "-h" | "--help" => help(),
            other if other.starts_with('-') => bail!("unknown option `{other}`\n\n{USAGE}"),
            other => {
                if path.is_some() {
                    bail!("only one .wasm can be run: `{other}`");
                }
                path = Some(PathBuf::from(other));
            }
        }
    }

    let path = path.context("pass the .wasm to run\n\n".to_string() + USAGE)?;
    // Without `--i2c-replay`, use the manifest default (already resolved
    // relative to the manifest's directory). The `[assets]` table is the
    // base that `--asset` overrides by name.
    //
    // An unreadable manifest is never ignored. The manifest is
    // `deny_unknown_fields`, so a misspelling such as `i2c_replay` makes
    // `load` fail. Dropping that error would run without a replay and
    // silently diverge from the recorded trace, which is exactly the "I set
    // it but it has no effect" failure `manifest.rs` warns about (`check` and
    // `deploy` propagate the error with `?`; this path once dropped it).
    let manifest = find_manifest()?;
    let i2c_replay = match i2c_replay {
        Some(p) => Some(p),
        None => manifest.as_ref().and_then(|m| m.default_i2c_replay.clone()),
    };
    let mut assets = manifest.map(|m| m.assets).unwrap_or_default();
    for (name, file) in overrides {
        assets.insert(name, file);
    }
    if assets.len() > wasmicon_port::assets::MAX_ENTRIES {
        bail!(
            "{} assets, more than the {} the slot holds",
            assets.len(),
            wasmicon_port::assets::MAX_ENTRIES
        );
    }
    Ok(run_cmd::Options {
        path,
        trace: trace_on,
        i2c_replay,
        assets: assets.into_iter().collect(),
        display_out,
        display_size,
    })
}

/// `<W>x<H>`, each 1 to 4096: the mock keeps two RGB565 buffers of that size.
fn parse_display_size(v: &str) -> Result<(u32, u32)> {
    let parse = |s: &str| s.parse::<u32>().ok().filter(|n| (1..=4096).contains(n));
    match v.split_once('x') {
        Some((w, h)) => match (parse(w), parse(h)) {
            (Some(w), Some(h)) => Ok((w, h)),
            _ => bail!("--display-size takes <W>x<H>, each 1 to 4096, got `{v}`"),
        },
        None => bail!("--display-size takes <W>x<H>, got `{v}`"),
    }
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
            "--board" => board = Some(args.next().context("--board needs a value")?),
            "--no-run" => no_run = true,
            "--monitor" => monitor = true,
            "-o" => out = Some(PathBuf::from(args.next().context("-o needs a value")?)),
            "--port" => port = Some(PathBuf::from(args.next().context("--port needs a value")?)),
            "-h" | "--help" => help(),
            other if other.starts_with('-') => bail!("unknown option `{other}`\n\n{USAGE}"),
            other => {
                if path.is_some() {
                    bail!("only one .wasm can be deployed: `{other}`");
                }
                path = Some(PathBuf::from(other));
            }
        }
    }

    let path = path.context("pass the .wasm to deploy\n\n".to_string() + USAGE)?;
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
            "-o" => out = Some(PathBuf::from(args.next().context("-o needs a value")?)),
            "--board" => board = Some(args.next().context("--board needs a value")?),
            "-h" | "--help" => help(),
            other if other.starts_with('-') => bail!("unknown option `{other}`\n\n{USAGE}"),
            other => {
                if path.is_some() {
                    bail!("only one .wasm can be packed: `{other}`");
                }
                path = Some(PathBuf::from(other));
            }
        }
    }

    let path = path.context("pass the .wasm to pack\n\n".to_string() + USAGE)?;
    // Without `--board`, use the manifest default (docs/app-workflow.md §4.7).
    // An unreadable manifest is an error, for the reason given in `parse_run`.
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
            "--port" => port = Some(PathBuf::from(args.next().context("--port needs a value")?)),
            "-o" => out = Some(PathBuf::from(args.next().context("-o needs a value")?)),
            "--baud" => {
                let v = args.next().context("--baud needs a value")?;
                baud = v
                    .parse()
                    .with_context(|| format!("--baud is not a number: `{v}`"))?;
            }
            "--idle" => {
                let v = args.next().context("--idle needs a value")?;
                let secs: u64 = v
                    .parse()
                    .with_context(|| format!("--idle is not a number: `{v}`"))?;
                // 0 means "never stop on silence" (stop with Ctrl-C).
                idle = (secs > 0).then(|| std::time::Duration::from_secs(secs));
            }
            "--timeout" => {
                let v = args.next().context("--timeout needs a value")?;
                let secs: u64 = v
                    .parse()
                    .with_context(|| format!("--timeout is not a number: `{v}`"))?;
                timeout = (secs > 0).then(|| std::time::Duration::from_secs(secs));
            }
            "-h" | "--help" => help(),
            other => bail!("unknown option `{other}`\n\n{USAGE}"),
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
    let sub = args
        .next()
        .context("trace needs a subcommand: diff or replay")?;
    // Look for help before checking the subcommand (`trace --help` used to be
    // rejected as "not diff").
    if sub == "-h" || sub == "--help" {
        help();
    }
    if sub != "diff" && sub != "replay" {
        bail!("unknown trace subcommand `{sub}` (diff or replay)");
    }
    let mut files: Vec<PathBuf> = Vec::new();
    let mut out: Option<PathBuf> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => help(),
            "-o" if sub == "replay" => {
                out = Some(PathBuf::from(args.next().context("-o needs a file")?));
            }
            other if other.starts_with('-') => bail!("unknown option `{other}`\n\n{USAGE}"),
            other => files.push(PathBuf::from(other)),
        }
    }
    if sub == "replay" {
        let [log] = files.as_slice() else {
            bail!("pass exactly one log to extract from (got {})", files.len());
        };
        return Ok(TraceCmd::Replay(trace::ReplayOptions {
            log: log.clone(),
            out,
        }));
    }
    let [a, b] = files.as_slice() else {
        bail!("pass exactly two logs to compare (got {})", files.len());
    };
    Ok(TraceCmd::Diff(trace::Options {
        a: a.clone(),
        b: b.clone(),
    }))
}

enum AssetsCmd {
    Pack(assets::PackOptions),
    Write(assets::WriteOptions),
}

fn parse_assets(mut args: impl Iterator<Item = String>) -> Result<AssetsCmd> {
    let sub = args
        .next()
        .context("assets needs a subcommand: pack or write")?;
    if sub == "-h" || sub == "--help" {
        help();
    }
    if sub != "pack" && sub != "write" {
        bail!("unknown assets subcommand `{sub}` (pack or write)");
    }
    let mut board: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut port: Option<PathBuf> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => help(),
            "--board" => board = Some(args.next().context("--board needs a value")?),
            "-o" if sub == "pack" => {
                out = Some(PathBuf::from(args.next().context("-o needs a value")?));
            }
            "--port" if sub == "write" => {
                port = Some(PathBuf::from(args.next().context("--port needs a value")?));
            }
            other => bail!("unknown option `{other}`\n\n{USAGE}"),
        }
    }
    // Both read the `[assets]` table, so the manifest is required here
    // (elsewhere it is optional).
    let manifest = find_manifest()?.context(
        "no wasmicon.toml found: assets pack / write read its [assets] table \
         (docs/app-workflow.md §4.7)",
    )?;
    let board = board.or_else(|| manifest.default_board.clone());
    Ok(if sub == "pack" {
        AssetsCmd::Pack(assets::PackOptions {
            manifest,
            board,
            out,
        })
    } else {
        AssetsCmd::Write(assets::WriteOptions {
            manifest,
            board,
            port,
        })
    })
}

fn help() -> ! {
    print!("{USAGE}");
    std::process::exit(0)
}

#[cfg(test)]
mod tests {
    use super::parse_display_size;

    #[test]
    fn display_size_takes_width_x_height_in_range() {
        assert_eq!(parse_display_size("720x1280").unwrap(), (720, 1280));
        assert_eq!(parse_display_size("1x4096").unwrap(), (1, 4096));
        for bad in [
            "720", "0x240", "320x4097", "320x", "x240", "320X240", "-1x2",
        ] {
            assert!(parse_display_size(bad).is_err(), "{bad}");
        }
    }
}
