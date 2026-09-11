//! Wasmicon の ESP32-P4 ポート。
//!
//! 3 層に分かれている。**ボードが増えても変わるのは `boards/` だけ**:
//!
//! - `chip`   — P4 のレジスタ操作と時間。ボードに依存しない
//! - `boards` — どのピンが何に繋がっているか。feature でちょうど 1 つ選ぶ
//! - `board`  — 上の 2 つを繋いで `wasmicon_port::Board` を実装する
//!
//! ゲストの `.wasm` はフラッシュに埋め込み、XIP 上のスライスをそのまま
//! ランタイムに渡す（RAM にコピーしない。design-notes §4）。
//! トレースの出力先はボード定義が決める（既定の Tab5 は UART0 115200 8N1）。
//!
//! **実機で動作確認していない**（配線とシリアル接続が未確認。docs/TODO.md §1.1）。
//! ビルドが通ることまでを確認した段階。
//!
//! ESP32-P4 は RISC-V (RV32IMAFC) なので upstream Rust でそのまま組める:
//! `cd ports/esp32p4 && cargo build --release`

#![no_std]
#![no_main]

mod board;
mod boards;
mod chip;

use wasmicon_core::{decode, instantiate, invoke, validate, Arena, Config, Exec};
use wasmicon_port::Hal;

use board::EspBoard;
use boards::{Serial, DEF};

// espflash が焼く前に検査する ESP-IDF のアプリ記述子。無いと書き込みを拒否される
// （ビルドは通るので、実機に焼くまで気づかない）。
esp_bootloader_esp_idf::esp_app_desc!();

/// ゲスト。`cd apps && cargo build --release` を先に実行しておく。
static GUEST: &[u8] =
    include_bytes!("../../../apps/target/wasm32-unknown-unknown/release/blink_rs.wasm");

/// ランタイムの arena。残りが線形メモリになる（`Arena::alloc_rest`）。
/// P4 は L2MEM 768 KB、さらに Tab5 の ESP32-P4NRW32 は 32 MB の PSRAM を積んで
/// いるので余裕は大きいが、**ESP32-S3 版と同じ大きさにしてある**。ボード間で
/// 挙動を揃えるのがこのプロジェクトの目的で、上限を広げても得るものが無い。
/// PSRAM は**意図的に使っていない**（使うなら esp-hal 側の初期化が要る）。
static mut ARENA: [u8; 300 * 1024] = [0; 300 * 1024];

/// 検証中だけ使う作業領域。`MCU_CONFIG` の上限に合わせてある。
static mut SCRATCH: [u8; 8 * 1024] = [0; 8 * 1024];

/// マイコン向けの上限。ホストの既定値のままだと scratch が足りない。
/// ESP32-S3 版と同一（上のコメントの通り、意図的に揃えている）。
const MCU_CONFIG: Config = Config {
    max_value_stack: 512,
    max_control_depth: 64,
    max_locals: 256,
    max_memory_pages: 4,
    max_table_elems: 256,
    max_call_depth: 32,
    operand_stack_slots: 1024,
};

#[esp_hal::main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default());

    let mut serial = boards::open_serial(p);
    serial.write(b"wasmicon ");
    serial.write(DEF.name.as_bytes());
    serial.write(b"\r\n");

    // SAFETY: EspBoard がこれ以降 GPIO / IO_MUX を排他的に使う。UART が占有する
    // ピンは、ボード定義の `reserved` でゲストから閉じてある。
    let board = unsafe { EspBoard::new(serial) };
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
    hal: &mut Hal<EspBoard<boards::Trace>>,
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

/// docs/handoff.md §3 #4: ログを出して停止する。再起動はしない。
///
/// ランタイム由来の失敗は `main` が捕まえて出すので、ここに来るのはポート自身か
/// esp-hal のバグに限られる。**その場所が分からないと実機では手も足も出ない**ので、
/// USB_DEVICE を奪い直して `panic at <file>:<line>` を出す。
///
/// `core::fmt` は使わない（バイナリが肥大するため）。行番号は手で 10 進に直す。
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // SAFETY: 以降は停止するだけなので、USB_DEVICE を二重に触っても競合しない。
    let mut out = unsafe { boards::steal_serial() };
    out.write(b"\r\npanic");
    if let Some(loc) = info.location() {
        out.write(b" at ");
        out.write(loc.file().as_bytes());
        out.write(b":");
        let mut buf = [0u8; 10];
        let mut n = loc.line();
        let mut i = buf.len();
        loop {
            i -= 1;
            buf[i] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 || i == 0 {
                break;
            }
        }
        out.write(&buf[i..]);
    }
    out.write(b"\r\n");
    loop {
        core::hint::spin_loop();
    }
}
