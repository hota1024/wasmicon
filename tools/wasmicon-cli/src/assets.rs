//! `wasmicon assets pack` / `assets write`: the assets slot image and its
//! flashing (docs/app-workflow.md §3.10).
//!
//! The image is built from the `[assets]` table of `wasmicon.toml` by the
//! builder the host mock uses (`wasmicon_host::assets::image`), so what
//! `run` serves is byte for byte what the board reads. `deploy` never
//! touches the assets slot: its data is per device and survives app updates,
//! and a multi-megabyte write does not belong in the app's edit-flash loop.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;

use crate::manifest::Manifest;
use crate::{deploy, pack};
use wasmicon_port::profile::{Flasher, Profile, Slot};

pub struct PackOptions {
    /// The manifest whose `[assets]` table is packed.
    pub manifest: Manifest,
    /// `--board`, or `[defaults] board`. When known, the image is checked
    /// against that board's slot and the flash command is printed.
    pub board: Option<String>,
    /// Output path. `None` = `assets.bin` next to the manifest.
    pub out: Option<PathBuf>,
}

pub struct WriteOptions {
    pub manifest: Manifest,
    /// Target board: `--board` or `[defaults] board`. Required.
    pub board: Option<String>,
    /// Serial port for espflash (`None` = let it pick).
    pub port: Option<PathBuf>,
    /// `--flash-baud`: the speed espflash flashes at (`None` = espflash's own
    /// choice: `ESPFLASH_BAUD` when set, otherwise 115200).
    pub flash_baud: Option<u32>,
}

/// Reads every file of the `[assets]` table, in table order.
///
/// # Errors
/// When a file cannot be read.
pub fn load(m: &Manifest) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    for (name, path) in &m.assets {
        let body = std::fs::read(path)
            .with_context(|| format!("cannot read asset `{name}` at `{}`", path.display()))?;
        out.push((name.clone(), body));
    }
    Ok(out)
}

/// Builds the slot image from loaded assets.
///
/// # Errors
/// When the list is one the firmware would refuse (the manifest has already
/// checked names and count, so in practice never).
pub fn image(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    wasmicon_host::assets::image(entries)
        .map_err(|e| anyhow::anyhow!("cannot build the assets image: {}", e.reason()))
}

/// The board's assets slot, when an image of `image_len` bytes fits it.
///
/// # Errors
/// When the board has no assets slot (the host), or the image does not fit.
pub fn fits(p: &Profile, image_len: usize) -> Result<Slot> {
    match p.assets {
        None => bail!("{} has no assets slot (the host reads the files)", p.name),
        Some(sl) if image_len > sl.len as usize => bail!(
            "the assets do not fit in the {} assets slot ({image_len} B > {} B)",
            p.name,
            sl.len
        ),
        Some(sl) => Ok(sl),
    }
}

/// `name 27 B, other 4100 B`.
fn describe(entries: &[(String, Vec<u8>)]) -> String {
    let parts: Vec<String> = entries
        .iter()
        .map(|(n, b)| format!("{n} {} B", b.len()))
        .collect();
    parts.join(", ")
}

/// Writes the slot image to a file.
///
/// # Errors
/// When a file cannot be read or written, the board is unknown, or the
/// image does not fit the board's slot.
pub fn pack(opts: &PackOptions) -> Result<bool> {
    let board = match &opts.board {
        None => None,
        Some(name) => Some(pack::resolve_board(name)?),
    };
    let entries = load(&opts.manifest)?;
    let image = image(&entries)?;
    let slot = match board {
        Some(p) => Some(fits(p, image.len())?),
        None => None,
    };

    let out = opts
        .out
        .clone()
        .unwrap_or_else(|| opts.manifest.dir.join("assets.bin"));
    std::fs::write(&out, &image).with_context(|| format!("cannot write `{}`", out.display()))?;
    println!(
        "{}  {} B ({} assets: {})",
        out.display(),
        image.len(),
        entries.len(),
        describe(&entries)
    );

    if let Some(sl) = slot {
        pack::flash_hint(sl, &out);
        println!("  `wasmicon assets write` does this in one command");
    }
    Ok(true)
}

/// Builds the image and flashes it to the board's assets slot.
///
/// # Errors
/// When no board is named, a file cannot be read or written, the image
/// does not fit, or the flasher cannot be started.
pub fn write(opts: &WriteOptions) -> Result<bool> {
    let Some(name) = &opts.board else {
        bail!("no target board: pass --board or set [defaults] board in wasmicon.toml");
    };
    let p = pack::resolve_board(name)?;
    let entries = load(&opts.manifest)?;
    let image = image(&entries)?;
    let sl = fits(p, image.len())?;
    deploy::check_flash_baud(opts.flash_baud, sl.flasher)?;

    let out = opts.manifest.dir.join("assets.bin");
    std::fs::write(&out, &image).with_context(|| format!("cannot write `{}`", out.display()))?;
    println!(
        "{} -> {} assets slot (+{:#x})  {} B ({} assets: {})",
        out.display(),
        p.name,
        sl.offset,
        image.len(),
        entries.len(),
        describe(&entries)
    );
    if !deploy::write_at(
        sl.flasher,
        sl.offset,
        &out,
        opts.port.as_deref(),
        false,
        opts.flash_baud,
    )? {
        return Ok(false);
    }
    // The app slot is not touched, so the board comes back in whatever app
    // it held. espflash resets it by itself; picotool leaves it in the
    // bootloader, so it is rebooted here as `deploy` does.
    if sl.flasher == Flasher::Picotool && !deploy::reboot()? {
        eprintln!("  written, but the reset failed; unplug and replug USB");
        return Ok(false);
    }
    println!("-> written; the board runs its app with the new assets");
    Ok(true)
}
