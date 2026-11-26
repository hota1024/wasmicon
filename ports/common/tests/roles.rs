//! Locks in the config slot (wiring table) format and checks (docs/app-workflow.md §3.9).
//!
//! The writer (CLI) and the reader (firmware) share `roles`, so this is the
//! contract for both. Bump `FORMAT_VERSION` when the shape changes. Version 2
//! adds the display entry (docs/device-layer.md §5.2), version 3 the touch
//! entry (docs/device-layer.md §11.3), version 4 the display `scale`
//! (docs/TODO.md §6-10), version 5 the ST7789 entry, version 6 the bus
//! lines (docs/abi-spec.md §8).

use wasmicon_port::fmt::Buf;
use wasmicon_port::profile;
use wasmicon_port::roles::{
    self, Buses, DisplayConfig, I2cPins, Limits, PinMux, RoleMap, RolesError, SpiPins, TouchConfig,
    Wiring,
};

/// Pico 2 W constraints (GP0/1 are the UART, GP4/5 and 16/18/19 are buses).
fn pico() -> Limits<'static> {
    Limits::of(&profile::RP2350)
}

/// Builds the config slot image (header + body) from a table, as the CLI does.
fn image(map: &RoleMap) -> Vec<u8> {
    image_of(&Wiring {
        roles: *map,
        display: None,
        touch: None,
        ..Wiring::EMPTY
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
    scale: 1,
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
        buses: wasmicon_port::roles::Buses::NONE,
        fixed_bus_pins: &[],
        mux: wasmicon_port::roles::PinMux::Fixed,
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
    // Even with the longest display and touch entries next to them, and the
    // boot line of all of it fits its buffer.
    let w = Wiring {
        roles: big,
        display: Some(DisplayConfig::Ili9341 {
            spi: 1,
            cs: 60,
            dc: 61,
            rst: 62,
            rotation: 270,
            scale: 4,
        }),
        touch: Some(TouchConfig::Xpt2046 {
            cs: 58,
            irq: Some(59),
            x_min: 4095,
            x_max: 4094,
            y_min: 4095,
            y_max: 4094,
        }),
        ..Wiring::EMPTY
    };
    let mut body = [0u8; roles::MAX_BODY];
    // Without the touch entry and the scale the body is version 2, which a
    // firmware from before version 3 reads only up to its own limit of 256
    // bytes.
    let v2 = Wiring {
        touch: None,
        display: Some(DisplayConfig::Ili9341 {
            spi: 1,
            cs: 60,
            dc: 61,
            rst: 62,
            rotation: 270,
            scale: 1,
        }),
        ..w
    };
    assert_eq!(v2.version(), 2);
    let n = roles::encode_body(&v2, &mut body);
    assert!(n < 256, "{n} bytes in version 2");
    // The longest ST7789 entry is shorter: it has two pins more but takes no
    // touch entry (an XPT2046 needs an ILI9341).
    let st7789 = Wiring {
        touch: None,
        display: Some(DisplayConfig::St7789 {
            spi: 1,
            cs: 60,
            dc: 61,
            rst: Some(62),
            bl: Some(63),
            rotation: 270,
            scale: 4,
        }),
        ..w
    };
    assert_eq!(st7789.version(), 5);
    let st7789_len = roles::encode_body(&st7789, &mut body);
    let n = roles::encode_body(&w, &mut body);
    assert!(st7789_len < n, "{st7789_len} bytes with the ST7789");
    assert!(n < roles::MAX_BODY, "{n} bytes");
    assert_eq!(w.version(), 4);
    let back = roles::parse_body(&body[..n], 4, &limits)
        .unwrap_or_else(|e| panic!("cannot parse: {}", e.reason()));
    assert_eq!(back.roles.len(), roles::MAX_ROLES);
    assert_eq!(back.display, w.display);
    assert_eq!(back.touch, w.touch);
    let mut line = [0u8; roles::DESCRIBE_LEN];
    let mut out = Buf::new(&mut line);
    roles::describe(&Ok(back), &mut out);
    let len = out.as_bytes().len();
    assert!(len < roles::DESCRIBE_LEN, "{len} bytes");
    assert!(out.as_bytes().ends_with(b" y-max=4094"));

    // And with both bus lines at two-digit pins on top of all of it.
    let moved = Limits {
        buses: Buses {
            spi0: Some(SpiPins {
                sck: 30,
                mosi: 31,
                miso: 32,
            }),
            i2c0: Some(I2cPins { sda: 33, scl: 34 }),
        },
        mux: PinMux::Matrix,
        ..limits
    };
    let wb = Wiring {
        buses: Buses {
            spi0: Some(SpiPins {
                sck: 50,
                mosi: 51,
                miso: 52,
            }),
            i2c0: Some(I2cPins { sda: 53, scl: 54 }),
        },
        ..w
    };
    assert_eq!(wb.version(), 6);
    let n = roles::encode_body(&wb, &mut body);
    assert!(n < roles::MAX_BODY, "{n} bytes with the bus lines");
    let back = roles::parse_body(&body[..n], 6, &moved)
        .unwrap_or_else(|e| panic!("cannot parse: {}", e.reason()));
    assert_eq!(back.buses, wb.buses);
    let mut line = [0u8; roles::DESCRIBE_LEN];
    let mut out = Buf::new(&mut line);
    roles::describe(&Ok(back), &mut out);
    let len = out.as_bytes().len();
    assert!(len < roles::DESCRIBE_LEN, "{len} bytes with the bus lines");
}

/// The Freenove ESP32-S3 board: SPI2 on 12/11/13, I2C0 on 8/9, GPIO48 on
/// `spi.bus` index 1, any pin through the GPIO matrix.
fn s3() -> Limits<'static> {
    Limits::of(&profile::ESP32S3)
}

/// Parses a body by hand, so the order of the lines is the test's.
fn parse_text(text: &str, limits: &Limits<'_>) -> Result<Wiring, RolesError> {
    roles::parse_body(text.as_bytes(), 6, limits)
}

#[test]
fn a_moved_bus_reads_back_as_version_6() {
    // 35..=37 are octal PSRAM lines on the Freenove board, so take free pins.
    let spi = SpiPins {
        sck: 1,
        mosi: 2,
        miso: 3,
    };
    let w = Wiring {
        roles: roles::from_table(&[("lcd-cs", 10)], &s3())
            .unwrap_or_else(|e| panic!("{}", e.reason())),
        buses: Buses {
            spi0: Some(spi),
            i2c0: Some(I2cPins { sda: 4, scl: 5 }),
        },
        ..Wiring::EMPTY
    };
    assert_eq!(w.version(), 6);
    let img = image_of(&w);
    assert_eq!(u16::from_le_bytes([img[4], img[5]]), 6);
    let body = &img[roles::HEADER_LEN..];
    assert!(
        body.windows(28)
            .any(|l| l == b"@spi0=sck=1 mosi=2 miso=3\n@i".as_slice()),
        "{}",
        String::from_utf8_lossy(body)
    );
    let back = roles::parse(&img, &s3()).unwrap_or_else(|e| panic!("{}", e.reason()));
    assert_eq!(back.buses, w.buses);
    assert_eq!(back.roles.get(b"lcd-cs"), Some(10));
    // The port opens on the moved pins.
    let pins = s3().bus_pins(&back);
    assert_eq!(pins.spi0(), Some(spi));
    assert_eq!(pins.i2c0(), Some(I2cPins { sda: 4, scl: 5 }));
    // The boot line names them.
    let mut line = [0u8; roles::DESCRIBE_LEN];
    let mut out = Buf::new(&mut line);
    roles::describe(&Ok(back), &mut out);
    assert_eq!(
        String::from_utf8_lossy(out.as_bytes()),
        "wasmicon: roles lcd-cs=10 spi0=sck=1 mosi=2 miso=3 i2c0=sda=4 scl=5"
    );
}

#[test]
fn a_table_without_bus_lines_keeps_the_defaults() {
    let w = Wiring::EMPTY;
    assert_ne!(w.version(), 6);
    let pins = s3().bus_pins(&w);
    assert_eq!(
        pins.spi0(),
        Some(SpiPins {
            sck: 12,
            mosi: 11,
            miso: 13
        })
    );
    assert_eq!(pins.i2c0(), Some(I2cPins { sda: 8, scl: 9 }));
}

#[test]
fn roles_are_checked_against_the_bus_pins_in_effect() {
    // Wherever the bus line stands, the roles see the moved bus: a default
    // pin it freed is fine, a pin it took is not.
    let freed = "lcd-cs=12\n@spi0=sck=1 mosi=2 miso=3\n";
    let w = parse_text(freed, &s3()).unwrap_or_else(|e| panic!("{}", e.reason()));
    assert_eq!(w.roles.get(b"lcd-cs"), Some(12));
    for text in [
        "lcd-cs=1\n@spi0=sck=1 mosi=2 miso=3\n",
        "@spi0=sck=1 mosi=2 miso=3\nlcd-cs=1\n",
    ] {
        assert!(
            parse_text(text, &s3()).err() == Some(RolesError::BusPin),
            "{text}"
        );
    }
    // So are the display's pins.
    let lcd = "@spi0=sck=1 mosi=2 miso=3\n@display=ili9341 spi=0 cs=1 dc=14 rst=15 rotation=90\n";
    assert!(parse_text(lcd, &s3()).err() == Some(RolesError::BusPin));
    // Without the line the default pins stay bus pins.
    assert!(parse_text("lcd-cs=12\n", &s3()).err() == Some(RolesError::BusPin));
}

#[test]
fn a_bad_bus_line_discards_the_table() {
    for (what, text) in [
        ("out of range", "@spi0=sck=49 mosi=2 miso=3\n"),
        ("reserved", "@spi0=sck=36 mosi=35 miso=37\n"),
        ("two signals on one pin", "@spi0=sck=1 mosi=1 miso=3\n"),
        ("the LED pin of spi bus 1", "@spi0=sck=48 mosi=2 miso=3\n"),
        ("spi on the default i2c pins", "@spi0=sck=8 mosi=2 miso=3\n"),
        (
            "both buses on one pin",
            "@spi0=sck=1 mosi=2 miso=3\n@i2c0=sda=1 scl=5\n",
        ),
        ("a missing pin", "@spi0=sck=1 mosi=2\n"),
        ("an unknown key", "@spi0=sck=1 mosi=2 miso=3 cs=4\n"),
        ("a repeated key", "@spi0=sck=1 sck=2 mosi=3 miso=4\n"),
        ("a repeated line", "@i2c0=sda=1 scl=2\n@i2c0=sda=1 scl=2\n"),
    ] {
        assert!(parse_text(text, &s3()).is_err(), "{what}");
    }
    // Only version 6 has bus lines: an older version is refused, so a
    // firmware that predates them never runs on the wrong pins.
    assert!(
        roles::parse_body(b"@spi0=sck=1 mosi=2 miso=3\n", 5, &s3()).err()
            == Some(RolesError::BadBus)
    );
}

#[test]
fn the_rp2350_moves_its_buses_only_where_funcsel_reaches() {
    // SPI0: RX on 0/4/16/20, SCK on 2/6/18/22, TX on 3/7/19/23 (GP0/1/23
    // are reserved). I2C0: SDA on every 4n, SCL on every 4n+1.
    let ok = "@spi0=sck=6 mosi=7 miso=20\n@i2c0=sda=8 scl=9\n";
    let w = parse_text(ok, &pico()).unwrap_or_else(|e| panic!("{}", e.reason()));
    assert_eq!(
        pico().bus_pins(&w).spi0(),
        Some(SpiPins {
            sck: 6,
            mosi: 7,
            miso: 20
        })
    );
    for text in [
        // SPI1's pins.
        "@spi0=sck=10 mosi=11 miso=12\n",
        // SCK and TX swapped.
        "@spi0=sck=19 mosi=18 miso=16\n",
        // I2C1's SDA.
        "@i2c0=sda=2 scl=5\n",
        // MISO on GP4 while I2C0 keeps its default SDA there.
        "@spi0=sck=18 mosi=19 miso=4\n",
    ] {
        assert!(
            parse_text(text, &pico()).err() == Some(RolesError::BadBus),
            "{text}"
        );
    }
}

#[test]
fn boards_whose_buses_do_not_move_refuse_a_bus_line() {
    let rp2040 = Limits::of(&profile::RP2040);
    let host = Limits::of(&profile::HOST);
    for limits in [rp2040, host] {
        assert!(parse_text("@i2c0=sda=8 scl=9\n", &limits).err() == Some(RolesError::BadBus));
    }
    // And a refused table leaves the port on its defaults.
    let bad = Wiring {
        buses: Buses {
            spi0: Some(SpiPins {
                sck: 36,
                mosi: 35,
                miso: 37,
            }),
            i2c0: None,
        },
        ..Wiring::EMPTY
    };
    assert_eq!(s3().bus_pins(&bad), s3().bus_pins(&Wiring::EMPTY));
}

#[test]
fn the_boot_line_lists_the_table_or_the_reason() {
    let describe = |r: Result<RoleMap, RolesError>| {
        let r = r.map(|roles| Wiring {
            roles,
            display: None,
            touch: None,
            ..Wiring::EMPTY
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
    // The CLI's deploy relies on the layout: it fills the gap between the two
    // with 0xff when that makes one espflash call (deploy::MERGE_GAP).
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
        touch: None,
        ..Wiring::EMPTY
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
        display: Some(DisplayConfig::TAB5),
        touch: None,
        ..Wiring::EMPTY
    };
    let limits = Limits::of(&profile::ESP32P4);
    let img = image_of(&tab5);
    assert_eq!(&img[roles::HEADER_LEN..], b"@display=tab5\n");
    let back = roles::parse(&img, &limits).unwrap_or_else(|e| panic!("{}", e.reason()));
    assert_eq!(back.display, Some(DisplayConfig::TAB5));

    // `scale=` is written only when it is not 1, so an entry at 1x stays
    // readable by a firmware that predates it (docs/TODO.md §6-10).
    let tab5_2x = Wiring {
        display: Some(DisplayConfig::Tab5 { scale: 2 }),
        ..tab5
    };
    let img = image_of(&tab5_2x);
    // A scale makes version 4; without it the version stays as it was.
    assert_eq!(tab5.version(), 2);
    assert_eq!(tab5_2x.version(), 4);
    assert_eq!(u16::from_le_bytes([img[4], img[5]]), 4);
    assert_eq!(&img[roles::HEADER_LEN..], b"@display=tab5 scale=2\n");
    let back = roles::parse(&img, &limits).unwrap_or_else(|e| panic!("{}", e.reason()));
    assert_eq!(back.display, Some(DisplayConfig::Tab5 { scale: 2 }));
    let lcd_4x = DisplayConfig::Ili9341 {
        spi: 0,
        cs: 17,
        dc: 20,
        rst: 21,
        rotation: 90,
        scale: 4,
    };
    let img = image_of(&Wiring {
        roles: RoleMap::EMPTY,
        display: Some(lcd_4x),
        touch: None,
        ..Wiring::EMPTY
    });
    assert_eq!(
        &img[roles::HEADER_LEN..],
        b"@display=ili9341 spi=0 cs=17 dc=20 rst=21 rotation=90 scale=4\n"
    );
    assert_eq!(parsed_wiring(&img).display, Some(lcd_4x));
}

#[test]
fn an_st7789_entry_round_trips_at_version_5() {
    // The panel of the Waveshare ESP32-S3-Touch-LCD-2: no reset GPIO, a
    // backlight GPIO.
    let lcd2 = DisplayConfig::St7789 {
        spi: 0,
        cs: 45,
        dc: 42,
        rst: None,
        bl: Some(1),
        rotation: 90,
        scale: 1,
    };
    let w = Wiring {
        roles: RoleMap::EMPTY,
        display: Some(lcd2),
        touch: None,
        ..Wiring::EMPTY
    };
    // A firmware from before the ST7789 refuses the table by its version.
    assert_eq!(w.version(), 5);
    let img = image_of(&w);
    assert_eq!(u16::from_le_bytes([img[4], img[5]]), 5);
    assert_eq!(
        &img[roles::HEADER_LEN..],
        b"@display=st7789 spi=0 cs=45 dc=42 bl=1 rotation=90\n"
    );
    let limits = Limits::of(&profile::ESP32S3_LCD2);
    let back = roles::parse(&img, &limits).unwrap_or_else(|e| panic!("{}", e.reason()));
    assert_eq!(back.display, Some(lcd2));
    assert_eq!(lcd2.pins(), [Some(45), Some(42), None, Some(1)]);
    assert_eq!(lcd2.panel_size(), (320, 240));

    // Every optional key, at 2x.
    let full = DisplayConfig::St7789 {
        spi: 0,
        cs: 17,
        dc: 20,
        rst: Some(21),
        bl: Some(22),
        rotation: 90,
        scale: 2,
    };
    let img = image_of(&Wiring {
        roles: RoleMap::EMPTY,
        display: Some(full),
        touch: None,
        ..Wiring::EMPTY
    });
    assert_eq!(
        &img[roles::HEADER_LEN..],
        b"@display=st7789 spi=0 cs=17 dc=20 rst=21 bl=22 rotation=90 scale=2\n"
    );
    assert_eq!(parsed_wiring(&img).display, Some(full));

    // The board reserves GPIO0 (the BOOT strap, the panel's optional reset)
    // and GPIO41 (the TF card's chip select), and SPI2 is on 38/39/40.
    for (pin, want) in [
        (0, RolesError::Reserved),
        (41, RolesError::Reserved),
        (39, RolesError::BusPin),
    ] {
        let d = DisplayConfig::St7789 {
            spi: 0,
            cs: 45,
            dc: 42,
            rst: Some(pin),
            bl: Some(1),
            rotation: 90,
            scale: 1,
        };
        assert!(d.check(&limits, &RoleMap::EMPTY) == Err(want), "GPIO{pin}");
    }
}

#[test]
fn a_bad_display_entry_drops_the_whole_table() {
    for (version, body, want) in [
        // Version 1 has no display entry.
        (1u16, &b"@display=tab5\n"[..], RolesError::BadDisplay),
        (2, b"@display=st7789\n", RolesError::BadDisplay),
        // An ST7789 entry needs version 5, and spi, cs, dc and rotation.
        (
            4,
            b"@display=st7789 spi=0 cs=17 dc=20 rotation=90\n",
            RolesError::BadDisplay,
        ),
        (
            5,
            b"@display=st7789 spi=0 cs=17 dc=20 bl=22\n",
            RolesError::BadDisplay,
        ),
        (
            5,
            b"@display=st7789 spi=0 cs=17 dc=20 bl=22 bl=22 rotation=90\n",
            RolesError::BadDisplay,
        ),
        // Its optional pins get the same checks as the others.
        (
            5,
            b"@display=st7789 spi=0 cs=17 dc=20 bl=17 rotation=90\n",
            RolesError::DuplicatePin,
        ),
        (
            5,
            b"@display=st7789 spi=0 cs=17 dc=20 rst=1 rotation=90\n",
            RolesError::Reserved,
        ),
        (
            5,
            b"led=22\n@display=st7789 spi=0 cs=17 dc=20 bl=22 rotation=90\n",
            RolesError::DuplicatePin,
        ),
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
        // A scale out of 1 to 4, one that does not divide the panel
        // (320x240, 720x1280), or repeated (docs/TODO.md §6-10).
        (4, b"@display=tab5 scale=0\n", RolesError::BadDisplay),
        (4, b"@display=tab5 scale=3\n", RolesError::BadDisplay),
        (4, b"@display=tab5 scale=8\n", RolesError::BadDisplay),
        (
            4,
            b"@display=tab5 scale=2 scale=2\n",
            RolesError::BadDisplay,
        ),
        // A `scale=` key needs version 4, even at 1.
        (2, b"@display=tab5 scale=2\n", RolesError::BadDisplay),
        (2, b"@display=tab5 scale=1\n", RolesError::BadDisplay),
        (
            3,
            b"@display=tab5 scale=2\n@touch=tab5\n",
            RolesError::BadDisplay,
        ),
        (
            4,
            b"@display=ili9341 spi=0 cs=17 dc=20 rst=21 rotation=90 scale=3\n",
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
    let newer = roles::FORMAT_VERSION + 1;
    img[4..6].copy_from_slice(&newer.to_le_bytes());
    assert!(parse_err(&img, "a newer version") == RolesError::UnsupportedVersion);
}

#[test]
fn the_boot_line_shows_the_display_entry() {
    let mut line = [0u8; 256];
    let mut out = Buf::new(&mut line);
    roles::describe(
        &Ok(Wiring {
            roles: RoleMap::EMPTY,
            display: Some(PICO_LCD),
            touch: None,
            ..Wiring::EMPTY
        }),
        &mut out,
    );
    assert_eq!(
        out.as_bytes(),
        b"wasmicon: roles none display=ili9341 spi=0 cs=17 dc=20 rst=21 rotation=90"
    );
    // A scale shows, so the serial port says what the board read.
    let mut line = [0u8; 256];
    let mut out = Buf::new(&mut line);
    roles::describe(
        &Ok(Wiring {
            roles: RoleMap::EMPTY,
            display: Some(DisplayConfig::Tab5 { scale: 2 }),
            touch: None,
            ..Wiring::EMPTY
        }),
        &mut out,
    );
    assert_eq!(out.as_bytes(), b"wasmicon: roles none display=tab5 scale=2");
}

#[test]
fn a_touch_entry_reads_back_and_makes_version_3() {
    let w = Wiring {
        roles: RoleMap::EMPTY,
        display: Some(DisplayConfig::TAB5),
        touch: Some(TouchConfig::Tab5),
        ..Wiring::EMPTY
    };
    let img = image_of(&w);
    assert_eq!(u16::from_le_bytes([img[4], img[5]]), 3);
    assert_eq!(&img[roles::HEADER_LEN..], b"@display=tab5\n@touch=tab5\n");
    let limits = Limits::of(&profile::ESP32P4);
    let back = roles::parse(&img, &limits).unwrap_or_else(|e| panic!("{}", e.reason()));
    assert_eq!(back.display, Some(DisplayConfig::TAB5));
    assert_eq!(back.touch, Some(TouchConfig::Tab5));

    let mut line = [0u8; 256];
    let mut out = Buf::new(&mut line);
    roles::describe(&Ok(back), &mut out);
    assert_eq!(
        out.as_bytes(),
        b"wasmicon: roles none display=tab5 touch=tab5"
    );
}

#[test]
fn a_bad_touch_entry_drops_the_whole_table() {
    let limits = Limits::of(&profile::ESP32P4);
    for (version, body) in [
        // Versions 1 and 2 have no touch entry.
        (2u16, &b"@display=tab5\n@touch=tab5\n"[..]),
        (1, b"@touch=tab5\n"),
        // A touch panel lies over a display, and `tab5` over the `tab5` one.
        (3, b"@touch=tab5\n"),
        (
            3,
            b"@display=ili9341 spi=0 cs=17 dc=20 rst=21 rotation=90\n@touch=tab5\n",
        ),
        // Unknown, repeated, or with keys `tab5` does not take.
        (3, b"@display=tab5\n@touch=xpt9999\n"),
        (3, b"@display=tab5\n@touch=tab5\n@touch=tab5\n"),
        (3, b"@display=tab5\n@touch=tab5 cs=1\n"),
    ] {
        let mut img = roles::header(body, version).to_vec();
        img.extend_from_slice(body);
        let got = match roles::parse(&img, &limits) {
            Ok(_) => panic!("{:?}: accepted", std::str::from_utf8(body)),
            Err(e) => e,
        };
        assert!(
            got == RolesError::BadTouch,
            "{:?}: expected {} but got {}",
            std::str::from_utf8(body),
            RolesError::BadTouch.reason(),
            got.reason()
        );
    }
    // Its INT pin gets the checks of the display pins, against the roles too.
    for body in [
        &b"led=23\n@display=tab5\n@touch=tab5\n"[..],
        b"@display=tab5\n@touch=tab5\nled=23\n",
    ] {
        let mut img = roles::header(body, 3).to_vec();
        img.extend_from_slice(body);
        assert!(
            roles::parse(&img, &limits).err() == Some(RolesError::DuplicatePin),
            "{:?}",
            std::str::from_utf8(body)
        );
    }
    // The touch line may come before the display line.
    let body = b"@touch=tab5\n@display=tab5\n";
    let mut img = roles::header(body, 3).to_vec();
    img.extend_from_slice(body);
    assert!(roles::parse(&img, &limits).is_ok());
}

/// The XPT2046 of the ILI9341 module on the ESP32-S3 (docs/device-layer.md §11.3).
const S3_TOUCH: TouchConfig = TouchConfig::Xpt2046 {
    cs: 16,
    irq: Some(17),
    x_min: 200,
    x_max: 3900,
    y_min: 3900,
    y_max: 200,
};

const S3_LCD: &[u8] = b"@display=ili9341 spi=0 cs=10 dc=14 rst=15 rotation=90\n";

#[test]
fn an_xpt2046_entry_reads_back() {
    let w = Wiring {
        roles: RoleMap::EMPTY,
        display: Some(DisplayConfig::Ili9341 {
            spi: 0,
            cs: 10,
            dc: 14,
            rst: 15,
            rotation: 90,
            scale: 1,
        }),
        touch: Some(S3_TOUCH),
        ..Wiring::EMPTY
    };
    let img = image_of(&w);
    assert_eq!(u16::from_le_bytes([img[4], img[5]]), 3);
    let mut want = S3_LCD.to_vec();
    want.extend_from_slice(
        b"@touch=xpt2046 cs=16 irq=17 x-min=200 x-max=3900 y-min=3900 y-max=200\n",
    );
    assert_eq!(&img[roles::HEADER_LEN..], &want[..]);
    let limits = Limits::of(&profile::ESP32S3);
    let back = roles::parse(&img, &limits).unwrap_or_else(|e| panic!("{}", e.reason()));
    assert_eq!(back.touch, Some(S3_TOUCH));
    // `irq` is optional.
    let mut body = S3_LCD.to_vec();
    body.extend_from_slice(b"@touch=xpt2046 cs=16 x-min=0 x-max=4095 y-min=0 y-max=4095\n");
    let mut img = roles::header(&body, 3).to_vec();
    img.extend_from_slice(&body);
    let back = roles::parse(&img, &limits).unwrap_or_else(|e| panic!("{}", e.reason()));
    assert!(matches!(
        back.touch,
        Some(TouchConfig::Xpt2046 { irq: None, .. })
    ));
}

#[test]
fn a_bad_xpt2046_entry_drops_the_whole_table() {
    let limits = Limits::of(&profile::ESP32S3);
    let cal = " x-min=200 x-max=3900 y-min=200 y-max=3900";
    for (touch, want) in [
        // A key missing, unknown or repeated; a calibration out of range or flat.
        (
            "cs=16 irq=17 x-min=200 x-max=3900 y-min=200".to_string(),
            RolesError::BadTouch,
        ),
        (format!("cs=16 irq=17{cal} z=1"), RolesError::BadTouch),
        (format!("cs=16 cs=16{cal}"), RolesError::BadTouch),
        (
            "cs=16 x-min=200 x-max=4096 y-min=200 y-max=3900".to_string(),
            RolesError::BadTouch,
        ),
        (
            "cs=16 x-min=200 x-max=200 y-min=200 y-max=3900".to_string(),
            RolesError::BadTouch,
        ),
        // Its pins get the checks of the display pins, against the display too.
        (format!("cs=10{cal}"), RolesError::DuplicatePin),
        (format!("cs=16 irq=16{cal}"), RolesError::DuplicatePin),
        (format!("cs=12{cal}"), RolesError::BusPin),
        (format!("cs=99{cal}"), RolesError::OutOfRange),
    ] {
        let mut body = S3_LCD.to_vec();
        body.extend_from_slice(format!("@touch=xpt2046 {touch}\n").as_bytes());
        let mut img = roles::header(&body, 3).to_vec();
        img.extend_from_slice(&body);
        let got = roles::parse(&img, &limits).err();
        assert!(
            got == Some(want),
            "{touch}: got {:?}",
            got.map(RolesError::reason)
        );
    }
    // An XPT2046 lies over an ILI9341, not over the Tab5's panel.
    let body = format!("@display=tab5\n@touch=xpt2046 cs=16{cal}\n");
    let mut img = roles::header(body.as_bytes(), 3).to_vec();
    img.extend_from_slice(body.as_bytes());
    let limits = Limits::of(&profile::ESP32P4);
    assert!(roles::parse(&img, &limits).err() == Some(RolesError::BadTouch));
}

/// A table written for one app is refused for another (`roles::bind`), so a
/// deploy that wrote the app and failed before the config slot hands the new
/// app no roles instead of the previous app's pins.
#[test]
fn a_table_bound_to_another_app_hands_out_no_roles() {
    let mut img = image(&table(&[("led", 15)]));
    img.extend_from_slice(&roles::binding_bytes(0x1234_5678));

    let ok = roles::parse_bound(&img, &pico(), Some(0x1234_5678))
        .unwrap_or_else(|e| panic!("{}", e.reason()));
    assert_eq!(ok.roles.get(b"led"), Some(15));

    let other = roles::parse_bound(&img, &pico(), Some(0x0bad_0bad)).err();
    assert!(other == Some(RolesError::OtherApp));

    // The boot line names the cause, as for any table that cannot be used.
    let mut line = [0u8; roles::DESCRIBE_LEN];
    let mut out = Buf::new(&mut line);
    roles::describe(&Err(RolesError::OtherApp), &mut out);
    assert_eq!(
        out.as_bytes(),
        b"wasmicon: roles belong to another app, no roles"
    );

    // No app to compare with: the app slot is empty or unreadable, and
    // nothing runs. The table itself is fine.
    assert!(roles::parse_bound(&img, &pico(), None).is_ok());
}

/// The binding sits after the body and outside the header's length, so a
/// firmware that predates it reads the same table (docs/TODO.md §6-5), and a
/// table without one (an older CLI) is read as before.
#[test]
fn the_binding_changes_nothing_for_a_reader_that_does_not_know_it() {
    let plain = image(&table(&[("led", 15)]));
    let mut bound = plain.clone();
    bound.extend_from_slice(&roles::binding_bytes(7));

    // The old reader: `parse` stops at the header's length.
    assert_eq!(parsed(&bound).get(b"led"), Some(15));
    assert_eq!(&bound[..plain.len()], plain.as_slice());

    // No binding: accepted for any app, as before.
    assert!(roles::parse_bound(&plain, &pico(), Some(7)).is_ok());
    assert!(roles::parse_bound(&plain, &pico(), Some(8)).is_ok());
    // Erased flash after the body is no binding either.
    let mut erased = plain.clone();
    erased.extend_from_slice(&[0xff; 8]);
    assert!(roles::binding(&erased[plain.len()..]).is_none());
    assert!(roles::parse_bound(&erased, &pico(), Some(8)).is_ok());
}

/// A bad table stays the error it was; the binding never turns it into a
/// good one.
#[test]
fn the_binding_does_not_hide_a_bad_table() {
    let mut img = image(&table(&[("led", 15)]));
    let last = img.len() - 1;
    img[last] ^= 1;
    img.extend_from_slice(&roles::binding_bytes(7));
    assert!(roles::parse_bound(&img, &pico(), Some(7)).err() == Some(RolesError::BadCrc));
}
