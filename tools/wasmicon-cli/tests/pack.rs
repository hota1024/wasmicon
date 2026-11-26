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

/// Lays the regions `deploy` writes over a slot image the way flash holds
/// them: everything outside the regions keeps what `under` had there.
fn overlay(under: &[u8], base: u32, regions: &[pack::Region]) -> Vec<u8> {
    let mut flash = under.to_vec();
    for r in regions {
        let at = (r.offset - base) as usize;
        flash[at..at + r.bytes.len()].copy_from_slice(&r.bytes);
    }
    flash
}

/// The app slot and the config slot of `p`, as one stretch of erased flash.
fn erased(p: &wasmicon_port::profile::Profile) -> (u32, Vec<u8>) {
    let (Some(sl), Some(rs)) = (p.slot, p.role_slot) else {
        panic!("{} has a slot", p.name);
    };
    (
        sl.offset,
        vec![0xff; (rs.offset + rs.len - sl.offset) as usize],
    )
}

/// Reads the regions `deploy` writes (app slot + config slot) back through
/// the firmware's paths, one slot at a time (docs/app-workflow.md §3.9).
#[test]
fn the_deploy_image_carries_the_app_and_the_wiring() {
    use wasmicon_port::profile;
    use wasmicon_port::roles::{self, DisplayConfig, Limits, Wiring};

    let p = &profile::RP2350;
    let limits = Limits::of(p);
    let wasm = b"\0asm\x01\0\0\0app";
    let lcd = DisplayConfig::Ili9341 {
        spi: 0,
        cs: 17,
        dc: 20,
        rst: 21,
        rotation: 90,
        scale: 1,
    };
    let w = Wiring {
        roles: roles::from_table(&[("status-led", 15)], &limits)
            .unwrap_or_else(|e| panic!("{}", e.reason())),
        display: Some(lcd),
        touch: None,
        ..Wiring::EMPTY
    };
    let regions = pack::regions(&pack::build(wasm), p, &w).expect("builds");

    let (Some(sl), Some(rs)) = (p.slot, p.role_slot) else {
        panic!("rp2350 has a slot");
    };
    // Whole sectors at the start of each slot.
    assert_eq!(regions[0].offset, sl.offset);
    assert_eq!(regions[1].offset, rs.offset);
    for r in &regions {
        assert_eq!(r.bytes.len() % pack::SECTOR as usize, 0, "{}", r.what);
        assert!(
            r.bytes[r.content..].iter().all(|b| *b == 0xff),
            "{}",
            r.what
        );
    }
    assert_eq!(regions[0].content, slot::image_len(wasm.len()));
    assert_eq!(regions[0].bytes.len(), 4096, "a small app is one sector");

    let (base, flash) = erased(p);
    let flash = overlay(&flash, base, &regions);
    let app = slot::parse(&flash[..sl.len as usize]).unwrap_or_else(|e| panic!("{}", e.reason()));
    assert_eq!(app, wasm);
    let at = (rs.offset - sl.offset) as usize;
    let back = roles::parse(&flash[at..], &limits).unwrap_or_else(|e| panic!("{}", e.reason()));
    assert_eq!(back.roles.get(b"status-led"), Some(15));
    assert_eq!(back.display, Some(lcd));
    assert_eq!(
        pack::describe_display(&w).as_deref(),
        Some("ili9341 spi=0 cs=17 dc=20 rst=21 rotation=90")
    );

    // An empty table is written too (a previously flashed table never
    // survives). It reads back as "no roles".
    let empty = pack::regions(&pack::build(wasm), p, &Wiring::EMPTY).expect("builds");
    let flash = overlay(&flash, base, &empty);
    let back = roles::parse(&flash[at..], &limits).unwrap_or_else(|e| panic!("{}", e.reason()));
    assert!(back.roles.is_empty() && back.display.is_none());

    // A table that moves I2C0 to GP8/9 is version 6, frees GP4 for a role,
    // and the firmware opens the bus where the table says.
    let buses = roles::Buses {
        i2c0: Some(roles::I2cPins { sda: 8, scl: 9 }),
        ..roles::Buses::NONE
    };
    let moved_limits = limits
        .with_buses(&buses)
        .unwrap_or_else(|e| panic!("{}", e.reason()));
    let moved = Wiring {
        roles: roles::from_table(&[("status-led", 4)], &moved_limits)
            .unwrap_or_else(|e| panic!("{}", e.reason())),
        buses,
        ..Wiring::EMPTY
    };
    let regions = pack::regions(&pack::build(wasm), p, &moved).expect("builds");
    let flash = overlay(&flash, base, &regions);
    assert_eq!(u16::from_le_bytes([flash[at + 4], flash[at + 5]]), 6);
    let back = roles::parse(&flash[at..], &limits).unwrap_or_else(|e| panic!("{}", e.reason()));
    assert_eq!(back.roles.get(b"status-led"), Some(4));
    assert_eq!(limits.bus_pins(&back).i2c0(), buses.i2c0);
    assert_eq!(
        limits
            .bus_pins(&back)
            .spi0()
            .map(|s| (s.sck, s.mosi, s.miso)),
        Some((18, 19, 16)),
        "SPI0 keeps its defaults"
    );
    assert_eq!(
        pack::describe_buses(&moved).as_deref(),
        Some("i2c0=sda=8 scl=9")
    );
}

/// `deploy` writes only the sectors the new app needs. A longer app flashed
/// before leaves its tail in the slot, and the firmware must read the new
/// app alone: it reads the header's length and checks the CRC over exactly
/// that (`slot::parse`; the ESP32 ports read the header, then that length).
#[test]
fn a_shorter_app_over_a_longer_one_reads_back_alone() {
    use wasmicon_port::profile;
    use wasmicon_port::roles::{self, Limits, Wiring};

    for p in [&profile::RP2350, &profile::ESP32S3] {
        let limits = Limits::of(p);
        let (Some(sl), Some(rs)) = (p.slot, p.role_slot) else {
            panic!("{} has a slot", p.name);
        };
        // Three sectors of an old app, then a short new one.
        let long: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        let short = b"\0asm\x01\0\0\0new";
        let wired = Wiring {
            roles: roles::from_table(&[("status-led", 15), ("lcd-cs", 17)], &limits)
                .unwrap_or_else(|e| panic!("{}", e.reason())),
            display: None,
            touch: None,
            ..Wiring::EMPTY
        };

        let (base, flash) = erased(p);
        let old = pack::regions(&pack::build(&long), p, &wired).expect("builds");
        assert_eq!(old[0].bytes.len(), 3 * 4096);
        let flash = overlay(&flash, base, &old);
        let new = pack::regions(&pack::build(short), p, &Wiring::EMPTY).expect("builds");
        assert_eq!(new[0].bytes.len(), 4096);
        let flash = overlay(&flash, base, &new);

        // The old app's second and third sectors are still there ...
        assert_eq!(&flash[4096..8192], &old[0].bytes[4096..8192], "{}", p.name);
        // ... and the firmware reads only the new app.
        let app = slot::parse(&flash[..sl.len as usize])
            .unwrap_or_else(|e| panic!("{}: {}", p.name, e.reason()));
        assert_eq!(app, short);
        let at = (rs.offset - sl.offset) as usize;
        let back = roles::parse(&flash[at..], &limits).unwrap_or_else(|e| panic!("{}", e.reason()));
        assert!(back.roles.is_empty(), "{}: the old table survived", p.name);

        // The ESP32 path: the header, then exactly its length.
        let h = slot::parse_header(&flash[..slot::HEADER_LEN])
            .unwrap_or_else(|e| panic!("{}", e.reason()));
        let body = &flash[slot::HEADER_LEN..slot::HEADER_LEN + h.len];
        assert!(slot::verify(body, &h).is_ok(), "{}", p.name);
    }
}

/// The UF2 file `deploy` hands picotool holds both regions at their XIP
/// addresses, in the 256-byte blocks picotool accepts.
#[test]
fn the_uf2_file_puts_each_region_at_its_address() {
    use wasmicon_port::profile;
    use wasmicon_port::roles::Wiring;

    let p = &profile::RP2350;
    let regions =
        pack::regions(&pack::build(b"\0asm\x01\0\0\0app"), p, &Wiring::EMPTY).expect("builds");
    let file = pack::uf2(&regions).expect("encodes");
    let blocks: Vec<&[u8]> = file.chunks(512).collect();
    let total = regions.iter().map(|r| r.bytes.len() / 256).sum::<usize>();
    assert_eq!(blocks.len(), total);

    let word = |b: &[u8], i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let mut back: Vec<(u32, Vec<u8>)> = Vec::new();
    for (n, b) in blocks.iter().enumerate() {
        assert_eq!(b.len(), 512);
        assert_eq!(word(b, 0), 0x0A32_4655);
        assert_eq!(word(b, 4), 0x9E5D_5157);
        assert_eq!(word(b, 8), 0x2000, "family ID present, nothing else");
        assert_eq!(word(b, 16), 256);
        assert_eq!(word(b, 20) as usize, n);
        assert_eq!(word(b, 24) as usize, total);
        assert_eq!(word(b, 28), pack::UF2_FAMILY_ABSOLUTE);
        assert_eq!(word(b, 508), 0x0AB1_6F30);
        let addr = word(b, 12);
        match back.last_mut() {
            Some((start, data)) if *start + data.len() as u32 == addr => {
                data.extend_from_slice(&b[32..288]);
            }
            _ => back.push((addr, b[32..288].to_vec())),
        }
    }
    let want: Vec<(u32, Vec<u8>)> = regions
        .iter()
        .map(|r| ((pack::XIP_BASE as u32) + r.offset, r.bytes.clone()))
        .collect();
    assert_eq!(back, want);
}

/// The Tab5's panel and touch entries go through the ESP32-P4's slot and come
/// back as the boot banner names them (docs/device-layer.md §11.3).
#[test]
fn the_deploy_image_carries_the_touch_entry() {
    use wasmicon_port::profile;
    use wasmicon_port::roles::{self, DisplayConfig, Limits, TouchConfig, Wiring};

    let p = &profile::ESP32P4;
    // Roles named like the entries do not leak into their descriptions.
    let limits = Limits::of(p);
    let w = Wiring {
        roles: roles::from_table(&[("display", 20), ("touch", 21)], &limits)
            .unwrap_or_else(|e| panic!("{}", e.reason())),
        display: Some(DisplayConfig::TAB5),
        touch: Some(TouchConfig::Tab5),
        ..Wiring::EMPTY
    };
    let regions = pack::regions(&pack::build(b"\0asm\x01\0\0\0app"), p, &w).expect("builds");
    let back =
        roles::parse(&regions[1].bytes, &limits).unwrap_or_else(|e| panic!("{}", e.reason()));
    assert_eq!(back.touch, Some(TouchConfig::Tab5));
    assert_eq!(pack::describe_display(&w).as_deref(), Some("tab5"));
    assert_eq!(pack::describe_touch(&w).as_deref(), Some("tab5"));
}

/// A region that is not whole 256-byte blocks would be written with zeros
/// after its end, and one past the address space would wrap; both are
/// refused rather than encoded.
#[test]
fn the_uf2_encoder_refuses_what_it_cannot_write_exactly() {
    let short = pack::Region {
        what: "app slot",
        offset: 0x10_0000,
        content: 100,
        bytes: vec![0xff; 100],
    };
    let msg = format!("{:#}", pack::uf2(&[short]).expect_err("refused"));
    assert!(msg.contains("256 B blocks"), "{msg}");
    let far = pack::Region {
        what: "app slot",
        offset: 0xffff_f000,
        content: 4096,
        bytes: vec![0xff; 4096],
    };
    let msg = format!("{:#}", pack::uf2(&[far]).expect_err("refused"));
    assert!(msg.contains("32-bit address space"), "{msg}");
}

/// Every board's profile gives regions that stay inside their slots.
#[test]
fn every_board_with_slots_gives_regions_inside_them() {
    use wasmicon_port::profile;
    use wasmicon_port::roles::Wiring;

    for p in profile::PROFILES {
        let (Some(sl), Some(rs)) = (p.slot, p.role_slot) else {
            continue;
        };
        let regions = pack::regions(&pack::build(b"\0asm\x01\0\0\0app"), p, &Wiring::EMPTY)
            .unwrap_or_else(|e| panic!("{}: {e:#}", p.name));
        assert!(regions[0].bytes.len() <= sl.len as usize, "{}", p.name);
        assert!(regions[1].bytes.len() <= rs.len as usize, "{}", p.name);
    }
    // An app that fills the slot to the last byte still fits; one byte more
    // is refused by `fits` before this, and by `regions` too.
    let p = &profile::RP2350;
    let sl = p.slot.expect("rp2350 has a slot");
    let big = vec![0u8; sl.len as usize - 16 + 1];
    let msg = format!(
        "{:#}",
        pack::regions(&pack::build(&big), p, &Wiring::EMPTY)
            .err()
            .expect("refused")
    );
    assert!(msg.contains("cannot hold"), "{msg}");
}

/// The failure `roles::bind` exists for: a deploy writes the new app, and
/// the config slot write fails, so the flash holds the new app and the
/// previous app's table. The firmware must hand the new app no roles.
#[test]
fn a_new_app_over_an_old_table_gets_no_roles() {
    use wasmicon_port::profile;
    use wasmicon_port::roles::{self, Limits, RolesError, Wiring};

    for p in [&profile::RP2350, &profile::ESP32S3] {
        let limits = Limits::of(p);
        let (Some(sl), Some(rs)) = (p.slot, p.role_slot) else {
            panic!("{} has a slot", p.name);
        };
        let wired = Wiring {
            roles: roles::from_table(&[("status-led", 15)], &limits)
                .unwrap_or_else(|e| panic!("{}", e.reason())),
            display: None,
            touch: None,
            ..Wiring::EMPTY
        };
        let (base, flash) = erased(p);
        let old = pack::regions(&pack::build(b"\0asm\x01\0\0\0old"), p, &wired).expect("builds");
        let flash = overlay(&flash, base, &old);
        let new = pack::regions(&pack::build(b"\0asm\x01\0\0\0new"), p, &wired).expect("builds");
        let at = (rs.offset - sl.offset) as usize;

        // The whole deploy: the new table is bound to the new app.
        let both = overlay(&flash, base, &new);
        let crc = slot::parse_header(&both).map(|h| h.crc).ok();
        let back = roles::parse_bound(&both[at..], &limits, crc)
            .unwrap_or_else(|e| panic!("{}: {}", p.name, e.reason()));
        assert_eq!(back.roles.get(b"status-led"), Some(15));

        // Only the app went in.
        let half = overlay(&flash, base, &new[..1]);
        let crc = slot::parse_header(&half).map(|h| h.crc).ok();
        let got = roles::parse_bound(&half[at..], &limits, crc).err();
        assert!(got == Some(RolesError::OtherApp), "{}", p.name);
    }
}
