//! `Hal::release_all` が abi-spec §5.2 を満たすことを固定する。
//!
//! §5.2 は「`run` から戻ったとき、ホストは残っている全ハンドルを drop する」と
//! 定めているが、**この掃除は長く未実装だった**（ゲスト側の `Drop` が隠していた）。
//! ローダが入ると次のアプリが `busy` を踏むので、ここで pin しておく。
//!
//! `ports/common` は `no_std` なので、テストは統合テスト（別クレート）に置く。
//! `runtime` が `tests/` を使っているのと同じ形。

use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::i2c::Speed;
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_core::generated::{self, ErrorCode};
use wasmicon_core::instance::Resolver;
use wasmicon_port::{Board, BoardResult, Hal};

/// 解放されたものを順番どおりに記録するだけのボード。
#[derive(Default)]
struct RecordingBoard {
    released: Vec<String>,
}

impl Board for RecordingBoard {
    fn pin_by_role(&self, role: &str) -> Option<u32> {
        match role {
            "led" => Some(2),
            _ => None,
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

    fn gpio_release(&mut self, index: u32) {
        self.released.push(format!("gpio {index}"));
    }

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

    fn i2c_close(&mut self, index: u32) {
        self.released.push(format!("i2c {index}"));
    }

    fn spi_open(&mut self, _index: u32, _frequency_hz: u32, _mode: SpiMode) -> BoardResult<()> {
        Ok(())
    }

    fn spi_write(&mut self, _index: u32, _data: &[u8]) -> BoardResult<()> {
        Ok(())
    }

    fn spi_transfer(&mut self, _index: u32, _data: &[u8], _buf: &mut [u8]) -> BoardResult<usize> {
        Err(ErrorCode::Unsupported)
    }

    fn spi_close(&mut self, index: u32) {
        self.released.push(format!("spi {index}"));
    }

    fn now_us(&mut self) -> u64 {
        0
    }

    fn sleep_ms(&mut self, _ms: u32) {}

    fn sleep_us(&mut self, _us: u32) {}

    fn log(&mut self, _level: generated::log::Level, _message: &[u8]) {}

    fn trace(&mut self, line: &[u8]) {
        // トレースが出たら分かるように同じ列に記録する
        // （release_all は 1 行も出してはいけない）。
        self.released
            .push(format!("trace {}", String::from_utf8_lossy(line)));
    }
}

/// `call` の失敗はテストの失敗。`wasmicon_core::Error` は `Debug` を実装しない
/// （コアで `core::fmt` を使わないため。CLAUDE.md）のでそのまま `expect` できない。
/// 出し方は `ports/host/tests/apps.rs` に揃える。
fn expect_ok(r: wasmicon_core::error::Result<()>, what: &str) {
    if let Err(e) = r {
        panic!("{what} に失敗: {} [{}]", e.reason(), e.kind().name());
    }
}

/// import 名から host 関数の番号を引く。
fn host(module: &str, name: &str) -> u32 {
    u32::from(
        generated::resolve(module, name)
            .expect("import 表に無い")
            .host_fn
            .index(),
    )
}

const GPIO: &str = "wasmicon:hal/gpio@0.1.0";
const I2C: &str = "wasmicon:hal/i2c@0.1.0";
const SPI: &str = "wasmicon:hal/spi@0.1.0";

/// gpio / i2c / spi を 1 つずつ開く。out ポインタの行き先はゲストメモリ。
fn open_one_of_each(hal: &mut Hal<RecordingBoard>, mem: &mut [u8]) {
    let mut out = [0u64; 1];

    // gpio.pin.open(index = 5, mode = output, out-handle = 0)
    expect_ok(
        hal.call(host(GPIO, "[static]pin.open"), &[5, 1, 0], &mut out, mem),
        "pin.open",
    );
    assert_eq!(out[0], 0, "pin.open が成功する");

    // i2c.bus.open(index = 0, speed = standard, out-handle = 8)
    expect_ok(
        hal.call(host(I2C, "[static]bus.open"), &[0, 0, 8], &mut out, mem),
        "i2c bus.open",
    );
    assert_eq!(out[0], 0, "i2c bus.open が成功する");

    // spi.bus.open(index = 0, frequency-hz = 1 MHz, mode = 0, out-handle = 16)
    expect_ok(
        hal.call(
            host(SPI, "[static]bus.open"),
            &[0, 1_000_000, 0, 16],
            &mut out,
            mem,
        ),
        "spi bus.open",
    );
    assert_eq!(out[0], 0, "spi bus.open が成功する");
}

#[test]
fn closes_every_leaked_handle_in_a_fixed_order() {
    let mut mem = vec![0u8; 64 * 1024];
    let mut hal = Hal::new(RecordingBoard::default(), false);

    open_one_of_each(&mut hal, &mut mem);
    assert!(
        hal.board_mut().released.is_empty(),
        "open しただけでは何も解放されない"
    );

    hal.release_all();
    assert_eq!(
        hal.board_mut().released,
        ["gpio 5", "i2c 0", "spi 0"],
        "gpio → i2c → spi の順で解放する（ポート間で揃える）"
    );
}

#[test]
fn is_idempotent() {
    let mut mem = vec![0u8; 64 * 1024];
    let mut hal = Hal::new(RecordingBoard::default(), false);

    open_one_of_each(&mut hal, &mut mem);
    hal.release_all();
    hal.board_mut().released.clear();

    // 2 度目は何もしない。ゲストが自分で drop したあとの経路と同じ。
    hal.release_all();
    assert!(hal.board_mut().released.is_empty());
}

#[test]
fn emits_no_trace_lines() {
    let mut mem = vec![0u8; 64 * 1024];
    // trace_on = true。記録済みのトレース（14,352 行）を変えてはいけない。
    let mut hal = Hal::new(RecordingBoard::default(), true);

    open_one_of_each(&mut hal, &mut mem);
    assert!(
        hal.board_mut()
            .released
            .iter()
            .any(|l| l.starts_with("trace")),
        "open の方はトレースを出す（この検査自体が効いていることの確認）"
    );
    hal.board_mut().released.clear();

    hal.release_all();
    assert_eq!(
        hal.board_mut().released,
        ["gpio 5", "i2c 0", "spi 0"],
        "release_all は board を直接叩くだけで、トレース行を出さない"
    );
}

#[test]
fn reopening_after_release_all_succeeds() {
    let mut mem = vec![0u8; 64 * 1024];
    let mut hal = Hal::new(RecordingBoard::default(), false);
    let mut out = [0u64; 1];

    open_one_of_each(&mut hal, &mut mem);

    // 掃除する前は二重 open なので busy（abi-spec §5.2）。
    expect_ok(
        hal.call(
            host(I2C, "[static]bus.open"),
            &[0, 0, 8],
            &mut out,
            &mut mem,
        ),
        "二重 open の呼び出し",
    );
    assert_eq!(
        out[0],
        u64::from(ErrorCode::Busy.status()),
        "掃除前は busy が返る"
    );

    // 掃除すれば次のアプリが開ける。ローダがこれに依存する。
    hal.release_all();
    expect_ok(
        hal.call(
            host(I2C, "[static]bus.open"),
            &[0, 0, 8],
            &mut out,
            &mut mem,
        ),
        "掃除後の i2c bus.open",
    );
    assert_eq!(out[0], 0, "掃除後は成功する");
}
