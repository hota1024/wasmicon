//! Shared HAL implementation for the ports.
//!
//! Import resolution (the exact-match link against the generated table), the
//! handle table, status assembly and trace formatting
//! (docs/abi-spec.md §6.4, §5.3, §4.4, §9) must be identical on every port.
//! Traces above all: the slightest difference in format breaks "identical
//! traces on both boards" (docs/decisions.md D-10).
//!
//! Only board-specific operations go through the `Board` trait; this crate is
//! `no_std`.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(clippy::undocumented_unsafe_blocks)]

pub mod assets;
pub mod fmt;
pub mod identity;
pub mod idle;
pub mod profile;
pub mod roles;
pub mod slot;

use wasmicon_core::error::{Error, Result, Trap};
use wasmicon_core::generated::display::PixelFormat;
use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::i2c::Speed;
use wasmicon_core::generated::log::Level as LogLevel;
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_core::generated::{self, ErrorCode, HostFn};
use wasmicon_core::instance::{Extern, ExternType, Resolver};

use assets::{Directory, MAX_ENTRIES};
use fmt::Buf;
use roles::{MAX_ROLES, RoleMap};

/// Number of simultaneous handles docs/abi-spec.md §5.3 guarantees.
pub const MAX_PINS: usize = 16;
pub const MAX_I2C: usize = 2;
pub const MAX_SPI: usize = 2;
pub const MAX_BLOBS: usize = 4;
pub const MAX_SCREENS: usize = 1;

/// Chunk size of the default `Board::assets_crc32`: how much of an asset's
/// body sits on the stack at a time while its CRC-32 is checked. The ESP
/// ports stream the app slot in 512-byte pieces too (`pick_guest`).
pub const ASSETS_CHUNK: usize = 512;

/// Identifier assigned to the `env.abort` that AssemblyScript imports.
/// The one import outside `world app` (docs/decisions.md P-02).
const HOST_ENV_ABORT: u32 = 0xffff;

/// The only import allowed outside `world app` (docs/decisions.md P-02).
///
/// Linking is decided here (`Resolver::resolve`). `wasmicon check` and
/// `verify/differential` look at the same pair, so the spelling is not
/// repeated in three places (a mismatch would make check call something
/// linkable "not in the table", or pass something that does not link).
pub const AS_ABORT: (&str, &str) = ("env", "abort");

/// Size of the scratch area for transfers whose input and output may overlap.
///
/// In `spi.transfer` and `i2c.write-read` both the source and the destination
/// are in guest linear memory and may overlap. The borrows cannot be split,
/// so the transmit side is copied here first. Enough for the v0.1 uses (6
/// bytes for SHT4x, the ILI9341 ID read); a longer transfer returns
/// `unsupported`.
pub const SCRATCH: usize = 128;

/// Maximum length of one trace line.
const LINE: usize = 160;

/// Result of a board operation. The board never traps, so it returns `ErrorCode` directly.
pub type BoardResult<T> = core::result::Result<T, ErrorCode>;

/// A non-empty rectangle inside a display, in pixels after orientation. `Hal`
/// builds it only after checking it against `Board::display_size`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Board-specific operations. A port implements only these.
///
/// Every index is a board GPIO or peripheral number, not a handle. `Hal`
/// manages the handles.
pub trait Board {
    /// Number of GPIOs, for range checks.
    fn gpio_count(&self) -> u32;

    /// GPIOs the guest may not open.
    ///
    /// The trace UART pins, pins wired to flash or PSRAM, numbers that do not
    /// exist on the chip, and so on. Opening one would cut the trace or hit
    /// the flash the firmware executes from and take the firmware down.
    fn gpio_reserved(&self, _index: u32) -> bool {
        false
    }

    fn gpio_configure(&mut self, index: u32, mode: PinMode) -> BoardResult<()>;
    fn gpio_write(&mut self, index: u32, level: Level) -> BoardResult<()>;
    fn gpio_read(&mut self, index: u32) -> BoardResult<Level>;
    fn gpio_release(&mut self, index: u32);

    fn i2c_open(&mut self, index: u32, speed: Speed) -> BoardResult<()>;
    fn i2c_write(&mut self, index: u32, address: u16, data: &[u8]) -> BoardResult<()>;
    fn i2c_read(&mut self, index: u32, address: u16, buf: &mut [u8]) -> BoardResult<usize>;
    fn i2c_write_read(
        &mut self,
        index: u32,
        address: u16,
        data: &[u8],
        buf: &mut [u8],
    ) -> BoardResult<usize>;
    fn i2c_close(&mut self, index: u32);

    fn spi_open(&mut self, index: u32, frequency_hz: u32, mode: SpiMode) -> BoardResult<()>;
    fn spi_write(&mut self, index: u32, data: &[u8]) -> BoardResult<()>;
    fn spi_transfer(&mut self, index: u32, data: &[u8], buf: &mut [u8]) -> BoardResult<usize>;
    fn spi_close(&mut self, index: u32);

    fn now_us(&mut self) -> u64;
    fn sleep_ms(&mut self, ms: u32);
    fn sleep_us(&mut self, us: u32);

    /// The guest's `log` output. UTF-8 is not validated (docs/abi-spec.md §4.2).
    fn log(&mut self, level: LogLevel, message: &[u8]);

    /// One trace line (without a newline). The port sends it to the serial port or similar.
    fn trace(&mut self, line: &[u8]);

    /// Reads `buf.len()` bytes of the assets slot, starting `offset` bytes
    /// into it (docs/app-workflow.md §3.10). `Hal` has checked that the range
    /// lies inside a body the directory describes.
    ///
    /// The default is for a port without an assets slot: `unsupported`.
    fn assets_read(&mut self, _offset: u32, _buf: &mut [u8]) -> BoardResult<()> {
        Err(ErrorCode::Unsupported)
    }

    /// CRC-32 of `len` bytes of the assets slot from `offset`: the body check
    /// at an asset's first `open`. The default streams through `assets_read`
    /// in `ASSETS_CHUNK` pieces; a board whose flash is memory-mapped
    /// overrides it with `crc32` over the slice.
    fn assets_crc32(&mut self, offset: u32, len: u32) -> BoardResult<u32> {
        // Word-aligned, so a port that reads flash by words (the ESP ports)
        // takes its direct path instead of copying through a window.
        #[repr(align(4))]
        struct Chunk([u8; ASSETS_CHUNK]);
        let mut chunk = Chunk([0; ASSETS_CHUNK]);
        let mut crc = CRC32_INIT;
        let (mut at, mut left) = (offset, len);
        while left > 0 {
            let n = left.min(ASSETS_CHUNK as u32) as usize;
            self.assets_read(at, &mut chunk.0[..n])?;
            crc = crc32_update(crc, &chunk.0[..n]);
            at += n as u32;
            left -= n as u32;
        }
        Ok(crc32_end(crc))
    }

    /// Whether this firmware implements `wasmicon:device/display`. Without it
    /// the display imports do not link, so an app that needs a display is
    /// refused before `run` (docs/device-layer.md §6).
    fn display_supported(&self) -> bool {
        false
    }

    /// Size of display `index` after orientation, as `(width, height)`, or
    /// `None` if the board has no display there. Both are at least 1.
    fn display_size(&self, _index: u32) -> Option<(u32, u32)> {
        None
    }

    /// Fills `rect` with one RGB565 color. `Hal` has checked `rect` against
    /// `display_size`.
    fn display_fill_rect(&mut self, _index: u32, _rect: Rect, _color: u16) -> BoardResult<()> {
        Err(ErrorCode::Unsupported)
    }

    /// Writes `rect.w * rect.h` RGB565 pixels, row by row, two bytes each,
    /// high byte first (docs/device-layer.md §4). `Hal` has checked `rect`
    /// and that `pixels` has exactly that length.
    fn display_write_rect(&mut self, _index: u32, _rect: Rect, _pixels: &[u8]) -> BoardResult<()> {
        Err(ErrorCode::Unsupported)
    }

    /// Makes everything drawn so far visible. A backend that draws straight
    /// to the panel keeps the default.
    fn display_present(&mut self, _index: u32) -> BoardResult<()> {
        Ok(())
    }

    /// The guest released display `index`. The picture stays.
    fn display_release(&mut self, _index: u32) {}
}

/// Verdict of an asset's body CRC, taken at its first `open`
/// (docs/app-workflow.md §3.10).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Verified {
    Unchecked,
    Ok,
    Bad,
}

#[derive(Clone, Copy, Default)]
struct BlobSlot {
    open: bool,
    /// Index into the assets directory.
    entry: u8,
}

#[derive(Clone, Copy, Default)]
struct PinSlot {
    open: bool,
    index: u32,
}

#[derive(Clone, Copy, Default)]
struct BusSlot {
    open: bool,
    index: u32,
}

/// Provides the HAL according to the generated import table.
pub struct Hal<B: Board> {
    board: B,
    trace_on: bool,
    pins: [PinSlot; MAX_PINS],
    i2c: [BusSlot; MAX_I2C],
    spi: [BusSlot; MAX_SPI],
    /// Wiring table (docs/app-workflow.md §3.9). `pin-by-role` answers from it.
    role_map: RoleMap,
    /// Whether each role in the table was handed out by `pin-by-role` (for the
    /// docs/abi-spec.md §9 normalization).
    handed: [bool; MAX_ROLES],
    scratch: [u8; SCRATCH],
    /// Assets directory (docs/app-workflow.md §3.10). `blob.open` looks names up here.
    assets: Directory,
    /// Each entry's body CRC verdict. A corrupt body stays `Bad` for the run,
    /// so it is read once, not on every `open`.
    verified: [Verified; MAX_ENTRIES],
    blobs: [BlobSlot; MAX_BLOBS],
    screens: [BusSlot; MAX_SCREENS],
}

impl<B: Board> Hal<B> {
    /// A `Hal` with no roles and no assets. The wiring table comes through
    /// `with_roles`, the assets directory through `with_assets`.
    #[must_use]
    pub fn new(board: B, trace_on: bool) -> Self {
        Hal {
            board,
            trace_on,
            pins: [PinSlot::default(); MAX_PINS],
            i2c: [BusSlot::default(); MAX_I2C],
            spi: [BusSlot::default(); MAX_SPI],
            role_map: RoleMap::EMPTY,
            handed: [false; MAX_ROLES],
            scratch: [0; SCRATCH],
            assets: Directory::EMPTY,
            verified: [Verified::Unchecked; MAX_ENTRIES],
            blobs: [BlobSlot::default(); MAX_BLOBS],
            screens: [BusSlot::default(); MAX_SCREENS],
        }
    }

    /// Sets the wiring table: on a board what the firmware read from the config
    /// slot, on the host the mock table in the profile (docs/app-workflow.md §3.9).
    #[must_use]
    pub fn with_roles(mut self, map: RoleMap) -> Self {
        self.role_map = map;
        self.handed = [false; MAX_ROLES];
        self
    }

    /// Sets the assets directory: on a board what the firmware read from the
    /// assets slot, on the host the image built from the `[assets]` table
    /// (docs/app-workflow.md §3.10). Forgets the body verdicts and closes
    /// every blob.
    #[must_use]
    pub fn with_assets(mut self, dir: Directory) -> Self {
        self.assets = dir;
        self.verified = [Verified::Unchecked; MAX_ENTRIES];
        self.blobs = [BlobSlot::default(); MAX_BLOBS];
        self
    }

    /// The board. Used by the port for cleanup.
    pub fn board_mut(&mut self) -> &mut B {
        &mut self.board
    }

    /// Drops every handle that is still open (docs/abi-spec.md §5.2).
    ///
    /// That section says the host drops all remaining handles when `run`
    /// returns. If the guest released them itself (`Drop` in the Rust
    /// bindings, an explicit `drop` in the AS version) this does nothing. It
    /// cleans up after a trap, and after a guest written without the bindings
    /// forgot to release.
    ///
    /// Without this cleanup the next app hits `busy` on `open` of the same pin
    /// or bus (docs/app-workflow.md §3.2).
    ///
    /// No trace line is emitted: this is not a guest host call, and a line
    /// here would change the recorded traces (docs/verification-report.md §6, §7).
    ///
    /// The release order is fixed: ascending handle, gpio then i2c then spi,
    /// then the blobs (which hold no peripheral, so the board is not told),
    /// then the screens. A per-port order would create a difference between
    /// boards that the trace does not show.
    ///
    /// The record of handed-out roles (`handed`) is not cleared here. It is
    /// not a handle but state for the docs/abi-spec.md §9 normalization, and
    /// outside the scope of the handle cleanup. A loader that reuses one `Hal`
    /// must clear it separately: carried over, a number the next app
    /// hard-codes that happens to match a role of the previous app would
    /// print as `role:` and change the trace. That is the "rebuild arena /
    /// `Hal` / role table per cycle" item in docs/TODO.md §5; today every
    /// port creates `Hal` once.
    pub fn release_all(&mut self) {
        // The board and the slots are separate fields, so they are borrowed separately.
        let board = &mut self.board;
        for slot in &mut self.pins {
            if slot.open {
                board.gpio_release(slot.index);
                *slot = PinSlot::default();
            }
        }
        for slot in &mut self.i2c {
            if slot.open {
                board.i2c_close(slot.index);
                *slot = BusSlot::default();
            }
        }
        for slot in &mut self.spi {
            if slot.open {
                board.spi_close(slot.index);
                *slot = BusSlot::default();
            }
        }
        // The body verdicts (`verified`) stay, like `handed`: they describe
        // the slot, not a handle.
        for slot in &mut self.blobs {
            *slot = BlobSlot::default();
        }
        for slot in &mut self.screens {
            if slot.open {
                board.display_release(slot.index);
                *slot = BusSlot::default();
            }
        }
    }

    /// The open display behind a screen handle.
    fn screen(&self, handle: u64) -> core::result::Result<u32, u32> {
        let i = slot_of(handle, MAX_SCREENS)?;
        if self.screens[i].open {
            Ok(self.screens[i].index)
        } else {
            Err(ErrorCode::InvalidHandle.status())
        }
    }

    /// Checks `x, y, w, h` against display `index`: non-empty and inside.
    fn rect(&self, index: u32, args: &[u64]) -> core::result::Result<Rect, u32> {
        let bad = ErrorCode::InvalidArgument.status();
        let (width, height) = self.board.display_size(index).ok_or(bad)?;
        let r = Rect {
            x: args[0] as u32,
            y: args[1] as u32,
            w: args[2] as u32,
            h: args[3] as u32,
        };
        let fits =
            |at: u32, len: u32, max: u32| len > 0 && at.checked_add(len).is_some_and(|e| e <= max);
        if fits(r.x, r.w, width) && fits(r.y, r.h, height) {
            Ok(r)
        } else {
            Err(bad)
        }
    }

    /// The body check at an asset's first `open` (docs/app-workflow.md §3.10).
    ///
    /// A wrong CRC marks the entry bad for the rest of the run, so a corrupt
    /// 4 MiB body is read once, not on every `open`. A read failure leaves it
    /// unchecked: that is the board, not the data.
    fn verify_asset(&mut self, i: usize) -> BoardResult<()> {
        match self.verified[i] {
            Verified::Ok => Ok(()),
            Verified::Bad => Err(ErrorCode::Io),
            Verified::Unchecked => {
                let e = *self.assets.entry(i);
                let crc = self.board.assets_crc32(e.offset, e.len)?;
                if crc == e.crc {
                    self.verified[i] = Verified::Ok;
                    Ok(())
                } else {
                    self.verified[i] = Verified::Bad;
                    Err(ErrorCode::Io)
                }
            }
        }
    }

    /// docs/abi-spec.md §9: a GPIO number handed out by role is printed as the
    /// role name, not the number.
    ///
    /// Only the number is available to decide, so a number the guest
    /// hard-codes that matches a role assignment prints as the role name too;
    /// that section states this assumption explicitly.
    ///
    /// Only handed-out roles are replaced. A number that is in the table but
    /// was never looked up through `pin-by-role` prints as a raw number, so
    /// changing the table does not change the trace of an app that uses no
    /// roles. The table never maps two roles to one number (`RoleMap::insert`
    /// rejects it), so at most one entry matches.
    fn write_pin(&self, out: &mut Buf<'_>, index: u32) {
        for i in 0..self.role_map.len() {
            if self.handed[i] && self.role_map.pin(i) == index {
                out.str("role:");
                out.bytes(self.role_map.name(i));
                return;
            }
        }
        out.u32(index);
    }
}

/// Converts a handle (1-based) to an array index. 0 is invalid (docs/abi-spec.md §5.1).
fn slot_of(handle: u64, len: usize) -> core::result::Result<usize, u32> {
    let h = handle as u32;
    if h == 0 || h as usize > len {
        return Err(ErrorCode::InvalidHandle.status());
    }
    Ok(h as usize - 1)
}

fn guest_slice(mem: &[u8], ptr: u64, len: u64) -> Result<&[u8]> {
    let a = ptr as u32 as usize;
    let b = len as u32 as usize;
    mem.get(
        a..a.checked_add(b)
            .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?,
    )
    .ok_or(Error::Trap(Trap::MemoryOutOfBounds))
}

fn put_u32(mem: &mut [u8], ptr: u64, v: u32) -> Result<()> {
    let a = ptr as u32 as usize;
    let s = mem
        .get_mut(
            a..a.checked_add(4)
                .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?,
        )
        .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?;
    s.copy_from_slice(&v.to_le_bytes());
    Ok(())
}

/// CRC-32 (IEEE). Used for long `list<u8>` values in traces (docs/abi-spec.md §9)
/// and for the slot check (`slot`).
#[must_use]
pub fn crc32(data: &[u8]) -> u32 {
    crc32_end(crc32_update(CRC32_INIT, data))
}

/// Initial value for an incremental computation.
///
/// Needed when the data cannot be passed at once. ESP32-S3 cannot read the
/// slot in one go (`esp-storage` reads it in small buffers), so checking the
/// CRC without using the arena needs this form (`pick_guest` in
/// `ports/esp32s3/src/main.rs`).
pub const CRC32_INIT: u32 = 0xffff_ffff;

/// Feeds more data. After all of `data` has been passed in order, finish with `crc32_end`.
#[must_use]
pub fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    for &b in data {
        crc ^= u32::from(b);
        let mut i = 0;
        while i < 8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            i += 1;
        }
    }
    crc
}

/// Finishes the computation.
#[must_use]
pub fn crc32_end(crc: u32) -> u32 {
    !crc
}

/// The `list<u8>` notation of docs/abi-spec.md §9: beyond 32 bytes, the first
/// 16 bytes plus a CRC-32. Used both for arguments and for the bytes a read
/// received (`data=` in the result).
fn write_bytes(out: &mut Buf<'_>, data: &[u8]) {
    if !out.is_on() {
        return;
    }
    out.str("0x");
    if data.len() <= 32 {
        for &b in data {
            out.hex(u32::from(b), 2);
        }
    } else {
        for &b in &data[..16] {
            out.hex(u32::from(b), 2);
        }
        out.str("..len=");
        out.u32(data.len() as u32);
        out.str(" crc32=");
        out.hex(crc32(data), 8);
    }
}

impl<B: Board> Resolver for Hal<B> {
    fn resolve(&mut self, module: &str, name: &str, ty: &ExternType<'_>) -> Option<Extern> {
        // AssemblyScript's env.abort is the one import outside world app
        // (docs/decisions.md P-02).
        if (module, name) == AS_ABORT {
            return Some(Extern::Func(HOST_ENV_ABORT));
        }
        let ExternType::Func(ft) = ty else {
            return None;
        };
        // Exact match of module + name + sig against the generated table
        // (docs/abi-spec.md §6.4).
        let desc = generated::resolve(module, name)?;
        if !ft.sig_matches(desc.sig) {
            return None;
        }
        // A device interface the firmware lacks is a link error: the app is
        // refused at instantiate instead of running half-way
        // (docs/device-layer.md §6).
        if desc.module.starts_with("wasmicon:device/display@") && !self.board.display_supported() {
            return None;
        }
        Some(Extern::Func(u32::from(desc.host_fn.index())))
    }

    #[allow(clippy::too_many_lines)]
    fn call(&mut self, host: u32, args: &[u64], results: &mut [u64], mem: &mut [u8]) -> Result<()> {
        if host == HOST_ENV_ABORT {
            return Err(Error::Trap(Trap::UnreachableHostCall));
        }
        let f =
            HostFn::from_index(host as u16).ok_or(Error::Unlinkable("unknown host function"))?;
        let desc = &generated::IMPORTS[host as usize];

        let mut argbuf = [0u8; LINE];
        let mut outbuf = [0u8; LINE];
        // No formatting while tracing is off. Formatting and discarding on every
        // call used to cost a little over 20% of run time in a host profile, all
        // of it the CRC-32 of `spi.write` (docs/TODO.md §4).
        let (mut a, mut o) = if self.trace_on {
            (Buf::new(&mut argbuf), Buf::new(&mut outbuf))
        } else {
            (Buf::off(&mut argbuf), Buf::off(&mut outbuf))
        };
        let mut status: u32 = 0;
        let mut result64: u64 = 0;

        match f {
            HostFn::DisplayScreenOpen => {
                let index = args[0] as u32;
                a.u32(index);
                let have =
                    (index as usize) < MAX_SCREENS && self.board.display_size(index).is_some();
                status = if !have {
                    ErrorCode::Unsupported.status()
                } else if self.screens[index as usize].open {
                    ErrorCode::Busy.status()
                } else {
                    self.screens[index as usize] = BusSlot { open: true, index };
                    let handle = index + 1;
                    put_u32(mem, args[1], handle)?;
                    o.u32(handle);
                    0
                };
            }
            // No `result`, so no error path: an invalid handle reads as 0
            // (wit-device/display.wit). The value is traced as the status.
            HostFn::DisplayScreenWidth | HostFn::DisplayScreenHeight => {
                a.u64(args[0]);
                let size = self
                    .screen(args[0])
                    .ok()
                    .and_then(|i| self.board.display_size(i));
                status = match (size, f) {
                    (Some((w, _)), HostFn::DisplayScreenWidth) => w,
                    (Some((_, h)), _) => h,
                    (None, _) => 0,
                };
            }
            HostFn::DisplayScreenFormat => {
                a.u64(args[0]);
                // `rgb565` is the only format; an invalid handle reads as it too.
                status = PixelFormat::Rgb565 as u32;
            }
            HostFn::DisplayScreenFillRect => {
                a.u64(args[0]);
                for v in &args[1..6] {
                    a.str(", ");
                    a.u64(*v);
                }
                let color = args[5] as u32;
                status = match self.screen(args[0]) {
                    Err(e) => e,
                    Ok(index) => match self.rect(index, &args[1..5]) {
                        Err(e) => e,
                        Ok(_) if color > 0xffff => ErrorCode::InvalidArgument.status(),
                        Ok(r) => match self.board.display_fill_rect(index, r, color as u16) {
                            Ok(()) => 0,
                            Err(e) => e.status(),
                        },
                    },
                };
            }
            HostFn::DisplayScreenWriteRect => {
                let pixels = guest_slice(mem, args[5], args[6])?;
                // docs/abi-spec.md §9: the pixels are traced as a CRC-32 only,
                // as for spi.write.
                a.u64(args[0]);
                for v in &args[1..5] {
                    a.str(", ");
                    a.u64(*v);
                }
                a.str(", crc32=");
                if a.is_on() {
                    a.hex(crc32(pixels), 8);
                }
                status = match self.screen(args[0]) {
                    Err(e) => e,
                    Ok(index) => match self.rect(index, &args[1..5]) {
                        Err(e) => e,
                        Ok(r) if (r.w as usize) * (r.h as usize) * 2 != pixels.len() => {
                            ErrorCode::InvalidArgument.status()
                        }
                        Ok(r) => match self.board.display_write_rect(index, r, pixels) {
                            Ok(()) => 0,
                            Err(e) => e.status(),
                        },
                    },
                };
            }
            HostFn::DisplayScreenPresent => {
                a.u64(args[0]);
                status = match self.screen(args[0]) {
                    Err(e) => e,
                    Ok(index) => match self.board.display_present(index) {
                        Ok(()) => 0,
                        Err(e) => e.status(),
                    },
                };
            }
            HostFn::DisplayScreenDrop => {
                a.u64(args[0]);
                if let Ok(index) = self.screen(args[0]) {
                    self.board.display_release(index);
                    self.screens[index as usize] = BusSlot::default();
                }
            }

            HostFn::BoardPinByRole => {
                let role = guest_slice(mem, args[0], args[1])?;
                a.byte(b'"');
                a.escaped(role);
                a.byte(b'"');
                match self.role_map.find(role) {
                    Some(i) => {
                        let index = self.role_map.pin(i);
                        put_u32(mem, args[2], index)?;
                        self.handed[i] = true;
                        self.write_pin(&mut o, index);
                    }
                    None => status = ErrorCode::Unsupported.status(),
                }
            }

            HostFn::GpioPinOpen => {
                let index = args[0] as u32;
                let mode = args[1] as u32;
                self.write_pin(&mut a, index);
                a.str(", ");
                a.u32(mode);
                let slot = self.pins.iter().position(|p| !p.open);
                let taken = self.pins.iter().any(|p| p.open && p.index == index);
                status = if index >= self.board.gpio_count()
                    || mode >= PinMode::COUNT
                    || self.board.gpio_reserved(index)
                {
                    ErrorCode::InvalidArgument.status()
                } else if taken {
                    ErrorCode::Busy.status()
                } else if let Some(slot) = slot {
                    let m = PinMode::from_u32(mode).ok_or(Error::Invalid("bad mode"))?;
                    match self.board.gpio_configure(index, m) {
                        Ok(()) => {
                            self.pins[slot] = PinSlot { open: true, index };
                            let handle = slot as u32 + 1;
                            put_u32(mem, args[2], handle)?;
                            o.u32(handle);
                            0
                        }
                        Err(e) => e.status(),
                    }
                } else {
                    ErrorCode::OutOfMemory.status()
                };
            }
            HostFn::GpioPinSetMode => {
                let mode = args[1] as u32;
                a.u64(args[0]);
                a.str(", ");
                a.u32(mode);
                status = match slot_of(args[0], MAX_PINS) {
                    Err(e) => e,
                    Ok(i) if !self.pins[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(_) if mode >= PinMode::COUNT => ErrorCode::InvalidArgument.status(),
                    Ok(i) => {
                        let index = self.pins[i].index;
                        let m = PinMode::from_u32(mode).ok_or(Error::Invalid("bad mode"))?;
                        match self.board.gpio_configure(index, m) {
                            Ok(()) => 0,
                            Err(e) => e.status(),
                        }
                    }
                };
            }
            HostFn::GpioPinRead => {
                a.u64(args[0]);
                status = match slot_of(args[0], MAX_PINS) {
                    Err(e) => e,
                    Ok(i) if !self.pins[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(i) => {
                        let index = self.pins[i].index;
                        match self.board.gpio_read(index) {
                            Ok(level) => {
                                put_u32(mem, args[1], level as u32)?;
                                o.u32(level as u32);
                                0
                            }
                            Err(e) => e.status(),
                        }
                    }
                };
            }
            HostFn::GpioPinWrite => {
                let level = args[1] as u32;
                a.u64(args[0]);
                a.str(", ");
                a.u32(level);
                status = match slot_of(args[0], MAX_PINS) {
                    Err(e) => e,
                    Ok(i) if !self.pins[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(_) if level >= Level::COUNT => ErrorCode::InvalidArgument.status(),
                    Ok(i) => {
                        let index = self.pins[i].index;
                        let l = Level::from_u32(level).ok_or(Error::Invalid("bad level"))?;
                        match self.board.gpio_write(index, l) {
                            Ok(()) => 0,
                            Err(e) => e.status(),
                        }
                    }
                };
            }
            HostFn::GpioPinToggle => {
                a.u64(args[0]);
                status = match slot_of(args[0], MAX_PINS) {
                    Err(e) => e,
                    Ok(i) if !self.pins[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(i) => {
                        let index = self.pins[i].index;
                        match self.board.gpio_read(index) {
                            Ok(level) => {
                                let next = if level == Level::Low {
                                    Level::High
                                } else {
                                    Level::Low
                                };
                                match self.board.gpio_write(index, next) {
                                    Ok(()) => 0,
                                    Err(e) => e.status(),
                                }
                            }
                            Err(e) => e.status(),
                        }
                    }
                };
            }
            HostFn::GpioPinDrop => {
                a.u64(args[0]);
                if let Ok(i) = slot_of(args[0], MAX_PINS)
                    && self.pins[i].open
                {
                    let index = self.pins[i].index;
                    self.board.gpio_release(index);
                    self.pins[i] = PinSlot::default();
                }
            }

            HostFn::I2cBusOpen => {
                let index = args[0] as u32;
                let speed = args[1] as u32;
                a.u32(index);
                a.str(", ");
                a.u32(speed);
                status = if index as usize >= MAX_I2C || speed >= Speed::COUNT {
                    ErrorCode::InvalidArgument.status()
                } else if self.i2c[index as usize].open {
                    ErrorCode::Busy.status()
                } else {
                    let s = Speed::from_u32(speed).ok_or(Error::Invalid("bad speed"))?;
                    match self.board.i2c_open(index, s) {
                        Ok(()) => {
                            self.i2c[index as usize] = BusSlot { open: true, index };
                            let handle = index + 1;
                            put_u32(mem, args[2], handle)?;
                            o.u32(handle);
                            0
                        }
                        Err(e) => e.status(),
                    }
                };
            }
            HostFn::I2cBusWrite => {
                let data = guest_slice(mem, args[2], args[3])?;
                a.u64(args[0]);
                a.str(", ");
                a.u64(args[1]);
                a.str(", ");
                write_bytes(&mut a, data);
                let n = data.len();
                status = match slot_of(args[0], MAX_I2C) {
                    Err(e) => e,
                    Ok(i) if !self.i2c[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(_) if n == 0 => ErrorCode::InvalidArgument.status(),
                    Ok(i) => {
                        let index = self.i2c[i].index;
                        let addr = args[1] as u16;
                        // data is only read from guest memory, so it is not copied.
                        let data = guest_slice(mem, args[2], args[3])?;
                        match self.board.i2c_write(index, addr, data) {
                            Ok(()) => 0,
                            Err(e) => e.status(),
                        }
                    }
                };
            }
            HostFn::I2cBusRead => {
                let want = args[2] as u32 as usize;
                let cap = args[4] as u32 as usize;
                a.u64(args[0]);
                a.str(", ");
                a.u64(args[1]);
                a.str(", ");
                a.u32(want as u32);
                status = match slot_of(args[0], MAX_I2C) {
                    Err(e) => e,
                    Ok(i) if !self.i2c[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(_) if want == 0 || want > cap => ErrorCode::InvalidArgument.status(),
                    Ok(_) if want > SCRATCH => ErrorCode::Unsupported.status(),
                    Ok(i) => {
                        let index = self.i2c[i].index;
                        let addr = args[1] as u16;
                        match self.board.i2c_read(index, addr, &mut self.scratch[..want]) {
                            Ok(got) => {
                                let src = &self.scratch[..got];
                                let dst = args[3] as u32 as usize;
                                let end = dst
                                    .checked_add(got)
                                    .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?;
                                mem.get_mut(dst..end)
                                    .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?
                                    .copy_from_slice(src);
                                put_u32(mem, args[5], got as u32)?;
                                o.str("len=");
                                o.u32(got as u32);
                                o.str(" data=");
                                write_bytes(&mut o, &self.scratch[..got]);
                                0
                            }
                            Err(e) => e.status(),
                        }
                    }
                };
            }
            HostFn::I2cBusWriteRead => {
                let want = args[4] as u32 as usize;
                let cap = args[6] as u32 as usize;
                let data_len = args[3] as u32 as usize;
                {
                    let data = guest_slice(mem, args[2], args[3])?;
                    a.u64(args[0]);
                    a.str(", ");
                    a.u64(args[1]);
                    a.str(", ");
                    write_bytes(&mut a, data);
                    a.str(", ");
                    a.u32(want as u32);
                }
                status = match slot_of(args[0], MAX_I2C) {
                    Err(e) => e,
                    Ok(i) if !self.i2c[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(_) if want == 0 || want > cap || data_len == 0 => {
                        ErrorCode::InvalidArgument.status()
                    }
                    Ok(_) if want > SCRATCH || data_len > SCRATCH => {
                        ErrorCode::Unsupported.status()
                    }
                    Ok(i) => {
                        let index = self.i2c[i].index;
                        let addr = args[1] as u16;
                        // The source and destination may overlap, so copy the transmit side first.
                        let mut tx = [0u8; SCRATCH];
                        tx[..data_len].copy_from_slice(guest_slice(mem, args[2], args[3])?);
                        match self.board.i2c_write_read(
                            index,
                            addr,
                            &tx[..data_len],
                            &mut self.scratch[..want],
                        ) {
                            Ok(got) => {
                                let dst = args[5] as u32 as usize;
                                let end = dst
                                    .checked_add(got)
                                    .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?;
                                mem.get_mut(dst..end)
                                    .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?
                                    .copy_from_slice(&self.scratch[..got]);
                                put_u32(mem, args[7], got as u32)?;
                                o.str("len=");
                                o.u32(got as u32);
                                o.str(" data=");
                                write_bytes(&mut o, &self.scratch[..got]);
                                0
                            }
                            Err(e) => e.status(),
                        }
                    }
                };
            }
            HostFn::I2cBusDrop => {
                a.u64(args[0]);
                if let Ok(i) = slot_of(args[0], MAX_I2C)
                    && self.i2c[i].open
                {
                    let index = self.i2c[i].index;
                    self.board.i2c_close(index);
                    self.i2c[i] = BusSlot::default();
                }
            }

            HostFn::SpiBusOpen => {
                let index = args[0] as u32;
                let freq = args[1] as u32;
                let mode = args[2] as u32;
                a.u32(index);
                a.str(", ");
                a.u32(freq);
                a.str(", ");
                a.u32(mode);
                status = if index as usize >= MAX_SPI || mode >= SpiMode::COUNT {
                    ErrorCode::InvalidArgument.status()
                } else if self.spi[index as usize].open {
                    ErrorCode::Busy.status()
                } else {
                    let m = SpiMode::from_u32(mode).ok_or(Error::Invalid("bad mode"))?;
                    match self.board.spi_open(index, freq, m) {
                        Ok(()) => {
                            self.spi[index as usize] = BusSlot { open: true, index };
                            let handle = index + 1;
                            put_u32(mem, args[3], handle)?;
                            o.u32(handle);
                            0
                        }
                        Err(e) => e.status(),
                    }
                };
            }
            HostFn::SpiBusWrite => {
                let data = guest_slice(mem, args[1], args[2])?;
                // docs/abi-spec.md §9: spi.write's data is traced as a CRC-32 only.
                a.u64(args[0]);
                a.str(", crc32=");
                if a.is_on() {
                    a.hex(crc32(data), 8);
                }
                let n = data.len();
                status = match slot_of(args[0], MAX_SPI) {
                    Err(e) => e,
                    Ok(i) if !self.spi[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(_) if n == 0 => ErrorCode::InvalidArgument.status(),
                    Ok(i) => {
                        let index = self.spi[i].index;
                        let data = guest_slice(mem, args[1], args[2])?;
                        match self.board.spi_write(index, data) {
                            Ok(()) => 0,
                            Err(e) => e.status(),
                        }
                    }
                };
            }
            HostFn::SpiBusTransfer => {
                let data_len = args[2] as u32 as usize;
                let cap = args[4] as u32 as usize;
                {
                    let data = guest_slice(mem, args[1], args[2])?;
                    a.u64(args[0]);
                    a.str(", crc32=");
                    if a.is_on() {
                        a.hex(crc32(data), 8);
                    }
                }
                status = match slot_of(args[0], MAX_SPI) {
                    Err(e) => e,
                    Ok(i) if !self.spi[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(_) if data_len == 0 || cap < data_len => ErrorCode::InvalidArgument.status(),
                    Ok(_) if data_len > SCRATCH => ErrorCode::Unsupported.status(),
                    Ok(i) => {
                        let index = self.spi[i].index;
                        // The source and destination may overlap, so copy the transmit side first.
                        let mut tx = [0u8; SCRATCH];
                        tx[..data_len].copy_from_slice(guest_slice(mem, args[1], args[2])?);
                        match self.board.spi_transfer(
                            index,
                            &tx[..data_len],
                            &mut self.scratch[..data_len],
                        ) {
                            Ok(got) => {
                                let dst = args[3] as u32 as usize;
                                let end = dst
                                    .checked_add(got)
                                    .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?;
                                mem.get_mut(dst..end)
                                    .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?
                                    .copy_from_slice(&self.scratch[..got]);
                                put_u32(mem, args[5], got as u32)?;
                                o.str("len=");
                                o.u32(got as u32);
                                o.str(" data=");
                                write_bytes(&mut o, &self.scratch[..got]);
                                0
                            }
                            Err(e) => e.status(),
                        }
                    }
                };
            }
            HostFn::SpiBusDrop => {
                a.u64(args[0]);
                if let Ok(i) = slot_of(args[0], MAX_SPI)
                    && self.spi[i].open
                {
                    let index = self.spi[i].index;
                    self.board.spi_close(index);
                    self.spi[i] = BusSlot::default();
                }
            }

            HostFn::AssetsBlobOpen => {
                let name = guest_slice(mem, args[0], args[1])?;
                a.byte(b'"');
                a.escaped(name);
                a.byte(b'"');
                // In this order so that a bad name is reported even when the
                // table is full, and no body is read for a name that could not
                // be opened anyway (docs/abi-spec.md §7).
                status = if !assets::valid_name(name) {
                    ErrorCode::InvalidArgument.status()
                } else if let Some(entry) = self.assets.find(name) {
                    match self.blobs.iter().position(|b| !b.open) {
                        None => ErrorCode::OutOfMemory.status(),
                        Some(slot) => match self.verify_asset(entry) {
                            Ok(()) => {
                                self.blobs[slot] = BlobSlot {
                                    open: true,
                                    entry: entry as u8,
                                };
                                let handle = slot as u32 + 1;
                                put_u32(mem, args[2], handle)?;
                                o.u32(handle);
                                0
                            }
                            Err(e) => e.status(),
                        },
                    }
                } else {
                    ErrorCode::NotFound.status()
                };
            }
            HostFn::AssetsBlobLen => {
                a.u64(args[0]);
                status = match slot_of(args[0], MAX_BLOBS) {
                    Err(e) => e,
                    Ok(i) if !self.blobs[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(i) => {
                        let len = self.assets.entry(usize::from(self.blobs[i].entry)).len;
                        put_u32(mem, args[1], len)?;
                        o.u32(len);
                        0
                    }
                };
            }
            HostFn::AssetsBlobRead => {
                let offset = args[1] as u32;
                let len = args[2] as u32;
                let cap = args[4] as u32;
                a.u64(args[0]);
                a.str(", ");
                a.u32(offset);
                a.str(", ");
                a.u32(len);
                status = match slot_of(args[0], MAX_BLOBS) {
                    Err(e) => e,
                    Ok(i) if !self.blobs[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(i) => {
                        let ent = *self.assets.entry(usize::from(self.blobs[i].entry));
                        match offset.checked_add(len) {
                            // Never a short read (docs/abi-spec.md §4.4, §7).
                            Some(end) if len > 0 && len <= cap && end <= ent.len => {
                                let dst = args[3] as u32 as usize;
                                let dst_end = dst
                                    .checked_add(len as usize)
                                    .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?;
                                let buf = mem
                                    .get_mut(dst..dst_end)
                                    .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?;
                                // Straight into guest memory: a body can be far
                                // larger than the scratch area. `ent.offset +
                                // offset` cannot overflow: the directory keeps
                                // every body inside the slot.
                                match self.board.assets_read(ent.offset + offset, buf) {
                                    Ok(()) => {
                                        put_u32(mem, args[5], len)?;
                                        o.str("len=");
                                        o.u32(len);
                                        o.str(" data=");
                                        write_bytes(&mut o, &mem[dst..dst_end]);
                                        0
                                    }
                                    Err(e) => e.status(),
                                }
                            }
                            _ => ErrorCode::InvalidArgument.status(),
                        }
                    }
                };
            }
            HostFn::AssetsBlobDrop => {
                a.u64(args[0]);
                if let Ok(i) = slot_of(args[0], MAX_BLOBS)
                    && self.blobs[i].open
                {
                    // No peripheral to release: the slot is simply freed.
                    self.blobs[i] = BlobSlot::default();
                }
            }

            HostFn::TimeNowUs => result64 = self.board.now_us(),
            HostFn::TimeSleepMs => self.board.sleep_ms(args[0] as u32),
            HostFn::TimeSleepUs => self.board.sleep_us(args[0] as u32),

            HostFn::LogLog => {
                let level = args[0] as u32;
                let msg = guest_slice(mem, args[1], args[2])?;
                a.u32(level);
                a.str(", ");
                a.byte(b'"');
                a.escaped(msg);
                a.byte(b'"');
                let l = LogLevel::from_u32(level).unwrap_or(LogLevel::Info);
                // Pass guest memory directly. A copy would truncate long messages
                // and break them when cut in the middle of a UTF-8 sequence.
                let msg = guest_slice(mem, args[1], args[2])?;
                self.board.log(l, msg);
            }
        }

        // Return value (docs/abi-spec.md §4.4: a function that returns a result
        // yields a single i32 status).
        if !desc.sig.ends_with(':') && !results.is_empty() {
            results[0] = if matches!(f, HostFn::TimeNowUs) {
                result64
            } else {
                u64::from(status)
            };
        }

        // Trace (time is not included; docs/abi-spec.md §9).
        if self.trace_on
            && !matches!(
                f,
                HostFn::TimeNowUs | HostFn::TimeSleepMs | HostFn::TimeSleepUs
            )
        {
            let mut line = [0u8; LINE * 2];
            let mut l = Buf::new(&mut line);
            l.str("> ");
            l.str(desc.module);
            l.byte(b'/');
            l.str(desc.name);
            l.byte(b'(');
            l.bytes(a.as_bytes());
            l.byte(b')');
            self.board.trace(l.as_bytes());

            l.clear();
            l.byte(b'<');
            if !desc.sig.ends_with(':') {
                l.byte(b' ');
                l.u32(status);
            }
            if !o.as_bytes().is_empty() {
                l.str(" [");
                l.bytes(o.as_bytes());
                l.byte(b']');
            }
            self.board.trace(l.as_bytes());
        }

        Ok(())
    }
}
