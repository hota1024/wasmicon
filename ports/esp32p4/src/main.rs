//! Wasmicon port for the ESP32-P4.
//!
//! Same flow as `ports/esp32s3`: the guest `.wasm` is read from the app slot
//! in flash with `esp-storage`, copied into the arena and handed to the
//! runtime. The firmware embeds no app (docs/app-workflow.md §3.3). If the
//! slot is empty, print the reason and park (`wasmicon_port::idle`; pins are
//! not driven).
//! Traces go to UART0 (GPIO37=TX, GPIO38=RX, the chip's default pads) at
//! 115200 8N1. On the M5Stack Tab5 UART0 reaches only the M5-Bus (pins 14 /
//! 13), so a 3.3 V USB serial adapter takes the trace.
//!
//! For silicon before v3.0 only (the Tab5 has v1.0): `pre-v3.x`, `src/mem.rs`
//! and `vendor/esp-sync` make up for esp-hal assuming v3.x, and the build
//! halts on a v3.x chip.
//!
//! Port in progress; status in docs/TODO.md §1.3.2.

#![no_std]
#![no_main]
#![warn(clippy::undocumented_unsafe_blocks)]

mod board;
mod mem;
mod tab5;
mod tab5_dpi;
mod tab5_ili9881c;

use esp_hal::clock::CpuClock;
use esp_hal::uart::{Config as UartConfig, Uart};
use esp_storage::FlashStorage;
use wasmicon_core::{decode, instantiate, invoke, validate, Arena, Config, Exec};
use wasmicon_port::assets::{self, Directory};
use wasmicon_port::fmt::Buf;
use wasmicon_port::identity::{self, Identity};
use wasmicon_port::roles::{self, DisplayConfig, Limits, Wiring};
use wasmicon_port::{idle, slot, Hal};

use board::{EspBoard, Serial};

// Places the app descriptor that the ESP-IDF second-stage bootloader reads
// near the start of flash. It holds the version and build time; the runtime
// does not use it. espflash 4.5 and later refuse an ELF without one (the
// counterpart of `IMAGE_DEF` on the RP2350).
esp_bootloader_esp_idf::esp_app_desc!();

/// Config slot (wiring table, docs/app-workflow.md §3.9), right after the app slot.
const ROLE_SLOT: wasmicon_port::profile::Slot = match wasmicon_port::profile::ESP32P4.role_slot {
    Some(s) => s,
    None => panic!("the ESP32-P4 profile has no config slot"),
};

/// Assets slot (docs/app-workflow.md §3.10), right after the config slot, to
/// the end of flash.
const ASSETS_SLOT: wasmicon_port::profile::Slot = match wasmicon_port::profile::ESP32P4.assets {
    Some(s) => s,
    None => panic!("the ESP32-P4 profile has no assets slot"),
};

/// Limits on the GPIOs the wiring table may assign.
const LIMITS: Limits<'static> = Limits::of(&wasmicon_port::profile::ESP32P4);

/// App slot (`profile::ESP32P4` in `ports/common`).
const SLOT: wasmicon_port::profile::Slot = match wasmicon_port::profile::ESP32P4.slot {
    Some(s) => s,
    None => panic!("the ESP32-P4 profile has no app slot"),
};

/// Runtime arena; whatever is left becomes linear memory (`Arena::alloc_rest`).
/// Sized like the ESP32-S3: four pages of linear memory (256 KB) plus the
/// runtime's structures. PSRAM is not used.
static mut ARENA: [u8; wasmicon_port::profile::ESP32P4.arena] =
    [0; wasmicon_port::profile::ESP32P4.arena];

/// Scratch space used only during validation. Sized for the `MCU_CONFIG` limits.
///
/// Not in the RAM reclaimed from the bootloader (`ram(reclaimed)`) as on the
/// C5: on the P4 that region lies above 0x4FFA0000, which is L2 cache, not
/// RAM, on silicon before v3.0 (`pre-v3.x`).
static mut SCRATCH: [u8; wasmicon_port::profile::ESP32P4.scratch] =
    [0; wasmicon_port::profile::ESP32P4.scratch];

/// Limits for the microcontroller. The source of truth is
/// `profile::ESP32P4.config` in `ports/common`; the CLI reads the same table
/// (docs/app-workflow.md §4.3). With the host defaults the scratch space would
/// be too small.
const MCU_CONFIG: Config = wasmicon_port::profile::ESP32P4.config;

/// Output to UART0.
struct SerialPort<'a>(Uart<'a, esp_hal::Blocking>);

impl Serial for SerialPort<'_> {
    /// Writes everything. `Uart::write` only queues what fits in the 128-byte
    /// TX FIFO and returns the count, so discarding the count silently drops
    /// the tail of any line longer than 128 bytes (the identity line did that
    /// once; trace lines are up to 160 bytes).
    fn write(&mut self, bytes: &[u8]) {
        let mut rest = bytes;
        while !rest.is_empty() {
            match self.0.write(rest) {
                Ok(n) if n > 0 => rest = &rest[n..],
                // An error or 0 bytes: give up. There is nowhere to report a
                // serial failure.
                _ => break,
            }
        }
        let _ = self.0.flush();
    }
}

#[esp_hal::main]
fn main() -> ! {
    // Not `CpuClock::max()`: that is 400 MHz, the v3.x maximum. ESP-IDF caps
    // silicon before v3.0 at 360 MHz unless the chips passed extra
    // qualification (`ESP_FORCE_400MHZ_ON_REV_LESS_V3` in
    // components/esp_system/port/soc/esp32p4/Kconfig.cpu, v5.5.3), and esp-hal
    // has no 360 MHz preset, so this takes the next one down. `time` is not
    // traced, so the clock does not affect the match with recorded traces.
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::_200MHz));

    let uart = Uart::new(p.UART0, UartConfig::default())
        .unwrap()
        .with_tx(p.GPIO37)
        .with_rx(p.GPIO38);
    let mut serial = SerialPort(uart);
    serial.write(b"wasmicon esp32p4\r\n");

    // This build links the pre-ECO5 ROM tables (`pre-v3.x`), so on v3.x
    // silicon ROM calls would land on the wrong functions and return wrong
    // values without crashing. The image header already keeps a v3.x chip
    // from booting it; this catches a bootloader that ignores the maximum.
    // Log and halt (docs/decisions.md A-04).
    let rev = esp_hal::efuse::chip_revision();
    if rev.major >= 3 {
        let mut line = [0u8; 96];
        let mut out = Buf::new(&mut line);
        out.str("wasmicon: this build is for ESP32-P4 before v3.0, the chip is v");
        out.u32(u32::from(rev.major));
        out.str(".");
        out.u32(u32::from(rev.minor));
        out.str(", halt");
        serial.write(out.as_bytes());
        serial.write(b"\r\n");
        loop {
            core::hint::spin_loop();
        }
    }
    // Identify this firmware in one line (docs/app-workflow.md §3.8): version,
    // git, ABI and configuration, so a captured trace records which build
    // produced it.
    {
        let mut line = [0u8; 256];
        let mut out = Buf::new(&mut line);
        identity::describe(
            &Identity {
                profile: &wasmicon_port::profile::ESP32P4,
                fw: env!("CARGO_PKG_VERSION"),
                git: env!("WASMICON_GIT"),
                trace: cfg!(feature = "trace"),
            },
            &mut out,
        );
        serial.write(out.as_bytes());
        serial.write(b"\r\n");
    }

    // The Tab5 panel's parts, taken only if the config slot names it.
    let tab5_parts = tab5::Parts {
        i2c: p.I2C1,
        sda: p.GPIO31,
        scl: p.GPIO32,
        backlight: p.GPIO22,
        dsi: p.MIPI_DSI,
        vdma: p.VDMA_CH0,
        psram: p.PSRAM,
    };

    // The app slot, the config slot and the assets are all read through it.
    // The board owns it: the guest's asset reads happen while the app runs.
    let storage = FlashStorage::new(p.FLASH);

    // SAFETY: from here on EspBoard has exclusive use of GPIO / IO_MUX / SPI2
    // / I2C0 and of the pins assigned to SPI2 and I2C0. The UART holds
    // GPIO37/38, which are reserved and overlap neither a role-assignable GPIO
    // nor the SPI2 pins (GPIO5/18/19) nor the I2C0 pins (GPIO54/53).
    let board = unsafe { EspBoard::new(serial, storage, ASSETS_SLOT.offset) };

    let mut hal = Hal::new(board, cfg!(feature = "trace"));
    // Roles come only from the table in the config slot. The firmware has no
    // default table: an empty or corrupt slot hands out no roles at all
    // (docs/app-workflow.md §3.9).
    let (storage, serial) = hal.board_mut().parts();
    let wiring = pick_roles(storage, serial);
    // The display entry selects the display backend (docs/device-layer.md §5.2).
    let mut hal = hal.with_roles(wiring.roles).with_display(wiring.display);
    if wiring.display == Some(DisplayConfig::Tab5) {
        bring_up_tab5(hal.board_mut(), tab5_parts);
    }
    // Assets come only from the assets slot (docs/app-workflow.md §3.10). The
    // bodies stay in flash; the guest's reads go through `esp-storage` by range.
    let (storage, serial) = hal.board_mut().parts();
    let dir = pick_assets(storage, serial);
    let mut hal = hal.with_assets(dir);

    // SAFETY: only one task ever touches ARENA, so this is the only place
    // that creates a reference to it, and it runs once.
    let arena_buf = unsafe { &mut *core::ptr::addr_of_mut!(ARENA) };
    // SAFETY: only one task ever touches SCRATCH, so this is the only place
    // that creates a reference to it, and it runs once.
    let scratch_buf = unsafe { &mut *core::ptr::addr_of_mut!(SCRATCH) };

    let outcome = run(&mut hal, arena_buf, scratch_buf);

    // The firmware embeds no app (docs/app-workflow.md §3.3). If the slot is
    // empty or corrupt, pick_guest has already printed the reason.
    if let Ok(false) = outcome {
        idle::park(hal.board_mut());
    }

    // Print the reason first. release_all below may wait without bound (the
    // `BSY` wait in `spi_close`, say; docs/TODO.md §2.1), so cleaning up first
    // would leave a hung peripheral with no reason printed, which breaks "log
    // and halt" (docs/decisions.md A-04). docs/abi-spec.md §6.6 lists drop
    // before log, but swapping the order only adds the reason to what can be
    // observed (recorded in docs/TODO.md §2).
    if let Err(e) = outcome {
        let s = hal.board_mut().serial();
        s.write(b"wasmicon: ");
        s.write(e.reason().as_bytes());
        s.write(b" [");
        s.write(e.kind().name().as_bytes());
        s.write(b"]\r\n");
    }

    // docs/abi-spec.md §5.2, §6.6: once `run` returns (trap included), drop
    // every handle still open. Guest destructors never run, so this is the
    // only cleanup. No trace line is written (see `Hal::release_all`).
    hal.release_all();
    loop {
        core::hint::spin_loop();
    }
}

/// Brings the Tab5 panel up before the app runs and says how it went. On
/// failure the board has no display, so the app's `open` returns
/// `unsupported`.
fn bring_up_tab5(board: &mut EspBoard<SerialPort<'static>>, parts: tab5::Parts) {
    let mut line = [0u8; 128];
    let mut out = Buf::new(&mut line);
    out.str("wasmicon: display tab5 ");
    match tab5::Tab5::up(parts) {
        Ok((panel, ldo)) => {
            board.attach_tab5(panel);
            out.str("up, d-phy ldo dref=");
            out.u32(u32::from(ldo.dref));
            out.str(" mul=");
            out.u32(u32::from(ldo.mul));
        }
        Err(e) => {
            out.str("failed: ");
            out.str(e.reason());
            if let tab5::UpError::Init(cmd, at) = e {
                out.str(" (command ");
                out.u32(u32::from(cmd));
                out.str(", ");
                out.str(at);
                out.str(")");
            }
        }
    }
    let s = board.serial();
    s.write(out.as_bytes());
    s.write(b"\r\n");
}

/// Picks the app to run from the slot.
///
/// Returns `None` when there is nothing to run; the firmware embeds no app
/// (docs/app-workflow.md §3.3). As on the ESP32-S3, an arbitrary flash
/// offset cannot be assumed to be mapped into XIP, so unlike the Pico ports
/// (`slot::read_xip`) the app is copied to RAM. Only the length in the header
/// is copied: the slot is 64 KiB but an app is a few KB, and the copy comes
/// from the arena, whose rest becomes linear memory. No static region grows.
fn pick_guest<'a>(
    storage: &mut FlashStorage<'_>,
    arena: &mut Arena<'a>,
    serial: &mut SerialPort<'static>,
) -> Option<&'a [u8]> {
    let mut line = [0u8; 96];
    let mut out = Buf::new(&mut line);

    let mut head = [0u8; slot::HEADER_LEN];
    if storage.read(SLOT.offset, &mut head).is_err() {
        serial.write(b"wasmicon: slot read failed, idle\r\n");
        return None;
    }
    let header = match slot::parse_header(&head) {
        Ok(h) => h,
        Err(e) => {
            out.str("wasmicon: ");
            out.str(e.reason());
            out.str(", idle");
            serial.write(out.as_bytes());
            serial.write(b"\r\n");
            return None;
        }
    };
    if header.len > (SLOT.len as usize - slot::HEADER_LEN) {
        serial.write(b"wasmicon: slot truncated, idle\r\n");
        return None;
    }

    // Check the CRC before taking arena space. `Arena` is a bump allocator
    // with no way to give memory back (`runtime/src/arena.rs` has no reset).
    // Today a corrupt slot only leads to park, so nothing is lost, but once
    // the supervisor loop (docs/TODO.md §5) runs the next app from the same
    // arena, a corrupt slot would shrink the arena for good, and the headroom
    // `wasmicon check` estimates would no longer match the board. So the app
    // is streamed from flash once just to compute the CRC.
    //
    // The read is split into small chunks. The `read` of `esp-storage`
    // allocates a 4 KiB sector buffer on the stack on every call, so a bigger
    // chunk here would not lower the stack peak; the calls are sequential, so
    // there is one peak (docs/TODO.md §1.4).
    let mut chunk = [0u8; 512];
    let mut crc = wasmicon_port::CRC32_INIT;
    let mut left = header.len;
    let mut at = SLOT.offset + slot::HEADER_LEN as u32;
    while left > 0 {
        let n = if left < chunk.len() {
            left
        } else {
            chunk.len()
        };
        if storage.read(at, &mut chunk[..n]).is_err() {
            serial.write(b"wasmicon: slot read failed, idle\r\n");
            return None;
        }
        crc = wasmicon_port::crc32_update(crc, &chunk[..n]);
        at += n as u32;
        left -= n;
    }
    if wasmicon_port::crc32_end(crc) != header.crc {
        serial.write(b"wasmicon: slot crc mismatch, idle\r\n");
        return None;
    }

    // The CRC matched, so take arena space only now. The only failure left
    // is "arena too small".
    let Ok(buf) = arena.alloc_bytes(header.len) else {
        serial.write(b"wasmicon: arena too small for the slot app, idle\r\n");
        return None;
    };
    if storage
        .read(SLOT.offset + slot::HEADER_LEN as u32, buf)
        .is_err()
    {
        serial.write(b"wasmicon: slot read failed, idle\r\n");
        return None;
    }
    // Check the copy again. The CRC above covered the streamed bytes; `buf`
    // is what decode gets. A corrupt second read is caught here (the arena
    // space is already taken, but the app does not run).
    if let Err(e) = slot::verify(buf, &header) {
        out.str("wasmicon: ");
        out.str(e.reason());
        out.str(", idle");
        serial.write(out.as_bytes());
        serial.write(b"\r\n");
        return None;
    }

    out.str("wasmicon: slot ");
    out.u32(header.len as u32);
    out.str(" B crc32=");
    out.hex(header.crc, 8);
    serial.write(out.as_bytes());
    serial.write(b"\r\n");
    Some(buf)
}

/// Reads the wiring table from the config slot and prints the table, or the
/// reason it cannot be read, to the serial port.
///
/// The body is at most `roles::MAX_BODY` (256 bytes), so it is read onto the
/// stack.
fn pick_roles(storage: &mut FlashStorage<'_>, serial: &mut SerialPort<'static>) -> Wiring {
    let result = read_roles(storage);
    let mut line = [0u8; 256];
    let mut out = Buf::new(&mut line);
    roles::describe(&result, &mut out);
    serial.write(out.as_bytes());
    serial.write(b"\r\n");
    result.unwrap_or(Wiring::EMPTY)
}

fn read_roles(storage: &mut FlashStorage<'_>) -> Result<Wiring, roles::RolesError> {
    let mut head = [0u8; roles::HEADER_LEN];
    if storage.read(ROLE_SLOT.offset, &mut head).is_err() {
        // Unreadable counts as empty: fail toward handing out no roles.
        return Err(roles::RolesError::Empty);
    }
    let header = roles::parse_header(&head)?;
    let mut body = [0u8; roles::MAX_BODY];
    let body = &mut body[..header.len];
    if storage
        .read(ROLE_SLOT.offset + roles::HEADER_LEN as u32, body)
        .is_err()
    {
        return Err(roles::RolesError::Empty);
    }
    roles::parse_checked(body, &header, &LIMITS)
}

/// Reads the assets directory from the assets slot and prints it, or the
/// reason it cannot be read, to the serial port (docs/app-workflow.md §3.10).
///
/// Only the directory (`assets::DIR_LEN` bytes) is read here; the bodies
/// stay in flash and the guest's reads go through `esp-storage` by range.
/// With an empty or corrupt slot there are no assets: the guest's `open`
/// answers `not-found`.
fn pick_assets(storage: &mut FlashStorage<'_>, serial: &mut SerialPort<'static>) -> Directory {
    let result = read_assets(storage);
    let mut line = [0u8; 384];
    let mut out = Buf::new(&mut line);
    assets::describe(&result, &mut out);
    serial.write(out.as_bytes());
    serial.write(b"\r\n");
    result.unwrap_or(Directory::EMPTY)
}

fn read_assets(storage: &mut FlashStorage<'_>) -> Result<Directory, assets::AssetsError> {
    let mut dir = [0u8; assets::DIR_LEN];
    if storage.read(ASSETS_SLOT.offset, &mut dir).is_err() {
        // Unreadable counts as empty: fail toward having no assets.
        return Err(assets::AssetsError::Empty);
    }
    assets::parse(&dir, ASSETS_SLOT.len)
}

/// From reading the slot to the `run` call.
///
/// `Ok(false)` when there is no app to run, `Ok(true)` once the app has run
/// and returned.
fn run(
    hal: &mut Hal<EspBoard<SerialPort<'static>>>,
    arena_buf: &'static mut [u8],
    scratch_buf: &'static mut [u8],
) -> Result<bool, wasmicon_core::Error> {
    let mut arena = Arena::new(arena_buf);
    let mut scratch = Arena::new(scratch_buf);

    let guest = {
        let (storage, serial) = hal.board_mut().parts();
        pick_guest(storage, &mut arena, serial)
    };
    let Some(guest) = guest else {
        return Ok(false);
    };
    let m = decode::decode(guest, &mut arena)?;
    let v = validate::validate(&m, &MCU_CONFIG, &mut arena, &mut scratch)?;
    // Exec is allocated before linear memory, which takes the rest of the arena.
    let mut exec = Exec::new(&MCU_CONFIG, &mut arena)?;
    let mut inst = instantiate(m, v, &MCU_CONFIG, &mut arena, hal)?;

    if let Some(start) = inst.module.start {
        invoke(&mut inst, &mut exec, hal, start, &[], &mut [])?;
    }
    let entry = inst
        .export_func("run")
        .ok_or(wasmicon_core::Error::Unlinkable("missing export `run`"))?;
    invoke(&mut inst, &mut exec, hal, entry, &[], &mut [])?;
    Ok(true)
}

/// Halts without printing. docs/decisions.md A-04 asks for "log and halt, no
/// restart", but the serial port belongs to the board and the panic handler
/// cannot reach it. Printing the reason would need the serial handle in a
/// place the handler can reach (a global behind a critical section). Runtime
/// failures are caught by `main` and printed to the UART, so a panic here is
/// always a bug in the port itself.
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
