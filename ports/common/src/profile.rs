//! Board profiles: runtime limits, implemented interfaces, GPIO constraints
//! and slot locations in one place (docs/app-workflow.md §4.3).
//!
//! The values were moved here unchanged from the ports' `main.rs` /
//! `board.rs`. `config` is the validation limit, so changing it changes which
//! apps pass; `tests/profiles.rs` locks in the original numbers.
//!
//! Why they are collected here:
//!
//! - So the CLI can read them. `wasmicon check --board rp2040` cannot warn
//!   "passes on the host but fails on the board" without the board's limits
//!   and implementation status. `ports/host` runs with `Config::DEFAULT`
//!   (`max_memory_pages` = 65536), looser than every board.
//! - So the wiring table (role map) can be checked before it is written.
//!   Boards have no default role table (`roles` module,
//!   docs/app-workflow.md §3.9). The CLI checks `[board.<name>.roles]` in
//!   `wasmicon.toml` against `gpio_count` / `reserved` / `bus_pins` here before
//!   writing the config slot, and the firmware repeats the same check
//!   (`roles::Limits`) at boot.
//!
//! Each port keeps its own `NUM_GPIO` / reserved pins and checks at compile
//! time that they match the values here (`board.rs`).

use wasmicon_core::Config;

/// The HAL interfaces this port implements (docs/abi-spec.md §7).
///
/// Unimplemented ones return `unsupported`. Without this, the static check for
/// `ports/rp2040` would pass, since SPI / I2C return `unsupported` only at run
/// time.
#[derive(Clone, Copy)]
pub struct Interfaces {
    pub gpio: bool,
    pub i2c: bool,
    pub spi: bool,
    pub time: bool,
    pub log: bool,
    pub board: bool,
    pub assets: bool,
}

impl Interfaces {
    /// Everything implemented.
    pub const ALL: Interfaces = Interfaces {
        gpio: true,
        i2c: true,
        spi: true,
        time: true,
        log: true,
        board: true,
        assets: true,
    };
}

/// Profile of one board.
pub struct Profile {
    /// Name used with `--board`. Matches the port's directory name.
    pub name: &'static str,
    /// Runtime limits.
    pub config: Config,
    /// Implemented interfaces.
    pub interfaces: Interfaces,
    /// Mock role name -> GPIO number table. Only the host has one.
    ///
    /// Empty on boards: `deploy` writes the wiring table to the config slot
    /// (`role_slot`) and the firmware reads it at boot
    /// (docs/app-workflow.md §3.9). The host mock has no flash, so its table
    /// lives here for `wasmicon run` and the tests.
    pub roles: &'static [(&'static str, u32)],
    /// Number of GPIOs, for range-checking the wiring table.
    pub gpio_count: u32,
    /// GPIOs the guest may not open (trace UART, flash, radio chip and so on).
    pub reserved: &'static [u32],
    /// GPIOs the port uses for SPI / I2C. Putting one in the wiring table breaks
    /// the bus silently (docs/app-workflow.md §3.9; they are not in the run-time
    /// `reserved`).
    pub bus_pins: &'static [u32],
    /// Size of the arena handed to the runtime. The port's `static ARENA` uses it.
    ///
    /// Linear memory takes the rest of the arena (`Arena::alloc_rest`), so
    /// satisfying `max_memory_pages` is not enough: if `min_pages * 64 KiB`
    /// does not fit in what is left after decode / validate / Exec /
    /// instantiate, the board fails in `instantiate`. `Config` cannot express
    /// this headroom, so the profile carries it.
    pub arena: usize,
    /// Working memory used only during validation. The port's `static SCRATCH` uses it.
    pub scratch: usize,
    /// Location of the app slot (docs/app-workflow.md §3.3, §3.4). `None` means
    /// the board has no flash (the host mock).
    pub slot: Option<Slot>,
    /// Location of the config slot (wiring table): 4 KiB right after the app slot.
    ///
    /// `deploy` writes the app slot and this one as a single image in one go
    /// (`pack::build`). `None` means no flash (the host).
    pub role_slot: Option<Slot>,
    /// Location of the assets slot (read-only data the developer ships next
    /// to the app; `assets` module, docs/app-workflow.md §3.10): right after
    /// the config slot, to the end of flash.
    ///
    /// Only `wasmicon assets write` writes it, never `deploy`, so the data
    /// survives an app update. Bodies are read by range and never copied to
    /// RAM as a whole, whatever `read` says. `None` means no flash (the host
    /// mock reads files instead).
    pub assets: Option<Slot>,
}

/// Location and size of the app slot, plus how to read and flash it.
///
/// The read and flash methods live here too. Branching on the board name
/// string would let a new port fall into the default branch without a compile
/// error (adding esp32p4 would try to flash with `picotool`, for example).
/// The one place that declares a board must state them.
#[derive(Clone, Copy)]
pub struct Slot {
    /// Offset from the start of flash.
    pub offset: u32,
    /// Reserved size (a multiple of the erase unit), header included.
    pub len: u32,
    /// How the slot is read. Decides whether it consumes arena.
    pub read: SlotRead,
    /// External tool used to write the slot.
    pub flasher: Flasher,
}

/// How the slot is read.
///
/// `wasmicon check` looks at this: on a `Copy` board the app's size comes out
/// of the arena, so the same `.wasm` has less `instantiate` headroom.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SlotRead {
    /// Memory-mapped (XIP) flash. Not copied to RAM, so the arena is not
    /// reduced. RP2040 / RP2350.
    Xip,
    /// Copied to RAM before decoding. The app's size comes out of the arena.
    /// ESP32-S3 (through `esp-storage`; arbitrary XIP offsets are not relied on).
    Copy,
}

/// External tool used to write the slot (one hop; docs/app-workflow.md §3.5).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Flasher {
    /// `picotool load -t bin -o <absolute address>`. Needs BOOTSEL held down.
    Picotool,
    /// `espflash write-bin <offset>`. Resets through DTR/RTS, so no button is
    /// needed, but writing and tracing share one port.
    Espflash,
}

/// Limits shared by the microcontrollers. Only `max_memory_pages` differs.
///
/// With the host defaults (`Config::DEFAULT`) the scratch area would be too small.
const MCU: Config = Config {
    max_value_stack: 512,
    max_control_depth: 64,
    max_locals: 256,
    max_memory_pages: 4,
    max_table_elems: 256,
    max_call_depth: 32,
    operand_stack_slots: 1024,
};

/// Role assignment of the host mock. The numbers need not match the boards
/// (traces are normalized to `role:`, so they still match).
const HOST_ROLES: &[(&str, u32)] = &[("led", 2), ("lcd-cs", 10), ("lcd-dc", 11), ("lcd-rst", 12)];

/// Reserved pins of the Pico / Pico 2 family (RP2040 / RP2350A).
/// GP0/GP1 are the trace UART0; GP23/24/25/29 are the CYW43439 or power circuitry.
const PICO_RESERVED: &[u32] = &[0, 1, 23, 24, 25, 29];

/// Bus pins of the Pico / Pico 2 family. I2C0 = GP4/GP5, SPI0 = GP16/GP18/GP19.
const PICO_BUS_PINS: &[u32] = &[4, 5, 16, 18, 19];

/// Reserved pins of the ESP32-S3. 22..=25 do not exist, 26..=32 are SPI flash /
/// PSRAM, 33..=37 are the octal PSRAM's extra lines, 43/44 are the trace UART0.
const ESP32S3_RESERVED: &[u32] = &[
    22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 43, 44,
];

/// Bus pins of the ESP32-S3. I2C0 = GPIO8/9, SPI2 = GPIO11/12/13, `spi.bus`
/// index 1 = GPIO48 (the on-board RGB LED; SPI2 drives only MOSI there).
const ESP32S3_BUS_PINS: &[u32] = &[8, 9, 11, 12, 13, 48];

/// Reserved pins of the ESP32-C5. 11/12 are the trace UART0 (wired to the
/// CH343), 13/14 are native USB (wired to the Type-C port), 15..=22 are SPI
/// flash / PSRAM.
const ESP32C5_RESERVED: &[u32] = &[11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22];

/// Bus pins of the ESP32-C5. I2C0 = GPIO8/9, SPI2 = GPIO24 (SCK) / 23 (MOSI) /
/// 5 (MISO), `spi.bus` index 1 = GPIO27 (the board's RGB LED; only SPI2 MOSI
/// is driven).
const ESP32C5_BUS_PINS: &[u32] = &[5, 8, 9, 23, 24, 27];

/// The config slot placed right after the app slot.
const fn role_slot_after(app: Slot) -> Slot {
    Slot {
        offset: app.offset + app.len,
        len: crate::roles::SLOT_LEN,
        read: app.read,
        flasher: app.flasher,
    }
}

/// The assets slot placed right after the config slot, to the end of flash
/// (docs/app-workflow.md §3.10). Underflow here is a compile error, so a
/// board whose flash ends before the config slot cannot be declared.
const fn assets_after(role: Slot, flash: u32) -> Slot {
    let offset = role.offset + role.len;
    Slot {
        offset,
        len: flash - offset,
        read: role.read,
        flasher: role.flasher,
    }
}

/// Flash sizes of the evaluation boards. The assets slot runs to the end.
const RP2040_FLASH: u32 = 2 << 20;
const RP2350_FLASH: u32 = 4 << 20;
const ESP32S3_FLASH: u32 = 8 << 20;
const ESP32C5_FLASH: u32 = 16 << 20;

/// App slot of the Pico family (same location on RP2040 and RP2350).
const PICO_SLOT: Slot = Slot {
    offset: 1 << 20,
    len: 64 * 1024,
    read: SlotRead::Xip,
    flasher: Flasher::Picotool,
};

/// Raspberry Pi Pico WH.
pub const RP2040: Profile = Profile {
    name: "rp2040",
    // 264 KB of SRAM. Linear memory is limited to 2 pages (128 KiB).
    config: Config {
        max_memory_pages: 2,
        ..MCU
    },
    // I2C / SPI are not implemented and return `unsupported` (docs/TODO.md §1.2).
    interfaces: Interfaces {
        i2c: false,
        spi: false,
        ..Interfaces::ALL
    },
    roles: &[],
    gpio_count: 30,
    reserved: PICO_RESERVED,
    bus_pins: PICO_BUS_PINS,
    // 160 KB of the 264 KB SRAM: 2 pages (128 KiB) plus the runtime's structures.
    arena: 160 * 1024,
    scratch: 8 * 1024,
    // 2 MB of flash (Pico / Pico W(H)). Same location and read method (XIP)
    // as the RP2350.
    slot: Some(PICO_SLOT),
    role_slot: Some(role_slot_after(PICO_SLOT)),
    assets: Some(assets_after(role_slot_after(PICO_SLOT), RP2040_FLASH)),
};

/// Raspberry Pi Pico 2 / Pico 2 W.
pub const RP2350: Profile = Profile {
    name: "rp2350",
    config: MCU,
    interfaces: Interfaces::ALL,
    roles: &[],
    gpio_count: 30,
    reserved: PICO_RESERVED,
    bus_pins: PICO_BUS_PINS,
    // 320 KB of the 520 KB SRAM. 4 pages (256 KiB) fit.
    arena: 320 * 1024,
    scratch: 8 * 1024,
    // 4 MB of flash (Pico 2 W; docs/TODO.md §1.1). The firmware takes a few
    // hundred KB from the start, so everything from 1 MiB on is free. Boot
    // checks that the slot does not overlap the firmware's end (an overlap
    // would corrupt the firmware itself).
    slot: Some(PICO_SLOT),
    role_slot: Some(role_slot_after(PICO_SLOT)),
    assets: Some(assets_after(role_slot_after(PICO_SLOT), RP2350_FLASH)),
};

/// Freenove ESP32-S3-WROOM board.
pub const ESP32S3: Profile = Profile {
    name: "esp32s3",
    config: MCU,
    interfaces: Interfaces::ALL,
    roles: &[],
    gpio_count: 49,
    reserved: ESP32S3_RESERVED,
    bus_pins: ESP32S3_BUS_PINS,
    // 300 KB of the 512 KB DRAM. More would eat into the native stack
    // (docs/TODO.md §1.4).
    arena: 300 * 1024,
    scratch: 8 * 1024,
    // 8 MB of flash (measured with `espflash board-info`; docs/TODO.md §5-2).
    // Starts at 1 MiB like the Pico family.
    //
    // No `partitions.csv` is needed: the default table lets `factory` extend
    // to the end of flash, but `espflash flash` erases only the app's sectors
    // (docs/verification-report.md §10).
    slot: Some(ESP32S3_SLOT),
    role_slot: Some(role_slot_after(ESP32S3_SLOT)),
    assets: Some(assets_after(role_slot_after(ESP32S3_SLOT), ESP32S3_FLASH)),
};

/// App slot of the ESP32-S3.
const ESP32S3_SLOT: Slot = Slot {
    offset: 1 << 20,
    len: 64 * 1024,
    read: SlotRead::Copy,
    flasher: Flasher::Espflash,
};

/// ESP32-C5 (Waveshare ESP32-C5-WIFI6-KIT with the ESP32-C5-WROOM-1 module).
pub const ESP32C5: Profile = Profile {
    name: "esp32c5",
    config: MCU,
    interfaces: Interfaces::ALL,
    roles: &[],
    gpio_count: 29,
    reserved: ESP32C5_RESERVED,
    bus_pins: ESP32C5_BUS_PINS,
    // The esp-hal linker script leaves only 314 KiB (0x4E5A0) of usable RAM
    // (the 384 KB HP SRAM minus cache and bootloader). The S3's 300 KiB would
    // leave no room for the native stack, so this is 4 pages (256 KiB) plus
    // the 32 KiB minimum headroom. The scratch area goes in the 64 KiB the
    // port reclaims from the bootloader (`ram(reclaimed)`), which leaves about
    // 20 KiB of stack (the S3 gets by with about 17 KB; docs/TODO.md §1.4).
    arena: 288 * 1024,
    scratch: 8 * 1024,
    // 16 MB of flash (N16R4). Starts at 1 MiB like the other ports.
    slot: Some(ESP32C5_SLOT),
    role_slot: Some(role_slot_after(ESP32C5_SLOT)),
    assets: Some(assets_after(role_slot_after(ESP32C5_SLOT), ESP32C5_FLASH)),
};

/// App slot of the ESP32-C5. Copied to RAM like on the S3.
const ESP32C5_SLOT: Slot = Slot {
    offset: 1 << 20,
    len: 64 * 1024,
    read: SlotRead::Copy,
    flasher: Flasher::Espflash,
};

/// The PC mock. Looser than every board, so passing here does not mean passing on a board.
pub const HOST: Profile = Profile {
    name: "host",
    config: Config::DEFAULT,
    interfaces: Interfaces::ALL,
    roles: HOST_ROLES,
    gpio_count: 48,
    reserved: &[],
    bus_pins: &[],
    // A PC, so take plenty.
    arena: 16 << 20,
    scratch: 4 << 20,
    // The mock has no flash.
    slot: None,
    role_slot: None,
    assets: None,
};

/// Lookup table by name. Used by the CLI and `info` (docs/app-workflow.md §3.8).
pub const PROFILES: &[&Profile] = &[&RP2040, &RP2350, &ESP32S3, &ESP32C5, &HOST];

/// Resolves `--board <name>`.
#[must_use]
pub fn by_name(name: &str) -> Option<&'static Profile> {
    let mut i = 0;
    while i < PROFILES.len() {
        if PROFILES[i].name == name {
            return Some(PROFILES[i]);
        }
        i += 1;
    }
    None
}

/// Checks that every role's GPIO number can be opened on the board.
///
/// Moving `ROLES` out of `board.rs` separated the numbers from `NUM_GPIO` /
/// `RESERVED`. An out-of-range or reserved assignment would still succeed in
/// `pin-by-role`, and only the guest's `pin.open` on the board would return
/// `invalid-argument`. Each port calls this at compile time to close that gap.
///
/// The reserved pins are a slice because a `const fn` cannot call a function
/// pointer (`ports/esp32s3` decides by range, so it writes the same check in
/// a const block in its `board.rs`).
///
/// # Panics
/// When a number is `gpio_count` or more, or appears in `reserved`.
pub const fn assert_roles_openable(roles: &[(&str, u32)], gpio_count: u32, reserved: &[u32]) {
    let mut i = 0;
    while i < roles.len() {
        let n = roles[i].1;
        assert!(
            n < gpio_count,
            "role gpio number exceeds the board's gpio count"
        );
        let mut j = 0;
        while j < reserved.len() {
            assert!(
                n != reserved[j],
                "role gpio number is a reserved pin (the guest cannot open it)"
            );
            j += 1;
        }
        i += 1;
    }
}

/// Whether two pin lists are identical (for `const` contexts).
///
/// A port keeps its own reserved pins, while the wiring table check (CLI and
/// firmware) uses the profile's `reserved`. A mismatch would make `pin.open`
/// fail on the board for a table the CLI accepted, so each port's `board.rs`
/// cross-checks them with this at compile time.
#[must_use]
pub const fn same_pins(a: &[u32], b: &[u32]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}
