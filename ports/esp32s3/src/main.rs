//! Wasmicon の ESP32-S3 (DevKitC-1) ポート。
//!
//! ゲストの `.wasm` はフラッシュに埋め込み、XIP 上のスライスをそのまま
//! ランタイムに渡す（RAM にコピーしない。design-notes §4）。
//! トレースは UART0 (GPIO43=TX, GPIO44=RX) 115200 8N1 に出す。
//! DevKitC-1 では UART0 が USB シリアル変換に繋がっている。
//!
//! SPI2 は実装済み。I2C はまだ `unsupported`。
//!
//! **実機で動作確認していない**（docs/TODO.md §1.1 の配線とシリアル接続が未確認）。
//! ビルドが通ることまでを確認した段階。`ports/rp2350` と同じゲスト
//! （`--features guest-lcd-demo` で埋め込む `lcd_demo_rs.wasm`）を焼けるように
//! してあるので、可搬性の検証はそのバイナリのトレースを突き合わせて行う
//! （手順は docs/verification-report.md §5.1）。
//!
//! ビルドには espup が入れる Xtensa の GCC が要る:
//! `. ~/export-esp.sh && cargo build --release`

#![no_std]
#![no_main]

mod board;
#[cfg(feature = "hw-probe")]
mod probe;

use esp_hal::uart::{Config as UartConfig, Uart};
use wasmicon_core::{decode, instantiate, invoke, validate, Arena, Config, Exec};
use wasmicon_port::Hal;

use board::{EspBoard, Serial};

// ESP-IDF の 2 段目ブートローダが読む app descriptor をフラッシュの先頭付近に置く。
// 中身はバージョンとビルド日時で、ランタイムからは使わない。espflash 4.5 以降は
// これが無い ELF を受け付けない（RP2350 の `IMAGE_DEF` に当たるもの）。
esp_bootloader_esp_idf::esp_app_desc!();

/// ゲスト。`cd apps && cargo build --release` を先に実行しておく。
///
/// 既定は blink。`--features guest-lcd-demo` で ILI9341 のデモに差し替わる。
/// **`ports/rp2350` と同じファイルを取り込む。** 2 ボードで同一バイナリを
/// 走らせるのが目的なので、ボードごとに別のゲストを作らない。
#[cfg(not(feature = "guest-lcd-demo"))]
static GUEST: &[u8] =
    include_bytes!("../../../apps/target/wasm32-unknown-unknown/release/blink_rs.wasm");
#[cfg(feature = "guest-lcd-demo")]
static GUEST: &[u8] =
    include_bytes!("../../../apps/target/wasm32-unknown-unknown/release/lcd_demo_rs.wasm");

/// ランタイムの arena。残りが線形メモリになる（`Arena::alloc_rest`）。
/// ESP32-S3 は PSRAM 無しで SRAM 512 KB。線形メモリ 4 ページ（256 KB）が
/// 収まる大きさにしてある（docs/handoff.md §5 Phase 4）。
static mut ARENA: [u8; 300 * 1024] = [0; 300 * 1024];

/// 検証中だけ使う作業領域。`MCU_CONFIG` の上限に合わせてある。
static mut SCRATCH: [u8; 8 * 1024] = [0; 8 * 1024];

/// マイコン向けの上限。ホストの既定値のままだと scratch が足りない。
const MCU_CONFIG: Config = Config {
    max_value_stack: 512,
    max_control_depth: 64,
    max_locals: 256,
    max_memory_pages: 4,
    max_table_elems: 256,
    max_call_depth: 32,
    operand_stack_slots: 1024,
};

/// UART0 への出力。
struct SerialPort<'a>(Uart<'a, esp_hal::Blocking>);

impl Serial for SerialPort<'_> {
    fn write(&mut self, bytes: &[u8]) {
        let _ = self.0.write(bytes);
        let _ = self.0.flush();
    }
}

#[esp_hal::main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default());

    let uart = Uart::new(p.UART0, UartConfig::default())
        .unwrap()
        .with_tx(p.GPIO43)
        .with_rx(p.GPIO44);
    let mut serial = SerialPort(uart);
    serial.write(b"wasmicon esp32s3\r\n");

    // SAFETY: EspBoard がこれ以降 GPIO / IO_MUX / SPI2 と SPI2 のピンを排他的に
    // 使う。UART は GPIO43/44 を占有するが、役割名に割り当てた GPIO とも
    // SPI2 のピン (GPIO11/12/13) とも重ねていない。
    // hw-probe のときだけ可変で借りる。
    #[cfg_attr(not(feature = "hw-probe"), allow(unused_mut))]
    let mut board = unsafe { EspBoard::new(serial) };

    // ポート層がパッドまで届いているかの切り分け（src/probe.rs）。
    // ゲストはこの後そのまま走るので、済んだら feature を外すだけでよい。
    #[cfg(feature = "hw-probe")]
    probe::run(&mut board);

    let mut hal = Hal::new(board, cfg!(feature = "trace"));

    // SAFETY: 単一のタスクからしか触らないので、可変静的への参照はここでしか作らない。
    let arena_buf = unsafe { &mut *core::ptr::addr_of_mut!(ARENA) };
    let scratch_buf = unsafe { &mut *core::ptr::addr_of_mut!(SCRATCH) };

    if let Err(e) = run(&mut hal, arena_buf, scratch_buf) {
        // docs/handoff.md §3 #4: ログを出して停止する。再起動はしない。
        let s = hal.board_mut().serial();
        s.write(b"wasmicon: ");
        s.write(e.reason().as_bytes());
        s.write(b" [");
        s.write(e.kind().name().as_bytes());
        s.write(b"]\r\n");
    }
    loop {
        core::hint::spin_loop();
    }
}

/// デコードから `run` の呼び出しまで。
fn run(
    hal: &mut Hal<EspBoard<SerialPort<'static>>>,
    arena_buf: &'static mut [u8],
    scratch_buf: &'static mut [u8],
) -> Result<(), wasmicon_core::Error> {
    let mut arena = Arena::new(arena_buf);
    let mut scratch = Arena::new(scratch_buf);

    let m = decode::decode(GUEST, &mut arena)?;
    let v = validate::validate(&m, &MCU_CONFIG, &mut arena, &mut scratch)?;
    // Exec は線形メモリ（arena の残り全部）より先に確保する。
    let mut exec = Exec::new(&MCU_CONFIG, &mut arena)?;
    let mut inst = instantiate(m, v, &MCU_CONFIG, &mut arena, hal)?;

    if let Some(start) = inst.module.start {
        invoke(&mut inst, &mut exec, hal, start, &[], &mut [])?;
    }
    let entry = inst
        .export_func("run")
        .ok_or(wasmicon_core::Error::Unlinkable("export run が無い"))?;
    invoke(&mut inst, &mut exec, hal, entry, &[], &mut [])
}

/// docs/handoff.md §3 #4 は「ログを出して停止、再起動しない」だが、**理由は出せない**。
///
/// シリアルはボードが持っていて panic handler からは届かないため、今は
/// 停止するだけ。理由を出すには、シリアルのハンドルを panic handler から
/// 触れる場所（critical-section 付きのグローバル）に置く必要がある。
/// ランタイム由来の失敗は `main` が捕まえて UART に出すので、ここに来るのは
/// ポート自身のバグに限られる。
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
