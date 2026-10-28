//! A board's own display comes up at the guest's first `open`, through
//! `Board::display_open` (the Tab5 panel, docs/device-layer.md §5.3), and a
//! failed bring-up is the `open`'s error.

use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::i2c::Speed;
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_core::generated::{self, ErrorCode};
use wasmicon_core::instance::Resolver;
use wasmicon_port::{Board, BoardResult, Hal};

/// A board with one 4x4 display that counts its bring-ups.
struct PanelBoard {
    opens: u32,
    fail: bool,
}

impl Board for PanelBoard {
    fn display_supported(&self) -> bool {
        true
    }

    fn display_size(&self, index: u32) -> Option<(u32, u32)> {
        (index == 0).then_some((4, 4))
    }

    fn display_open(&mut self, _index: u32) -> BoardResult<()> {
        self.opens += 1;
        if self.fail {
            Err(ErrorCode::Io)
        } else {
            Ok(())
        }
    }

    fn gpio_count(&self) -> u32 {
        32
    }

    fn gpio_configure(&mut self, _index: u32, _mode: PinMode) -> BoardResult<()> {
        Ok(())
    }

    fn gpio_write(&mut self, _index: u32, _level: Level) -> BoardResult<()> {
        Ok(())
    }

    fn gpio_read(&mut self, _index: u32) -> BoardResult<Level> {
        Ok(Level::Low)
    }

    fn gpio_release(&mut self, _index: u32) {}

    fn i2c_open(&mut self, _index: u32, _speed: Speed) -> BoardResult<()> {
        Ok(())
    }

    fn i2c_write(&mut self, _index: u32, _address: u16, _data: &[u8]) -> BoardResult<()> {
        Ok(())
    }

    fn i2c_read(&mut self, _index: u32, _address: u16, _buf: &mut [u8]) -> BoardResult<usize> {
        Err(ErrorCode::Nack)
    }

    fn i2c_write_read(
        &mut self,
        _index: u32,
        _address: u16,
        _data: &[u8],
        _buf: &mut [u8],
    ) -> BoardResult<usize> {
        Err(ErrorCode::Nack)
    }

    fn i2c_close(&mut self, _index: u32) {}

    fn spi_open(&mut self, _index: u32, _frequency_hz: u32, _mode: SpiMode) -> BoardResult<()> {
        Ok(())
    }

    fn spi_write(&mut self, _index: u32, _data: &[u8]) -> BoardResult<()> {
        Ok(())
    }

    fn spi_transfer(&mut self, _index: u32, _data: &[u8], _buf: &mut [u8]) -> BoardResult<usize> {
        Ok(0)
    }

    fn spi_close(&mut self, _index: u32) {}

    fn now_us(&mut self) -> u64 {
        0
    }

    fn sleep_ms(&mut self, _ms: u32) {}

    fn sleep_us(&mut self, _us: u32) {}

    fn log(&mut self, _level: generated::log::Level, _message: &[u8]) {}

    fn trace(&mut self, _line: &[u8]) {}
}

const DISPLAY: &str = "wasmicon:device/display@0.1.0";

fn host(name: &str) -> u32 {
    u32::from(
        generated::resolve(DISPLAY, name)
            .expect("not in the import table")
            .host_fn
            .index(),
    )
}

/// `screen.open(0)` with the handle written at 0; returns the status.
fn open(hal: &mut Hal<PanelBoard>, mem: &mut [u8]) -> u64 {
    let mut out = [0u64; 1];
    if let Err(e) = hal.call(host("[static]screen.open"), &[0, 0], &mut out, mem) {
        panic!("screen.open failed: {}", e.reason());
    }
    out[0]
}

fn drop_screen(hal: &mut Hal<PanelBoard>, mem: &mut [u8]) {
    let mut out = [0u64; 0];
    if let Err(e) = hal.call(host("[resource-drop]screen"), &[1], &mut out, mem) {
        panic!("screen drop failed: {}", e.reason());
    }
}

#[test]
fn the_board_brings_its_display_up_at_each_open() {
    let mut hal = Hal::new(
        PanelBoard {
            opens: 0,
            fail: false,
        },
        false,
    );
    let mut mem = [0u8; 64];
    assert_eq!(hal.board_mut().opens, 0, "nothing comes up before an open");
    assert_eq!(open(&mut hal, &mut mem), 0);
    assert_eq!(hal.board_mut().opens, 1);
    drop_screen(&mut hal, &mut mem);
    // Called again; the board returns at once when it is already up.
    assert_eq!(open(&mut hal, &mut mem), 0);
    assert_eq!(hal.board_mut().opens, 2);
}

#[test]
fn a_failed_bring_up_is_the_open_error() {
    let mut hal = Hal::new(
        PanelBoard {
            opens: 0,
            fail: true,
        },
        false,
    );
    let mut mem = [0u8; 64];
    assert_eq!(open(&mut hal, &mut mem), u64::from(ErrorCode::Io.status()));
    // No handle came of it: the next open is not `busy`.
    assert_eq!(open(&mut hal, &mut mem), u64::from(ErrorCode::Io.status()));
}

#[test]
fn an_open_refused_as_busy_does_not_reach_the_board() {
    let mut hal = Hal::new(
        PanelBoard {
            opens: 0,
            fail: false,
        },
        false,
    );
    let mut mem = [0u8; 64];
    assert_eq!(open(&mut hal, &mut mem), 0);
    assert_eq!(
        open(&mut hal, &mut mem),
        u64::from(ErrorCode::Busy.status())
    );
    assert_eq!(hal.board_mut().opens, 1);
}
