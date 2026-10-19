//! App slot format (docs/app-workflow.md §3.4).
//!
//! The wasm starts at a 16-byte boundary, so the slice read from the slot can
//! be passed to `decode` as is (an XIP slice on RP2350, a copy in RAM on
//! ESP32-S3). The layout is on `Header`.
//!
//! The CRC separates transfer accidents from app bugs. Without it, a corrupted
//! byte sequence reaches `decode` and produces an unrelated reason such as
//! "unsupported instruction" (one fewer of the silent failures in
//! docs/TODO.md §1.4).
//!
//! The writer (CLI) and the reader (firmware) share this module so the format
//! is written once.

use crate::crc32;
use crate::profile::Slot;

/// Identifier at the start of the slot.
pub const MAGIC: [u8; 4] = *b"WMCA";

/// Format version. Bump it only when the layout changes (one of the
/// compatibility axes, docs/app-workflow.md §3.6).
pub const FORMAT_VERSION: u16 = 1;

/// Header size. The wasm starts right after it.
pub const HEADER_LEN: usize = 16;

/// Why the slot cannot be read.
///
/// `ports/common` is written under the same constraints as the ports, so no
/// `core::fmt`: the text is a `&'static str` and the port prints the numbers
/// with its own `Buf`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SlotError {
    /// Erased (`0xff`) or never written (`0x00`). Not a failure but "empty";
    /// the firmware parks on it.
    Empty,
    /// Wrong magic. Something else is written there.
    BadMagic,
    /// A version this firmware cannot read.
    UnsupportedVersion { found: u16 },
    /// The length points past the slot.
    Truncated { need: usize, have: usize },
    /// CRC mismatch. A transfer or write accident.
    BadCrc { want: u32, got: u32 },
    /// The firmware's end reaches into the slot. Must not be read: it would
    /// feed the firmware's own code to the runtime as wasm.
    Overlap { fw_end: usize, slot_start: usize },
}

impl SlotError {
    /// Text for the log.
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            SlotError::Empty => "slot empty",
            SlotError::BadMagic => "slot magic mismatch",
            SlotError::UnsupportedVersion { .. } => "slot format version not supported",
            SlotError::Truncated { .. } => "slot truncated",
            SlotError::BadCrc { .. } => "slot crc mismatch",
            SlotError::Overlap { .. } => "firmware overlaps the app slot",
        }
    }

    /// Whether the slot is empty (nothing deployed yet). The one case that is not an error.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self == SlotError::Empty
    }
}

/// What the header says.
///
/// Slot layout:
///
/// ```text
/// offset  size  contents
/// 0       4     magic "WMCA" (Wasmicon App)
/// 4       2     format version (u16 LE, = 1)
/// 6       2     reserved (0)
/// 8       4     wasm length (u32 LE)
/// 12      4     CRC-32 of the wasm (u32 LE)
/// 16      n     wasm
/// ```
#[derive(Clone, Copy)]
pub struct Header {
    /// Length of the wasm.
    pub len: usize,
    /// CRC-32 stored in the header.
    pub crc: u32,
}

/// Reads only the header.
///
/// Boards that cannot read through XIP (ESP32-S3) read this first, then take
/// just that much arena for the body. An app is a few KB in a 64 KiB slot, so
/// RAM the size of the slot is not needed; DRAM is nearly exhausted
/// (docs/TODO.md §1.4).
///
/// # Errors
/// Empty, wrong magic or wrong version. The length and CRC are not checked
/// yet (`verify` does that once the body is read).
pub fn parse_header(head: &[u8]) -> Result<Header, SlotError> {
    let Some(head) = head.get(..HEADER_LEN) else {
        return Err(SlotError::Empty);
    };
    let magic: &[u8] = &head[..4];
    if magic != MAGIC {
        // Erased flash reads 0xff, never-written RAM reads 0x00. Both are "empty"
        // and are kept distinct from a wrong magic (the log means something else).
        if magic.iter().all(|b| *b == 0xff) || magic.iter().all(|b| *b == 0) {
            return Err(SlotError::Empty);
        }
        return Err(SlotError::BadMagic);
    }
    let version = u16::from_le_bytes([head[4], head[5]]);
    if version != FORMAT_VERSION {
        return Err(SlotError::UnsupportedVersion { found: version });
    }
    Ok(Header {
        len: u32::from_le_bytes([head[8], head[9], head[10], head[11]]) as usize,
        crc: u32::from_le_bytes([head[12], head[13], head[14], head[15]]),
    })
}

/// Checks that the body read matches the header.
///
/// # Errors
/// Wrong length or CRC mismatch.
pub fn verify(wasm: &[u8], h: &Header) -> Result<(), SlotError> {
    if wasm.len() != h.len {
        return Err(SlotError::Truncated {
            need: h.len,
            have: wasm.len(),
        });
    }
    let got = crc32(wasm);
    if got != h.crc {
        return Err(SlotError::BadCrc { want: h.crc, got });
    }
    Ok(())
}

/// Extracts the wasm from the slot.
///
/// `bytes` may be the whole slot region; the header carries the length and
/// the rest is ignored.
///
/// # Errors
/// Empty, wrong magic, wrong version, short length or CRC mismatch.
pub fn parse(bytes: &[u8]) -> Result<&[u8], SlotError> {
    let h = parse_header(bytes)?;
    let Some(need) = image_end(h.len) else {
        return Err(SlotError::Truncated {
            need: usize::MAX,
            have: bytes.len(),
        });
    };
    let Some(wasm) = bytes.get(HEADER_LEN..need) else {
        return Err(SlotError::Truncated {
            need,
            have: bytes.len(),
        });
    };
    verify(wasm, &h)?;
    Ok(wasm)
}

/// Reads the slot from memory-mapped (XIP) flash.
///
/// On RP2040 / RP2350 the flash is memory-mapped, so the slot is returned as
/// a slice without copying it to RAM (docs/design-notes.md §4). Both ports do
/// the same, so it lives here and the `unsafe` that creates the raw slice
/// exists in one place.
///
/// `fw_end` is the address of the firmware's end as placed by the linker. If
/// it reaches into the slot, nothing is read and `Overlap` is returned.
///
/// # Errors
/// Overlap, or whatever `parse` reports.
///
/// # Safety
/// `slot.len` bytes starting at `xip_base + slot.offset` must be readable
/// memory-mapped flash that stays mapped for the rest of the program
/// (reading outside it is a bus fault).
pub unsafe fn read_xip(
    xip_base: usize,
    slot: Slot,
    fw_end: usize,
) -> Result<&'static [u8], SlotError> {
    // SAFETY: this function's own contract guarantees that `slot.len` bytes
    // from `xip_base + slot.offset` are readable memory-mapped flash that stays
    // mapped for the rest of the program, which is exactly what `xip_region`
    // requires.
    parse(unsafe { xip_region(xip_base, slot, fw_end) }?)
}

/// Returns one region of memory-mapped (XIP) flash as a slice, without
/// interpreting it.
///
/// Used by both the app slot (`read_xip`) and the config slot (`roles`). This
/// is the only place that creates a raw slice with `unsafe`.
///
/// # Errors
/// `Overlap` when the firmware's end reaches into the region.
///
/// # Safety
/// `slot.len` bytes starting at `xip_base + slot.offset` must be readable
/// memory-mapped flash that stays mapped for the rest of the program.
pub unsafe fn xip_region(
    xip_base: usize,
    slot: Slot,
    fw_end: usize,
) -> Result<&'static [u8], SlotError> {
    let slot_start = xip_base + slot.offset as usize;
    if fw_end > slot_start {
        // Returning here means no slice is created when the regions overlap.
        return Err(SlotError::Overlap { fw_end, slot_start });
    }
    // SAFETY: the caller guarantees that `slot.len` bytes from `slot_start` are
    // readable memory-mapped flash that stays mapped for the rest of the
    // program, so a `'static` shared slice over them is valid. The check above
    // puts the region past the firmware's end, and flash is never written
    // through a reference, so no `&mut` aliases it.
    Ok(unsafe { core::slice::from_raw_parts(slot_start as *const u8, slot.len as usize) })
}

/// Builds the header (for the writer).
#[must_use]
pub fn header(wasm: &[u8]) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[..4].copy_from_slice(&MAGIC);
    h[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    // 6..8 is reserved (0).
    let len = u32::try_from(wasm.len()).unwrap_or(u32::MAX);
    h[8..12].copy_from_slice(&len.to_le_bytes());
    h[12..16].copy_from_slice(&crc32(wasm).to_le_bytes());
    h
}

/// Total size of the image placed in the slot (for the writer).
///
/// The writer's length is the size of a real file, so it cannot overflow. The
/// reader uses `image_end`.
#[must_use]
pub fn image_len(wasm_len: usize) -> usize {
    HEADER_LEN + wasm_len
}

/// Total bytes needed for the length the header claims, or `None` on overflow.
///
/// This is the one place the reader does the addition. The length comes from
/// possibly corrupt flash, and on targets where `usize` is 32 bits (thumbv6m /
/// thumbv8m / xtensa) `0xffff_fff8 + 16` wraps to 8. Wrapped, `Truncated`
/// reports a self-contradictory reason such as "need 8 / have 65536", and a
/// debug build panics on the addition itself: the path that exists to report
/// the reason dies silently (contrary to docs/decisions.md A-04).
///
/// This does not reproduce on the host through `parse` (the length is a u32
/// and the host's `usize` is 64 bits), so the test calls this function
/// directly (`tests/slot.rs`).
#[must_use]
pub fn image_end(wasm_len: usize) -> Option<usize> {
    HEADER_LEN.checked_add(wasm_len)
}
