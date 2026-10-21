//! Locks in the assets slot format and checks (docs/app-workflow.md §3.10).
//!
//! The writer (CLI, host mock) and the reader (firmware) share `assets`, so
//! this is the contract for both. Bump `FORMAT_VERSION` when the shape changes.

use wasmicon_port::assets::{
    self, AssetsError, BODY_ALIGN, BODY_START, DIR_LEN, Directory, ENTRY_LEN, HEADER_LEN,
    MAX_ENTRIES, MAX_NAME,
};
use wasmicon_port::crc32;
use wasmicon_port::fmt::Buf;

/// Builds the slot image as the CLI does: the directory, then each body on a
/// 4 KiB boundary, `0xff` in between (erased flash).
fn image(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut dir = Directory::EMPTY;
    for (name, body) in entries {
        dir.insert(name.as_bytes(), body.len() as u32, crc32(body))
            .unwrap_or_else(|e| panic!("cannot insert {name}: {}", e.reason()));
    }
    let mut out = vec![0xff; dir.image_len() as usize];
    let mut head = [0u8; DIR_LEN];
    assets::encode(&dir, &mut head);
    out[..DIR_LEN].copy_from_slice(&head);
    for (i, (_, body)) in entries.iter().enumerate() {
        let at = dir.entry(i).offset as usize;
        out[at..at + body.len()].copy_from_slice(body);
    }
    out
}

fn parsed(img: &[u8]) -> Directory {
    assets::parse(img, img.len() as u32).unwrap_or_else(|e| panic!("cannot parse: {}", e.reason()))
}

fn parse_err(img: &[u8], what: &str) -> AssetsError {
    match assets::parse(img, img.len() as u32) {
        Err(e) => e,
        Ok(_) => panic!("expected {what} but it parsed"),
    }
}

/// Re-signs the entry table after a test edited it, so that only the edited
/// field is wrong.
fn resign(img: &mut [u8]) {
    let count = usize::from(u16::from_le_bytes([img[6], img[7]]));
    let crc = crc32(&img[HEADER_LEN..HEADER_LEN + count * ENTRY_LEN]);
    img[8..12].copy_from_slice(&crc.to_le_bytes());
}

fn u32_at(img: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([img[at], img[at + 1], img[at + 2], img[at + 3]])
}

fn put_u32(img: &mut [u8], at: usize, v: u32) {
    img[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// Byte offset of the `i`-th entry's `offset` field.
fn offset_field(i: usize) -> usize {
    HEADER_LEN + i * ENTRY_LEN + MAX_NAME
}

#[test]
fn a_written_directory_reads_back() {
    let wad = [0xabu8; 100];
    let font: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
    let img = image(&[("doom1.wad", &wad), ("font.psf", &font)]);
    let dir = parsed(&img);
    assert_eq!(dir.len(), 2);

    let w = dir.entry(dir.find(b"doom1.wad").expect("found"));
    assert_eq!(w.name(), b"doom1.wad");
    assert_eq!(w.offset, BODY_START);
    assert_eq!(w.len, 100);
    assert_eq!(w.crc, crc32(&wad));
    assert_eq!(&img[4096..4196], &wad[..]);
    assert_eq!(img[4196], 0xff, "padding reads as erased flash");

    let f = dir.entry(dir.find(b"font.psf").expect("found"));
    assert_eq!(
        f.offset, 8192,
        "the second body starts on the next 4 KiB boundary"
    );
    assert_eq!(f.len, 5000);
    assert_eq!(f.crc, crc32(&font));
    assert_eq!(&img[8192..8192 + 5000], &font[..]);

    assert_eq!(dir.image_len(), 16384, "the image ends on a body boundary");
    assert_eq!(img.len(), 16384);
    assert_eq!(
        dir.find(b"nope"),
        None,
        "a name that was not written is absent"
    );
}

#[test]
fn the_header_is_the_documented_layout() {
    let img = image(&[("a", b"xyz")]);
    assert_eq!(&img[..4], b"WMCB");
    assert_eq!(&img[4..6], &[1, 0], "format version 1, u16 LE");
    assert_eq!(&img[6..8], &[1, 0], "one entry, u16 LE");
    assert_eq!(
        u32_at(&img, 8),
        crc32(&img[HEADER_LEN..HEADER_LEN + ENTRY_LEN]),
        "the CRC covers the entry table"
    );
    assert_eq!(&img[12..16], &[0; 4], "reserved");
    // The entry: NUL-padded name, then offset, length and CRC.
    assert_eq!(&img[16..17], b"a");
    assert!(
        img[17..48].iter().all(|b| *b == 0),
        "the name is NUL-padded"
    );
    assert_eq!(u32_at(&img, 48), 4096);
    assert_eq!(u32_at(&img, 52), 3);
    assert_eq!(u32_at(&img, 56), crc32(b"xyz"));
    assert!(
        img[60..DIR_LEN].iter().all(|b| *b == 0),
        "unused entries are zero"
    );
    assert!(
        img[DIR_LEN..4096].iter().all(|b| *b == 0xff),
        "the rest of the directory sector is erased"
    );
    assert_eq!(DIR_LEN, 368);
}

#[test]
fn bodies_start_at_4096_and_are_4k_aligned() {
    let one = [1u8; 1];
    let sector = [2u8; 4096];
    let sector_plus = [3u8; 4097];
    let img = image(&[("a", &one), ("b", &sector), ("c", &sector_plus), ("d", b"")]);
    let dir = parsed(&img);
    let offsets: Vec<u32> = dir.entries().iter().map(|e| e.offset).collect();
    assert_eq!(offsets, [4096, 8192, 12288, 20480]);
    assert!(offsets.iter().all(|o| o % BODY_ALIGN == 0));
    assert_eq!(dir.image_len(), 20480, "a zero-length body takes no space");
    assert_eq!(
        Directory::EMPTY.image_len(),
        BODY_START,
        "the directory alone"
    );
}

#[test]
fn an_erased_slot_is_empty_not_an_error() {
    for (bytes, what) in [
        (vec![0xffu8; 4096], "erased"),
        (vec![0u8; 4096], "zeroed"),
        (vec![0xffu8; 8], "shorter than a header"),
        (Vec::new(), "nothing"),
    ] {
        let e = parse_err(&bytes, what);
        assert!(e.is_empty(), "{what}: {}", e.reason());
    }
}

#[test]
fn a_corrupt_directory_is_rejected_whole() {
    let good = image(&[("a", b"xyz"), ("b", b"q")]);
    assert_eq!(parsed(&good).len(), 2);

    let mut magic = good.clone();
    magic[..4].copy_from_slice(b"WMCA");
    assert!(matches!(
        parse_err(&magic, "bad magic"),
        AssetsError::BadMagic
    ));

    let mut version = good.clone();
    version[4] = 2;
    assert!(matches!(
        parse_err(&version, "version 2"),
        AssetsError::UnsupportedVersion
    ));

    let mut count = good.clone();
    count[6] = (MAX_ENTRIES + 1) as u8;
    assert!(matches!(
        parse_err(&count, "9 entries"),
        AssetsError::TooMany
    ));

    let mut crc = good.clone();
    crc[HEADER_LEN] ^= 0x01;
    assert!(matches!(
        parse_err(&crc, "flipped bit"),
        AssetsError::BadCrc
    ));

    // A directory cut in the middle of its table (count says 2, bytes hold less).
    let e = match assets::parse(&good[..HEADER_LEN + ENTRY_LEN + 10], good.len() as u32) {
        Err(e) => e,
        Ok(_) => panic!("a truncated table parsed"),
    };
    assert!(matches!(e, AssetsError::BadLayout), "{}", e.reason());
}

#[test]
fn a_bad_entry_drops_the_whole_directory() {
    let good = image(&[("doom1.wad", b"xyz"), ("b", b"q")]);

    let mut unaligned = good.clone();
    put_u32(&mut unaligned, offset_field(0), 4097);
    resign(&mut unaligned);
    assert!(matches!(
        parse_err(&unaligned, "unaligned body"),
        AssetsError::BadLayout
    ));

    let mut in_directory = good.clone();
    put_u32(&mut in_directory, offset_field(0), 0);
    resign(&mut in_directory);
    assert!(matches!(
        parse_err(&in_directory, "body inside the directory sector"),
        AssetsError::BadLayout
    ));

    let mut past_slot = good.clone();
    put_u32(&mut past_slot, offset_field(1) + 4, good.len() as u32);
    resign(&mut past_slot);
    assert!(matches!(
        parse_err(&past_slot, "body past the slot"),
        AssetsError::BadLayout
    ));

    let mut overflow = good.clone();
    put_u32(&mut overflow, offset_field(1) + 4, u32::MAX);
    resign(&mut overflow);
    assert!(matches!(
        parse_err(&overflow, "offset + len overflows"),
        AssetsError::BadLayout
    ));

    let mut overlapping = good.clone();
    put_u32(&mut overlapping, offset_field(1), 4096);
    resign(&mut overlapping);
    assert!(matches!(
        parse_err(&overlapping, "second body on the first"),
        AssetsError::BadLayout
    ));

    let mut uppercase = good.clone();
    uppercase[HEADER_LEN] = b'D';
    resign(&mut uppercase);
    assert!(matches!(
        parse_err(&uppercase, "uppercase name"),
        AssetsError::BadName
    ));

    let mut slash = good.clone();
    slash[HEADER_LEN + 4] = b'/';
    resign(&mut slash);
    assert!(matches!(
        parse_err(&slash, "slash in name"),
        AssetsError::BadName
    ));

    // "doom1" NUL "wad": a byte after the terminator must be NUL too.
    let mut padding = good.clone();
    padding[HEADER_LEN + 5] = 0;
    resign(&mut padding);
    assert!(matches!(
        parse_err(&padding, "non-NUL padding"),
        AssetsError::BadName
    ));

    let mut duplicate = good.clone();
    duplicate[HEADER_LEN + ENTRY_LEN..HEADER_LEN + ENTRY_LEN + MAX_NAME]
        .copy_from_slice(&good[HEADER_LEN..HEADER_LEN + MAX_NAME]);
    resign(&mut duplicate);
    assert!(matches!(
        parse_err(&duplicate, "same name twice"),
        AssetsError::DuplicateName
    ));
}

#[test]
fn a_slot_that_is_too_small_for_its_bodies_is_rejected() {
    // The firmware reads only the directory and passes the slot length it
    // has. A directory copied from a bigger slot must not be believed.
    let img = image(&[("a", &[0u8; 100])]);
    assert!(assets::parse(&img[..DIR_LEN], img.len() as u32).is_ok());
    let e = match assets::parse(&img[..DIR_LEN], 4096) {
        Err(e) => e,
        Ok(_) => panic!("a body past the slot parsed"),
    };
    assert!(matches!(e, AssetsError::BadLayout), "{}", e.reason());
}

#[test]
fn names_are_limited_to_a_safe_alphabet() {
    for ok in [
        "doom1.wad",
        "a",
        "0",
        "font-12_px.psf",
        &"a".repeat(MAX_NAME),
    ] {
        assert!(assets::valid_name(ok.as_bytes()), "{ok}");
    }
    for bad in [
        "",
        &"a".repeat(MAX_NAME + 1),
        "-a",
        "_a",
        ".a",
        "A",
        "Doom1.WAD",
        "a/b",
        "a b",
        "a\n",
    ] {
        assert!(!assets::valid_name(bad.as_bytes()), "{bad:?}");
    }
}

#[test]
fn at_most_max_entries() {
    let mut dir = Directory::EMPTY;
    for i in 0..MAX_ENTRIES {
        let name = format!("a{i}");
        let at = dir
            .insert(name.as_bytes(), 1, 0)
            .unwrap_or_else(|e| panic!("{name}: {}", e.reason()));
        assert_eq!(at, i);
    }
    assert!(matches!(
        dir.insert(b"one-more", 1, 0),
        Err(AssetsError::TooMany)
    ));
    assert_eq!(dir.len(), MAX_ENTRIES);
    // The writer rejects what the reader would.
    let mut dir = Directory::EMPTY;
    assert!(matches!(
        dir.insert(b"Bad", 1, 0),
        Err(AssetsError::BadName)
    ));
    assert!(matches!(dir.insert(b"a", 1, 0), Ok(0)));
    assert!(matches!(
        dir.insert(b"a", 1, 0),
        Err(AssetsError::DuplicateName)
    ));
    assert!(matches!(
        dir.insert(b"huge", u32::MAX, 0),
        Err(AssetsError::BadLayout)
    ));
    assert_eq!(dir.len(), 1, "a rejected insert changes nothing");
}

#[test]
fn the_boot_line_lists_the_directory_or_the_reason() {
    let describe = |r: Result<Directory, AssetsError>| {
        let mut buf = [0u8; 384];
        let mut out = Buf::new(&mut buf);
        assets::describe(&r, &mut out);
        assert!(!out.overflowed());
        String::from_utf8(out.as_bytes().to_vec()).expect("ASCII")
    };
    let wad = [0u8; 100];
    let font = [0u8; 5000];
    assert_eq!(
        describe(Ok(parsed(&image(&[
            ("doom1.wad", &wad),
            ("font.psf", &font)
        ])))),
        "wasmicon: assets doom1.wad=100 font.psf=5000"
    );
    assert_eq!(describe(Ok(Directory::EMPTY)), "wasmicon: assets none");
    assert_eq!(
        describe(Err(AssetsError::Empty)),
        "wasmicon: assets empty, no assets"
    );
    assert_eq!(
        describe(Err(AssetsError::BadCrc)),
        "wasmicon: assets crc mismatch, no assets"
    );
    // The longest line: every entry with a 32-byte name and a 10-digit length.
    let mut dir = Directory::EMPTY;
    for i in 0..MAX_ENTRIES {
        let name = format!("{i}").repeat(MAX_NAME);
        dir.insert(name.as_bytes(), 500_000_000, 0)
            .unwrap_or_else(|e| panic!("fits: {}", e.reason()));
    }
    let longest = describe(Ok(dir));
    assert!(longest.len() <= 384, "{} bytes", longest.len());
    // The bound `describe` documents: the prefix, then per entry a space, the
    // name, `=` and up to ten digits.
    assert!("wasmicon: assets".len() + MAX_ENTRIES * (1 + MAX_NAME + 1 + 10) <= 384);
}
