//! Locks in the app slot format (docs/app-workflow.md §3.4).
//!
//! The writer (CLI) and the reader (firmware) share the format, so this is the
//! contract for both. Bump `FORMAT_VERSION` when the shape changes (one of the
//! compatibility axes, docs/app-workflow.md §3.6).

use wasmicon_port::profile::{Flasher, Slot, SlotRead};
use wasmicon_port::slot::{self, SlotError};

/// Checks that the image parses and returns its contents.
///
/// `SlotError` does not implement `Debug` (`ports/common` is written under the
/// same constraints as the ports, so no `core::fmt`). Failures print `reason()`.
fn parsed(img: &[u8]) -> &[u8] {
    slot::parse(img).unwrap_or_else(|e| panic!("cannot parse: {}", e.reason()))
}

/// Checks that the image does not parse and returns the reason.
fn parse_err(img: &[u8], what: &str) -> SlotError {
    match slot::parse(img) {
        Err(e) => e,
        Ok(_) => panic!("expected {what} but it parsed"),
    }
}

/// Builds an image (header + wasm).
fn image(wasm: &[u8]) -> Vec<u8> {
    let mut v = slot::header(wasm).to_vec();
    v.extend_from_slice(wasm);
    assert_eq!(v.len(), slot::image_len(wasm.len()));
    v
}

#[test]
fn a_written_slot_reads_back_byte_for_byte() {
    // The contents are not checked, only that the bytes round-trip.
    let wasm = b"\0asm\x01\0\0\0not-really-wasm-but-round-trips";
    let img = image(wasm);
    assert_eq!(parsed(&img), wasm);
}

#[test]
fn the_wasm_starts_at_a_16_byte_boundary() {
    // The premise for passing the slice straight to decode (docs/app-workflow.md §3.4).
    assert_eq!(slot::HEADER_LEN, 16);
    let img = image(b"xyz");
    assert_eq!(&img[16..], b"xyz");
}

#[test]
fn an_empty_slot_is_not_an_error() {
    // Erased flash reads 0xff, never-written RAM reads 0x00. Both are "empty",
    // and the firmware parks on them (docs/app-workflow.md §3.1).
    for fill in [0xffu8, 0x00] {
        let e = parse_err(&[fill; 256], "empty");
        assert!(e.is_empty(), "fill={fill:#x} is treated as empty");
        assert_eq!(e.reason(), "slot empty");
    }
    // Too short for a header counts as empty too (same as looking at an erased start).
    assert!(parse_err(&[], "empty").is_empty());
    assert!(parse_err(&[0xff; 4], "empty").is_empty());
}

#[test]
fn other_contents_are_distinguished_from_empty() {
    // Something else written there is not "empty"; the log would mean something else.
    let mut img = image(b"xyz");
    img[0] = b'X';
    let e = parse_err(&img, "magic mismatch");
    assert!(!e.is_empty());
    assert!(matches!(e, SlotError::BadMagic), "{}", e.reason());
}

#[test]
fn a_newer_format_is_refused_with_its_version() {
    let mut img = image(b"xyz");
    img[4..6].copy_from_slice(&2u16.to_le_bytes());
    let e = parse_err(&img, "version mismatch");
    assert!(
        matches!(e, SlotError::UnsupportedVersion { found: 2 }),
        "{}",
        e.reason()
    );
}

#[test]
fn a_length_past_the_slot_is_refused() {
    let mut img = image(b"xyz");
    img[8..12].copy_from_slice(&999u32.to_le_bytes());
    let e = parse_err(&img, "length past the slot");
    let have = img.len();
    assert!(
        matches!(e, SlotError::Truncated { need, have: h } if need == 16 + 999 && h == have),
        "{}",
        e.reason()
    );
}

#[test]
fn a_corrupted_byte_is_caught_by_the_crc() {
    // This is why the CRC exists. Without it a corrupted byte sequence reaches
    // decode and produces an unrelated reason such as "unsupported instruction".
    let wasm = b"\0asm\x01\0\0\0aaaaaaaaaaaaaaaa";
    let mut img = image(wasm);
    let last = img.len() - 1;
    img[last] ^= 0x01;
    match parse_err(&img, "crc mismatch") {
        SlotError::BadCrc { want, got } => assert_ne!(want, got),
        other => panic!("expected a crc mismatch: {}", other.reason()),
    }
}

#[test]
fn trailing_bytes_in_the_slot_are_ignored() {
    // The whole slot region may be passed (the header carries the length). The
    // intended use reads a whole 4 KiB sector and passes it.
    let wasm = b"\0asm\x01\0\0\0";
    let mut img = image(wasm);
    img.resize(4096, 0xff);
    assert_eq!(parsed(&img), wasm);
}

#[test]
fn an_overlapping_firmware_is_refused_without_reading() {
    // No slice is created on overlap (it would feed the firmware's own code to
    // the runtime as wasm). Only addresses are passed, so this runs on the host.
    let slot = Slot {
        offset: 0x10_0000,
        len: 64 * 1024,
        read: SlotRead::Xip,
        flasher: Flasher::Picotool,
    };
    let xip = 0x1000_0000;
    let slot_start = xip + slot.offset as usize;
    // SAFETY: `fw_end` is past `slot_start`, so `read_xip` takes the overlap
    // branch and returns before creating a slice; no memory is dereferenced.
    let e = match unsafe { slot::read_xip(xip, slot, slot_start + 1) } {
        Err(e) => e,
        Ok(_) => panic!("overlap not detected"),
    };
    assert_eq!(e.reason(), "firmware overlaps the app slot");
    assert!(matches!(e, SlotError::Overlap { .. }));
}

#[test]
fn a_mapped_slot_is_read_in_place() {
    // The XIP read itself also runs on the host: a Vec stands in for flash and
    // its address is passed as xip_base.
    let wasm = b"\0asm\x01\0\0\0mapped";
    let mut region = image(wasm);
    region.resize(4096, 0xff);
    let slot = Slot {
        offset: 0,
        len: u32::try_from(region.len()).expect("fits in u32"),
        read: SlotRead::Xip,
        flasher: Flasher::Picotool,
    };
    // SAFETY: `region` holds `slot.len` readable bytes, is not mutated after
    // this point and outlives every use of the returned slice.
    let read = match unsafe { slot::read_xip(region.as_ptr() as usize, slot, 0) } {
        Ok(w) => w,
        Err(e) => panic!("cannot read: {}", e.reason()),
    };
    assert_eq!(read, wasm);
}

#[test]
fn an_empty_app_is_still_a_valid_slot() {
    // A zero-length wasm is rejected by decode (not this module's job). As a
    // slot it is well-formed, and this test fixes that.
    let img = image(b"");
    assert_eq!(parsed(&img), b"");
}

#[test]
fn a_corrupt_length_is_refused_without_arithmetic_overflow() {
    // Corrupt flash can put anything in the length. A plain `HEADER_LEN + len`
    // wraps on targets where `usize` is 32 bits (thumbv6m / thumbv8m / xtensa)
    // and panics in a debug build: the path that exists to report the reason
    // dies silently. The host is 64-bit, so the wrap itself does not reproduce,
    // but returning `Truncated` without failing can be locked in here.
    let mut bytes = image(b"\0asm\x01\0\0\0");
    bytes.resize(4096, 0xff);
    bytes[8..12].copy_from_slice(&0xffff_fff8u32.to_le_bytes());
    let e = match slot::parse(&bytes) {
        Err(e) => e,
        Ok(_) => panic!("accepted a corrupt length"),
    };
    assert_eq!(e.reason(), "slot truncated");
    assert!(matches!(e, SlotError::Truncated { .. }));

    // Now the wrap itself. The path above cannot overflow on the 64-bit host
    // because the length is a u32, so call `image_end`, the one place that does
    // the addition, directly.
    assert_eq!(slot::image_end(0), Some(slot::HEADER_LEN));
    assert_eq!(slot::image_end(usize::MAX), None, "overflow missed");
    assert_eq!(
        slot::image_end(usize::MAX - slot::HEADER_LEN),
        Some(usize::MAX),
        "an exact fit must not count as overflow"
    );
}

#[test]
fn the_incremental_crc_matches_the_one_shot_one() {
    // ESP32-S3 checks the slot CRC before taking arena (finding corruption
    // afterwards would not help, since `Arena` is a bump allocator and cannot
    // give memory back). It computes the CRC in chunks, so the result must
    // match the one-shot computation.
    let data: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
    for chunk in [1usize, 7, 251, 512, 1000, 4096] {
        let mut crc = wasmicon_port::CRC32_INIT;
        for part in data.chunks(chunk) {
            crc = wasmicon_port::crc32_update(crc, part);
        }
        assert_eq!(
            wasmicon_port::crc32_end(crc),
            wasmicon_port::crc32(&data),
            "feeding {chunk} bytes at a time does not match"
        );
    }
    // Empty input matches too (a zero-length app is rejected, but lock in the boundary).
    assert_eq!(
        wasmicon_port::crc32_end(wasmicon_port::CRC32_INIT),
        wasmicon_port::crc32(&[])
    );
}

#[test]
fn the_table_crc_matches_the_bitwise_one() {
    // `crc32_update` looks bytes up in a table. Every CRC already recorded
    // (slot headers, the assets directory, traces) was made by the bitwise
    // loop, so the two must agree on every input.
    fn bitwise(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &b in data {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
            }
        }
        !crc
    }
    assert_eq!(wasmicon_port::crc32(b"123456789"), 0xcbf4_3926);
    let mut x = 0x1234_5678u32;
    let data: Vec<u8> = (0..65_536)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect();
    for len in [0usize, 1, 2, 3, 255, 256, 257, 4096, 65_536] {
        assert_eq!(
            wasmicon_port::crc32(&data[..len]),
            bitwise(&data[..len]),
            "{len} bytes"
        );
    }
    for b in 0..=255u8 {
        assert_eq!(wasmicon_port::crc32(&[b]), bitwise(&[b]), "byte {b:#04x}");
    }
}
