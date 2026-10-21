//! Thin safe wrappers over the generated extern declarations.
//!
//! - The i32 status becomes `Result<_, ErrorCode>` (docs/abi-spec.md §4.4)
//! - Handles call `[resource-drop]` from `Drop` (docs/abi-spec.md §5.2)
//! - `list<u8>` return values go into a buffer the caller passes (docs/abi-spec.md §4.4)
//!
//! No numbers are hard-coded here. Error codes come from the generated files and
//! pin numbers from the `board` interface.

use crate::generated::types::ErrorCode;

/// Result of a HAL call.
pub type Result<T> = core::result::Result<T, ErrorCode>;

/// Converts a status value into a `Result`.
///
/// `ErrorCode::from_status` returns `None` both for 0 (success) and for out-of-range
/// values, so 0 is checked first. Treating an out-of-range status as success would,
/// for example, let `Pin::open` wrap and return the invalid handle 0
/// (docs/abi-spec.md §5.1).
#[inline]
fn check(status: u32) -> Result<()> {
    if status == 0 {
        return Ok(());
    }
    Err(ErrorCode::from_status(status).unwrap_or(ErrorCode::Io))
}

/// General-purpose digital I/O.
pub mod gpio {
    use super::{Result, check};
    use crate::generated::gpio as raw;

    pub use crate::generated::gpio::{Level, PinMode};

    /// An open GPIO pin. `Drop` releases it.
    pub struct Pin(u32);

    impl Pin {
        /// Opens the pin in the given mode.
        ///
        /// # Errors
        /// `Busy` if the pin is already open, `InvalidArgument` if `index` is out of range.
        pub fn open(index: u32, mode: PinMode) -> Result<Pin> {
            let mut h: u32 = 0;
            // SAFETY: `h` is a live `u32` on this stack frame and `&raw mut h` points to it for
            // the whole call. The host writes at most 4 bytes there, and only on success.
            check(unsafe { raw::pin_open(index, mode as u32, &raw mut h) })?;
            Ok(Pin(h))
        }

        /// Changes the pin mode.
        ///
        /// # Errors
        /// `InvalidHandle` if the handle is invalid.
        pub fn set_mode(&self, mode: PinMode) -> Result<()> {
            // SAFETY: `self.0` is the handle `open` returned and has not been released; the
            // host validates it. The other argument is a scalar passed by value.
            check(unsafe { raw::pin_set_mode(self.0, mode as u32) })
        }

        /// Reads the current input level.
        ///
        /// # Errors
        /// `InvalidHandle` if the handle is invalid.
        pub fn read(&self) -> Result<Level> {
            let mut v: u32 = 0;
            // SAFETY: `self.0` is the handle `open` returned and has not been released; the
            // host validates it. `v` is a live `u32` on this stack frame and `&raw mut v`
            // points to it for the whole call; the host writes at most 4 bytes there.
            check(unsafe { raw::pin_read(self.0, &raw mut v) })?;
            // The spec never lets the host return an out-of-range value, but do not hide one.
            Level::from_u32(v).ok_or(super::ErrorCode::Io)
        }

        /// Sets the output level.
        ///
        /// # Errors
        /// `InvalidArgument` in input mode.
        pub fn write(&self, level: Level) -> Result<()> {
            // SAFETY: `self.0` is the handle `open` returned and has not been released; the
            // host validates it. The other argument is a scalar passed by value.
            check(unsafe { raw::pin_write(self.0, level as u32) })
        }

        /// Inverts the output level.
        ///
        /// # Errors
        /// `InvalidArgument` in input mode.
        pub fn toggle(&self) -> Result<()> {
            // SAFETY: `self.0` is the handle `open` returned and has not been released; the
            // host validates it. There are no pointer arguments.
            check(unsafe { raw::pin_toggle(self.0) })
        }
    }

    impl Drop for Pin {
        fn drop(&mut self) {
            // SAFETY: `self.0` is the handle `open` returned and this is its only release.
            // The host ignores an invalid handle without an error or a trap
            // (docs/abi-spec.md §5.2). There are no pointer arguments.
            unsafe { raw::pin_drop(self.0) }
        }
    }
}

/// I2C master.
pub mod i2c {
    use super::{Result, check};
    use crate::generated::i2c as raw;

    pub use crate::generated::i2c::Speed;

    /// An open I2C bus. `Drop` releases it.
    pub struct Bus(u32);

    impl Bus {
        /// Initializes and opens the bus.
        ///
        /// # Errors
        /// `Busy` if the bus is already open.
        pub fn open(index: u32, speed: Speed) -> Result<Bus> {
            let mut h: u32 = 0;
            // SAFETY: `h` is a live `u32` on this stack frame and `&raw mut h` points to it for
            // the whole call. The host writes at most 4 bytes there, and only on success.
            check(unsafe { raw::bus_open(index, speed as u32, &raw mut h) })?;
            Ok(Bus(h))
        }

        /// Writes `data` to the device at `address`.
        ///
        /// # Errors
        /// `Nack` if the device answers with NACK, `InvalidArgument` if `data` is empty.
        pub fn write(&self, address: u16, data: &[u8]) -> Result<()> {
            // SAFETY: `self.0` is the handle `open` returned and has not been released; the
            // host validates it. `data.as_ptr()` and `data.len()` describe one live slice that
            // outlives the call; the host reads at most `len` bytes from it and writes nothing.
            check(unsafe {
                raw::bus_write(self.0, u32::from(address), data.as_ptr(), data.len() as u32)
            })
        }

        /// Reads `buf.len()` bytes from the device at `address`. Returns the number of bytes
        /// read.
        ///
        /// # Errors
        /// `Nack` if the device answers with NACK.
        pub fn read(&self, address: u16, buf: &mut [u8]) -> Result<usize> {
            let mut len: u32 = 0;
            let cap = buf.len() as u32;
            // SAFETY: `self.0` is the handle `open` returned and has not been released; the
            // host validates it. `buf.as_mut_ptr()` and `cap` describe one live mutable slice
            // that outlives the call; the host writes at most `cap` bytes into it. `len` is a
            // live `u32` on this stack frame; the host writes at most 4 bytes there.
            check(unsafe {
                raw::bus_read(
                    self.0,
                    u32::from(address),
                    cap,
                    buf.as_mut_ptr(),
                    cap,
                    &raw mut len,
                )
            })?;
            Ok(len as usize)
        }

        /// Writes `data`, then reads `buf.len()` bytes after a repeated START.
        ///
        /// # Errors
        /// `Nack` if the device answers with NACK.
        pub fn write_read(&self, address: u16, data: &[u8], buf: &mut [u8]) -> Result<usize> {
            let mut len: u32 = 0;
            let cap = buf.len() as u32;
            // SAFETY: `self.0` is the handle `open` returned and has not been released; the
            // host validates it. `data.as_ptr()` and `data.len()` describe one live slice the
            // host only reads; `buf.as_mut_ptr()` and `cap` describe one live mutable slice
            // the host writes at most `cap` bytes into. Both outlive the call and cannot
            // overlap (`&` and `&mut`). `len` is a live `u32` on this stack frame; the host
            // writes at most 4 bytes there.
            check(unsafe {
                raw::bus_write_read(
                    self.0,
                    u32::from(address),
                    data.as_ptr(),
                    data.len() as u32,
                    cap,
                    buf.as_mut_ptr(),
                    cap,
                    &raw mut len,
                )
            })?;
            Ok(len as usize)
        }
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            // SAFETY: `self.0` is the handle `open` returned and this is its only release.
            // The host ignores an invalid handle without an error or a trap
            // (docs/abi-spec.md §5.2). There are no pointer arguments.
            unsafe { raw::bus_drop(self.0) }
        }
    }
}

/// SPI master. The guest drives CS and DC through `gpio` (docs/decisions.md D-08).
pub mod spi {
    use super::{Result, check};
    use crate::generated::spi as raw;

    pub use crate::generated::spi::Mode;

    /// An open SPI bus. `Drop` releases it.
    pub struct Bus(u32);

    impl Bus {
        /// Initializes and opens the bus.
        ///
        /// # Errors
        /// `Busy` if the bus is already open.
        pub fn open(index: u32, frequency_hz: u32, mode: Mode) -> Result<Bus> {
            let mut h: u32 = 0;
            // SAFETY: `h` is a live `u32` on this stack frame and `&raw mut h` points to it for
            // the whole call. The host writes at most 4 bytes there, and only on success.
            check(unsafe { raw::bus_open(index, frequency_hz, mode as u32, &raw mut h) })?;
            Ok(Bus(h))
        }

        /// Sends `data` and discards the received bytes. The hot path of display drawing.
        ///
        /// # Errors
        /// `InvalidArgument` if `data` is empty.
        pub fn write(&self, data: &[u8]) -> Result<()> {
            // SAFETY: `self.0` is the handle `open` returned and has not been released; the
            // host validates it. `data.as_ptr()` and `data.len()` describe one live slice that
            // outlives the call; the host reads at most `len` bytes from it and writes nothing.
            check(unsafe { raw::bus_write(self.0, data.as_ptr(), data.len() as u32) })
        }

        /// Full-duplex transfer. `buf` must be at least as long as `data`.
        ///
        /// # Errors
        /// `InvalidArgument` if `buf` is shorter than `data`.
        pub fn transfer(&self, data: &[u8], buf: &mut [u8]) -> Result<usize> {
            let mut len: u32 = 0;
            // SAFETY: `self.0` is the handle `open` returned and has not been released; the
            // host validates it. `data.as_ptr()` and `data.len()` describe one live slice the
            // host only reads; `buf.as_mut_ptr()` and `buf.len()` describe one live mutable
            // slice the host writes at most `buf.len()` bytes into. Both outlive the call and
            // cannot overlap (`&` and `&mut`). `len` is a live `u32` on this stack frame; the
            // host writes at most 4 bytes there.
            check(unsafe {
                raw::bus_transfer(
                    self.0,
                    data.as_ptr(),
                    data.len() as u32,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &raw mut len,
                )
            })?;
            Ok(len as usize)
        }
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            // SAFETY: `self.0` is the handle `open` returned and this is its only release.
            // The host ignores an invalid handle without an error or a trap
            // (docs/abi-spec.md §5.2). There are no pointer arguments.
            unsafe { raw::bus_drop(self.0) }
        }
    }
}

/// Monotonic time and blocking delays.
pub mod time {
    use crate::generated::time as raw;

    /// Monotonic microseconds since boot.
    #[must_use]
    pub fn now_us() -> u64 {
        // SAFETY: the import takes no arguments and touches no guest memory; it only returns
        // a scalar.
        unsafe { raw::now_us() }
    }

    /// Blocks for `ms` milliseconds.
    pub fn sleep_ms(ms: u32) {
        // SAFETY: the import takes one scalar argument by value and touches no guest memory.
        unsafe { raw::sleep_ms(ms) }
    }

    /// Blocks for `us` microseconds.
    pub fn sleep_us(us: u32) {
        // SAFETY: the import takes one scalar argument by value and touches no guest memory.
        unsafe { raw::sleep_us(us) }
    }
}

/// Debug output.
pub mod log {
    use crate::generated::log as raw;

    pub use crate::generated::log::Level;

    /// Writes a message at `level`. The host appends the newline.
    pub fn write(level: Level, message: &str) {
        // SAFETY: `message.as_ptr()` and `message.len()` describe one live UTF-8 slice that
        // outlives the call; the host reads at most `len` bytes from it and writes nothing.
        unsafe { raw::log(level as u32, message.as_ptr(), message.len() as u32) }
    }

    /// Writes a message at `info`.
    pub fn info(message: &str) {
        write(Level::Info, message);
    }

    /// Writes a message at `error`.
    pub fn error(message: &str) {
        write(Level::Error, message);
    }
}

/// Board-specific pin assignment.
pub mod board {
    use super::{Result, check};
    use crate::generated::board as raw;

    /// Looks up the GPIO number of a role name.
    ///
    /// Lets the same binary run on several boards (docs/abi-spec.md §8).
    ///
    /// # Errors
    /// `Unsupported` if the board has no assignment for the role.
    pub fn pin_by_role(role: &str) -> Result<u32> {
        let mut index: u32 = 0;
        // SAFETY: `role.as_ptr()` and `role.len()` describe one live UTF-8 slice that outlives
        // the call; the host only reads it. `index` is a live `u32` on this stack frame and
        // `&raw mut index` points to it for the whole call; the host writes at most 4 bytes
        // there, and only on success.
        check(unsafe { raw::pin_by_role(role.as_ptr(), role.len() as u32, &raw mut index) })?;
        Ok(index)
    }
}

/// Read-only assets shipped next to the app (docs/app-workflow.md §3.10).
pub mod assets {
    use super::{Result, check};
    use crate::generated::assets as raw;

    /// An open asset. `Drop` releases the handle.
    pub struct Blob(u32);

    // A blob has a length but no `is_empty`: the question "is this asset empty"
    // is one more host call, and an empty asset is a packaging mistake, not a
    // state to branch on.
    #[allow(clippy::len_without_is_empty)]
    impl Blob {
        /// Opens the asset called `name`.
        ///
        /// # Errors
        /// `NotFound` if no asset has that name, `InvalidArgument` if `name` is not a valid
        /// asset name, `Io` if the body's CRC-32 does not match, `OutOfMemory` if four blobs
        /// are already open.
        pub fn open(name: &str) -> Result<Blob> {
            let mut h: u32 = 0;
            // SAFETY: `name.as_ptr()` and `name.len()` describe one live UTF-8 slice that
            // outlives the call; the host only reads it. `h` is a live `u32` on this stack
            // frame and `&raw mut h` points to it for the whole call; the host writes at most
            // 4 bytes there, and only on success.
            check(unsafe { raw::blob_open(name.as_ptr(), name.len() as u32, &raw mut h) })?;
            Ok(Blob(h))
        }

        /// Length of the asset in bytes.
        ///
        /// # Errors
        /// `InvalidHandle` if the handle is invalid.
        pub fn len(&self) -> Result<u32> {
            let mut v: u32 = 0;
            // SAFETY: `self.0` is the handle `open` returned and has not been released; the
            // host validates it. `v` is a live `u32` on this stack frame and `&raw mut v`
            // points to it for the whole call; the host writes at most 4 bytes there.
            check(unsafe { raw::blob_len(self.0, &raw mut v) })?;
            Ok(v)
        }

        /// Reads `buf.len()` bytes starting `offset` bytes into the asset. Returns the number
        /// of bytes read, which is always `buf.len()` (the host never reads short).
        ///
        /// # Errors
        /// `InvalidArgument` if `buf` is empty or `offset + buf.len()` exceeds the asset's
        /// length.
        pub fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize> {
            let mut len: u32 = 0;
            let cap = buf.len() as u32;
            // SAFETY: `self.0` is the handle `open` returned and has not been released; the
            // host validates it. `buf.as_mut_ptr()` and `cap` describe one live mutable slice
            // that outlives the call; the host writes at most `cap` bytes into it. `len` is a
            // live `u32` on this stack frame; the host writes at most 4 bytes there.
            check(unsafe {
                raw::blob_read(self.0, offset, cap, buf.as_mut_ptr(), cap, &raw mut len)
            })?;
            Ok(len as usize)
        }
    }

    impl Drop for Blob {
        fn drop(&mut self) {
            // SAFETY: `self.0` is the handle `open` returned and this is its only release.
            // The host ignores an invalid handle without an error or a trap
            // (docs/abi-spec.md §5.2). There are no pointer arguments.
            unsafe { raw::blob_drop(self.0) }
        }
    }
}
