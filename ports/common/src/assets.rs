//! Assets slot: read-only data the developer ships next to the app
//! (docs/app-workflow.md §3.10).
//!
//! An asset is a named byte string (a WAD, a font, an image) that the guest
//! opens through `wasmicon:hal/assets@0.1.0` and reads by range. The CLI
//! writes the slot (`wasmicon assets write`); the firmware only reads it, so
//! a board needs no flash-write routine, and `deploy` never touches it, so
//! the data survives an app update. The slot starts right after the config
//! slot and runs to the end of flash (`Profile::assets`).
//!
//! The slot holds a directory followed by the bodies. The directory is a
//! fixed-size table (`MAX_ENTRIES` entries of `ENTRY_LEN` bytes), so it is
//! parsed without `alloc` and can be read from flash in one go on boards
//! without XIP. Each body starts on a 4 KiB boundary (the erase unit), so a
//! body is one contiguous range of flash and can be rewritten on its own
//! later. The layout is on `encode`.
//!
//! The writer (CLI, host mock) and the reader (firmware) share this module so
//! the checks are written once.

use crate::crc32;
use crate::fmt::Buf;
use crate::profile::Slot;

/// Identifier at the start of the slot. Distinct from `"WMCA"` (app) and
/// `"WMCR"` (roles).
pub const MAGIC: [u8; 4] = *b"WMCB";

/// Format version.
pub const FORMAT_VERSION: u16 = 1;

/// Header size. The entry table starts right after it.
pub const HEADER_LEN: usize = 16;

/// Maximum number of assets.
pub const MAX_ENTRIES: usize = 8;

/// Maximum asset name length in bytes.
pub const MAX_NAME: usize = 32;

/// Size of one entry: the name plus offset, length and CRC-32.
pub const ENTRY_LEN: usize = MAX_NAME + 12;

/// Size of the directory (header plus a full entry table). The firmware reads
/// this many bytes at boot.
pub const DIR_LEN: usize = HEADER_LEN + MAX_ENTRIES * ENTRY_LEN;

/// Alignment of every body: one flash erase unit.
pub const BODY_ALIGN: u32 = 4096;

/// Offset of the first body from the start of the slot. The directory has the
/// first sector to itself.
pub const BODY_START: u32 = BODY_ALIGN;

/// Why the directory cannot be read or built.
///
/// The text is a `&'static str` (no `core::fmt`). Which entry failed is
/// reported by the writer (CLI), which calls `insert` one entry at a time.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AssetsError {
    /// Erased (`0xff`) or never written. Not a failure but "no assets".
    Empty,
    /// Wrong magic.
    BadMagic,
    /// A version this firmware cannot read.
    UnsupportedVersion,
    /// The entry table's CRC does not match.
    BadCrc,
    /// More than `MAX_ENTRIES` assets.
    TooMany,
    /// An asset name has a disallowed byte or a bad length.
    BadName,
    /// The same asset name appears twice.
    DuplicateName,
    /// A body is not 4 KiB aligned, overlaps another, or runs past the slot.
    BadLayout,
    /// The firmware's end reaches into the assets slot. Must not be read.
    Overlap,
}

impl AssetsError {
    /// Text for the log.
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            AssetsError::Empty => "assets empty",
            AssetsError::BadMagic => "assets magic mismatch",
            AssetsError::UnsupportedVersion => "assets format version not supported",
            AssetsError::BadCrc => "assets crc mismatch",
            AssetsError::TooMany => "too many assets",
            AssetsError::BadName => {
                "asset name must be 1..32 of a-z 0-9 - _ . starting with a-z or 0-9"
            }
            AssetsError::DuplicateName => "asset name appears twice",
            AssetsError::BadLayout => "asset body is misplaced or does not fit the slot",
            AssetsError::Overlap => "firmware overlaps the assets slot",
        }
    }

    /// Whether the slot is empty. The one case that is not an error.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self == AssetsError::Empty
    }
}

/// Whether `name` is a valid asset name: 1..=32 bytes of `a-z` / `0-9` / `-` /
/// `_` / `.`, starting with a letter or a digit.
///
/// `.` lets a name carry a file extension (`doom1.wad`). `/` is not allowed:
/// the slot is a flat table, not a file system.
#[must_use]
pub fn valid_name(name: &[u8]) -> bool {
    if name.is_empty() || name.len() > MAX_NAME {
        return false;
    }
    if !(name[0].is_ascii_lowercase() || name[0].is_ascii_digit()) {
        return false;
    }
    name.iter().all(|&b| {
        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_' || b == b'.'
    })
}

/// Rounds up to the next body boundary. `None` when that would overflow.
const fn align_up(v: u32) -> Option<u32> {
    match v.checked_add(BODY_ALIGN - 1) {
        Some(v) => Some(v & !(BODY_ALIGN - 1)),
        None => None,
    }
}

/// One asset: where its body is in the slot and what it should contain.
#[derive(Clone, Copy)]
pub struct Entry {
    /// NUL-padded; the padding is written to the slot as is.
    name: [u8; MAX_NAME],
    name_len: u8,
    /// Offset of the body from the start of the slot (a multiple of `BODY_ALIGN`).
    pub offset: u32,
    /// Length of the body in bytes.
    pub len: u32,
    /// CRC-32 of the body.
    pub crc: u32,
}

impl Entry {
    const EMPTY: Entry = Entry {
        name: [0; MAX_NAME],
        name_len: 0,
        offset: 0,
        len: 0,
        crc: 0,
    };

    /// The asset's name.
    #[must_use]
    pub fn name(&self) -> &[u8] {
        &self.name[..self.name_len as usize]
    }

    /// Offset just past the body. Cannot overflow: `insert` and `parse` check it.
    #[must_use]
    pub fn end(&self) -> u32 {
        self.offset + self.len
    }
}

/// The directory: name -> body location table. Fixed size, no `alloc`.
#[derive(Clone, Copy)]
pub struct Directory {
    entries: [Entry; MAX_ENTRIES],
    n: usize,
}

impl Directory {
    /// A directory with no assets. The firmware runs with this when the slot
    /// is empty or corrupt.
    pub const EMPTY: Directory = Directory {
        entries: [Entry::EMPTY; MAX_ENTRIES],
        n: 0,
    };

    /// Number of assets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.n
    }

    /// Whether there are no assets.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// The entries, in slot order (ascending body offsets).
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries[..self.n]
    }

    /// The `i`-th entry.
    ///
    /// # Panics
    /// When `i` is `len()` or more.
    #[must_use]
    pub fn entry(&self, i: usize) -> &Entry {
        &self.entries()[i]
    }

    /// Looks up an asset name. Returns its position in the table when found.
    #[must_use]
    pub fn find(&self, name: &[u8]) -> Option<usize> {
        self.entries().iter().position(|e| e.name() == name)
    }

    /// Adds one asset (for the writer). Its body goes at the next 4 KiB
    /// boundary after the previous one (`BODY_START` for the first). Returns
    /// the entry's position.
    ///
    /// # Errors
    /// Invalid or duplicate name, over the limit, or a body that would push
    /// the slot past 4 GiB.
    pub fn insert(&mut self, name: &[u8], len: u32, crc: u32) -> Result<usize, AssetsError> {
        if !valid_name(name) {
            return Err(AssetsError::BadName);
        }
        if self.find(name).is_some() {
            return Err(AssetsError::DuplicateName);
        }
        if self.n == MAX_ENTRIES {
            return Err(AssetsError::TooMany);
        }
        let offset = self.image_len();
        let end = offset.checked_add(len).ok_or(AssetsError::BadLayout)?;
        // The next body (or the end of the image) must be addressable too.
        align_up(end).ok_or(AssetsError::BadLayout)?;
        let i = self.n;
        let e = &mut self.entries[i];
        e.name = [0; MAX_NAME];
        e.name[..name.len()].copy_from_slice(name);
        e.name_len = name.len() as u8;
        e.offset = offset;
        e.len = len;
        e.crc = crc;
        self.n += 1;
        Ok(i)
    }

    /// Length of the slot image: the end of the last body rounded up to a
    /// body boundary (`BODY_START` when there are no assets). The next body
    /// would start here.
    #[must_use]
    pub fn image_len(&self) -> u32 {
        match self.entries().last() {
            None => BODY_START,
            // `insert` / `parse` refuse an end that cannot be rounded up.
            Some(e) => align_up(e.end()).unwrap_or(u32::MAX),
        }
    }
}

/// Encodes the directory (for the writer). Unused entries and the reserved
/// field are zero.
///
/// Slot layout:
///
/// ```text
/// offset  size  contents
/// 0       4     magic "WMCB" (Wasmicon Blobs)
/// 4       2     format version (u16 LE, = 1)
/// 6       2     number of entries (u16 LE, at most 8)
/// 8       4     CRC-32 of the entry table (u32 LE; the `count * 44` bytes from offset 16)
/// 12      4     reserved (0)
/// 16      44*n  entries: name (32 bytes, NUL-padded), body offset (u32 LE, from the
///               start of the slot, a multiple of 4096), body length (u32 LE),
///               body CRC-32 (u32 LE)
/// 4096    ...   bodies, each starting on a 4096-byte boundary, in table order
/// ```
pub fn encode(dir: &Directory, out: &mut [u8; DIR_LEN]) {
    *out = [0; DIR_LEN];
    out[..4].copy_from_slice(&MAGIC);
    out[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    out[6..8].copy_from_slice(&(dir.n as u16).to_le_bytes());
    for (i, e) in dir.entries().iter().enumerate() {
        let at = HEADER_LEN + i * ENTRY_LEN;
        let raw = &mut out[at..at + ENTRY_LEN];
        raw[..MAX_NAME].copy_from_slice(&e.name);
        raw[MAX_NAME..MAX_NAME + 4].copy_from_slice(&e.offset.to_le_bytes());
        raw[MAX_NAME + 4..MAX_NAME + 8].copy_from_slice(&e.len.to_le_bytes());
        raw[MAX_NAME + 8..MAX_NAME + 12].copy_from_slice(&e.crc.to_le_bytes());
    }
    let table_len = dir.n * ENTRY_LEN;
    let crc = crc32(&out[HEADER_LEN..HEADER_LEN + table_len]);
    out[8..12].copy_from_slice(&crc.to_le_bytes());
}

/// Parses the directory at the start of a slot of `slot_len` bytes. `bytes`
/// may be the whole slot or just its first `DIR_LEN` bytes.
///
/// One bad entry discards the whole directory (the caller runs with
/// `Directory::EMPTY`). Bodies are not read here; their CRC is checked when
/// the guest first opens them.
///
/// # Errors
/// Empty, wrong magic or version, a bad table CRC, or an entry that
/// `AssetsError` describes.
pub fn parse(bytes: &[u8], slot_len: u32) -> Result<Directory, AssetsError> {
    let Some(head) = bytes.get(..HEADER_LEN) else {
        return Err(AssetsError::Empty);
    };
    let magic = &head[..4];
    if magic != MAGIC {
        if magic.iter().all(|b| *b == 0xff) || magic.iter().all(|b| *b == 0) {
            return Err(AssetsError::Empty);
        }
        return Err(AssetsError::BadMagic);
    }
    if u16::from_le_bytes([head[4], head[5]]) != FORMAT_VERSION {
        return Err(AssetsError::UnsupportedVersion);
    }
    let count = usize::from(u16::from_le_bytes([head[6], head[7]]));
    if count > MAX_ENTRIES {
        return Err(AssetsError::TooMany);
    }
    let Some(table) = bytes.get(HEADER_LEN..HEADER_LEN + count * ENTRY_LEN) else {
        return Err(AssetsError::BadLayout);
    };
    if crc32(table) != u32::from_le_bytes([head[8], head[9], head[10], head[11]]) {
        return Err(AssetsError::BadCrc);
    }

    let mut dir = Directory::EMPTY;
    // Where the next body may start: bodies are ascending and 4 KiB aligned.
    let mut next = BODY_START;
    for raw in table.chunks_exact(ENTRY_LEN) {
        let name_len = raw[..MAX_NAME]
            .iter()
            .position(|b| *b == 0)
            .unwrap_or(MAX_NAME);
        if raw[name_len..MAX_NAME].iter().any(|b| *b != 0) {
            return Err(AssetsError::BadName);
        }
        let name = &raw[..name_len];
        if !valid_name(name) {
            return Err(AssetsError::BadName);
        }
        if dir.find(name).is_some() {
            return Err(AssetsError::DuplicateName);
        }
        let word = |at: usize| u32::from_le_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]]);
        let (offset, len, crc) = (word(MAX_NAME), word(MAX_NAME + 4), word(MAX_NAME + 8));
        if offset % BODY_ALIGN != 0 || offset < next {
            return Err(AssetsError::BadLayout);
        }
        let end = offset.checked_add(len).ok_or(AssetsError::BadLayout)?;
        if end > slot_len {
            return Err(AssetsError::BadLayout);
        }
        next = align_up(end).ok_or(AssetsError::BadLayout)?;
        let e = &mut dir.entries[dir.n];
        e.name.copy_from_slice(&raw[..MAX_NAME]);
        e.name_len = name_len as u8;
        e.offset = offset;
        e.len = len;
        e.crc = crc;
        dir.n += 1;
    }
    Ok(dir)
}

/// Reads the assets slot from memory-mapped (XIP) flash (RP2040 / RP2350).
///
/// Returns the whole slot as a slice (the bodies are read from it in place)
/// together with the parsed directory.
///
/// # Errors
/// Overlap with the firmware, or whatever `parse` reports.
///
/// # Safety
/// `slot.len` bytes starting at `xip_base + slot.offset` must be readable
/// memory-mapped flash that stays mapped for the rest of the program (the
/// contract of `crate::slot::xip_region`).
pub unsafe fn read_xip(
    xip_base: usize,
    slot: Slot,
    fw_end: usize,
) -> Result<(&'static [u8], Directory), AssetsError> {
    // SAFETY: this function's own contract guarantees that `slot.len` bytes
    // from `xip_base + slot.offset` are readable memory-mapped flash that stays
    // mapped for the rest of the program, which is exactly what `xip_region`
    // requires.
    let region = unsafe { crate::slot::xip_region(xip_base, slot, fw_end) }
        .map_err(|_| AssetsError::Overlap)?;
    let dir = parse(region, slot.len)?;
    Ok((region, dir))
}

/// Builds the one line printed to the serial port at boot.
///
/// `wasmicon: assets doom1.wad=4196020 font.psf=3092` when the directory was
/// read (`assets none` when empty), `wasmicon: <reason>, no assets`
/// otherwise. A banner line, not a trace line (`>` / `<`), so `trace diff`
/// does not compare it.
///
/// `out` of 384 bytes holds even `MAX_ENTRIES` entries with the longest names.
pub fn describe(result: &Result<Directory, AssetsError>, out: &mut Buf<'_>) {
    out.str("wasmicon: ");
    match result {
        Ok(dir) if dir.is_empty() => out.str("assets none"),
        Ok(dir) => {
            out.str("assets");
            for e in dir.entries() {
                out.byte(b' ');
                out.bytes(e.name());
                out.byte(b'=');
                out.u32(e.len);
            }
        }
        Err(e) => {
            out.str(e.reason());
            out.str(", no assets");
        }
    }
}
