//! The XPT2046 backend (docs/device-layer.md §11.4): the mapping from raw
//! readings to pixels, and a read's bus traffic on a board that answers
//! like an XPT2046.

use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::i2c::Speed;
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_core::generated::{self, ErrorCode};
use wasmicon_port::xpt2046::{self, Calibration, Xpt2046};
use wasmicon_port::{Board, BoardResult, ili9341};

const CAL: Calibration = Calibration {
    x_min: 0,
    x_max: 239,
    y_min: 0,
    y_max: 319,
};

#[test]
fn the_rotation_lays_points_out_as_madctl_lays_the_picture_out() {
    // Rotation 0 is the calibrated pixel; the others turn it with the panel,
    // so the four corners of the panel stay the four corners.
    let corners = [(0, 0), (239, 0), (0, 319), (239, 319)];
    let at = |rotation| -> Vec<(u16, u16)> {
        corners
            .iter()
            .map(|&(x, y)| xpt2046::to_pixels(x, y, &CAL, rotation))
            .collect()
    };
    assert_eq!(at(0), [(0, 0), (239, 0), (0, 319), (239, 319)]);
    assert_eq!(at(90), [(0, 239), (0, 0), (319, 239), (319, 0)]);
    assert_eq!(at(180), [(239, 319), (0, 319), (239, 0), (0, 0)]);
    assert_eq!(at(270), [(319, 0), (319, 239), (0, 0), (0, 239)]);
}

#[test]
fn the_calibration_scales_flips_and_clamps() {
    let cal = Calibration {
        x_min: 3900,
        x_max: 200,
        y_min: 200,
        y_max: 3900,
    };
    assert_eq!(xpt2046::to_pixels(3900, 200, &cal, 0), (0, 0));
    assert_eq!(xpt2046::to_pixels(200, 3900, &cal, 0), (239, 319));
    assert_eq!(xpt2046::to_pixels(2050, 2050, &cal, 0), (119, 159));
    // Past the edges: clamped to the panel.
    assert_eq!(xpt2046::to_pixels(4095, 0, &cal, 0), (0, 0));
    assert_eq!(xpt2046::to_pixels(0, 4095, &cal, 0), (239, 319));
}

#[derive(Debug, PartialEq, Eq)]
enum Ev {
    Open(u32),
    Close,
    Cs(bool),
    Transfer(u8),
}

/// A board whose SPI answers each control byte as an XPT2046 pressed at
/// `x`, `y` (or lifted), with PENIRQ on GPIO17.
struct TouchBoard {
    pressed: bool,
    /// PENIRQ stays high under the finger.
    irq_high_while_pressed: bool,
    /// MISO stuck at this byte: every reply byte, the BUSY bit included.
    miso: Option<u8>,
    /// What Z1 and Z2 read.
    z: (u16, u16),
    x: u16,
    y: u16,
    log: Vec<Ev>,
}

impl Board for TouchBoard {
    fn gpio_count(&self) -> u32 {
        32
    }

    fn gpio_configure(&mut self, _index: u32, _mode: PinMode) -> BoardResult<()> {
        Ok(())
    }

    fn gpio_write(&mut self, index: u32, level: Level) -> BoardResult<()> {
        if index == 16 {
            self.log.push(Ev::Cs(level == Level::High));
        }
        Ok(())
    }

    fn gpio_read(&mut self, _index: u32) -> BoardResult<Level> {
        Ok(if self.pressed && !self.irq_high_while_pressed {
            Level::Low
        } else {
            Level::High
        })
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

    fn spi_open(&mut self, _index: u32, frequency_hz: u32, _mode: SpiMode) -> BoardResult<()> {
        self.log.push(Ev::Open(frequency_hz));
        Ok(())
    }

    fn spi_write(&mut self, _index: u32, _data: &[u8]) -> BoardResult<()> {
        Ok(())
    }

    fn spi_transfer(&mut self, _index: u32, data: &[u8], buf: &mut [u8]) -> BoardResult<usize> {
        self.log.push(Ev::Transfer(data[0]));
        if let Some(m) = self.miso {
            buf[..3].fill(m);
            return Ok(3);
        }
        let v: u16 = match data[0] {
            0xD0 => self.x,
            0x90 => self.y,
            // Lifted, Z1 reads 0.
            0xB0 if !self.pressed => 0,
            0xB0 => self.z.0,
            0xC0 => self.z.1,
            _ => 0,
        };
        let v = v << 3;
        buf[..3].copy_from_slice(&[0, (v >> 8) as u8, v as u8]);
        Ok(3)
    }

    fn spi_close(&mut self, _index: u32) {
        self.log.push(Ev::Close);
    }

    fn now_us(&mut self) -> u64 {
        0
    }

    fn sleep_ms(&mut self, _ms: u32) {}

    fn sleep_us(&mut self, _us: u32) {}

    fn log(&mut self, _level: generated::log::Level, _message: &[u8]) {}

    fn trace(&mut self, _line: &[u8]) {}
}

#[test]
fn a_read_samples_at_the_slow_clock_and_gives_the_bus_back() {
    let mut b = TouchBoard {
        pressed: true,
        irq_high_while_pressed: false,
        miso: None,
        z: (1000, 2000),
        x: 100,
        y: 200,
        log: Vec::new(),
    };
    let mut t = Xpt2046::new(0, 16, Some(17), CAL, 0);
    assert!(t.init(&mut b).is_ok());
    // `init` enables PENIRQ with one conversion at the slow clock.
    assert_eq!(
        b.log,
        [
            Ev::Cs(true),
            Ev::Close,
            Ev::Open(xpt2046::SPI_HZ),
            Ev::Cs(false),
            Ev::Transfer(0xB0),
            Ev::Cs(true),
            Ev::Close,
            Ev::Open(ili9341::SPI_HZ),
        ]
    );
    b.log.clear();
    let mut buf = [0u8; 32];
    assert_eq!(t.read(&mut b, &mut buf).unwrap_or(99), 1);
    assert_eq!(&buf[..4], &[0, 100, 0, 200]);

    // Pressure first, then the samples, then the pressure again, all inside
    // one chip select at the slow clock; the panel's clock comes back.
    let mut want = vec![
        Ev::Close,
        Ev::Open(xpt2046::SPI_HZ),
        Ev::Cs(false),
        Ev::Transfer(0xB0),
        Ev::Transfer(0xC0),
    ];
    for _ in 0..5 {
        want.extend([Ev::Transfer(0xD0), Ev::Transfer(0x90)]);
    }
    want.extend([
        Ev::Transfer(0xB0),
        Ev::Transfer(0xC0),
        Ev::Cs(true),
        Ev::Close,
        Ev::Open(ili9341::SPI_HZ),
    ]);
    assert_eq!(b.log, want);

    // Lifted: the read right after a touch samples and sees it. Then PENIRQ
    // high leaves the bus alone, except that every 16th such read samples
    // anyway, in case PENIRQ has stopped working.
    b.pressed = false;
    assert_eq!(t.read(&mut b, &mut buf).unwrap_or(99), 0);
    b.log.clear();
    for _ in 0..15 {
        assert_eq!(t.read(&mut b, &mut buf).unwrap_or(99), 0);
    }
    assert!(b.log.is_empty(), "{:?}", b.log);
    b.irq_high_while_pressed = true;
    b.pressed = true;
    assert_eq!(t.read(&mut b, &mut buf).unwrap_or(99), 1);
    assert_eq!(b.log[1], Ev::Open(xpt2046::SPI_HZ));
}

#[test]
fn a_touch_the_idle_sample_finds_is_followed_without_penirq() {
    // PENIRQ stuck high under the finger: the 16th read finds the touch,
    // and every read after it samples until the finger lifts.
    let mut b = TouchBoard {
        pressed: true,
        irq_high_while_pressed: true,
        miso: None,
        z: (1000, 2000),
        x: 100,
        y: 200,
        log: Vec::new(),
    };
    let mut t = Xpt2046::new(0, 16, Some(17), CAL, 0);
    assert!(t.init(&mut b).is_ok());
    b.log.clear();
    let mut buf = [0u8; 4];
    for _ in 0..15 {
        assert_eq!(t.read(&mut b, &mut buf).unwrap_or(99), 0);
    }
    assert!(b.log.is_empty(), "{:?}", b.log);
    for _ in 0..3 {
        assert_eq!(t.read(&mut b, &mut buf).unwrap_or(99), 1);
    }
    // Lifted: one more sample sees it, then the bus is left alone again.
    b.pressed = false;
    assert_eq!(t.read(&mut b, &mut buf).unwrap_or(99), 0);
    b.log.clear();
    assert_eq!(t.read(&mut b, &mut buf).unwrap_or(99), 0);
    assert!(b.log.is_empty(), "{:?}", b.log);
}

#[test]
fn a_stuck_bus_is_no_touch() {
    // MISO held low reads every conversion as 0, PENIRQ low or not.
    let mut b = TouchBoard {
        pressed: true,
        irq_high_while_pressed: false,
        miso: Some(0),
        z: (1000, 2000),
        x: 100,
        y: 200,
        log: Vec::new(),
    };
    let mut t = Xpt2046::new(0, 16, Some(17), CAL, 0);
    assert!(t.init(&mut b).is_ok());
    let mut buf = [0u8; 4];
    assert_eq!(t.read(&mut b, &mut buf).unwrap_or(99), 0);
    // Held high it reads 4095 everywhere, the BUSY bit masked off: no touch
    // either.
    b.miso = Some(0xFF);
    assert_eq!(t.read(&mut b, &mut buf).unwrap_or(99), 0);
}
