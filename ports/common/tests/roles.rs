//! Locks in the config slot (wiring table) format and checks (docs/app-workflow.md §3.9).
//!
//! The writer (CLI) and the reader (firmware) share `roles`, so this is the
//! contract for both. Bump `FORMAT_VERSION` when the shape changes. Version 2
//! adds the display entry (docs/device-layer.md §5.2).

use wasmicon_port::fmt::Buf;
use wasmicon_port::profile;
use wasmicon_port::roles::{self, DisplayConfig, Limits, RoleMap, RolesError, Wiring};

/// Pico 2 W constraints (GP0/1 are the UART, GP4/5 and 16/18/19 are buses).
fn pico() -> Limits<'static> {
    Limits::of(&profile::RP2350)
}

/// Builds the config slot image (header + body) from a table, as the CLI does.
fn image(map: &RoleMap) -> Vec<u8> {
    image_of(&Wiring {
        roles: *map,
        display: None,
    })
}

fn image_of(w: &Wiring) -> Vec<u8> {
    let mut body = [0u8; roles::MAX_BODY];
    let n = roles::encode_body(w, &mut body);
    let mut v = roles::header(&body[..n], w.version()).to_vec();
    v.extend_from_slice(&body[..n]);
    v
}

/// The ILI9341 of apps/lcd-demo-rs on the Pico 2 W.
const PICO_LCD: DisplayConfig = DisplayConfig::Ili9341 {
    spi: 0,
    cs: 17,
    dc: 20,
    rst: 21,
    rotation: 90,
};

fn table(entries: &[(&str, u32)]) -> RoleMap {
    roles::from_table(entries, &pico()).unwrap_or_else(|e| panic!("cannot build: {}", e.reason()))
}

fn parsed(img: &[u8]) -> RoleMap {
    parsed_wiring(img).roles
}

fn parsed_wiring(img: &[u8]) -> Wiring {
    roles::parse(img, &pico()).unwrap_or_else(|e| panic!("cannot parse: {}", e.reason()))
}

fn parse_err(img: &[u8], what: &str) -> RolesError {
    match roles::parse(img, &pico()) {
        Err(e) => e,
        Ok(_) => panic!("expected {what} but it parsed"),
    }
}

#[test]
fn a_written_table_reads_back() {
    let map = table(&[("lcd-cs", 17), ("lcd-dc", 20), ("lcd-rst", 21)]);
    let back = parsed(&image(&map));
    assert_eq!(back.len(), 3);
    assert_eq!(back.get(b"lcd-cs"), Some(17));
    assert_eq!(back.get(b"lcd-dc"), Some(20));
    assert_eq!(back.get(b"lcd-rst"), Some(21));
    assert_eq!(
        back.get(b"led"),
        None,
        "a role that was not written is absent"
    );
}

#[test]
fn the_body_is_human_readable() {
    // Readable in a flash dump (why docs/app-workflow.md §3.9 chose text).
    let img = image(&table(&[("status-led", 2), ("lcd-cs", 17)]));
    assert_eq!(&img[..4], b"WMCR");
    assert_eq!(&img[roles::HEADER_LEN..], b"status-led=2\nlcd-cs=17\n");
}

#[test]
fn an_erased_slot_is_empty_not_an_error() {
    // Erased (0xff) and never written (0x00) are "empty". The firmware runs
    // without handing out any role (there is no default table).
    for fill in [0xffu8, 0x00] {
        let e = parse_err(&[fill; 4096], "empty");
        assert!(e.is_empty(), "{}", e.reason());
    }
}

#[test]
fn a_corrupted_table_is_rejected_whole() {
    let mut img = image(&table(&[("lcd-cs", 17), ("lcd-dc", 20)]));
    let last = img.len() - 2;
    img[last] ^= 0x01; // corrupt one body character (the "0" of "20")
    assert!(parse_err(&img, "crc mismatch") == RolesError::BadCrc);

    let mut img = image(&table(&[("lcd-cs", 17)]));
    img[0] = b'X';
    assert!(parse_err(&img, "magic mismatch") == RolesError::BadMagic);
}

#[test]
fn a_well_formed_but_invalid_entry_drops_the_whole_table() {
    // The CRC matches but the contents do not fit the board (written for another
    // board, the profile changed, and so on). One bad entry discards everything.
    for (body, want) in [
        (&b"lcd-cs=0\n"[..], RolesError::Reserved), // GP0 is the trace UART
        (b"lcd-cs=4\n", RolesError::BusPin),        // GP4 is I2C0
        (b"lcd-cs=30\n", RolesError::OutOfRange),   // RP2350A ends at GP29
        (b"a=15\nb=15\n", RolesError::DuplicatePin), // two roles on one pin
        (b"a=15\na=17\n", RolesError::DuplicateName), // the same name twice
        (b"LCD=15\n", RolesError::BadName),         // uppercase
        (b"lcd-cs\n", RolesError::BadLine),         // no =
        (b"lcd-cs=x\n", RolesError::BadNumber),     // not a number
        (b"lcd-cs=99999999999\n", RolesError::BadNumber), // overflows u32
    ] {
        let mut img = roles::header(body, 1).to_vec();
        img.extend_from_slice(body);
        let got = parse_err(&img, std::str::from_utf8(body).unwrap_or("?"));
        assert!(
            got == want,
            "{:?}: expected {} but got {}",
            std::str::from_utf8(body),
            want.reason(),
            got.reason()
        );
    }
}

#[test]
fn names_are_limited_to_a_safe_alphabet() {
    assert!(roles::valid_name(b"status-led"));
    assert!(roles::valid_name(b"a1"));
    assert!(roles::valid_name(b"abcdefghijklmnop"), "16 characters pass");
    assert!(
        !roles::valid_name(b"abcdefghijklmnopq"),
        "17 characters are rejected"
    );
    assert!(!roles::valid_name(b""));
    assert!(!roles::valid_name(b"1led"), "must not start with a digit");
    assert!(!roles::valid_name(b"lcd_cs"));
    assert!(
        !roles::valid_name(b"lcd=cs"),
        "collides with the body separator"
    );
}

#[test]
fn at_most_max_roles_entries() {
    let limits = Limits {
        gpio_count: 64,
        reserved: &[],
        bus_pins: &[],
    };
    let mut map = RoleMap::EMPTY;
    for i in 0..roles::MAX_ROLES as u32 {
        let name = format!("r{i}");
        assert!(map.insert(name.as_bytes(), i, &limits).is_ok());
    }
    assert!(map.insert(b"one-more", 63, &limits) == Err(RolesError::TooMany));
    // Even at the limit the body fits in MAX_BODY.
    let mut big = RoleMap::EMPTY;
    for i in 0..roles::MAX_ROLES as u32 {
        let name = format!("abcdefghijklmn{i:02}");
        assert!(big.insert(name.as_bytes(), i + 40, &limits).is_ok());
    }
    // Even with the longest display entry next to them.
    let w = Wiring {
        roles: big,
        display: Some(DisplayConfig::Ili9341 {
            spi: 1,
            cs: 60,
            dc: 61,
            rst: 62,
            rotation: 270,
        }),
    };
    let mut body = [0u8; roles::MAX_BODY];
    let n = roles::encode_body(&w, &mut body);
    assert!(n < roles::MAX_BODY, "{n} bytes");
    let back = roles::parse_body(&body[..n], 2, &limits)
        .unwrap_or_else(|e| panic!("cannot parse: {}", e.reason()));
    assert_eq!(back.roles.len(), roles::MAX_ROLES);
    assert_eq!(back.display, w.display);
}

#[test]
fn the_boot_line_lists_the_table_or_the_reason() {
    let describe = |r: Result<RoleMap, RolesError>| {
        let r = r.map(|roles| Wiring {
            roles,
            display: None,
        });
        let mut line = [0u8; 256];
        let mut out = Buf::new(&mut line);
        roles::describe(&r, &mut out);
        String::from_utf8(out.as_bytes().to_vec()).expect("ASCII")
    };
    assert_eq!(
        describe(Ok(table(&[("lcd-cs", 17), ("lcd-dc", 20)]))),
        "wasmicon: roles lcd-cs=17 lcd-dc=20"
    );
    assert_eq!(describe(Ok(RoleMap::EMPTY)), "wasmicon: roles none");
    assert_eq!(
        describe(Err(RolesError::Empty)),
        "wasmicon: roles empty, no roles"
    );
}

#[test]
fn every_profile_puts_the_roles_slot_right_after_the_app_slot() {
    // deploy writes the app slot and the config slot as one image (pack::with_roles).
    for p in profile::PROFILES {
        match (p.slot, p.role_slot) {
            (Some(s), Some(r)) => {
                assert_eq!(r.offset, s.offset + s.len, "{}", p.name);
                assert_eq!(r.len, roles::SLOT_LEN, "{}", p.name);
            }
            (None, None) => {}
            _ => panic!("{}: app slot and config slot go together", p.name),
        }
        // Boards have no default table (only the host mock has one).
        assert_eq!(p.roles.is_empty(), p.name != "host", "{}", p.name);
    }
}

#[test]
fn a_table_without_a_display_stays_version_1() {
    // A firmware that reads only version 1 keeps reading every app that has
    // no display entry (docs/TODO.md §6-5).
    let img = image(&table(&[("lcd-cs", 17)]));
    assert_eq!(u16::from_le_bytes([img[4], img[5]]), 1);
}

#[test]
fn a_display_entry_reads_back_and_makes_version_2() {
    let w = Wiring {
        roles: table(&[("led", 15)]),
        display: Some(PICO_LCD),
    };
    let img = image_of(&w);
    assert_eq!(u16::from_le_bytes([img[4], img[5]]), 2);
    assert_eq!(
        &img[roles::HEADER_LEN..],
        b"led=15\n@display=ili9341 spi=0 cs=17 dc=20 rst=21 rotation=90\n"
    );
    let back = parsed_wiring(&img);
    assert_eq!(back.roles.get(b"led"), Some(15));
    assert_eq!(back.display, Some(PICO_LCD));

    let tab5 = Wiring {
        roles: RoleMap::EMPTY,
        display: Some(DisplayConfig::Tab5),
    };
    let limits = Limits::of(&profile::ESP32P4);
    let img = image_of(&tab5);
    assert_eq!(&img[roles::HEADER_LEN..], b"@display=tab5\n");
    let back = roles::parse(&img, &limits).unwrap_or_else(|e| panic!("{}", e.reason()));
    assert_eq!(back.display, Some(DisplayConfig::Tab5));
}

#[test]
fn a_bad_display_entry_drops_the_whole_table() {
    for (version, body, want) in [
        // Version 1 has no display entry.
        (1u16, &b"@display=tab5\n"[..], RolesError::BadDisplay),
        (2, b"@display=st7789\n", RolesError::BadDisplay),
        (2, b"@display=tab5\n@display=tab5\n", RolesError::BadDisplay),
        (2, b"@display=tab5 spi=0\n", RolesError::BadDisplay),
        // A key missing, unknown, repeated, or out of range.
        (
            2,
            b"@display=ili9341 spi=0 cs=17 dc=20 rst=21\n",
            RolesError::BadDisplay,
        ),
        (
            2,
            b"@display=ili9341 spi=0 cs=17 dc=20 rst=21 rotation=90 x=1\n",
            RolesError::BadDisplay,
        ),
        (
            2,
            b"@display=ili9341 spi=0 spi=0 cs=17 dc=20 rst=21 rotation=90\n",
            RolesError::BadDisplay,
        ),
        (
            2,
            b"@display=ili9341 spi=2 cs=17 dc=20 rst=21 rotation=90\n",
            RolesError::BadDisplay,
        ),
        (
            2,
            b"@display=ili9341 spi=0 cs=17 dc=20 rst=21 rotation=45\n",
            RolesError::BadDisplay,
        ),
        // The pins get the role checks, and may not be shared with a role
        // (whichever line comes first).
        (
            2,
            b"@display=ili9341 spi=0 cs=0 dc=20 rst=21 rotation=90\n",
            RolesError::Reserved,
        ),
        (
            2,
            b"@display=ili9341 spi=0 cs=18 dc=20 rst=21 rotation=90\n",
            RolesError::BusPin,
        ),
        (
            2,
            b"@display=ili9341 spi=0 cs=17 dc=17 rst=21 rotation=90\n",
            RolesError::DuplicatePin,
        ),
        (
            2,
            b"@display=ili9341 spi=0 cs=17 dc=20 rst=21 rotation=90\nled=20\n",
            RolesError::DuplicatePin,
        ),
        (
            2,
            b"led=21\n@display=ili9341 spi=0 cs=17 dc=20 rst=21 rotation=90\n",
            RolesError::DuplicatePin,
        ),
    ] {
        let mut img = roles::header(body, version).to_vec();
        img.extend_from_slice(body);
        let got = parse_err(&img, std::str::from_utf8(body).unwrap_or("?"));
        assert!(
            got == want,
            "{:?}: expected {} but got {}",
            std::str::from_utf8(body),
            want.reason(),
            got.reason()
        );
    }
}

#[test]
fn a_newer_version_is_refused() {
    let mut img = image(&table(&[("lcd-cs", 17)]));
    img[4] = 3;
    assert!(parse_err(&img, "version 3") == RolesError::UnsupportedVersion);
}

#[test]
fn the_boot_line_shows_the_display_entry() {
    let mut line = [0u8; 256];
    let mut out = Buf::new(&mut line);
    roles::describe(
        &Ok(Wiring {
            roles: RoleMap::EMPTY,
            display: Some(PICO_LCD),
        }),
        &mut out,
    );
    assert_eq!(
        out.as_bytes(),
        b"wasmicon: roles none display=ili9341 spi=0 cs=17 dc=20 rst=21 rotation=90"
    );
}
