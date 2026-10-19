//! Checks that what `wasmicon pack` writes can be read back through the
//! firmware's path (`slot::parse`).
//!
//! Writer (CLI) and reader (firmware) share `slot` in `ports/common`, but
//! without a real round trip "both make the same mistake" goes undetected.
//! That the CRC is the standard CRC-32 is also checked from outside here.

use std::path::{Path, PathBuf};

use wasmicon_cli::pack;
use wasmicon_port::slot;

/// Asserts that parsing fails and returns the reason (`SlotError` does not
/// implement `Debug`).
fn parse_err(img: &[u8], what: &str) -> slot::SlotError {
    match slot::parse(img) {
        Err(e) => e,
        Ok(_) => panic!("expected {what}, but it parsed"),
    }
}

fn tmp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("pack-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the work directory");
    dir
}

/// Runs `pack` and returns the image it wrote.
fn pack_bytes(name: &str, wasm: &[u8]) -> Vec<u8> {
    let dir = tmp(name);
    let input = dir.join("app.wasm");
    std::fs::write(&input, wasm).expect("cannot write");
    let out = dir.join("app.slot");
    pack::run(&pack::Options {
        path: input,
        out: Some(out.clone()),
        board: None,
    })
    .expect("pack succeeds");
    std::fs::read(&out).expect("cannot read")
}

#[test]
fn what_pack_writes_is_what_the_firmware_reads() {
    let wasm = b"\0asm\x01\0\0\0some-bytes-standing-in-for-a-guest";
    let image = pack_bytes("roundtrip", wasm);
    let read = slot::parse(&image).unwrap_or_else(|e| panic!("cannot parse: {}", e.reason()));
    assert_eq!(read, wasm, "round-trips byte for byte");
}

#[test]
fn the_default_output_sits_next_to_the_input() {
    let dir = tmp("default-out");
    let input = dir.join("app.wasm");
    std::fs::write(&input, b"\0asm\x01\0\0\0").expect("cannot write");
    pack::run(&pack::Options {
        path: input,
        out: None,
        board: None,
    })
    .expect("pack succeeds");
    assert!(
        dir.join("app.bin").is_file(),
        "writes <input>.bin (picotool decides the file type by extension)"
    );
}

#[test]
fn the_header_is_the_documented_layout() {
    // The table in docs/app-workflow.md §3.4. This is the contract between
    // writer and reader, so a layout change bumps FORMAT_VERSION.
    let wasm = b"abcd";
    let image = pack_bytes("layout", wasm);

    assert_eq!(&image[0..4], b"WMCA", "magic");
    assert_eq!(
        u16::from_le_bytes([image[4], image[5]]),
        1,
        "format version"
    );
    assert_eq!(u16::from_le_bytes([image[6], image[7]]), 0, "reserved");
    assert_eq!(
        u32::from_le_bytes([image[8], image[9], image[10], image[11]]),
        4,
        "wasm length"
    );
    assert_eq!(&image[16..], wasm, "the wasm starts at byte 16");

    // CRC-32 (IEEE), checked from outside with a known value.
    // The CRC-32 of "abcd" is 0xed82cd11.
    let crc = u32::from_le_bytes([image[12], image[13], image[14], image[15]]);
    assert_eq!(crc, 0xed82_cd11, "standard CRC-32");
}

#[test]
fn a_board_with_a_slot_gets_its_offset_checked() {
    // With `--board`, the slot size is checked before writing. Flashing an
    // image that does not fit is only noticed when "slot truncated" appears
    // on hardware.
    let dir = tmp("board-ok");
    let input = dir.join("app.wasm");
    std::fs::write(&input, b"\0asm\x01\0\0\0").expect("cannot write");
    pack::run(&pack::Options {
        path: input.clone(),
        out: None,
        board: Some("rp2350".to_string()),
    })
    .expect("fits the rp2350 slot");

    // Larger than the slot (64 KiB) is rejected.
    let big = dir.join("big.wasm");
    std::fs::write(&big, vec![0u8; 100 * 1024]).expect("cannot write");
    let e = pack::run(&pack::Options {
        path: big,
        out: None,
        board: Some("rp2350".to_string()),
    })
    .expect_err("does not fit");
    assert!(format!("{e:#}").contains("does not fit"), "{e:#}");
}

#[test]
fn a_board_without_a_slot_is_refused() {
    // The host has no flash. A missing slot location is not ignored
    // silently.
    let dir = tmp("board-undecided");
    let input = dir.join("app.wasm");
    std::fs::write(&input, b"\0asm\x01\0\0\0").expect("cannot write");
    let e = pack::run(&pack::Options {
        path: input,
        out: None,
        board: Some("host".to_string()),
    })
    .expect_err("no slot");
    assert!(format!("{e:#}").contains("not decided"), "{e:#}");
    assert!(!dir.join("app.bin").exists(), "rejected before writing");
}

#[test]
fn a_flipped_bit_in_the_image_is_caught() {
    // Why the slot has a CRC. Without it, corrupt bytes reach decode and
    // produce an unrelated reason such as "unsupported instruction".
    let mut image = pack_bytes("corrupt", b"\0asm\x01\0\0\0aaaaaaaa");
    let last = image.len() - 1;
    image[last] ^= 0x01;
    let e = parse_err(&image, "a CRC mismatch");
    assert_eq!(e.reason(), "slot crc mismatch");
}

/// Reads the single image `deploy` writes (app slot + config slot) back
/// through the firmware's paths, one slot at a time
/// (docs/app-workflow.md §3.9).
#[test]
fn the_deploy_image_carries_the_app_and_the_wiring() {
    use wasmicon_port::profile;
    use wasmicon_port::roles::{self, Limits, RoleMap};

    let p = &profile::RP2350;
    let limits = Limits::of(p);
    let map = roles::from_table(&[("lcd-cs", 17), ("status-led", 15)], &limits)
        .unwrap_or_else(|e| panic!("{}", e.reason()));
    let wasm = b"\0asm\x01\0\0\0app";
    let image = pack::with_roles(&pack::build(wasm), p, &map).expect("builds");

    let (Some(sl), Some(rs)) = (p.slot, p.role_slot) else {
        panic!("rp2350 has a slot");
    };
    // The app slot starts at the beginning, the config slot right after it.
    let app = slot::parse(&image[..sl.len as usize]).unwrap_or_else(|e| panic!("{}", e.reason()));
    assert_eq!(app, wasm);
    let at = (rs.offset - sl.offset) as usize;
    let back = roles::parse(&image[at..], &limits).unwrap_or_else(|e| panic!("{}", e.reason()));
    assert_eq!(back.get(b"lcd-cs"), Some(17));
    assert_eq!(back.get(b"status-led"), Some(15));

    // An empty table is written too (a previously flashed table never
    // survives). It reads back as "no roles".
    let empty = pack::with_roles(&pack::build(wasm), p, &RoleMap::EMPTY).expect("builds");
    let back = roles::parse(&empty[at..], &limits).unwrap_or_else(|e| panic!("{}", e.reason()));
    assert!(back.is_empty());
}
