//! `wasmicon run`: runs an app on the host port (docs/app-workflow.md §4.2).
//!
//! It only calls `run_wasm_opts` of `wasmicon-host`. The crate stays and is
//! used as a library (`verify/differential` depends on it too).
//!
//! The host is more permissive than every board (`Config::DEFAULT` = 65536
//! pages). An app can pass here and fail validate on hardware, so the
//! hardware verdict comes from `wasmicon check --board <board>`
//! (docs/app-workflow.md §4.3).

use anyhow::{Context, Result};
use std::path::PathBuf;
use wasmicon_host::Usage;
use wasmicon_port::profile::{self, Profile};

pub struct Options {
    pub path: PathBuf,
    /// Print every host call to stdout in the docs/abi-spec.md §9 format.
    pub trace: bool,
    /// Recorded I2C responses. The host has no real sensor, so what reads
    /// return comes from a file (`verify/sht4x-replay.txt`).
    pub i2c_replay: Option<PathBuf>,
    /// Assets the guest may open, as `(name, file)`: the `[assets]` table of
    /// `wasmicon.toml` with `--asset` applied on top
    /// (docs/app-workflow.md §3.10).
    pub assets: Vec<(String, PathBuf)>,
    /// Where to write the mock display's visible picture as a PNG after the
    /// run (docs/device-layer.md §5.3).
    pub display_out: Option<PathBuf>,
    /// Size of the mock display; `None` is the 320x240 default.
    pub display_size: Option<(u32, u32)>,
}

/// Runs the app. `true` when it finishes without a trap.
///
/// # Errors
/// When a file cannot be read. A guest trap is `Ok(false)` (it is not a
/// usage error).
pub fn run(opts: &Options) -> Result<bool> {
    let wasm = std::fs::read(&opts.path)
        .with_context(|| format!("cannot read `{}`", opts.path.display()))?;

    // Without `--i2c-replay`, the environment variable is consulted. The
    // `ports/host` binary reads it (the capture procedure in
    // docs/verification-report.md §6), so if the CLI ignored it, the run
    // would silently fall back to "no sensor" and the trace would diverge
    // from the recorded lines.
    let from_env = std::env::var("WASMICON_I2C_REPLAY").ok().map(PathBuf::from);
    let replay_path = opts.i2c_replay.clone().or(from_env);
    let i2c_replay = match &replay_path {
        None => Vec::new(),
        Some(p) => wasmicon_host::load_i2c_replay(p)
            .with_context(|| format!("cannot read `{}`", p.display()))?,
    };

    // A missing asset file stops the run. Silently serving fewer assets than
    // the table names would make the guest's `not-found` look like its own
    // mistake.
    let mut assets = Vec::new();
    for (name, path) in &opts.assets {
        let body = std::fs::read(path)
            .with_context(|| format!("cannot read asset `{name}` at `{}`", path.display()))?;
        assets.push((name.clone(), body));
    }

    let host_opts = wasmicon_host::Options {
        trace: opts.trace,
        i2c_replay,
        spi_unsupported: false,
        assets,
        no_display: false,
        display_size: opts.display_size,
        display: None,
        bus_log: false,
    };

    // The entry point that returns the trace even on failure. A trap is
    // exactly when the trace is wanted (which host call diverged is only
    // there).
    let (out, result) = wasmicon_host::run_wasm_capture(&wasm, host_opts);
    if opts.trace {
        print!("{}", out.trace);
    }
    if let (Some(path), Some(frame)) = (&opts.display_out, &out.frame) {
        std::fs::write(path, crate::png::encode(frame))
            .with_context(|| format!("cannot write `{}`", path.display()))?;
        eprintln!(
            "display: {}x{} -> {}",
            frame.width,
            frame.height,
            path.display()
        );
    }
    // stderr, so stdout stays the trace that `trace diff` compares.
    if let Some(u) = &out.usage {
        for line in usage_report(u, profile::PROFILES) {
            eprintln!("{line}");
        }
        if let Err(wasmicon_core::Error::Exhausted("call stack exhausted")) = &result {
            // The run stopped at the host's own limit, so the depth it would
            // have needed is not known.
            eprintln!("  max_call_depth is the host's limit: the run needed more");
        }
    }
    match result {
        Ok(()) => Ok(true),
        Err(e) => {
            // docs/decisions.md A-04: after a trap, log the reason and halt.
            eprintln!("wasmicon: {} [{}]", e.reason(), e.kind().name());
            if opts.trace {
                let n = out.trace.lines().count();
                eprintln!("  the {n} trace lines up to this point went to stdout");
            }
            Ok(false)
        }
    }
}

/// What the run needed against every board's `Config`.
///
/// `check` can judge the validate-time limits, but call depth and the operand
/// stack depend on the path a run takes, so a module that passes `check` can
/// still trap on a board. A board run with other input (a sensor, a button)
/// can go deeper than this host run.
#[must_use]
pub fn usage_report(u: &Usage, boards: &[&Profile]) -> Vec<String> {
    let mut lines = vec![format!(
        "usage: max_control_depth {}, max_value_stack {}, max_locals {}, \
         max_call_depth {}, operand_stack_slots {}",
        u.max_control_depth,
        u.max_value_stack,
        u.max_locals,
        u.max_call_depth,
        u.operand_stack_slots
    )];
    for p in boards.iter().filter(|p| p.name != profile::HOST.name) {
        let over: Vec<String> = u
            .over(&p.config)
            .into_iter()
            .map(|(field, need, limit)| format!("{field} {need} > {limit}"))
            .collect();
        if !over.is_empty() {
            lines.push(format!("  over {}: {}", p.name, over.join(", ")));
        }
    }
    if lines.len() == 1 {
        lines.push("  within every board's limits".to_string());
    }
    lines
}
