//! Mock board for the PC.
//!
//! The shared part of the HAL (import resolution, handle table, status, trace
//! formatting) is in `wasmicon-port`. This file implements only the
//! board-specific operations. Keeping the same shape as the board ports
//! guarantees that the trace format matches (docs/abi-spec.md §9).

use std::time::Instant;

use wasmicon_core::generated::ErrorCode;
use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::i2c::Speed;
use wasmicon_core::generated::log::Level as LogLevel;
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_port::{Board, BoardResult, Rect};

/// Number of GPIOs on the mock board.
const NUM_GPIO: usize = 48;

/// Role name -> GPIO number table of the mock board. Unlike a board, the host
/// has no flash, so the table lives here (`profile::HOST.roles`,
/// docs/app-workflow.md §3.9).
const ROLES: &[(&str, u32)] = wasmicon_port::profile::HOST.roles;

// Check at compile time that the numbers can be opened on this mock, as the
// board ports do (moving the table to the profile separated it from NUM_GPIO;
// this ties them back together). The mock has no reserved pins.
const _: () = wasmicon_port::profile::assert_roles_openable(ROLES, NUM_GPIO as u32, &[]);

/// Size of the mock display: the ILI9341's landscape size, so a display app
/// traced on the host compares with the evaluation boards (docs/device-layer.md §5.2).
pub const MOCK_DISPLAY: (u32, u32) = (320, 240);

/// The visible picture of a display, one RGB565 value per pixel, row by row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u16>,
}

/// The mock display backend (docs/device-layer.md §5.3).
///
/// It keeps what was drawn apart from what is visible, and only `present`
/// copies one to the other. A guest that never calls `present` shows nothing
/// here, as it may on a frame buffer backend, so the portability bug shows
/// on the host first (docs/decisions.md P-13).
struct MockDisplay {
    drawn: Frame,
    visible: Frame,
}

impl MockDisplay {
    fn new((width, height): (u32, u32)) -> Self {
        let blank = Frame {
            width,
            height,
            pixels: vec![0; (width * height) as usize],
        };
        MockDisplay {
            drawn: blank.clone(),
            visible: blank,
        }
    }

    /// Calls `put(offset, i)` for the `i`-th pixel of `r`, row by row.
    fn each(&mut self, r: Rect, mut put: impl FnMut(&mut u16, usize)) {
        let width = self.drawn.width as usize;
        let mut i = 0;
        for y in r.y..r.y + r.h {
            let row = y as usize * width;
            for x in r.x..r.x + r.w {
                put(&mut self.drawn.pixels[row + x as usize], i);
                i += 1;
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct PinState {
    configured: bool,
    mode: u32,
    level: u32,
}

/// Mock board on the PC. No peripherals are attached.
///
/// I2C replays recorded responses in order (`WASMICON_I2C_REPLAY`) so that a
/// guest can run to completion without the real sensor.
pub struct HostBoard {
    pins: [PinState; NUM_GPIO],
    start: Instant,
    trace: String,
    /// Responses returned to reads, in order. Empty means `Nack`.
    i2c_replay: Vec<Vec<u8>>,
    replay_pos: usize,
    /// Makes SPI return `unsupported`, as a port without SPI does (rp2040), to
    /// test the failure path.
    spi_unsupported: bool,
    /// The assets slot image (`crate::assets::image`), what a board has in
    /// flash. Empty means no assets.
    assets: Vec<u8>,
    /// `None` mirrors a firmware without a display (`with_display(false)`).
    display: Option<MockDisplay>,
}

impl HostBoard {
    #[must_use]
    pub fn new() -> Self {
        HostBoard {
            pins: [PinState::default(); NUM_GPIO],
            start: Instant::now(),
            trace: String::new(),
            i2c_replay: Vec::new(),
            replay_pos: 0,
            spi_unsupported: false,
            assets: Vec::new(),
            display: Some(MockDisplay::new(MOCK_DISPLAY)),
        }
    }

    /// Removes the display, as on a firmware without one: the display imports
    /// then do not link (`wasmicon check` runs board profiles this way).
    #[must_use]
    pub fn with_display(mut self, yes: bool) -> Self {
        self.display = yes.then(|| MockDisplay::new(MOCK_DISPLAY));
        self
    }

    /// Gives the mock display another size, to see how an app lays out on,
    /// say, the Tab5's 720x1280 panel. Traces then differ from the 320x240
    /// ones (docs/device-layer.md §7).
    #[must_use]
    pub fn with_display_size(mut self, size: (u32, u32)) -> Self {
        if self.display.is_some() {
            self.display = Some(MockDisplay::new(size));
        }
        self
    }

    /// The visible picture: what the guest drew up to its last `present`.
    #[must_use]
    pub fn frame(&self) -> Option<&Frame> {
        self.display.as_ref().map(|d| &d.visible)
    }

    /// Sets the assets slot image (built by `crate::assets::image`).
    #[must_use]
    pub fn with_assets(mut self, image: Vec<u8>) -> Self {
        self.assets = image;
        self
    }

    /// Makes SPI return `unsupported` (for testing the failure path).
    #[must_use]
    pub fn with_spi_unsupported(mut self, yes: bool) -> Self {
        self.spi_unsupported = yes;
        self
    }

    /// Sets the recorded I2C responses.
    #[must_use]
    pub fn with_i2c_replay(mut self, replay: Vec<Vec<u8>>) -> Self {
        self.i2c_replay = replay;
        self
    }

    /// The trace collected so far.
    #[must_use]
    pub fn trace_output(&self) -> &str {
        &self.trace
    }
}

impl Default for HostBoard {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether the mode is an output (`Output` or `OutputOpenDrain`).
fn is_output(mode: u32) -> bool {
    mode >= PinMode::Output as u32
}

/// The mock's wiring table, for `Hal::with_roles`.
///
/// # Panics
/// When `profile::HOST.roles` fails the check (the same condition as the
/// compile-time `assert_roles_openable`, so it does not happen in practice).
#[must_use]
pub fn host_roles() -> wasmicon_port::roles::RoleMap {
    let limits = wasmicon_port::roles::Limits::of(&wasmicon_port::profile::HOST);
    match wasmicon_port::roles::from_table(ROLES, &limits) {
        Ok(map) => map,
        Err(e) => panic!("profile::HOST.roles is invalid: {}", e.reason()),
    }
}

impl Board for HostBoard {
    fn gpio_count(&self) -> u32 {
        NUM_GPIO as u32
    }

    fn gpio_configure(&mut self, index: u32, mode: PinMode) -> BoardResult<()> {
        let p = self
            .pins
            .get_mut(index as usize)
            .ok_or(ErrorCode::InvalidArgument)?;
        p.configured = true;
        p.mode = mode as u32;
        Ok(())
    }

    fn gpio_write(&mut self, index: u32, level: Level) -> BoardResult<()> {
        let p = self
            .pins
            .get_mut(index as usize)
            .ok_or(ErrorCode::InvalidArgument)?;
        if !is_output(p.mode) {
            return Err(ErrorCode::InvalidArgument);
        }
        p.level = level as u32;
        Ok(())
    }

    fn gpio_read(&mut self, index: u32) -> BoardResult<Level> {
        let p = self
            .pins
            .get(index as usize)
            .ok_or(ErrorCode::InvalidArgument)?;
        Level::from_u32(p.level).ok_or(ErrorCode::Io)
    }

    fn gpio_release(&mut self, index: u32) {
        if let Some(p) = self.pins.get_mut(index as usize) {
            *p = PinState::default();
        }
    }

    // --- I2C / SPI: no device is attached to the mock ---

    fn i2c_open(&mut self, _index: u32, _speed: Speed) -> BoardResult<()> {
        Ok(())
    }

    fn i2c_write(&mut self, _index: u32, _address: u16, _data: &[u8]) -> BoardResult<()> {
        // With recorded responses, writes are accepted (measurement commands and so on).
        if self.i2c_replay.is_empty() {
            return Err(ErrorCode::Nack);
        }
        Ok(())
    }

    fn i2c_read(&mut self, _index: u32, _address: u16, buf: &mut [u8]) -> BoardResult<usize> {
        let Some(resp) = self.i2c_replay.get(self.replay_pos) else {
            return Err(ErrorCode::Nack);
        };
        // A short response is not consumed, so a retry fails the same way.
        if resp.len() < buf.len() {
            return Err(ErrorCode::Io);
        }
        self.replay_pos += 1;
        let n = buf.len();
        buf.copy_from_slice(&resp[..n]);
        Ok(n)
    }

    fn i2c_write_read(
        &mut self,
        index: u32,
        address: u16,
        _data: &[u8],
        buf: &mut [u8],
    ) -> BoardResult<usize> {
        self.i2c_read(index, address, buf)
    }

    fn i2c_close(&mut self, _index: u32) {}

    fn spi_open(&mut self, _index: u32, _frequency_hz: u32, _mode: SpiMode) -> BoardResult<()> {
        if self.spi_unsupported {
            return Err(ErrorCode::Unsupported);
        }
        Ok(())
    }

    fn spi_write(&mut self, _index: u32, _data: &[u8]) -> BoardResult<()> {
        if self.spi_unsupported {
            return Err(ErrorCode::Unsupported);
        }
        Ok(())
    }

    fn spi_transfer(&mut self, _index: u32, data: &[u8], buf: &mut [u8]) -> BoardResult<usize> {
        // MISO reads 0.
        let n = data.len().min(buf.len());
        buf[..n].fill(0);
        Ok(n)
    }

    fn spi_close(&mut self, _index: u32) {}

    // --- assets: the slot image in memory ---

    fn assets_read(&mut self, offset: u32, buf: &mut [u8]) -> BoardResult<()> {
        let at = offset as usize;
        let src = at
            .checked_add(buf.len())
            .and_then(|end| self.assets.get(at..end))
            .ok_or(ErrorCode::Io)?;
        buf.copy_from_slice(src);
        Ok(())
    }

    // --- time ---

    fn display_supported(&self) -> bool {
        self.display.is_some()
    }

    fn display_size(&self, index: u32) -> Option<(u32, u32)> {
        let d = self.display.as_ref().filter(|_| index == 0)?;
        Some((d.drawn.width, d.drawn.height))
    }

    fn display_fill_rect(&mut self, _index: u32, rect: Rect, color: u16) -> BoardResult<()> {
        let d = self.display.as_mut().ok_or(ErrorCode::Unsupported)?;
        d.each(rect, |p, _| *p = color);
        Ok(())
    }

    fn display_write_rect(&mut self, _index: u32, rect: Rect, pixels: &[u8]) -> BoardResult<()> {
        let d = self.display.as_mut().ok_or(ErrorCode::Unsupported)?;
        // High byte first (docs/device-layer.md §4).
        d.each(rect, |p, i| {
            *p = u16::from_be_bytes([pixels[2 * i], pixels[2 * i + 1]])
        });
        Ok(())
    }

    fn display_present(&mut self, _index: u32) -> BoardResult<()> {
        let d = self.display.as_mut().ok_or(ErrorCode::Unsupported)?;
        d.visible.pixels.copy_from_slice(&d.drawn.pixels);
        Ok(())
    }

    fn now_us(&mut self) -> u64 {
        self.start.elapsed().as_micros() as u64
    }

    fn sleep_ms(&mut self, ms: u32) {
        std::thread::sleep(std::time::Duration::from_millis(u64::from(ms)));
    }

    fn sleep_us(&mut self, us: u32) {
        std::thread::sleep(std::time::Duration::from_micros(u64::from(us)));
    }

    // --- output ---

    fn log(&mut self, _level: LogLevel, message: &[u8]) {
        // docs/abi-spec.md §4.2: UTF-8 is not validated.
        println!("[wasm] {}", String::from_utf8_lossy(message));
    }

    fn trace(&mut self, line: &[u8]) {
        self.trace.push_str(&String::from_utf8_lossy(line));
        self.trace.push('\n');
    }
}
