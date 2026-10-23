//! `wasmicon deploy`: sends an app to the board's slot (docs/app-workflow.md §3.5).
//!
//! Stage 1 hands the image to an external flasher. Until the USB control
//! channel (stage 2) exists, `picotool` is called, so the Pico still needs
//! BOOTSEL. Stage 2 removes that.
//!
//! `check` runs before anything is sent. If an app that does not run on that
//! board were written to the slot, the failure would show only on hardware
//! (docs/app-workflow.md §4.3). If `check` fails, nothing is flashed.
//!
//! The verdict and the image preparation (`prepare`) are separate from the
//! flasher call (`run`); the former can be called from tests.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

use wasmicon_port::profile::{Flasher, Profile, Slot};

use crate::manifest::Manifest;
use crate::{check, monitor, pack};
use wasmicon_port::roles::RoleMap;

pub struct Options {
    pub path: PathBuf,
    /// Target board: `--board` or `[defaults] board` in `wasmicon.toml`.
    /// Required (otherwise there is no way to know where to write).
    pub board: Option<String>,
    /// `wasmicon.toml`. `[requirements] pin-roles`, when present, is
    /// cross-checked.
    pub manifest: Option<Manifest>,
    /// Do not reset after flashing (the default resets and runs).
    pub no_run: bool,
    /// After flashing and resetting, capture the trace.
    pub monitor: bool,
    /// Where `--monitor` writes the capture (`None` = stdout only).
    pub out: Option<PathBuf>,
    /// Serial port for `--monitor` (`None` = pick from `/dev/cu.usb*`).
    pub port: Option<PathBuf>,
}

/// Everything decided before flashing. The only side effect is writing the
/// image.
///
/// No `Debug`: `Profile` / `Slot` on the port side do not implement it (the
/// port avoids `core::fmt`). Tests take the fields out with `match`.
pub struct Plan {
    pub board: &'static Profile,
    pub slot: Slot,
    /// The slot image that was written.
    pub image: PathBuf,
    pub wasm_len: usize,
    pub image_len: usize,
    pub crc: u32,
    /// The wiring table written alongside (`lcd-cs=17 ...`). `None` = no
    /// roles handed out.
    pub roles: Option<String>,
}

/// How to flash. Decided by the flasher, not by the board name.
///
/// Split out of `run` because it is the only branch that can be tested (the
/// part that calls the external flasher needs hardware).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mode {
    /// Do not run after writing (`espflash --after no-reset`).
    pub keep_halted: bool,
    /// Hand reset and capture to `espflash monitor`.
    pub hand_off: bool,
    /// Flashing and the trace share one serial port.
    pub shared_port: bool,
}

impl Mode {
    /// Decides from the options and the flasher.
    #[must_use]
    pub fn of(opts: &Options, flasher: Flasher) -> Self {
        // A board with a shared port leaves the capture to the same tool
        // (docs/verification-report.md §10).
        let shared_port = matches!(flasher, Flasher::Espflash { .. });
        let hand_off = opts.monitor && !opts.no_run && shared_port;
        Mode {
            // `--no-run` must not run the app either. `espflash write-bin`
            // does `--after hard-reset` by default, so without this the app
            // runs right after flashing while the output says "runs on the
            // next reset" (a lie). Nobody wants that before attaching a probe
            // or on a board wired to a motor. picotool resets in a separate
            // call, so it needs no suppression.
            keep_halted: hand_off || (opts.no_run && shared_port),
            hand_off,
            shared_port,
        }
    }
}

/// Checks the app and prepares the image.
///
/// # Errors
/// When no board is named, the slot location is not decided, the image does
/// not fit the slot, `check` fails, or a file cannot be read or written.
pub fn prepare(opts: &Options) -> Result<Plan> {
    let Some(name) = opts
        .board
        .clone()
        .or_else(|| opts.manifest.as_ref().and_then(|m| m.default_board.clone()))
    else {
        bail!("no target board: pass --board or set [defaults] board in wasmicon.toml");
    };
    let board = pack::resolve_board(&name)?;

    let wasm = std::fs::read(&opts.path)
        .with_context(|| format!("cannot read `{}`", opts.path.display()))?;

    // Never flash what does not run. Declared role names are part of the
    // guarantee (docs/app-workflow.md §4.7).
    let facts = check::facts(&wasm)?;
    let declared: Option<&[String]> = opts
        .manifest
        .as_ref()
        .filter(|m| !m.pin_roles.is_empty())
        .map(|m| m.pin_roles.as_slice());
    let provided = check::provided_roles(board, opts.manifest.as_ref());
    if facts.failures() > 0 {
        bail!(
            "check failed ({} failures common to every board); see `wasmicon check {}`",
            facts.failures(),
            opts.path.display()
        );
    }
    let verdict = check::judge(&wasm, board, &facts, declared, &provided);
    if !verdict.is_ok() {
        bail!(
            "check failed ({} failures on {}); see `wasmicon check {} --board {}`",
            verdict.fails(),
            board.name,
            opts.path.display(),
            board.name
        );
    }

    let app = pack::build(&wasm);
    let slot = pack::fits(board, app.len())?;
    // The wiring table (docs/app-workflow.md §3.9). The firmware has no
    // default table, so it is written every time.
    let roles = match &opts.manifest {
        Some(m) => m.role_map(board)?,
        None => RoleMap::EMPTY,
    };
    let image = pack::with_roles(&app, board, &roles)?;

    // The image goes next to the app (same name as `pack` uses). It is kept
    // so the same image can be flashed again and its CRC cross-checked
    // against the device's log.
    let mut out = opts.path.clone();
    out.set_extension("bin");
    std::fs::write(&out, &image).with_context(|| format!("cannot write `{}`", out.display()))?;

    Ok(Plan {
        board,
        slot,
        image: out,
        wasm_len: wasm.len(),
        image_len: app.len(),
        crc: pack::crc_of(&image),
        roles: pack::describe_roles(&roles),
    })
}

/// Checks, then flashes.
///
/// # Errors
/// When `prepare` fails. A flasher failure is `Ok(false)` (it is not a usage
/// error).
pub fn run(opts: &Options) -> Result<bool> {
    let plan = prepare(opts)?;

    // The summary shows the slot offset. The absolute address picotool
    // takes (`write_at` derives it) means nothing on ESP32-S3, which flashes
    // by offset; showing it would be wrong for one board or the other.
    println!(
        "{} -> {} slot (+{:#x})  {} B (wasm {} B, crc32 {:08x})",
        plan.image.display(),
        plan.board.name,
        plan.slot.offset,
        plan.image_len,
        plan.wasm_len,
        plan.crc
    );
    match &plan.roles {
        Some(r) => println!("  wiring: {r}"),
        None => println!(
            "  wiring: none (no roles handed out; see [board.{}.roles] in wasmicon.toml)",
            plan.board.wiring
        ),
    }

    let mode = Mode::of(opts, plan.slot.flasher);

    // Stage 1 uses an external flasher (docs/app-workflow.md §3.5). The tool
    // differs per board.
    if !write_slot(&plan, opts.port.as_deref(), mode.keep_halted)? {
        return Ok(false);
    }

    if opts.no_run {
        println!("-> written; the app runs on the next reset");
        return Ok(true);
    }

    // Only espflash hands off, so the chip name is there.
    if let (true, Flasher::Espflash { chip }) = (mode.hand_off, plan.slot.flasher) {
        // `espflash monitor` resets through DTR/RTS and then reads. The port
        // is not opened here (an open port makes espflash fail with
        // `Resource busy`).
        println!("-> handing reset and capture to espflash monitor:");
        let mut args = vec!["monitor", "-c", chip, "--non-interactive"];
        let p;
        if let Some(dev) = opts.port.as_deref() {
            p = dev.to_string_lossy().into_owned();
            args.extend_from_slice(&["--port", &p]);
        }
        let taken = monitor::capture_cmd(
            "espflash",
            &args,
            &monitor::Options {
                port: None,
                baud: monitor::DEFAULT_BAUD,
                out: opts.out.clone(),
                idle: Some(monitor::DEFAULT_IDLE),
                timeout: None,
            },
        );
        if taken.is_err() {
            // The image was flashed with `--after no-reset`, so the board is
            // parked in the bootloader. Exiting silently looks like "powered
            // but nothing comes out" (`espflash reset` does not help: it
            // never exits; docs/verification-report.md §10).
            eprintln!(
                "  the board is parked in the bootloader (flashed with --after no-reset).\n  \
                 press EN or run deploy again"
            );
        }
        return taken;
    }

    // With `--monitor`, open the port and set the baud rate before the reset.
    // Setting it while data flows mixes up the lines (item 2 in the `monitor`
    // module docs).
    let session = if opts.monitor {
        let port = monitor::choose(&monitor::ports(), opts.port.as_deref())?;
        let s = monitor::open(&port, monitor::DEFAULT_BAUD)?;
        eprintln!("opened {} (before reset)", s.port().display());
        Some(s)
    } else {
        None
    };

    // espflash `write-bin` resets by default (see above).
    if !mode.shared_port && !reboot()? {
        eprintln!("  written, but the reset failed; unplug and replug USB");
        return Ok(false);
    }

    let Some(session) = session else {
        println!("-> running. To capture the trace:");
        println!("    wasmicon monitor -o app.log   # then reset the board");
        println!("  next time, deploy --monitor does both in one step");
        return Ok(true);
    };

    println!("-> running; capturing the trace:");
    monitor::capture(
        session,
        &monitor::Options {
            port: None,
            baud: monitor::DEFAULT_BAUD,
            out: opts.out.clone(),
            idle: Some(monitor::DEFAULT_IDLE),
            timeout: None,
        },
    )
}

/// Writes the slot. The tool differs per board.
fn write_slot(plan: &Plan, port: Option<&Path>, keep_halted: bool) -> Result<bool> {
    write_at(
        plan.slot.flasher,
        plan.slot.offset,
        &plan.image,
        port,
        keep_halted,
    )
}

/// Writes `image` at `offset` of flash with the board's flasher. `deploy`
/// uses it for the app slot, `assets write` for the assets slot
/// (docs/app-workflow.md §3.10).
///
/// # Errors
/// When the flasher cannot be started. A flasher that runs and fails is
/// `Ok(false)`.
pub fn write_at(
    flasher: Flasher,
    offset: u32,
    image: &Path,
    port: Option<&Path>,
    keep_halted: bool,
) -> Result<bool> {
    let image = image.to_string_lossy().into_owned();
    // Never branch on the board name string. The profile carries the tool, so
    // a new port cannot fall into a default branch.
    match flasher {
        // ESP32-S3 enters the ROM bootloader through DTR/RTS, so no button is
        // needed. `write-bin` takes a flash offset.
        Flasher::Espflash { .. } => {
            // `write-bin` takes a flash offset (unlike picotool's `-o`, which
            // is an address). It does `--after hard-reset` by default, so
            // reset is not called separately: `espflash reset` never exits,
            // keeps DTR/RTS asserted and leaves the board parked in download
            // mode (seen on hardware; docs/verification-report.md §10).
            let offset = format!("{offset:#x}");
            let mut args = vec!["write-bin"];
            let p;
            if let Some(dev) = port {
                p = dev.to_string_lossy().into_owned();
                args.extend_from_slice(&["--port", &p]);
            }
            // With `--monitor` the board is not reset here. If it ran now,
            // the output would be over before the port is opened
            // (docs/verification-report.md §10). `espflash monitor` does the
            // reset.
            if keep_halted {
                args.extend_from_slice(&["--after", "no-reset"]);
            }
            args.extend_from_slice(&[&offset, &image]);
            let ok = spawn("espflash", &args)?;
            if !ok {
                eprintln!("  if several serial ports exist, pick one with --port");
            }
            Ok(ok)
        }
        // Pico boards need BOOTSEL (the stage 2 USB control channel removes
        // that). `-t bin` is explicit so the file type does not depend on
        // the extension. `-o` is an address, as picotool's help says ("Load
        // offset (memory address)"); an offset is rejected.
        Flasher::Picotool => {
            let addr = format!("{:#x}", pack::XIP_BASE + u64::from(offset));
            let ok = spawn("picotool", &["load", &image, "-t", "bin", "-o", &addr])?;
            if !ok {
                eprintln!(
                    "  hold BOOTSEL while plugging in USB, then run again\n  \
                     (the USB control channel will remove this step; docs/app-workflow.md §3.5)"
                );
            }
            Ok(ok)
        }
    }
}

/// Resets a Pico board so the app runs (ESP32's `write-bin` does it by
/// default). `picotool load` leaves the board in the bootloader; `deploy`
/// and `assets write` both call this after it.
///
/// # Errors
/// When picotool cannot be started. A failed reboot is `Ok(false)`.
pub fn reboot() -> Result<bool> {
    spawn("picotool", &["reboot"])
}

/// Runs an external command. Not found is an error; failure is `false`.
fn spawn(cmd: &str, args: &[&str]) -> Result<bool> {
    let status = Command::new(cmd).args(args).status().with_context(|| {
        format!("cannot start {cmd} (run `wasmicon doctor` to see what is installed)")
    })?;
    Ok(status.success())
}
