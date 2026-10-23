//! `wasmicon pack`: turns a `.wasm` into a slot image (docs/app-workflow.md §3.4).
//!
//! The format belongs to `slot` in `ports/common` (the writer and the reader
//! share one definition).
//!
//! The image is the input of `deploy --persist`; stage 1 hands it to an
//! external flasher (`espflash write-bin <offset>` / `picotool load -o
//! <offset>`; docs/app-workflow.md §3.5). The offset is a per-board decision,
//! so this command only builds the image.
//!
//! Run `check` before sending. Writing a corrupt app, or one that does not
//! run on that board, into the slot is only noticed when it fails on
//! hardware.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;

use wasmicon_port::profile;
use wasmicon_port::profile::Flasher;
use wasmicon_port::roles::{self, RoleMap};
use wasmicon_port::slot;

pub struct Options {
    pub path: PathBuf,
    /// Output path. `None` = `<input>.bin`.
    pub out: Option<PathBuf>,
    /// `--board`. When given, the image is checked against the slot size and
    /// the flash command is printed.
    pub board: Option<String>,
}

/// Writes the slot image.
///
/// # Errors
/// When the input cannot be read or the output cannot be written.
pub fn run(opts: &Options) -> Result<bool> {
    let wasm = std::fs::read(&opts.path)
        .with_context(|| format!("cannot read `{}`", opts.path.display()))?;
    let image = build(&wasm);

    // The extension is `.bin`. `picotool load` decides the file type by
    // extension, so a custom name like `.slot` is rejected with "does not
    // have a recognized file type (extension)" (seen on Pico 2 W;
    // docs/verification-report.md §9).
    let out = opts.out.clone().unwrap_or_else(|| {
        let mut p = opts.path.clone();
        p.set_extension("bin");
        p
    });

    // When the board is known, check that the image fits the slot before
    // writing. Flashing one that does not fit is only noticed when
    // "slot truncated" appears on hardware.
    let board = match &opts.board {
        None => None,
        Some(name) => Some(resolve_board(name)?),
    };
    if let Some(p) = board {
        fits(p, image.len())?;
    }

    std::fs::write(&out, &image).with_context(|| format!("cannot write `{}`", out.display()))?;
    let crc = crc_of(&image);
    println!(
        "{}  {} B (wasm {} B + header {} B, crc32 {crc:08x})",
        out.display(),
        image.len(),
        wasm.len(),
        slot::HEADER_LEN
    );

    // Stage 1 hands the image to an external flasher
    // (docs/app-workflow.md §3.5). Tool and arguments differ per board.
    if let Some(p) = board
        && let Some(sl) = p.slot
    {
        flash_hint(sl, &out);
        println!("  `wasmicon deploy` does this in one command");
    }
    Ok(true)
}

/// Prints the flasher command that writes `image` to `sl`. Shared by `pack`
/// (the app slot) and `assets pack` (the assets slot).
pub fn flash_hint(sl: profile::Slot, image: &std::path::Path) {
    if matches!(sl.flasher, Flasher::Espflash { .. }) {
        // espflash takes a flash offset. It does `--after hard-reset` by
        // default, so no separate reset is needed.
        println!(
            "-> flash: espflash write-bin --port <dev> {:#x} {}",
            sl.offset,
            image.display()
        );
        println!("  (no button needed; DTR/RTS resets the board)");
    } else {
        // picotool's `-o` is an absolute address, not a flash offset.
        // An offset is rejected with "invalid memory range
        // 0x00100000-..." (seen on Pico 2 W;
        // docs/verification-report.md §9). XIP starts at 0x1000_0000.
        let addr = XIP_BASE + u64::from(sl.offset);
        println!("-> flash: picotool load -o {addr:#x} {}", image.display());
        println!("  (hold BOOTSEL while plugging in USB first)");
    }
}

/// Start of XIP on RP2040 / RP2350. Base of the address passed to `picotool`.
pub const XIP_BASE: u64 = 0x1000_0000;

/// Builds the slot image (header + wasm).
#[must_use]
pub fn build(wasm: &[u8]) -> Vec<u8> {
    let mut image = Vec::with_capacity(slot::image_len(wasm.len()));
    image.extend_from_slice(&slot::header(wasm));
    image.extend_from_slice(wasm);
    image
}

/// Appends the config slot (wiring table) to the app slot image, making one
/// image that is flashed in one go (used by `deploy`;
/// docs/app-workflow.md §3.9).
///
/// The space after the app is filled with `0xff` (same as erased flash) up
/// to the end of the slot. The wiring table is always written, even when
/// empty, so the result of `deploy` depends only on the current
/// `wasmicon.toml` (a previously flashed table never survives).
///
/// # Errors
/// When the config slot does not directly follow the app slot (a profile
/// error).
pub fn with_roles(app: &[u8], p: &profile::Profile, map: &RoleMap) -> Result<Vec<u8>> {
    let (Some(sl), Some(rs)) = (p.slot, p.role_slot) else {
        bail!("{} has no app slot or no config slot", p.name);
    };
    if rs.offset != sl.offset + sl.len {
        bail!("the config slot of {} does not follow its app slot", p.name);
    }
    let mut body = [0u8; roles::MAX_BODY];
    let n = roles::encode_body(map, &mut body);
    let mut image = app.to_vec();
    image.resize(sl.len as usize, 0xff);
    image.extend_from_slice(&roles::header(&body[..n]));
    image.extend_from_slice(&body[..n]);
    Ok(image)
}

/// Describes the table on one line (`lcd-cs=17 lcd-dc=20`; `None` when empty).
#[must_use]
pub fn describe_roles(map: &RoleMap) -> Option<String> {
    if map.is_empty() {
        return None;
    }
    let parts: Vec<String> = (0..map.len())
        .map(|i| format!("{}={}", String::from_utf8_lossy(map.name(i)), map.pin(i)))
        .collect();
    Some(parts.join(" "))
}

/// The CRC stored in the image header (bytes 12..16, docs/app-workflow.md §3.4).
///
/// Printed so it can be cross-checked against the device's log.
#[must_use]
pub fn crc_of(image: &[u8]) -> u32 {
    u32::from_le_bytes([image[12], image[13], image[14], image[15]])
}

/// Resolves `--board`.
///
/// # Errors
/// When the name is unknown.
pub fn resolve_board(name: &str) -> Result<&'static profile::Profile> {
    profile::by_name(name).with_context(|| {
        let known: Vec<&str> = profile::PROFILES.iter().map(|p| p.name).collect();
        format!(
            "unknown board `{name}` (known boards: {})",
            known.join(", ")
        )
    })
}

/// Checks that the image fits the board's slot.
///
/// # Errors
/// When the slot location is not decided, or the image does not fit.
pub fn fits(p: &profile::Profile, image_len: usize) -> Result<profile::Slot> {
    match p.slot {
        None => bail!(
            "the slot location for {} is not decided yet (docs/TODO.md §5-2)",
            p.name
        ),
        Some(sl) if image_len > sl.len as usize => bail!(
            "does not fit in the {} slot ({image_len} B > {} B)",
            p.name,
            sl.len
        ),
        Some(sl) => Ok(sl),
    }
}
