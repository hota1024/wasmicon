//! Locks in the board profile values.
//!
//! `profile` collects values that used to be scattered across the ports'
//! `main.rs` / `board.rs`. The original numbers are written here as literals,
//! so both a copying mistake and a careless later change fail here. `config`
//! is the validation limit: changing it changes which apps pass.
//!
//! Boards have no default role table; the wiring table is read from the
//! config slot (docs/app-workflow.md §3.9). This file locks in the GPIO
//! constraints (count, reserved pins, bus pins) that the wiring table check
//! uses.

use wasmicon_core::Config;
use wasmicon_port::profile::{self, Profile};

/// The values each `main.rs` had before they were collected (identical across
/// the ports except `max_memory_pages`).
fn assert_mcu_limits(p: &Profile, pages: u32) {
    assert_eq!(p.config.max_value_stack, 512, "{}", p.name);
    assert_eq!(p.config.max_control_depth, 64, "{}", p.name);
    assert_eq!(p.config.max_locals, 256, "{}", p.name);
    assert_eq!(p.config.max_memory_pages, pages, "{}", p.name);
    assert_eq!(p.config.max_table_elems, 256, "{}", p.name);
    assert_eq!(p.config.max_call_depth, 32, "{}", p.name);
    assert_eq!(p.config.operand_stack_slots, 1024, "{}", p.name);
}

#[test]
fn mcu_configs_are_what_the_ports_had() {
    // RP2040 has 264 KB of SRAM, so 2 pages. The others have 4.
    assert_mcu_limits(&profile::RP2040, 2);
    assert_mcu_limits(&profile::RP2350, 4);
    assert_mcu_limits(&profile::ESP32S3, 4);
    assert_mcu_limits(&profile::ESP32C5, 4);
}

#[test]
fn arenas_are_what_the_ports_had() {
    // The ports' `static ARENA` / `static SCRATCH` use these values, so changing
    // them changes the memory layout on the board.
    assert_eq!(profile::RP2040.arena, 160 * 1024);
    assert_eq!(profile::RP2350.arena, 320 * 1024);
    assert_eq!(profile::ESP32S3.arena, 300 * 1024);
    assert_eq!(profile::ESP32C5.arena, 288 * 1024);
    for p in profile::PROFILES {
        assert_eq!(p.scratch, if p.name == "host" { 4 << 20 } else { 8 * 1024 });
    }
}

#[test]
fn each_board_arena_holds_the_pages_it_promises() {
    // Satisfying `max_memory_pages` is not enough. decode / validate / Exec and
    // linear memory all come out of the arena, so if the page limit does not
    // physically fit, the board's Config is a lie.
    for p in profile::PROFILES {
        if p.name == "host" {
            continue; // see the test below
        }
        let promised = p.config.max_memory_pages as usize * 64 * 1024;
        // `<=` is not enough: decode / validate / Exec take from the arena first,
        // so an exact fit always fails in instantiate. A few KB are needed in
        // practice; 32 KiB are kept free (the tightest case is the ESP32-S3 with
        // 4 pages in 300 KiB, leaving 44 KiB).
        let headroom = p.arena - promised;
        assert!(
            promised < p.arena && headroom >= 32 * 1024,
            "{}: {} pages ({promised} B) in an arena of {} B leave only {headroom} B",
            p.name,
            p.config.max_memory_pages,
            p.arena
        );
    }
}

#[test]
fn host_promises_more_pages_than_its_arena_can_hold() {
    // The host alone does not satisfy this invariant. `Config::DEFAULT` is the
    // host PC default and claims 65536 pages (4 GiB), but the arena is only
    // 16 MiB; a guest with more than 256 pages fails in instantiate on the host
    // too.
    //
    // Not something to fix. It is recorded as one more reason why `wasmicon
    // run` and `check --board host` say nothing about a board: besides the
    // loose page limit, the arena is looser than on a board as well.
    let p = &profile::HOST;
    let promised = p.config.max_memory_pages as usize * 64 * 1024;
    assert!(
        promised > p.arena,
        "once the host is as tight as a board, move it into the test above"
    );
    // In practice up to 256 pages fit.
    assert_eq!(p.arena / (64 * 1024), 256);
}

#[test]
fn every_board_has_a_decided_slot() {
    // All four ports use the same location (64 KiB at 1 MiB). Only the read
    // method differs (an XIP slice on the Pico family, a copy to RAM on
    // ESP32-S3 / C5).
    for (p, flash) in [
        (&profile::RP2040, 2usize << 20),
        (&profile::RP2350, 4usize << 20),
        (&profile::ESP32S3, 8usize << 20),
        (&profile::ESP32C5, 16usize << 20),
    ] {
        let sl = p
            .slot
            .unwrap_or_else(|| panic!("{} has a decided slot", p.name));
        assert_eq!(
            sl.offset,
            1 << 20,
            "{}: the first 1 MiB is kept for the firmware",
            p.name
        );
        assert_eq!(sl.len, 64 * 1024, "{}", p.name);
        assert!(
            (sl.offset + sl.len) as usize <= flash,
            "{}: does not fit in {flash} B of flash",
            p.name
        );
    }
}

#[test]
fn the_host_mock_has_no_slot() {
    // The mock has no flash.
    assert!(profile::HOST.slot.is_none());
}

#[test]
fn host_config_is_the_runtime_default() {
    // The host is deliberately loose. Check that it equals `Config::DEFAULT`
    // (if the profile grew its own values, `wasmicon run` and `check --board
    // host` would disagree).
    let d = Config::default();
    let h = profile::HOST.config;
    assert_eq!(h.max_value_stack, d.max_value_stack);
    assert_eq!(h.max_control_depth, d.max_control_depth);
    assert_eq!(h.max_locals, d.max_locals);
    assert_eq!(h.max_memory_pages, d.max_memory_pages);
    assert_eq!(h.max_table_elems, d.max_table_elems);
    assert_eq!(h.max_call_depth, d.max_call_depth);
    assert_eq!(h.operand_stack_slots, d.operand_stack_slots);
}

#[test]
fn host_is_looser_than_every_board() {
    // As long as this holds, "passing on the host does not mean passing on a
    // board" is true, which is why `wasmicon check --board <board>` exists
    // (docs/app-workflow.md §4.3).
    for p in profile::PROFILES {
        if p.name == "host" {
            continue;
        }
        assert!(
            profile::HOST.config.max_memory_pages > p.config.max_memory_pages,
            "host is not looser than board {}",
            p.name
        );
    }
}

#[test]
fn only_the_host_mock_has_a_role_table() {
    // Boards have no default table. The host mock has no flash, so its table is
    // in the profile (used by `wasmicon run` and the tests).
    for p in [
        &profile::RP2040,
        &profile::RP2350,
        &profile::ESP32S3,
        &profile::ESP32C5,
    ] {
        assert!(p.roles.is_empty(), "{} must have no default table", p.name);
    }
    assert_eq!(
        profile::HOST.roles,
        [("led", 2), ("lcd-cs", 10), ("lcd-dc", 11), ("lcd-rst", 12)]
    );
}

#[test]
fn gpio_limits_are_what_the_ports_have() {
    // Each port's `board.rs` cross-checks the same values at compile time.
    let pico_reserved = [0, 1, 23, 24, 25, 29];
    for p in [&profile::RP2040, &profile::RP2350] {
        assert_eq!(p.gpio_count, 30, "{}", p.name);
        assert_eq!(p.reserved, pico_reserved, "{}", p.name);
        assert_eq!(p.bus_pins, [4, 5, 16, 18, 19], "{}", p.name);
    }
    assert_eq!(profile::ESP32S3.gpio_count, 49);
    assert_eq!(
        profile::ESP32S3.reserved,
        [
            22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 43, 44
        ]
    );
    assert_eq!(profile::ESP32S3.bus_pins, [8, 9, 11, 12, 13, 48]);
    assert_eq!(profile::ESP32C5.gpio_count, 29);
    assert_eq!(
        profile::ESP32C5.reserved,
        [11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22]
    );
    assert_eq!(profile::ESP32C5.bus_pins, [5, 8, 9, 23, 24, 27]);
}

#[test]
fn rp2040_declares_i2c_and_spi_unimplemented() {
    // `spi_open` / `i2c_open` return `unsupported` (docs/TODO.md §1.2). Without
    // this flag the static check for rp2040 would pass (docs/app-workflow.md §4.3).
    let i = profile::RP2040.interfaces;
    assert!(!i.i2c, "rp2040 I2C is not implemented");
    assert!(!i.spi, "rp2040 SPI is not implemented");
    assert!(i.gpio && i.time && i.log && i.board && i.assets);

    for p in [
        &profile::RP2350,
        &profile::ESP32S3,
        &profile::ESP32C5,
        &profile::HOST,
    ] {
        let i = p.interfaces;
        assert!(
            i.gpio && i.i2c && i.spi && i.time && i.log && i.board && i.assets,
            "{} implements every interface",
            p.name
        );
    }
}

#[test]
fn every_board_has_an_assets_slot_to_the_end_of_flash() {
    // Right after the config slot (docs/app-workflow.md §3.10), so it never
    // moves when the app is replaced, and as large as the flash allows. The
    // flash sizes are the boards' (docs/TODO.md §5-2).
    for (p, flash) in [
        (&profile::RP2040, 2u32 << 20),
        (&profile::RP2350, 4 << 20),
        (&profile::ESP32S3, 8 << 20),
        (&profile::ESP32C5, 16 << 20),
    ] {
        let r = p
            .role_slot
            .unwrap_or_else(|| panic!("{} has a config slot", p.name));
        let a = p
            .assets
            .unwrap_or_else(|| panic!("{} has an assets slot", p.name));
        assert_eq!(a.offset, r.offset + r.len, "{}", p.name);
        assert_eq!(a.offset, 0x11_1000, "{}", p.name);
        assert_eq!(a.offset % 4096, 0, "{}: erase-unit aligned", p.name);
        assert_eq!(a.offset + a.len, flash, "{}: runs to the end", p.name);
        assert!(a.read == r.read && a.flasher == r.flasher, "{}", p.name);
    }
    assert!(profile::HOST.assets.is_none(), "the mock reads files");
}

#[test]
fn by_name_resolves_every_profile_and_nothing_else() {
    for p in profile::PROFILES {
        let found = profile::by_name(p.name).expect("not found by name");
        assert_eq!(found.name, p.name);
    }
    assert!(
        profile::by_name("rp2350 ").is_none(),
        "lookup is an exact match"
    );
    assert!(profile::by_name("esp32p4").is_none());
}
