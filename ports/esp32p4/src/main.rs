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
//! トレースの出力先はボード定義が決める（Tab5 は UART0 115200 8N1）。
//!
//! **2026-09-21 に Tab5 実機で blink が完走した。** トレースは UART0 (G37) から
//! 3.3V USB シリアル変換で取り込む。`verify/diff-traces.sh` でホストの出力と
//! **差分ゼロ**（20 行一致）を確認済み。
//!
//! この個体は **P4 v1.0 / ROM esp32p4-eco2 / 使える L2MEM は 640KB** で、
//! esp-hal 1.2 が前提にしている v3.x / ECO5 / 768KB と噛み合わない。
//! そのための回避が 4 つ入っている（すべて上流のバグ回避であって好みではない）:
//!
//! - `.cargo/config.toml` の `ESP_HAL_CONFIG_MIN_CHIP_REVISION`（書き込み拒否）
//! - `vendor/esp-sync`（CSR 0x347 が不正命令）
//! - `rom-pre-eco5.x` の ROM アドレス表 + `src/mem.rs`（64bit シフトと memcpy 系）
//! - `rom-pre-eco5.x` の `_stack_start`（スタックが実在しない RAM に置かれる）
//!
//! 詳細と削除条件は docs/TODO.md §1.1.5。
//!
//! ESP32-P4 は RISC-V (RV32IMAFC) なので upstream Rust でそのまま組める:
//! `cd ports/esp32p4 && cargo build --release`

#![no_std]
#![no_main]

mod board;
mod boards;
mod chip;
mod mem;

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
    // **`esp_hal::init` より前**に出す。ブートローダが設定した UART0 へ生で
    // 書くので、クロックにもドライバにも依存しない。ここまで出れば「アプリに
    // 制御が渡った」ことが確定し、以降のどこで止まっても切り分けられる。
    chip::early_write(b"\r\nwasmicon: entry\r\n");

    let p = esp_hal::init(esp_hal::Config::default());

    let hw = boards::open(p);
    let mut serial = hw.serial;
    serial.write(b"wasmicon ");
    serial.write(DEF.name.as_bytes());
    serial.write(b"\r\n");

    // SAFETY: EspBoard がこれ以降 GPIO / IO_MUX を排他的に使う。UART が占有する
    // ピンは、ボード定義の `reserved` でゲストから閉じてある。
    let board = unsafe { EspBoard::new(serial, hw.i2c_porta) };
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
    let entry = inst
        .export_func("run")
        .ok_or(wasmicon_core::Error::Unlinkable("export run が無い"))?;
    invoke(&mut inst, &mut exec, hal, entry, &[], &mut [])
}

/// docs/handoff.md §3 #4: ログを出して停止する。再起動はしない。
///
/// ランタイム由来の失敗は `main` が捕まえて出すので、ここに来るのはポート自身か
/// esp-hal のバグに限られる。**その場所が分からないと実機では手も足も出ない**ので、
/// `chip::early_write` で `panic at <file>:<line>` を出す。
///
/// **ドライバを作り直さない。** 以前はここで UART を組み立てていたが、
/// `esp_hal::init` の中で落ちた場合は panic handler 自身がクロック未設定の
/// まま UART 生成に入って止まり、**panic が一切見えなかった**（2026-09-21、
/// Tab5）。生レジスタ書き込みなら初期化状態に依存しない。
///
/// **ここでだけ `core::fmt` を使う。** 規約が禁じているのは `runtime/` コアで、
/// ポートのこの経路は別。しかも esp-hal の例外ハンドラ自身が `panic!` に
/// フォーマット引数（例外コード・`mepc`・`mtval`・レジスタ）を渡しているので、
/// fmt の機構はどのみちリンクされる。**実機では例外コードと `mepc` が読めないと
/// 何も分からない**ので、場所だけでなくメッセージ全文を出す。
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write;
    let _ = write!(EarlyOut, "\r\npanic: {info}\r\n");
    loop {
        core::hint::spin_loop();
    }
}

/// `chip::early_write` を `core::fmt` の出力先にする。panic 経路専用。
struct EarlyOut;

impl core::fmt::Write for EarlyOut {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        chip::early_write(s.as_bytes());
        Ok(())
    }
}
