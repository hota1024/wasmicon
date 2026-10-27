//! `wasmicon check`: checks with the real runtime whether an app runs on a board.
//!
//! What it looks at (docs/app-workflow.md §4.3):
//!
//! - Every import matches the table by name and signature
//!   (docs/abi-spec.md §6.4, §7). The table is the generated file in
//!   `runtime`, derived from `wit/`.
//! - The `run` and `memory` exports (docs/abi-spec.md §3.3, §6.2).
//! - `validate` with that board's `Config` (`max_memory_pages` and so on).
//! - Whether every interface the app imports is implemented by that port.
//!   `ports/rp2040` returns `unsupported` for SPI / I2C, so without this the
//!   static check passes and the app fails only on hardware.
//! - Whether every role the app declares (`[requirements] pin-roles` in
//!   `wasmicon.toml`) is in that board's wiring table. On hardware the table
//!   is `[board.<name>.roles]` (which `deploy` writes to the config slot); on
//!   the host it is the mock's table. The firmware has no default table, so a
//!   role missing from the table is `unsupported` on hardware
//!   (docs/app-workflow.md §3.9).
//! - Whether every file of `[assets]` exists, and whether the slot image they
//!   make fits that board's assets slot (docs/app-workflow.md §3.10). The
//!   shareware DOOM IWAD fits the ESP boards and not the Pico 2 W, and that
//!   is better learned here than from `not-found` on the device.
//!
//! What it cannot do: enumerate role names. The argument of `pin-by-role` is
//! a `string` passed at run time, so it cannot be read reliably from the
//! `.wasm`. An app that declares no roles has none cross-checked.
//!
//! The verdict (`facts` / `judge`) is separate from the printing so that tests
//! can call the verdict alone.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::manifest::Manifest;
use crate::pack;
use wasmicon_core::module::{ExportDesc, ImportDesc};
use wasmicon_core::{Arena, Exec, decode, generated, instantiate, validate};
use wasmicon_host::Usage;
use wasmicon_port::Hal;
use wasmicon_port::assets::Directory;
use wasmicon_port::profile::{self, Interfaces, Profile, SlotRead};
use wasmicon_port::roles::{DisplayConfig, Wiring};

/// Arena for the decode that only extracts facts (board-independent).
///
/// Sized like the most permissive profile (host). If it were smaller, a
/// `.wasm` that `wasmicon run` and `check --board host` can read would stop
/// here with "cannot decode", before the module section and the per-board
/// verdicts are printed. The per-board check uses that board's real size
/// (`Profile::arena`).
const FACTS_ARENA: usize = profile::HOST.arena;

/// The one import allowed outside `world app`. The spelling is owned by
/// `ports/common` (its `Resolver` decides what links; if the two disagreed,
/// check would call something that links "not in the table").
use wasmicon_port::AS_ABORT;

/// Verdict on one import.
#[derive(PartialEq, Eq)]
pub enum ImportState {
    /// Name and signature both match.
    Ok,
    /// Not in the table (link error).
    Unknown,
    /// The name exists but the signature differs (link error).
    SigMismatch { want: &'static str },
    /// AssemblyScript's `env.abort`.
    AsAbort,
    /// Not a function import (docs/abi-spec.md §6.2: no memory import).
    NotAFunc,
    /// The type index points outside the type section (corrupt `.wasm`).
    BadTypeIndex { idx: u32 },
}

impl ImportState {
    /// Whether this import breaks linking.
    #[must_use]
    pub fn is_failure(&self) -> bool {
        !matches!(self, ImportState::Ok | ImportState::AsAbort)
    }
}

pub struct ImportCheck {
    pub module: String,
    pub name: String,
    pub state: ImportState,
}

/// Board-independent facts. Owned, so they outlive the arena.
pub struct Facts {
    pub imports: Vec<ImportCheck>,
    /// Import count per interface, keyed by the `<iface>` of
    /// `wasmicon:hal/<iface>@0.1.0`.
    pub per_iface: BTreeMap<String, usize>,
    /// The `run` export. `None` = missing, `Some(Err(..))` = wrong type.
    pub run_export: Option<std::result::Result<(), String>>,
    pub has_memory_export: bool,
    pub mem_pages: Option<u32>,
    pub table_elems: Option<u32>,
    /// The validate-time stack limits the module needs, measured under the
    /// host's looser `Config` so that a board's failure can say by how much.
    /// `None` when even the host refuses it (the board sections say why).
    pub needs: Option<Usage>,
}

impl Facts {
    /// Number of imports that break linking.
    #[must_use]
    pub fn import_failures(&self) -> usize {
        self.imports.iter().filter(|i| i.state.is_failure()).count()
    }

    /// Number of board-independent failures (imports, exports, memory
    /// definition).
    ///
    /// Independent defects are not folded into one. Folding them means the
    /// person who fixes "the one" and reruns is only then told about the
    /// second, which was already known.
    #[must_use]
    pub fn failures(&self) -> usize {
        self.import_failures()
            + usize::from(!matches!(self.run_export, Some(Ok(()))))
            + usize::from(!self.has_memory_export)
            // docs/abi-spec.md §6.2: define and export exactly one memory.
            + usize::from(self.mem_pages.is_none())
    }
}

/// The stage that failed. `validate` and `instantiate` are kept apart.
///
/// Running out of arena happens in instantiate; reporting it as "validate
/// failed" steers people toward shrinking `max_memory_pages`, when the fix is
/// a larger arena.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Decode,
    Validate,
    /// Allocating `Exec` and instantiating (linear memory takes the rest of
    /// the arena).
    Instantiate,
}

impl Stage {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Stage::Decode => "decode",
            Stage::Validate => "validate",
            Stage::Instantiate => "instantiate",
        }
    }
}

/// Verdict for one board.
pub struct Verdict {
    pub board: &'static str,
    /// `Ok(the board's page limit)` or the reason it failed.
    pub validate: std::result::Result<u32, String>,
    /// The stage that failed (`Instantiate` when everything passed, meaning
    /// it went all the way).
    pub stage: Stage,
    /// Interfaces the app uses that this port does not implement.
    pub unimplemented: Vec<String>,
    /// Roles declared in `wasmicon.toml` that are missing from this board's
    /// wiring table.
    pub missing_declared_roles: Vec<String>,
    /// The display the app uses, as this board would run it: `None` when the
    /// app imports no display or the board cannot have one (`unimplemented`
    /// says so), else the backend, or `Err` with the reason the board would
    /// have no display: `[board.<name>.display]` is missing, or names a
    /// backend this firmware lacks (docs/device-layer.md §6).
    pub display: Option<std::result::Result<String, String>>,
}

impl Verdict {
    /// Row label. Names the stage that failed (never calls `instantiate`
    /// `validate`).
    #[must_use]
    pub fn stage_label(&self) -> String {
        if self.validate.is_err() {
            self.stage.label().to_string()
        } else {
            "validate".to_string()
        }
    }

    /// Number of failures specific to this board.
    ///
    /// Import and export defects are board-independent, so they are not
    /// counted here (the module section counts them once; `Facts::failures`).
    /// Adding the same count to every board would hide which problems are
    /// board-specific.
    #[must_use]
    pub fn fails(&self) -> usize {
        usize::from(self.validate.is_err())
            + usize::from(matches!(self.display, Some(Err(_))))
            + self.unimplemented.len()
            + self.missing_declared_roles.len()
    }

    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.fails() == 0
    }
}

pub struct Options {
    pub path: PathBuf,
    /// `--board`. `None` checks every board (the default, since one binary
    /// for all boards is the point).
    pub board: Option<String>,
    /// `wasmicon.toml` (docs/app-workflow.md §4.7). `[requirements] pin-roles`
    /// is cross-checked against `[board.<name>.roles]`.
    pub manifest: Option<Manifest>,
}

/// One `[assets]` entry as `check` sees it (docs/app-workflow.md §3.10).
pub struct AssetCheck {
    pub name: String,
    pub path: PathBuf,
    /// The file's length, or `None` when there is no file there (a
    /// directory counts as no file: `run` and `assets pack` cannot read it).
    pub len: Option<u64>,
}

/// Looks at every file of the manifest's `[assets]` table.
#[must_use]
pub fn asset_checks(manifest: Option<&Manifest>) -> Vec<AssetCheck> {
    manifest
        .map(|m| {
            m.assets
                .iter()
                .map(|(name, path)| AssetCheck {
                    name: name.clone(),
                    path: path.clone(),
                    len: std::fs::metadata(path)
                        .ok()
                        .filter(std::fs::Metadata::is_file)
                        .map(|md| md.len()),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Length of the slot image the table would produce: the directory sector
/// plus every body on a 4 KiB boundary. `None` when a file is missing, or
/// when the bodies cannot be laid out (one over 4 GiB, or the lot past
/// the end of a 32-bit offset).
#[must_use]
pub fn assets_image_len(checks: &[AssetCheck]) -> Option<u64> {
    let mut dir = Directory::EMPTY;
    for c in checks {
        let len = u32::try_from(c.len?).ok()?;
        // The CRC is not needed for the length.
        dir.insert(c.name.as_bytes(), len, 0).ok()?;
    }
    Some(u64::from(dir.image_len()))
}

/// Whether an image of `image_len` bytes fits the board's assets slot.
/// `None` when the board has no slot (the host serves files).
#[must_use]
pub fn assets_fit(p: &Profile, image_len: u64) -> Option<bool> {
    p.assets.map(|sl| image_len <= u64::from(sl.len))
}

/// Checks and prints the result. `true` when every board passes.
pub fn run(opts: &Options) -> Result<bool> {
    let wasm = std::fs::read(&opts.path)
        .with_context(|| format!("cannot read `{}`", opts.path.display()))?;

    // Board names are resolved in `pack::resolve_board` only (two copies of
    // the message and the candidate list would be fixed on one side only).
    let boards: Vec<&'static Profile> = match &opts.board {
        Some(name) => vec![pack::resolve_board(name)?],
        None => profile::PROFILES.to_vec(),
    };

    let facts = facts(&wasm)?;
    let declared: Option<&[String]> = opts
        .manifest
        .as_ref()
        .filter(|m| !m.pin_roles.is_empty())
        .map(|m| m.pin_roles.as_slice());
    let assets = asset_checks(opts.manifest.as_ref());
    // A missing asset file is common to every board, like a bad import.
    let missing = print_module(&opts.path, &wasm, &facts, declared, &assets);

    let common = facts.failures() + missing;
    let mut all_ok = common == 0;
    for board in boards {
        let provided = provided_roles(board, opts.manifest.as_ref());
        let display = opts.manifest.as_ref().and_then(|m| m.display(board));
        let v = judge(&wasm, board, &facts, declared, &provided, display);
        println!();
        let fit_fails = print_verdict(&v, &facts, common, board, declared, &assets);
        all_ok &= v.is_ok() && fit_fails == 0;
    }
    Ok(all_ok)
}

/// Decodes and extracts the board-independent facts.
///
/// # Errors
/// When decoding fails (corrupt `.wasm`, unsupported feature set, ...).
pub fn facts(wasm: &[u8]) -> Result<Facts> {
    let mut buf = vec![0u8; FACTS_ARENA];
    let mut arena = Arena::new(&mut buf);
    let m = decode::decode(wasm, &mut arena).map_err(|e| anyhow::anyhow!(reason(e)))?;

    let mut scratch_buf = vec![0u8; profile::HOST.scratch];
    let mut scratch = Arena::new(&mut scratch_buf);
    let needs = validate::validate(&m, &profile::HOST.config, &mut arena, &mut scratch)
        .ok()
        .map(|v| Usage::of_module(&v));

    let mut imports = Vec::new();
    let mut per_iface = BTreeMap::new();
    for imp in m.imports {
        let state = match imp.desc {
            ImportDesc::Func(ty) => match m.types.get(ty as usize) {
                // decode does not check import type indices. Diagnosing a
                // corrupt `.wasm` is this command's job, so it must not
                // panic here.
                None => ImportState::BadTypeIndex { idx: ty },
                Some(ft) => match generated::resolve(imp.module, imp.name) {
                    Some(d) if ft.sig_matches(d.sig) => ImportState::Ok,
                    Some(d) => ImportState::SigMismatch { want: d.sig },
                    None if (imp.module, imp.name) == AS_ABORT => ImportState::AsAbort,
                    None => ImportState::Unknown,
                },
            },
            _ => ImportState::NotAFunc,
        };
        if state == ImportState::Ok
            && let Some(iface) = iface_of(imp.module)
        {
            *per_iface.entry(iface.to_string()).or_insert(0) += 1;
        }
        imports.push(ImportCheck {
            module: imp.module.to_string(),
            name: imp.name.to_string(),
            state,
        });
    }

    // docs/abi-spec.md §3.3 says `run: func()`. A `run` with parameters or
    // results fails at run time with `wrong arity`, so the type is checked
    // here too (checking only that the export exists lets a wrong signature
    // through).
    let run_export = m
        .exports
        .iter()
        .find_map(|e| match e.desc {
            ExportDesc::Func(i) if e.name == "run" => Some(i),
            _ => None,
        })
        .map(|idx| run_signature(&m, idx));
    let has_memory_export = m
        .exports
        .iter()
        .any(|e| e.name == "memory" && matches!(e.desc, ExportDesc::Memory(_)));

    Ok(Facts {
        imports,
        per_iface,
        run_export,
        has_memory_export,
        mem_pages: m.mems.first().map(|l| l.min),
        table_elems: m.tables.first().map(|l| l.min),
        needs,
    })
}

/// Judges one board.
///
/// `declared` is `[requirements] pin-roles` from `wasmicon.toml`; `provided`
/// is the roles that board hands out (`provided_roles`). A declared role that
/// is not provided counts as a failure.
#[must_use]
pub fn judge(
    wasm: &[u8],
    p: &'static Profile,
    f: &Facts,
    declared: Option<&[String]>,
    provided: &[String],
    display: Option<DisplayConfig>,
) -> Verdict {
    let (validate, stage) = match instantiate_with(wasm, p) {
        Ok(()) => (Ok(p.config.max_memory_pages), Stage::Instantiate),
        Err((stage, e)) => (Err(e), stage),
    };

    let unimplemented = f
        .per_iface
        .keys()
        .filter(|iface| implemented(&p.interfaces, iface) == Some(false))
        .cloned()
        .collect();

    let missing_declared_roles = declared
        .unwrap_or(&[])
        .iter()
        .filter(|r| !provided.contains(r))
        .cloned()
        .collect();

    // A board reads its display backend from the config slot, so a display
    // app needs `[board.<name>.display]`; the host's mock is built in.
    let display = (f.per_iface.contains_key("display") && p.interfaces.display).then(|| {
        match display {
            _ if p.role_slot.is_none() => Ok("built-in mock".to_string()),
            None => Err(format!(
                "none for this board  <- fails (add [board.{}.display] to wasmicon.toml)",
                p.wiring
            )),
            // No firmware has the Tab5 panel's backend yet: on the board
            // `open` would return `unsupported` (docs/TODO.md §6).
            Some(DisplayConfig::Tab5) => {
                Err("tab5  <- fails (no firmware has this backend yet)".to_string())
            }
            Some(d) => Ok(pack::describe_display(&Wiring {
                roles: wasmicon_port::roles::RoleMap::EMPTY,
                display: Some(d),
            })
            .unwrap_or_default()),
        }
    });

    Verdict {
        board: p.name,
        validate,
        stage,
        unimplemented,
        missing_declared_roles,
        display,
    }
}

/// Runs decode -> validate -> `Exec` -> instantiate with the board's real
/// arena sizes.
///
/// `validate` alone is not enough. Linear memory takes the rest of the arena
/// (`Arena::alloc_rest`), so even within `max_memory_pages` the
/// `min_pages * 64 KiB` may not fit in what decode / validate / `Exec` left.
/// On hardware that is where `instantiate` fails. The board's real `arena`
/// and `scratch` sizes are used here to reproduce that headroom
/// (`ports/rp2040` has only 160 KiB, leaving about 20 KiB with 2 pages).
///
/// Linking is done by `Hal` (exact match of import names and signatures,
/// docs/abi-spec.md §6.4). The board is the host mock; `instantiate` does not
/// run the guest, so the result is the same with any board implementation.
///
/// # Errors
/// When decode, validate, `Exec` or instantiate fails. The string has the
/// form `reason() [kind]`.
pub fn instantiate_with(wasm: &[u8], p: &Profile) -> std::result::Result<(), (Stage, String)> {
    let mut buf = vec![0u8; p.arena];
    let mut scratch_buf = vec![0u8; p.scratch];
    let mut arena = Arena::new(&mut buf);
    let mut scratch = Arena::new(&mut scratch_buf);

    // A board that copies the app into RAM has that much less arena (ESP32-S3
    // cannot rely on XIP at an arbitrary offset, so it copies through
    // `esp-storage`). The port allocates it before decode, so the same order
    // is reproduced here. Without this the headroom is overestimated by the
    // app's size, producing exactly the "`check` passes but instantiate fails
    // on hardware" that this function exists to prevent.
    if p.slot.is_some_and(|sl| sl.read == SlotRead::Copy) {
        arena
            .alloc_bytes(wasm.len())
            .map_err(|e| (Stage::Instantiate, reason(e)))?;
    }

    let m = decode::decode(wasm, &mut arena).map_err(|e| (Stage::Decode, reason(e)))?;
    let v = validate::validate(&m, &p.config, &mut arena, &mut scratch)
        .map_err(|e| (Stage::Validate, reason(e)))?;
    // Exec is allocated before linear memory (which takes the rest of the
    // arena), in the same order as the port.
    let _exec = Exec::new(&p.config, &mut arena).map_err(|e| (Stage::Instantiate, reason(e)))?;
    // The mock always has a display here, so a display app links and the
    // rest of the module is still judged. Whether `p` has one is the
    // `unimplemented` row: on the board it is a link error
    // (docs/device-layer.md §6), and one row says so instead of two.
    let board = wasmicon_host::hal::HostBoard::new();
    let mut hal = Hal::new(board, false).with_roles(wasmicon_host::hal::host_roles());
    // Linear memory is taken from the rest of the arena here. If it does not
    // fit, this fails the same way hardware does (which is the point).
    let _inst = instantiate(m, v, &p.config, &mut arena, &mut hal)
        .map_err(|e| (Stage::Instantiate, reason(e)))?;
    Ok(())
}

/// Checks that `run` has type `func()` (docs/abi-spec.md §3.3).
///
/// The error is a human-readable description of the type. `Ok(())` means
/// `() -> ()`.
fn run_signature(
    m: &wasmicon_core::Module<'_, '_>,
    func_idx: u32,
) -> std::result::Result<(), String> {
    // Imported functions come first in the function index space
    // (docs/abi-spec.md §6.4).
    let imported = m
        .imports
        .iter()
        .filter(|i| matches!(i.desc, ImportDesc::Func(_)))
        .count();
    let Some(local) = (func_idx as usize).checked_sub(imported) else {
        return Err("an imported function is exported as run".to_string());
    };
    let Some(&ty) = m.funcs.get(local) else {
        return Err(format!(
            "function {func_idx} does not exist (corrupt .wasm)"
        ));
    };
    let Some(ft) = m.types.get(ty as usize) else {
        return Err(format!("type {ty} does not exist (corrupt .wasm)"));
    };
    if ft.sig_matches(":") {
        Ok(())
    } else {
        Err(format!(
            "{} params / {} results",
            ft.params().count(),
            ft.results().count()
        ))
    }
}

/// Extracts `<iface>` from `wasmicon:hal/<iface>@0.1.0` or
/// `wasmicon:device/<iface>@<version>` (docs/abi-spec.md §3.1).
fn iface_of(module: &str) -> Option<&str> {
    module
        .strip_prefix("wasmicon:hal/")
        .or_else(|| module.strip_prefix("wasmicon:device/"))?
        .split('@')
        .next()
        .filter(|s| !s.is_empty())
}

/// The role names a board hands out.
///
/// The host uses the mock's table (`profile::HOST.roles`). Hardware boards
/// have no default table, so only the roles given a number in
/// `[board.<wiring>.roles]` of `wasmicon.toml` count (`deploy` writes them to
/// the config slot; docs/app-workflow.md §3.9).
#[must_use]
pub fn provided_roles(p: &Profile, manifest: Option<&Manifest>) -> Vec<String> {
    if !p.roles.is_empty() {
        return p.roles.iter().map(|(n, _)| (*n).to_string()).collect();
    }
    manifest
        .and_then(|m| m.board_roles.get(p.wiring))
        .map(|t| {
            t.iter()
                .filter(|(_, pin)| pin.is_some())
                .map(|(n, _)| n.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn implemented(i: &Interfaces, iface: &str) -> Option<bool> {
    match iface {
        "gpio" => Some(i.gpio),
        "i2c" => Some(i.i2c),
        "spi" => Some(i.spi),
        "time" => Some(i.time),
        "log" => Some(i.log),
        "board" => Some(i.board),
        "assets" => Some(i.assets),
        "display" => Some(i.display),
        // types has no functions, so it never appears as an import
        // (docs/abi-spec.md §7).
        _ => None,
    }
}

/// `wasmicon_core::Error` does not implement `Debug` (the core avoids
/// `core::fmt`). The text is built from `reason()` and `kind().name()`.
fn reason(e: wasmicon_core::Error) -> String {
    format!("{} [{}]", e.reason(), e.kind().name())
}

// ---- printing ----

/// Prints the board-independent section. Returns the number of `[assets]`
/// files that are missing (failures common to every board).
fn print_module(
    path: &Path,
    wasm: &[u8],
    f: &Facts,
    declared: Option<&[String]>,
    assets: &[AssetCheck],
) -> usize {
    let name = path.file_name().unwrap_or(path.as_os_str());
    println!("{}  {} B", name.to_string_lossy(), thousands(wasm.len()));
    println!();

    row(
        "abi",
        if f.imports
            .iter()
            .any(|i| i.module.starts_with("wasmicon:device/"))
        {
            "wasmicon:hal@0.1.0 + wasmicon:device@0.1.0"
        } else {
            "wasmicon:hal@0.1.0"
        },
        "enforced through the import names",
    );

    let bad = f.import_failures();
    if bad == 0 {
        let breakdown: Vec<String> = f
            .per_iface
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect();
        let mut note = format!("({})", breakdown.join(" / "));
        if f.imports.iter().any(|i| i.state == ImportState::AsAbort) {
            note.push_str(" + env.abort");
        }
        row("imports", &format!("all {} match", f.imports.len()), &note);
    } else {
        row("imports", &format!("{bad} do not match"), "");
        for i in &f.imports {
            let what = match &i.state {
                ImportState::Unknown => "not in the table (link error)".to_string(),
                ImportState::SigMismatch { want } => {
                    format!("signature differs (expected {want})")
                }
                ImportState::NotAFunc => "not a function (docs/abi-spec.md §6.2)".to_string(),
                ImportState::BadTypeIndex { idx } => {
                    format!("type {idx} is outside the type section (corrupt .wasm)")
                }
                ImportState::Ok | ImportState::AsAbort => continue,
            };
            println!("         - {}/{}  {what}", i.module, i.name);
        }
    }

    let run_note = match &f.run_export {
        Some(Ok(())) => None,
        Some(Err(sig)) => Some(format!(
            "run has the wrong type ({sig}; docs/abi-spec.md §3.3 wants run: func())"
        )),
        None => Some("missing export `run` (docs/abi-spec.md §3.3)".to_string()),
    };
    let mem_note =
        (!f.has_memory_export).then_some("missing export `memory` (docs/abi-spec.md §6.2)");
    match (run_note, mem_note) {
        (None, None) => row("exports", "run, memory", "present"),
        (Some(r), None) => row("exports", "memory", &r),
        (None, Some(m)) => row("exports", "run", m),
        (Some(r), Some(m)) => {
            row("exports", "none", &r);
            row("", "", m);
        }
    }

    match f.mem_pages {
        Some(p) => row("memory", &format!("initial {p} pages"), ""),
        None => row("memory", "none", "docs/abi-spec.md §6.2 requires one"),
    }
    match f.table_elems {
        Some(n) => row("table", &format!("{n} elements"), ""),
        None => row("table", "none", ""),
    }
    if let Some(u) = &f.needs {
        row(
            "stacks",
            &format!(
                "control {}, values {}, locals {}",
                u.max_control_depth, u.max_value_stack, u.max_locals
            ),
            "call depth and operand slots: see `wasmicon run`",
        );
    }

    match declared {
        Some(roles) => row("roles", &roles.join(", "), "declared in wasmicon.toml"),
        None => row(
            "roles",
            "none declared",
            "declare the roles the app looks up in pin-roles of wasmicon.toml",
        ),
    }

    // `[assets]`: every file must exist, whatever the board.
    let missing: Vec<&AssetCheck> = assets.iter().filter(|a| a.len.is_none()).collect();
    if assets.is_empty() {
        row("assets", "none", "");
    } else if missing.is_empty() {
        let list: Vec<String> = assets
            .iter()
            .map(|a| format!("{} ({} B)", a.name, thousands(a.len.unwrap_or(0) as usize)))
            .collect();
        row("assets", &list.join(", "), "from wasmicon.toml");
    } else {
        row(
            "assets",
            &format!("{} of {} files missing", missing.len(), assets.len()),
            "<- fails",
        );
        for a in &missing {
            println!("         - {}  not found at {}", a.name, a.path.display());
        }
    }

    // Everything above is board-independent. Mixing it into the per-board
    // sections would repeat the same failure for every board and hide which
    // ones are board-specific.
    let common = f.failures() + missing.len();
    if common > 0 {
        println!("-> fails on every board ({common} failures)");
    }
    missing.len()
}

/// Prints one board's section. Returns the number of asset failures on this
/// board (0 or 1: the image does not fit its slot).
fn print_verdict(
    v: &Verdict,
    f: &Facts,
    common: usize,
    p: &Profile,
    declared: Option<&[String]>,
    assets: &[AssetCheck],
) -> usize {
    println!("[{}]", v.board);

    match (&v.validate, f.mem_pages) {
        // A module without a memory definition is not "0 pages fit" (the
        // runtime accepts it, but docs/abi-spec.md §6.2 requires one). It is
        // counted in the module section, so no mark here.
        (Ok(_), None) => row(
            "validate",
            "skipped: no memory definition",
            "(reported above)",
        ),
        (Ok(limit), Some(pages)) => row(
            &v.stage_label(),
            &format!("initial {pages} pages <= {limit}"),
            "ok",
        ),
        (Err(e), _) => {
            // Under a board's limit the validator stops at that limit; the
            // host measurement says what the module needs.
            let over: Vec<String> = f
                .needs
                .map(|u| u.over(&p.config))
                .unwrap_or_default()
                .into_iter()
                .map(|(field, need, limit)| format!("{field} {need} > {limit}"))
                .collect();
            if over.is_empty() {
                row(&v.stage_label(), e, "<- fails");
            } else {
                row(
                    &v.stage_label(),
                    e,
                    &format!("<- fails (needs {})", over.join(", ")),
                );
            }
        }
    }

    for iface in &v.unimplemented {
        let what = if iface == "display" {
            "not implemented by this port (link error)"
        } else {
            "not implemented by this port (unsupported)"
        };
        row(iface, what, "<- fails");
    }

    match &v.display {
        None => {}
        Some(Ok(backend)) => row("display", backend, "ok"),
        Some(Err(why)) => {
            let (what, note) = why.split_once("  ").unwrap_or((why, ""));
            row("display", what, note.trim_start());
        }
    }

    if let Some(declared) = declared {
        if v.missing_declared_roles.is_empty() {
            row(
                "roles",
                &declared.join(", "),
                &format!("in the {} wiring table", p.wiring),
            );
        } else {
            row(
                "roles",
                &declared.join(", "),
                &format!(
                    "not in the {} wiring table: {}  <- fails (add to [board.{}.roles])",
                    p.wiring,
                    v.missing_declared_roles.join(", "),
                    p.wiring
                ),
            );
        }
    }

    // `[assets]` against this board's slot. Only sized when every file
    // exists (a missing one is reported in the module section).
    let mut fit_fails = 0;
    if !assets.is_empty() && assets.iter().all(|a| a.len.is_some()) {
        match assets_image_len(assets) {
            None => {
                // The files exist but cannot be laid out: `assets pack`
                // would refuse them too.
                fit_fails = 1;
                row(
                    "assets",
                    "cannot size the image",
                    "a file is larger than the slot format allows  <- fails",
                );
            }
            Some(image_len) => match (assets_fit(p, image_len), p.assets) {
                (Some(true), Some(sl)) => row(
                    "assets",
                    &format!("{} B image", thousands(image_len as usize)),
                    &format!(
                        "fits in the {} assets slot ({} B)",
                        p.name,
                        thousands(sl.len as usize)
                    ),
                ),
                (Some(false), Some(sl)) => {
                    fit_fails = 1;
                    row(
                        "assets",
                        &format!("{} B image", thousands(image_len as usize)),
                        &format!(
                            "does not fit in the {} assets slot ({} B)  <- fails",
                            p.name,
                            thousands(sl.len as usize)
                        ),
                    );
                }
                _ => row(
                    "assets",
                    &format!("{} B image", thousands(image_len as usize)),
                    "served from the files (no slot on the host)",
                ),
            },
        }
    }

    match (v.fails() + fit_fails, common) {
        (0, 0) => println!("-> pass"),
        (0, n) => println!("-> fail ({n} common to every board; reported above)"),
        (m, 0) => println!("-> fail ({m} on this board)"),
        (m, n) => println!("-> fail ({m} on this board + {n} common to every board)"),
    }
    fit_fails
}

/// Aligns the columns. `{:<n}` counts characters, not terminal columns;
/// `display_width` keeps the alignment correct if a wide character ever
/// appears in a cell.
fn row(label: &str, value: &str, note: &str) {
    if note.is_empty() {
        println!("{}{value}", pad(label, 9));
    } else {
        println!("{}{}{note}", pad(label, 9), pad(value, 44));
    }
}

/// Pads with spaces to width `w`, always leaving at least one space (a value
/// of exactly `w` columns would otherwise run into the note; validate's error
/// text can be exactly 44 columns).
fn pad(s: &str, w: usize) -> String {
    let mut out = String::from(s);
    for _ in display_width(s)..w.max(display_width(s) + 1) {
        out.push(' ');
    }
    out
}

/// Display width in a terminal. Wide (CJK) characters count as two columns.
fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| if is_wide(c) { 2 } else { 1 })
        .sum::<usize>()
}

fn is_wide(c: char) -> bool {
    matches!(u32::from(c),
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6)
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iface_names_come_from_the_module_name() {
        assert_eq!(iface_of("wasmicon:hal/gpio@0.1.0"), Some("gpio"));
        assert_eq!(iface_of("wasmicon:hal/i2c@0.1.0"), Some("i2c"));
        assert_eq!(iface_of("wasmicon:device/display@0.1.0"), Some("display"));
        // docs/abi-spec.md §3.1 requires the version and an exact match, so
        // other shapes are not picked up.
        assert_eq!(iface_of("env"), None);
        assert_eq!(iface_of("wasmicon:hal/"), None);
    }

    #[test]
    fn interfaces_map_to_the_profile_fields() {
        let i = profile::RP2040.interfaces;
        assert_eq!(implemented(&i, "i2c"), Some(false));
        assert_eq!(implemented(&i, "spi"), Some(false));
        assert_eq!(implemented(&i, "gpio"), Some(true));
        assert_eq!(implemented(&i, "assets"), Some(true));
        assert_eq!(implemented(&i, "display"), Some(false));
        assert_eq!(
            implemented(&profile::HOST.interfaces, "display"),
            Some(true)
        );
        // types has no functions, so it never appears as an import.
        assert_eq!(implemented(&i, "types"), None);
    }

    #[test]
    fn columns_count_wide_chars_as_two() {
        assert_eq!(display_width("ok"), 2);
        assert_eq!(display_width("通る"), 4); // i18n:allow-cjk
        // "1 " is 2 columns + 3 wide characters x 2 columns.
        assert_eq!(display_width("1 ページ"), 2 + 6); // i18n:allow-cjk
        assert_eq!(pad("ok", 4), "ok  ");
        assert_eq!(pad("通る", 6), "通る  "); // i18n:allow-cjk
    }

    #[test]
    fn sizes_are_grouped() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(775), "775");
        assert_eq!(thousands(4549), "4,549");
        assert_eq!(thousands(1_146_862), "1,146,862");
    }
}
