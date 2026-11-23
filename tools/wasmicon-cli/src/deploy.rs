//! `wasmicon deploy`: sends an app to the board's slot (docs/app-workflow.md §3.5).
//!
//! Stage 1 hands the image to an external flasher. A Pico is flashed with
//! `picotool load -F` and started with `picotool reboot`: a firmware with the
//! picotool reset interface (`ports/rp2350`, docs/decisions.md A-11) reboots
//! into BOOTSEL when asked, so only a board running other firmware still
//! needs the button. The USB control channel (stage 2) is a separate item.
//!
//! Only the sectors that hold the app and the config slot are written, not
//! the whole app slot (`pack::regions`).
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
use wasmicon_port::roles::Wiring;

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
    /// `--flash-baud`: the speed espflash flashes at (`None` = espflash's
    /// own choice: `ESPFLASH_BAUD` when set, otherwise 115200). Not the
    /// firmware's UART speed, which `--monitor` always reads at
    /// `monitor::DEFAULT_BAUD`.
    pub flash_baud: Option<u32>,
    /// `--idle`: silence that ends the `--monitor` capture (`None` = never).
    pub idle: Option<std::time::Duration>,
    /// `--timeout`: overall limit of the `--monitor` capture (`None` = none).
    pub timeout: Option<std::time::Duration>,
}

/// Everything decided before flashing. The only side effect is writing the
/// image.
///
/// No `Debug`: `Profile` / `Slot` on the port side do not implement it (the
/// port avoids `core::fmt`). Tests take the fields out with `match`.
pub struct Plan {
    pub board: &'static Profile,
    pub slot: Slot,
    /// What goes to flash: the app image, then the config slot, each padded
    /// to whole sectors (`pack::regions`).
    pub regions: [pack::Region; 2],
    /// The files handed to the flasher, in the order they are written: one
    /// UF2 holding both regions for picotool, one `.bin` per region for
    /// espflash (`write-bin` takes one file at one offset).
    pub files: Vec<PathBuf>,
    pub wasm_len: usize,
    /// The slot image: header + wasm.
    pub image_len: usize,
    pub crc: u32,
    /// The wiring table written alongside (`lcd-cs=17 ...`). `None` = no
    /// roles handed out.
    pub roles: Option<String>,
    /// The display entry written alongside (`ili9341 spi=0 ...`), if any
    /// (docs/device-layer.md §5.2).
    pub display: Option<String>,
    /// The touch entry written alongside (`tab5`), if any
    /// (docs/device-layer.md §11.3).
    pub touch: Option<String>,
}

/// How to flash. Decided by the flasher, not by the board name.
///
/// Split out of `run` because it is the only branch that can be tested (the
/// part that calls the external flasher needs hardware).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mode {
    /// Do not run after writing (`espflash --after no-reset`, `picotool -F`).
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
            // does `--after hard-reset` by default, and `deploy` follows
            // `picotool load` with `picotool reboot`, so
            // without this the app runs right after flashing while the output
            // says "runs on the next reset" (a lie). Nobody wants that before
            // attaching a probe or on a board wired to a motor.
            keep_halted: hand_off || opts.no_run,
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
    let display = opts.manifest.as_ref().and_then(|m| m.display(board));
    let touch = opts.manifest.as_ref().and_then(|m| m.touch(board));
    let verdict = check::judge(&wasm, board, &facts, declared, &provided, display, touch);
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
    check_flash_baud(opts.flash_baud, slot.flasher)?;
    // The wiring table (docs/app-workflow.md §3.9). The firmware has no
    // default table, so it is written every time.
    let wiring = match &opts.manifest {
        Some(m) => m.wiring(board)?,
        None => Wiring::EMPTY,
    };
    let regions = pack::regions(&app, board, &wiring)?;

    // The files go next to the app. They are kept so the same write can be
    // repeated and the CRC cross-checked against the device's log.
    let files = match slot.flasher {
        Flasher::Picotool => vec![(opts.path.with_extension("uf2"), pack::uf2(&regions))],
        Flasher::Espflash { .. } => vec![
            (opts.path.with_extension("bin"), regions[0].bytes.clone()),
            (
                opts.path.with_extension("config.bin"),
                regions[1].bytes.clone(),
            ),
        ],
    };
    for (path, bytes) in &files {
        std::fs::write(path, bytes)
            .with_context(|| format!("cannot write `{}`", path.display()))?;
    }

    Ok(Plan {
        board,
        slot,
        regions,
        files: files.into_iter().map(|(p, _)| p).collect(),
        wasm_len: wasm.len(),
        image_len: app.len(),
        crc: pack::crc_of(&app),
        roles: pack::describe_roles(&wiring.roles),
        display: pack::describe_display(&wiring),
        touch: pack::describe_touch(&wiring),
    })
}

/// Checks, then flashes.
///
/// # Errors
/// When `prepare` fails. A flasher failure is `Ok(false)` (it is not a usage
/// error).
pub fn run(opts: &Options) -> Result<bool> {
    let plan = prepare(opts)?;

    print_summary(&plan, opts);
    match &plan.roles {
        Some(r) => println!("  wiring: {r}"),
        None => println!(
            "  wiring: none (no roles handed out; see [board.{}.roles] in wasmicon.toml)",
            plan.board.wiring
        ),
    }
    if let Some(d) = &plan.display {
        println!("  display: {d}");
    }
    if let Some(t) = &plan.touch {
        println!("  touch: {t}");
    }

    let mode = Mode::of(opts, plan.slot.flasher);

    // With `--monitor` on a board whose trace has its own port, open the port
    // and set the baud rate before anything is flashed: the app starts right
    // after the write (`picotool reboot`). Setting it while data flows mixes
    // up the lines (item 2 in the `monitor` module docs).
    let session = if opts.monitor && !opts.no_run && !mode.shared_port {
        let port = monitor::choose(&monitor::ports(), opts.port.as_deref())?;
        let s = monitor::open(&port, monitor::DEFAULT_BAUD)?;
        eprintln!("opened {} (before flashing)", s.port().display());
        Some(s)
    } else {
        None
    };

    // Stage 1 uses an external flasher (docs/app-workflow.md §3.5). The tool
    // differs per board.
    if !write_slot(
        &plan,
        opts.port.as_deref(),
        mode.keep_halted,
        opts.flash_baud,
    )? {
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
        let args = espflash_monitor_args(chip, opts.port.as_deref());
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let taken = monitor::capture_cmd(
            "espflash",
            &args,
            &monitor::Options {
                port: None,
                baud: monitor::DEFAULT_BAUD,
                out: opts.out.clone(),
                idle: opts.idle,
                timeout: opts.timeout,
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

    // Both flashers have started the app by now (`write-bin` resets,
    // `picotool reboot` runs it).
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
            idle: opts.idle,
            timeout: opts.timeout,
        },
    )
}

/// Prints what is about to be written.
///
/// The summary shows the slot offset. The absolute address picotool works
/// with means nothing on the ESP32 boards, which flash by offset.
fn print_summary(plan: &Plan, _opts: &Options) {
    let files: Vec<String> = plan.files.iter().map(|f| f.display().to_string()).collect();
    println!(
        "{} -> {} slot (+{:#x})  {} B (wasm {} B, crc32 {:08x})",
        files.join(", "),
        plan.board.name,
        plan.slot.offset,
        plan.image_len,
        plan.wasm_len,
        plan.crc
    );
}

impl Plan {
    /// Bytes that reach flash: the regions, whole sectors each.
    #[must_use]
    pub fn flash_len(&self) -> usize {
        self.regions.iter().map(|r| r.bytes.len()).sum()
    }
}

/// Writes the app and config regions. The tool differs per board.
fn write_slot(
    plan: &Plan,
    port: Option<&Path>,
    keep_halted: bool,
    flash_baud: Option<u32>,
) -> Result<bool> {
    match plan.slot.flasher {
        // Both regions in one UF2, so one write. `-x` cannot start the app
        // from a UF2 that holds no executable image (picotool 2.3.1 fails
        // with "Cannot execute as file does not contain a valid RP2
        // executable image" after writing), so the write leaves the board in
        // BOOTSEL (`-F`, whether it was running or already there) and a
        // separate reboot starts the app once.
        Flasher::Picotool => {
            let file = plan.files[0].to_string_lossy().into_owned();
            if !picotool(&picotool_load_uf2_args(&file))? {
                return Ok(false);
            }
            if keep_halted {
                return Ok(true);
            }
            picotool(&["reboot".to_string()])
        }
        // `write-bin` takes one file at one offset, so one call per region.
        // Every call but the last leaves the chip in its serial loader,
        // and the next one connects without resetting it again.
        Flasher::Espflash { .. } => {
            let last = plan.files.len() - 1;
            for (i, (file, region)) in plan.files.iter().zip(&plan.regions).enumerate() {
                let args = espflash_write_args(
                    region.offset,
                    &file.to_string_lossy(),
                    port,
                    i < last || keep_halted,
                    flash_baud,
                );
                let args = if i > 0 {
                    after_a_loader_call(args)
                } else {
                    args
                };
                if !espflash(&args, flash_baud)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
    }
}

/// Makes an `espflash write-bin` call connect to a chip that the previous
/// call left in its serial loader (`--after no-reset`): `--before no-reset`
/// skips the DTR/RTS reset into the loader, which is where the chip is.
#[must_use]
pub fn after_a_loader_call(mut args: Vec<String>) -> Vec<String> {
    args.splice(1..1, ["--before".to_string(), "no-reset".to_string()]);
    args
}

/// Refuses `--flash-baud` for a flasher that has no such setting, before
/// anything is written. picotool talks USB, so a speed would be silently
/// ignored and the next slow flash would look like a bug.
///
/// # Errors
/// When a speed is given for a picotool board.
pub fn check_flash_baud(flash_baud: Option<u32>, flasher: Flasher) -> Result<()> {
    // A `match`, so that a new flasher has to decide.
    match (flash_baud, flasher) {
        (None, _) | (Some(_), Flasher::Espflash { .. }) => Ok(()),
        (Some(_), Flasher::Picotool) => {
            bail!("--flash-baud is for boards flashed with espflash; picotool has no speed to set")
        }
    }
}

/// The arguments of `espflash monitor` for the capture after a deploy.
///
/// `--monitor-baud`: UART0 runs at `monitor::DEFAULT_BAUD`, and espflash
/// would read at its own default, 115200.
///
/// `--no-addresses`: by default espflash rewrites what it reads, appending
/// a symbol line in color after any text that looks like a code address. A
/// `blob.read` result whose data starts with `0x40004800` gained two lines
/// and a color code before the next call (docs/verification-report.md §32),
/// so the capture was no longer the trace the board printed.
#[must_use]
pub fn espflash_monitor_args(chip: &str, port: Option<&Path>) -> Vec<String> {
    let mut args = ["monitor", "-c", chip, "--non-interactive", "--no-addresses"]
        .map(str::to_string)
        .to_vec();
    // UART0's speed (`--baud` would be the ROM loader's, which flashing uses).
    args.extend([
        "--monitor-baud".to_string(),
        monitor::DEFAULT_BAUD.to_string(),
    ]);
    if let Some(dev) = port {
        args.extend(["--port".to_string(), dev.to_string_lossy().into_owned()]);
    }
    args
}

/// The arguments of `espflash write-bin` for one image.
///
/// `--baud` raises only the flashing speed: espflash switches the ROM
/// loader to it after connecting at 115200, and the native USB port of an
/// ESP32-S3 ignores it. A 4.2 MB assets image took 202 s at 115200
/// (docs/verification-report.md §30).
#[must_use]
pub fn espflash_write_args(
    offset: u32,
    image: &str,
    port: Option<&Path>,
    keep_halted: bool,
    flash_baud: Option<u32>,
) -> Vec<String> {
    let mut args = vec!["write-bin".to_string()];
    if let Some(dev) = port {
        args.extend(["--port".to_string(), dev.to_string_lossy().into_owned()]);
    }
    if let Some(b) = flash_baud {
        args.extend(["--baud".to_string(), b.to_string()]);
    }
    // With `--monitor` the board is not reset here. If it ran now, the output
    // would be over before the port is opened
    // (docs/verification-report.md §10). `espflash monitor` does the reset.
    if keep_halted {
        args.extend(["--after".to_string(), "no-reset".to_string()]);
    }
    args.extend([format!("{offset:#x}"), image.to_string()]);
    args
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
    flash_baud: Option<u32>,
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
            let args = espflash_write_args(offset, &image, port, keep_halted, flash_baud);
            espflash(&args, flash_baud)
        }
        // A Pico in BOOTSEL, or running a firmware with the picotool reset
        // interface, which `-f` / `-F` reboot into BOOTSEL first.
        Flasher::Picotool => {
            let addr = pack::XIP_BASE + u64::from(offset);
            picotool(&picotool_load_args(&image, addr, keep_halted))
        }
    }
}

/// Runs espflash and explains the usual causes of a failure.
fn espflash(args: &[String], flash_baud: Option<u32>) -> Result<bool> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let ok = spawn("espflash", &args)?;
    if !ok {
        eprintln!("  if several serial ports exist, pick one with --port");
        if let Some(b) = flash_baud {
            eprintln!(
                "  if it failed partway, the USB-serial bridge may not hold {b} baud; \
                 try a lower --flash-baud"
            );
        }
    }
    Ok(ok)
}

/// Runs picotool and explains the usual cause of a failure.
fn picotool(args: &[String]) -> Result<bool> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let ok = spawn("picotool", &args)?;
    if !ok {
        eprintln!(
            "  picotool reaches a board in BOOTSEL, or one running wasmicon firmware\n  \
             with the reset interface. Other firmware: hold BOOTSEL while plugging in\n  \
             USB, then run again"
        );
    }
    Ok(ok)
}

/// The arguments of `picotool load` for a UF2 file (`pack::uf2`), which
/// carries its own addresses. `-F` reboots a running firmware into BOOTSEL
/// through its reset interface and leaves it there; a board already in
/// BOOTSEL stays there too.
#[must_use]
pub fn picotool_load_uf2_args(file: &str) -> Vec<String> {
    ["load", file, "-t", "uf2", "-F"]
        .map(str::to_string)
        .to_vec()
}

/// The arguments of `picotool load` for one image at `addr`.
///
/// `-t bin` is explicit so the file type does not depend on the extension.
/// `-o` is an address, as picotool's help says ("Load offset (memory
/// address)"); an offset is rejected. `-f` reboots a running firmware into
/// BOOTSEL through its reset interface and back into the app afterwards,
/// and `-x` starts the app for a board that was in BOOTSEL already, so the
/// app starts once either way. `keep_halted` uses `-F` instead, which leaves
/// the board in BOOTSEL.
#[must_use]
pub fn picotool_load_args(image: &str, addr: u64, keep_halted: bool) -> Vec<String> {
    let mut args = ["load", image, "-t", "bin", "-o"]
        .map(str::to_string)
        .to_vec();
    args.push(format!("{addr:#x}"));
    if keep_halted {
        args.push("-F".to_string());
    } else {
        args.extend(["-f".to_string(), "-x".to_string()]);
    }
    args
}

/// Runs an external command. Not found is an error; failure is `false`.
fn spawn(cmd: &str, args: &[&str]) -> Result<bool> {
    let status = Command::new(cmd).args(args).status().with_context(|| {
        format!("cannot start {cmd} (run `wasmicon doctor` to see what is installed)")
    })?;
    Ok(status.success())
}
