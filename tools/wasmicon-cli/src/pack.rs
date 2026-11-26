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
use wasmicon_port::roles::{self, RoleMap, Wiring};
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
        println!("  `wasmicon deploy` does this in one command, with the wiring table");
        println!(
            "  (a table that `deploy` wrote is bound to its app: this image alone runs with no roles)"
        );
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

/// Flash erase unit of every board (RP2040 / RP2350 and the ESP32 family).
/// `deploy` writes whole sectors only.
pub const SECTOR: u32 = 4096;

/// One stretch of flash that `deploy` writes, at a flash offset.
pub struct Region {
    /// What it is, for the summary (`app slot`, `config slot`).
    pub what: &'static str,
    /// Offset from the start of flash (a multiple of `SECTOR`).
    pub offset: u32,
    /// Bytes with meaning: the slot image or the config header and body.
    pub content: usize,
    /// `content`, then `0xff` (erased flash) up to the end of its last
    /// sector. A multiple of `SECTOR`.
    pub bytes: Vec<u8>,
}

/// The two regions `deploy` writes: the app image at the start of the app
/// slot and the config slot (wiring table, display and touch entries;
/// docs/app-workflow.md §3.9, docs/device-layer.md §5.2, §11.3). The config
/// slot has the lowest format version that holds the entries
/// (`Wiring::version`).
///
/// Only the sectors that hold the app are written, not the whole app slot.
/// The firmware reads the header first and then exactly the length it
/// states, checking the CRC over it (`slot::parse` on RP2040 / RP2350,
/// `pick_guest` on the ESP32 ports), so whatever an earlier, longer app left
/// behind the new one is never read. The header sits in the first sector,
/// which is always rewritten.
///
/// Each region is padded with `0xff` to whole sectors so the result does
/// not depend on what a flasher does with a partly written sector:
/// `picotool load` erases the sector and fills the rest with zeros
/// (`load_guts` in picotool 2.3.1).
///
/// The wiring table is always written, even when empty, so the result of
/// `deploy` depends only on the current `wasmicon.toml` (a previously
/// flashed table never survives).
///
/// The config slot is bound to the app: the app's CRC follows the body
/// (`roles::binding_bytes`), and a firmware that reads it refuses a table
/// bound to another app.
///
/// # Errors
/// When the board has no app slot or no config slot, a slot is not whole
/// sectors, the two slots overlap (profile errors), or a padded region does
/// not fit its slot.
pub fn regions(app: &[u8], p: &profile::Profile, w: &Wiring) -> Result<[Region; 2]> {
    let (Some(sl), Some(rs)) = (p.slot, p.role_slot) else {
        bail!("`{}` has no app slot or no config slot", p.name);
    };
    // Whole sectors, so padding a region never reaches past its slot into
    // the next one.
    for (what, s) in [("app slot", sl), ("config slot", rs)] {
        if s.offset % SECTOR != 0 || s.len % SECTOR != 0 || s.len == 0 {
            bail!("the {what} of `{}` is not whole 4 KiB sectors", p.name);
        }
    }
    let (app_end, rs_end) = (
        u64::from(sl.offset) + u64::from(sl.len),
        u64::from(rs.offset) + u64::from(rs.len),
    );
    if u64::from(rs.offset) < app_end && u64::from(sl.offset) < rs_end {
        bail!("the app slot and the config slot of `{}` overlap", p.name);
    }
    let mut body = [0u8; roles::MAX_BODY];
    let n = roles::encode_body(w, &mut body);
    let mut config = roles::header(&body[..n], w.version()).to_vec();
    config.extend_from_slice(&body[..n]);
    // Holds the table to this app (`roles::bind`): if the app is written and
    // the config slot is not, the firmware hands the new app no roles rather
    // than the previous app's pins.
    config.extend_from_slice(&roles::binding_bytes(crc_of(app)));
    let regions = [
        Region::padded("app slot", sl.offset, app.to_vec()),
        Region::padded("config slot", rs.offset, config),
    ];
    for (r, s) in regions.iter().zip([sl, rs]) {
        if r.bytes.len() > s.len as usize {
            bail!(
                "the {} of `{}` cannot hold {} B ({} B)",
                r.what,
                p.name,
                r.bytes.len(),
                s.len
            );
        }
    }
    Ok(regions)
}

impl Region {
    fn padded(what: &'static str, offset: u32, mut bytes: Vec<u8>) -> Self {
        let content = bytes.len();
        let sector = SECTOR as usize;
        // An empty region still writes one sector (its header must go).
        let len = content.max(1).div_ceil(sector) * sector;
        bytes.resize(len, 0xff);
        Region {
            what,
            offset,
            content,
            bytes,
        }
    }
}

/// UF2 family ID `absolute` (picotool's name): the payload goes to the
/// address in each block, outside any partition. `picotool load` takes any
/// family as long as every block has one (`build_rmap_uf2` in picotool
/// 2.3.1); this one says what the file is.
pub const UF2_FAMILY_ABSOLUTE: u32 = 0xe48b_ff57;

/// Encodes regions at flash offsets as one UF2 file for `picotool load`, so
/// two separate stretches of flash are written in one call (one USB
/// connection, one reboot).
///
/// Each 512-byte block carries 256 bytes, the size picotool accepts. A
/// block's address is `XIP_BASE + offset`.
///
/// # Errors
/// When a region is not whole 256-byte blocks (a short last block would be
/// written with zeros after it) or reaches past the 32-bit address space.
pub fn uf2(regions: &[Region]) -> Result<Vec<u8>> {
    const PAYLOAD: usize = 256;
    for r in regions {
        if r.bytes.len() % PAYLOAD != 0 {
            bail!(
                "the {} is {} B, not whole 256 B blocks",
                r.what,
                r.bytes.len()
            );
        }
        if XIP_BASE + u64::from(r.offset) + r.bytes.len() as u64 > u64::from(u32::MAX) {
            bail!(
                "the {} at `{:#x}` is past the 32-bit address space",
                r.what,
                r.offset
            );
        }
    }
    let total: usize = regions.iter().map(|r| r.bytes.len() / PAYLOAD).sum();
    let total = u32::try_from(total).context("too many UF2 blocks")?;
    let mut out = Vec::with_capacity(total as usize * 512);
    let mut index: u32 = 0;
    for r in regions {
        for (i, chunk) in r.bytes.chunks(PAYLOAD).enumerate() {
            // Fits: checked above.
            let at = (XIP_BASE + u64::from(r.offset) + (i * PAYLOAD) as u64) as u32;
            let mut block = [0u8; 512];
            let words: [u32; 8] = [
                0x0A32_4655, // magic start 0 ("UF2\n")
                0x9E5D_5157, // magic start 1
                0x0000_2000, // flags: family ID present
                at,
                PAYLOAD as u32,
                index,
                total,
                UF2_FAMILY_ABSOLUTE,
            ];
            for (k, w) in words.iter().enumerate() {
                block[k * 4..k * 4 + 4].copy_from_slice(&w.to_le_bytes());
            }
            block[32..32 + chunk.len()].copy_from_slice(chunk);
            block[508..].copy_from_slice(&0x0AB1_6F30u32.to_le_bytes()); // magic end
            out.extend_from_slice(&block);
            index += 1;
        }
    }
    Ok(out)
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

/// Describes the display entry as the boot banner does
/// (`ili9341 spi=0 cs=10 dc=14 rst=15 rotation=90`).
#[must_use]
pub fn describe_display(w: &Wiring) -> Option<String> {
    // The banner of the display entry alone: a role may be named `display`.
    let only = Wiring {
        roles: roles::RoleMap::EMPTY,
        display: Some(w.display?),
        touch: None,
        ..Wiring::EMPTY
    };
    let mut line = [0u8; 256];
    let mut out = wasmicon_port::fmt::Buf::new(&mut line);
    roles::describe(&Ok(only), &mut out);
    let text = String::from_utf8_lossy(out.as_bytes()).into_owned();
    text.split_once(" display=").map(|(_, d)| d.to_string())
}

/// Describes the bus pins the table moves as the boot banner does
/// (`spi0=sck=1 mosi=2 miso=3 i2c0=sda=4 scl=5`). `None` when it moves
/// none and the firmware's defaults stay.
#[must_use]
pub fn describe_buses(w: &Wiring) -> Option<String> {
    if w.buses.is_none() {
        return None;
    }
    let only = Wiring {
        buses: w.buses,
        ..Wiring::EMPTY
    };
    let mut line = [0u8; 256];
    let mut out = wasmicon_port::fmt::Buf::new(&mut line);
    roles::describe(&Ok(only), &mut out);
    let text = String::from_utf8_lossy(out.as_bytes()).into_owned();
    text.strip_prefix("wasmicon: roles none ")
        .map(str::to_string)
}

/// Describes the touch entry as the boot banner does (`tab5`,
/// `xpt2046 cs=16 irq=17 x-min=200 ...`).
#[must_use]
pub fn describe_touch(w: &Wiring) -> Option<String> {
    let only = Wiring {
        roles: roles::RoleMap::EMPTY,
        display: None,
        touch: Some(w.touch?),
        ..Wiring::EMPTY
    };
    let mut line = [0u8; 256];
    let mut out = wasmicon_port::fmt::Buf::new(&mut line);
    roles::describe(&Ok(only), &mut out);
    let text = String::from_utf8_lossy(out.as_bytes()).into_owned();
    text.split_once(" touch=").map(|(_, t)| t.to_string())
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
