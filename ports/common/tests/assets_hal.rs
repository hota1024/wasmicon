//! Locks in `wasmicon:hal/assets@0.1.0` as `Hal` implements it (docs/abi-spec.md §7, §9).
//!
//! A board that serves an assets slot image from memory stands in for flash,
//! so the handle table, the checks, the once-only body CRC and the trace
//! lines are tested without hardware. Every port runs this same code.

use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::i2c::Speed;
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_core::generated::{self, ErrorCode};
use wasmicon_core::instance::Resolver;
use wasmicon_port::assets::{self, DIR_LEN, Directory};
use wasmicon_port::{ASSETS_CHUNK, Board, BoardResult, Hal, MAX_BLOBS, crc32};

const ASSETS: &str = "wasmicon:hal/assets@0.1.0";
const GPIO: &str = "wasmicon:hal/gpio@0.1.0";

/// A board whose assets slot is a byte vector. Records every slot read.
#[derive(Default)]
struct MemoryBoard {
    slot: Vec<u8>,
    reads: Vec<(u32, usize)>,
    /// Makes every slot read fail with `io` (a flash fault, not bad data).
    fail_reads: bool,
    trace: Vec<String>,
}

impl Board for MemoryBoard {
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
        Err(ErrorCode::Unsupported)
    }

    fn i2c_write(&mut self, _index: u32, _address: u16, _data: &[u8]) -> BoardResult<()> {
        Err(ErrorCode::Unsupported)
    }

    fn i2c_read(&mut self, _index: u32, _address: u16, _buf: &mut [u8]) -> BoardResult<usize> {
        Err(ErrorCode::Unsupported)
    }

    fn i2c_write_read(
        &mut self,
        _index: u32,
        _address: u16,
        _data: &[u8],
        _buf: &mut [u8],
    ) -> BoardResult<usize> {
        Err(ErrorCode::Unsupported)
    }

    fn i2c_close(&mut self, _index: u32) {}

    fn spi_open(&mut self, _index: u32, _frequency_hz: u32, _mode: SpiMode) -> BoardResult<()> {
        Err(ErrorCode::Unsupported)
    }

    fn spi_write(&mut self, _index: u32, _data: &[u8]) -> BoardResult<()> {
        Err(ErrorCode::Unsupported)
    }

    fn spi_transfer(&mut self, _index: u32, _data: &[u8], _buf: &mut [u8]) -> BoardResult<usize> {
        Err(ErrorCode::Unsupported)
    }

    fn spi_close(&mut self, _index: u32) {}

    fn now_us(&mut self) -> u64 {
        0
    }

    fn sleep_ms(&mut self, _ms: u32) {}

    fn sleep_us(&mut self, _us: u32) {}

    fn log(&mut self, _level: generated::log::Level, _message: &[u8]) {}

    fn trace(&mut self, line: &[u8]) {
        self.trace.push(String::from_utf8_lossy(line).into_owned());
    }

    fn assets_read(&mut self, offset: u32, buf: &mut [u8]) -> BoardResult<()> {
        self.reads.push((offset, buf.len()));
        if self.fail_reads {
            return Err(ErrorCode::Io);
        }
        let at = offset as usize;
        let src = self.slot.get(at..at + buf.len()).ok_or(ErrorCode::Io)?;
        buf.copy_from_slice(src);
        Ok(())
    }
}

/// Builds the slot image as the CLI does and the directory the firmware parses.
fn image(entries: &[(&str, &[u8])]) -> (Vec<u8>, Directory) {
    let mut dir = Directory::EMPTY;
    for (name, body) in entries {
        dir.insert(name.as_bytes(), body.len() as u32, crc32(body))
            .unwrap_or_else(|e| panic!("cannot insert {name}: {}", e.reason()));
    }
    let mut out = vec![0xff; dir.image_len() as usize];
    let mut head = [0u8; DIR_LEN];
    assets::encode(&dir, &mut head);
    out[..DIR_LEN].copy_from_slice(&head);
    for (i, (_, body)) in entries.iter().enumerate() {
        let at = dir.entry(i).offset as usize;
        out[at..at + body.len()].copy_from_slice(body);
    }
    (out, dir)
}

fn hal(entries: &[(&str, &[u8])]) -> Hal<MemoryBoard> {
    let (slot, dir) = image(entries);
    let board = MemoryBoard {
        slot,
        ..MemoryBoard::default()
    };
    Hal::new(board, true).with_assets(dir)
}

fn host(module: &str, name: &str) -> u32 {
    u32::from(
        generated::resolve(module, name)
            .expect("not in the import table")
            .host_fn
            .index(),
    )
}

/// One host call. Returns the status (0 for a function without a result).
fn call(hal: &mut Hal<MemoryBoard>, name: &str, args: &[u64], mem: &mut [u8]) -> u32 {
    let mut out = [0u64; 1];
    let results = if name.starts_with("[resource-drop]") {
        &mut out[..0]
    } else {
        &mut out[..]
    };
    if let Err(e) = hal.call(host(ASSETS, name), args, results, mem) {
        panic!("{name} failed: {} [{}]", e.reason(), e.kind().name());
    }
    out[0] as u32
}

/// `blob.open(name)` with the name placed at guest address 0 and the handle
/// written at 0x100. Returns the status and the handle.
fn open(hal: &mut Hal<MemoryBoard>, name: &str, mem: &mut [u8]) -> (u32, u32) {
    mem[..name.len()].copy_from_slice(name.as_bytes());
    let status = call(
        hal,
        "[static]blob.open",
        &[0, name.len() as u64, 0x100],
        mem,
    );
    (status, u32_at(mem, 0x100))
}

fn u32_at(mem: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([mem[at], mem[at + 1], mem[at + 2], mem[at + 3]])
}

fn status(code: ErrorCode) -> u32 {
    code.status()
}

/// A 1,000-byte body: two CRC chunks, the second a partial one.
fn body() -> Vec<u8> {
    (0..1000u32).map(|i| (i * 7) as u8).collect()
}

#[test]
fn open_len_read_drop_trace_as_documented() {
    let mut mem = vec![0u8; 64 * 1024];
    let mut hal = hal(&[("hello.txt", b"Hello, assets!")]);

    let (st, handle) = open(&mut hal, "hello.txt", &mut mem);
    assert_eq!((st, handle), (0, 1));
    assert_eq!(call(&mut hal, "[method]blob.len", &[1, 0x104], &mut mem), 0);
    assert_eq!(u32_at(&mem, 0x104), 14);

    // read(handle, offset = 7, len = 7, buf = 0x200, cap = 7, len-out = 0x108)
    assert_eq!(
        call(
            &mut hal,
            "[method]blob.read",
            &[1, 7, 7, 0x200, 7, 0x108],
            &mut mem
        ),
        0
    );
    assert_eq!(&mem[0x200..0x207], b"assets!");
    assert_eq!(u32_at(&mem, 0x108), 7, "len-out is the requested length");

    call(&mut hal, "[resource-drop]blob", &[1], &mut mem);
    assert_eq!(
        call(&mut hal, "[method]blob.len", &[1, 0x104], &mut mem),
        status(ErrorCode::InvalidHandle),
        "a dropped handle is invalid"
    );

    assert_eq!(
        hal.board_mut().trace,
        [
            "> wasmicon:hal/assets@0.1.0/[static]blob.open(\"hello.txt\")",
            "< 0 [1]",
            "> wasmicon:hal/assets@0.1.0/[method]blob.len(1)",
            "< 0 [14]",
            "> wasmicon:hal/assets@0.1.0/[method]blob.read(1, 7, 7)",
            "< 0 [len=7 data=0x61737365747321]",
            "> wasmicon:hal/assets@0.1.0/[resource-drop]blob(1)",
            "<",
            "> wasmicon:hal/assets@0.1.0/[method]blob.len(1)",
            "< 2",
        ]
    );
}

#[test]
fn open_checks_in_a_fixed_order() {
    let mut mem = vec![0u8; 64 * 1024];
    let good = body();
    let mut hal = hal(&[("a", &good), ("bad", &good)]);
    // Corrupt the second body after the image was built: the directory (and
    // its CRC) still describes the original.
    let at = hal.board_mut().slot.len() - 4096;
    hal.board_mut().slot[at] ^= 0xff;

    // A bad name is reported before anything else, and no body is read.
    assert_eq!(
        open(&mut hal, "Bad", &mut mem).0,
        status(ErrorCode::InvalidArgument)
    );
    assert_eq!(
        open(&mut hal, "nope", &mut mem).0,
        status(ErrorCode::NotFound)
    );
    assert!(hal.board_mut().reads.is_empty());

    // A corrupt body is read once, in CRC chunks, and then remembered.
    assert_eq!(open(&mut hal, "bad", &mut mem).0, status(ErrorCode::Io));
    assert_eq!(
        hal.board_mut().reads,
        [
            (8192, ASSETS_CHUNK),
            (8192 + ASSETS_CHUNK as u32, 1000 - ASSETS_CHUNK)
        ]
    );
    assert_eq!(open(&mut hal, "bad", &mut mem).0, status(ErrorCode::Io));
    assert_eq!(hal.board_mut().reads.len(), 2, "not read again");

    // A good body is checked once; the same name opens as often as the table allows.
    for expected in 1..=MAX_BLOBS as u32 {
        assert_eq!(open(&mut hal, "a", &mut mem), (0, expected));
    }
    assert_eq!(hal.board_mut().reads.len(), 4, "one CRC pass for `a`");
    assert_eq!(
        open(&mut hal, "a", &mut mem).0,
        status(ErrorCode::OutOfMemory),
        "a fifth blob"
    );
    assert_eq!(hal.board_mut().reads.len(), 4, "a full table reads nothing");
}

#[test]
fn a_read_fault_leaves_the_verdict_open() {
    let mut mem = vec![0u8; 64 * 1024];
    let mut hal = hal(&[("a", &body())]);
    hal.board_mut().fail_reads = true;
    assert_eq!(open(&mut hal, "a", &mut mem).0, status(ErrorCode::Io));
    // The fault was the board's, not the data's: the next open tries again.
    hal.board_mut().fail_reads = false;
    assert_eq!(open(&mut hal, "a", &mut mem), (0, 1));
    assert_eq!(hal.board_mut().reads.len(), 1 + 2);
}

#[test]
fn read_rejects_bad_ranges_without_touching_the_board() {
    let mut mem = vec![0u8; 64 * 1024];
    let mut hal = hal(&[("a", &body())]);
    assert_eq!(open(&mut hal, "a", &mut mem), (0, 1));
    let reads = hal.board_mut().reads.len();
    let bad = status(ErrorCode::InvalidArgument);
    for (what, args) in [
        ("len 0", [1u64, 0, 0, 0x200, 16, 0x108]),
        ("len > cap", [1, 0, 17, 0x200, 16, 0x108]),
        ("past the end", [1, 999, 2, 0x200, 16, 0x108]),
        (
            "offset + len overflows",
            [1, u64::from(u32::MAX), 1, 0x200, 16, 0x108],
        ),
    ] {
        assert_eq!(
            call(&mut hal, "[method]blob.read", &args, &mut mem),
            bad,
            "{what}"
        );
    }
    assert_eq!(
        call(
            &mut hal,
            "[method]blob.read",
            &[2, 0, 1, 0x200, 16, 0x108],
            &mut mem
        ),
        status(ErrorCode::InvalidHandle),
        "a handle that was never opened"
    );
    assert_eq!(hal.board_mut().reads.len(), reads);
    // The last byte is readable.
    assert_eq!(
        call(
            &mut hal,
            "[method]blob.read",
            &[1, 999, 1, 0x200, 16, 0x108],
            &mut mem
        ),
        0
    );
    assert_eq!(mem[0x200], (999u32 * 7) as u8);
}

#[test]
fn read_goes_straight_into_guest_memory() {
    let mut mem = vec![0u8; 64 * 1024];
    let big: Vec<u8> = (0..4096u32).map(|i| (i % 253) as u8).collect();
    let mut hal = hal(&[("big", &big)]);
    assert_eq!(open(&mut hal, "big", &mut mem), (0, 1));
    // Far more than the 128-byte scratch area, in one board read.
    assert_eq!(
        call(
            &mut hal,
            "[method]blob.read",
            &[1, 0, 4096, 0x1000, 4096, 0x108],
            &mut mem
        ),
        0
    );
    assert_eq!(&mem[0x1000..0x2000], &big[..]);
    assert_eq!(u32_at(&mem, 0x108), 4096);
    assert_eq!(hal.board_mut().reads.last(), Some(&(4096, 4096)));
    // Beyond 32 bytes the trace carries 16 bytes and a CRC (docs/abi-spec.md §9).
    let line = hal.board_mut().trace.last().cloned().unwrap_or_default();
    assert_eq!(
        line,
        format!(
            "< 0 [len=4096 data=0x000102030405060708090a0b0c0d0e0f..len=4096 crc32={:08x}]",
            crc32(&big)
        )
    );
}

#[test]
fn read_traps_on_a_guest_range_out_of_bounds() {
    let mut mem = vec![0u8; 64 * 1024];
    let mut hal = hal(&[("a", &body())]);
    assert_eq!(open(&mut hal, "a", &mut mem), (0, 1));
    let reads = hal.board_mut().reads.len();
    let end = mem.len() as u64;
    // The buffer runs past linear memory: a trap, before any board read.
    assert!(
        hal.call(
            host(ASSETS, "[method]blob.read"),
            &[1, 0, 8, end - 4, 8, 0x108],
            &mut [0u64; 1],
            &mut mem
        )
        .is_err()
    );
    assert_eq!(hal.board_mut().reads.len(), reads);
    // So does an out-pointer outside memory.
    assert!(
        hal.call(
            host(ASSETS, "[method]blob.read"),
            &[1, 0, 8, 0x200, 8, end - 2],
            &mut [0u64; 1],
            &mut mem
        )
        .is_err()
    );
}

#[test]
fn drop_of_an_invalid_handle_is_ignored() {
    let mut mem = vec![0u8; 64 * 1024];
    let mut hal = hal(&[("a", &body())]);
    for h in [0, 2, MAX_BLOBS as u64 + 1] {
        call(&mut hal, "[resource-drop]blob", &[h], &mut mem);
    }
    assert_eq!(
        open(&mut hal, "a", &mut mem),
        (0, 1),
        "nothing was disturbed"
    );
    assert_eq!(
        hal.board_mut().trace[..2],
        ["> wasmicon:hal/assets@0.1.0/[resource-drop]blob(0)", "<"]
    );
}

#[test]
fn release_all_frees_the_blobs_and_keeps_the_verdicts() {
    let mut mem = vec![0u8; 64 * 1024];
    let mut hal = hal(&[("a", &body())]);
    assert_eq!(open(&mut hal, "a", &mut mem), (0, 1));
    assert_eq!(open(&mut hal, "a", &mut mem), (0, 2));
    let reads = hal.board_mut().reads.len();
    let lines = hal.board_mut().trace.len();

    hal.release_all();
    assert_eq!(hal.board_mut().trace.len(), lines, "no trace line");
    assert_eq!(
        call(&mut hal, "[method]blob.len", &[1, 0x104], &mut mem),
        status(ErrorCode::InvalidHandle)
    );
    // Handle 1 is free again, and the body is not checked a second time.
    assert_eq!(open(&mut hal, "a", &mut mem), (0, 1));
    assert_eq!(hal.board_mut().reads.len(), reads);
}

#[test]
fn blob_handles_are_independent_from_pin_handles() {
    let mut mem = vec![0u8; 64 * 1024];
    let mut hal = hal(&[("a", &body())]);
    let mut out = [0u64; 1];
    // gpio.pin.open(index = 5, mode = output, out = 0x180)
    if let Err(e) = hal.call(
        host(GPIO, "[static]pin.open"),
        &[5, 1, 0x180],
        &mut out,
        &mut mem,
    ) {
        panic!("pin.open failed: {}", e.reason());
    }
    assert_eq!((out[0], u32_at(&mem, 0x180)), (0, 1));
    assert_eq!(open(&mut hal, "a", &mut mem), (0, 1), "its own table");
}

#[test]
fn without_a_directory_every_name_is_not_found() {
    let mut mem = vec![0u8; 64 * 1024];
    let mut hal = Hal::new(MemoryBoard::default(), true);
    assert_eq!(open(&mut hal, "a", &mut mem).0, status(ErrorCode::NotFound));
    assert!(hal.board_mut().reads.is_empty());
}
