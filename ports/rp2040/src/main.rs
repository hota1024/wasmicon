//! Wasmicon port for the RP2040 (Raspberry Pi Pico WH).
//!
//! The guest `.wasm` is read from the app slot in flash and handed to the
//! runtime as a slice of XIP memory; it is not copied to RAM
//! (docs/design-notes.md §4). The firmware embeds no app
//! (docs/app-workflow.md §3.3). If the slot is empty, print the reason and
//! park (`wasmicon_port::idle`; pins are not driven).
//! Traces go to UART0 (GP0=TX, GP1=RX) at 115200 8N1.
//!
//! Not an evaluation board (README.md): never run on hardware, and I2C / SPI
//! stay `unsupported`. CI only checks that it builds.

#![no_std]
#![no_main]
#![warn(clippy::undocumented_unsafe_blocks)]

mod board;

// On panic the firmware only halts. It cannot print the reason: the serial
// port belongs to the board and the panic handler cannot reach it. Runtime
// failures are caught by `main` and printed to the UART, so a panic here is
// always a bug in the port itself.
use panic_halt as _;
use rp2040_hal as hal;
use rp2040_hal::Clock;
use rp2040_hal::fugit::RateExtU32;
use wasmicon_core::{Arena, Config, Exec, decode, instantiate, invoke, validate};
use wasmicon_port::fmt::Buf;
use wasmicon_port::identity::{self, Identity};
use wasmicon_port::roles::{self, Limits, RoleMap};
use wasmicon_port::{Hal, idle, slot};

use board::{PicoBoard, Serial};

/// Second-stage bootloader. Must occupy the first 256 bytes of flash.
#[unsafe(link_section = ".boot2")]
#[used]
pub static BOOT2: [u8; 256] = rp2040_boot2::BOOT_LOADER_W25Q080;

/// Crystal oscillator of the Pico.
const XTAL_HZ: u32 = 12_000_000;

/// Start of XIP: flash is memory-mapped from here.
/// On the RP2040 the first 256 bytes hold the second-stage bootloader (`.boot2`).
const XIP_BASE: usize = 0x1000_0000;

/// Config slot (wiring table, docs/app-workflow.md §3.9), right after the app slot.
const ROLE_SLOT: wasmicon_port::profile::Slot = match wasmicon_port::profile::RP2040.role_slot {
    Some(s) => s,
    None => panic!("the RP2040 profile has no config slot"),
};

/// Limits on the GPIOs the wiring table may assign.
const LIMITS: Limits<'static> = Limits::of(&wasmicon_port::profile::RP2040);

/// App slot (`profile::RP2040` in `ports/common`).
const SLOT: wasmicon_port::profile::Slot = match wasmicon_port::profile::RP2040.slot {
    Some(s) => s,
    None => panic!("the RP2040 profile has no app slot"),
};

// SAFETY: the symbol is defined by the linker (`.wasmicon_fw_end` in
// `memory.x`) and marks the end of the firmware image. Only its address is
// used, to check at boot that the slots do not overlap the firmware.
unsafe extern "C" {
    static __flash_binary_end: u8;
}

/// Runtime arena; whatever is left becomes linear memory (`Arena::alloc_rest`).
/// The RP2040 has 264 KB of SRAM, so the size fits two pages of linear memory
/// (128 KB).
static mut ARENA: [u8; wasmicon_port::profile::RP2040.arena] =
    [0; wasmicon_port::profile::RP2040.arena];

/// Scratch space used only during validation. Sized for the `MCU_CONFIG` limits.
static mut SCRATCH: [u8; wasmicon_port::profile::RP2040.scratch] =
    [0; wasmicon_port::profile::RP2040.scratch];

/// Limits for the microcontroller. The source of truth is
/// `profile::RP2040.config` in `ports/common`; the CLI reads the same table
/// (docs/app-workflow.md §4.3). With the host defaults the scratch space would
/// be too small.
const MCU_CONFIG: Config = wasmicon_port::profile::RP2040.config;

/// Type of UART0, aliased because it is long.
type Uart0 = hal::uart::UartPeripheral<
    hal::uart::Enabled,
    hal::pac::UART0,
    (
        hal::gpio::Pin<hal::gpio::bank0::Gpio0, hal::gpio::FunctionUart, hal::gpio::PullDown>,
        hal::gpio::Pin<hal::gpio::bank0::Gpio1, hal::gpio::FunctionUart, hal::gpio::PullDown>,
    ),
>;

/// Output to UART0.
struct Uart(Uart0);

impl Serial for Uart {
    fn write(&mut self, bytes: &[u8]) {
        self.0.write_full_blocking(bytes);
    }
}

#[hal::entry]
fn main() -> ! {
    let mut pac = hal::pac::Peripherals::take().unwrap();
    let mut watchdog = hal::Watchdog::new(pac.WATCHDOG);
    let clocks = hal::clocks::init_clocks_and_plls(
        XTAL_HZ,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .ok()
    .unwrap();

    let sio = hal::Sio::new(pac.SIO);
    let pins = hal::gpio::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );

    let uart_pins = (
        pins.gpio0.into_function::<hal::gpio::FunctionUart>(),
        pins.gpio1.into_function::<hal::gpio::FunctionUart>(),
    );
    let uart = hal::uart::UartPeripheral::new(pac.UART0, uart_pins, &mut pac.RESETS)
        .enable(
            hal::uart::UartConfig::new(
                115_200.Hz(),
                hal::uart::DataBits::Eight,
                None,
                hal::uart::StopBits::One,
            ),
            clocks.peripheral_clock.freq(),
        )
        .ok()
        .unwrap();

    // TIMER comes out of boot held in reset, so release it here; rp2040-hal
    // only does that inside Timer::new. Reading TIMELR while it is still in
    // reset bus-faults or reads 0 forever, and sleep never finishes.
    let _timer = hal::Timer::new(pac.TIMER, &mut pac.RESETS, &clocks);

    let mut serial = Uart(uart);
    serial.write(b"wasmicon rp2040\r\n");
    // Identify this firmware in one line (docs/app-workflow.md §3.8): version,
    // git, ABI and configuration, so a captured trace records which build
    // produced it.
    {
        let mut line = [0u8; 256];
        let mut out = Buf::new(&mut line);
        identity::describe(
            &Identity {
                profile: &wasmicon_port::profile::RP2040,
                fw: env!("CARGO_PKG_VERSION"),
                git: env!("WASMICON_GIT"),
                trace: cfg!(feature = "trace"),
            },
            &mut out,
        );
        serial.write(out.as_bytes());
        serial.write(b"\r\n");
    }

    // SAFETY: from here on PicoBoard has exclusive use of SIO / IO_BANK0 /
    // PADS_BANK0 / TIMER. The Pins taken above are only the UART's GP0/GP1,
    // which are reserved and never assigned to a role.
    let board = unsafe { PicoBoard::new(serial) };
    let mut hal = Hal::new(board, cfg!(feature = "trace"));
    // Roles come only from the table in the config slot. The firmware has no
    // default table: an empty or corrupt slot hands out no roles at all
    // (docs/app-workflow.md §3.9).
    let map = pick_roles(hal.board_mut().serial());
    let mut hal = hal.with_roles(map);

    // SAFETY: single core, and no interrupt touches ARENA, so this is the only
    // place that creates a reference to it, and it runs once.
    let arena_buf = unsafe { &mut *core::ptr::addr_of_mut!(ARENA) };
    // SAFETY: single core, and no interrupt touches SCRATCH, so this is the
    // only place that creates a reference to it, and it runs once.
    let scratch_buf = unsafe { &mut *core::ptr::addr_of_mut!(SCRATCH) };

    // The firmware embeds no app (docs/app-workflow.md §3.3). If the slot is
    // empty or corrupt, pick_guest has already printed the reason.
    let Some(guest) = pick_guest(hal.board_mut().serial()) else {
        idle::park(hal.board_mut());
    };
    let outcome = run(&mut hal, guest, arena_buf, scratch_buf);

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
        cortex_m::asm::wfi();
    }
}

/// Reads the wiring table from the config slot and prints the table, or the
/// reason it cannot be read, to the serial port.
fn pick_roles(serial: &mut impl Serial) -> RoleMap {
    // `__flash_binary_end` is a linker symbol; only its address is used, never
    // its contents.
    let fw_end = (&raw const __flash_binary_end) as usize;
    // SAFETY: XIP is mapped read-only, and the config slot (4 KiB right after
    // the app slot) lies inside the flash. Overlap with the firmware is
    // rejected by xip_region using fw_end.
    let result = unsafe { roles::read_xip(XIP_BASE, ROLE_SLOT, fw_end, &LIMITS) };
    let mut line = [0u8; 192];
    let mut out = Buf::new(&mut line);
    roles::describe(&result, &mut out);
    serial.write(out.as_bytes());
    serial.write(b"\r\n");
    result.unwrap_or(RoleMap::EMPTY)
}

/// Picks the app to run from the slot.
///
/// Returns `None` when there is nothing to run; the firmware embeds no app
/// (docs/app-workflow.md §3.3). The reason the slot cannot be read goes to
/// the serial port (docs/app-workflow.md §3.1); an empty slot is not a
/// failure, so that is printed as well. Reading is `slot::read_xip` in
/// `ports/common`, the same as on the RP2350.
fn pick_guest(serial: &mut impl Serial) -> Option<&'static [u8]> {
    // `__flash_binary_end` is a linker symbol; only its address is used, never
    // its contents.
    let fw_end = (&raw const __flash_binary_end) as usize;

    let mut line = [0u8; 96];
    let mut out = Buf::new(&mut line);
    // SAFETY: XIP is mapped read-only, and offset + len (1 MiB + 64 KiB) lies
    // inside the 2 MB flash. Overlap with the firmware is rejected by read_xip
    // using fw_end.
    match unsafe { slot::read_xip(XIP_BASE, SLOT, fw_end) } {
        Ok(wasm) => {
            out.str("wasmicon: slot ");
            out.u32(wasm.len() as u32);
            out.str(" B crc32=");
            out.hex(wasmicon_port::crc32(wasm), 8);
            serial.write(out.as_bytes());
            serial.write(b"\r\n");
            Some(wasm)
        }
        Err(e) => {
            out.str("wasmicon: ");
            out.str(e.reason());
            out.str(", idle");
            serial.write(out.as_bytes());
            serial.write(b"\r\n");
            None
        }
    }
}

/// From decoding to the `run` call.
fn run(
    hal: &mut Hal<PicoBoard<Uart>>,
    guest: &[u8],
    arena_buf: &'static mut [u8],
    scratch_buf: &'static mut [u8],
) -> Result<(), wasmicon_core::Error> {
    let mut arena = Arena::new(arena_buf);
    let mut scratch = Arena::new(scratch_buf);

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
    invoke(&mut inst, &mut exec, hal, entry, &[], &mut [])
}
