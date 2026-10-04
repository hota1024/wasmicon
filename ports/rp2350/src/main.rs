//! Wasmicon の RP2350 (Raspberry Pi Pico 2 / Pico 2 W) ポート。
//!
//! ゲストの `.wasm` はフラッシュに埋め込み、XIP 上のスライスをそのまま
//! ランタイムに渡す（RAM にコピーしない。design-notes §4）。
//! トレースは UART0 (GP0=TX, GP1=RX) 115200 8N1 に出す。
//!
//! Cortex-M33 側だけを使う（RISC-V の Hazard3 は対象外）。hard-float ABI で
//! 組むので f32 は FPU で回る。FPU は `cortex-m-rt` のリセットハンドラが
//! 有効にし、FPSCR は既定のまま（最近接丸め、flush-to-zero 無効）なので
//! IEEE 準拠。f64 はソフトフロートのまま（DCP は使わない。Cargo.toml 参照）。
//!
//! SPI0 は実装済み。I2C はまだ `unsupported`。
//!
//! **Pico 2 W 実機で確認済み**（2026-09-26）。`lcd-demo-rs` を走らせ、トレースが
//! host ポートと完全一致し、ILI9341 に絵が出た（docs/verification-report.md §6）。
//! `led` の役割名と I2C の配線は未確認のまま（docs/TODO.md §1.1）。

#![no_std]
#![no_main]

mod board;

// panic 時は停止するだけ。理由は出せない（シリアルがボード側にあり
// panic handler から届かない）。ランタイム由来の失敗は main が捕まえて
// UART に出すので、ここに来るのはポート自身のバグに限られる。
use panic_halt as _;
use rp235x_hal as hal;
use rp235x_hal::Clock;
use rp235x_hal::fugit::RateExtU32;
use wasmicon_core::{Arena, Config, Exec, decode, instantiate, invoke, validate};
use wasmicon_port::fmt::Buf;
use wasmicon_port::{Hal, slot};

use board::{Pico2Board, Serial};

/// Boot ROM に「これは実行イメージだ」と伝えるブロック。
/// フラッシュの先頭 4 KB（`.start_block`）に置く必要がある。
/// RP2040 の二段目ブートローダに相当するものは RP2350 には要らない。
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

/// Pico 2 の水晶振動子。
const XTAL_HZ: u32 = 12_000_000;

/// 内蔵のゲスト。**スロットが空のときだけ使う。**
///
/// `cd apps && cargo build --release` を先に実行しておく。既定は blink で、
/// `--features guest-lcd-demo` で ILI9341 のデモに差し替わる。
///
/// **これは撤去する予定**（`docs/app-workflow.md` §3.3 の 2026-10-04 決定）。
/// 3 ポートがスロットを読めるようになったら落とす。今は RP2350 だけが
/// 読めるので、フォールバックとして残してある
/// （スロットの読み出しに不備があっても焼き直しで戻れるように）。
#[cfg(not(feature = "guest-lcd-demo"))]
static BUILTIN: &[u8] =
    include_bytes!("../../../apps/target/wasm32-unknown-unknown/release/blink_rs.wasm");
#[cfg(feature = "guest-lcd-demo")]
static BUILTIN: &[u8] =
    include_bytes!("../../../apps/target/wasm32-unknown-unknown/release/lcd_demo_rs.wasm");

/// XIP の先頭。フラッシュはここから memory-mapped で読める。
const XIP_BASE: usize = 0x1000_0000;

/// アプリスロット（`ports/common` の `profile::RP2350`）。
const SLOT: wasmicon_port::profile::Slot = match wasmicon_port::profile::RP2350.slot {
    Some(s) => s,
    None => panic!("RP2350 のプロファイルにスロットが無い"),
};

// リンカが置く「ファームの末尾」（`memory.x` の `.end_block`）。
// スロットと重なっていないことを起動時に検査する。
unsafe extern "C" {
    static __flash_binary_end: u8;
}

/// ランタイムの arena。残りが線形メモリになる（`Arena::alloc_rest`）。
/// RP2350 の SRAM は 520 KB（512 KB + 4 KB × 2）なので、線形メモリ
/// 4 ページ（256 KB）が収まる大きさにしてある。
static mut ARENA: [u8; wasmicon_port::profile::RP2350.arena] =
    [0; wasmicon_port::profile::RP2350.arena];

/// 検証中だけ使う作業領域。`MCU_CONFIG` の上限に合わせてある。
static mut SCRATCH: [u8; wasmicon_port::profile::RP2350.scratch] =
    [0; wasmicon_port::profile::RP2350.scratch];

/// マイコン向けの上限。**正は `ports/common` の `profile::RP2350.config`**
/// （CLI が同じ表を読む。docs/app-workflow.md §4.3）。
/// ホストの既定値のままだと scratch が足りない。
const MCU_CONFIG: Config = wasmicon_port::profile::RP2350.config;

/// UART0 の型。長いので別名にする。
type Uart0 = hal::uart::UartPeripheral<
    hal::uart::Enabled,
    hal::pac::UART0,
    (
        hal::gpio::Pin<hal::gpio::bank0::Gpio0, hal::gpio::FunctionUart, hal::gpio::PullDown>,
        hal::gpio::Pin<hal::gpio::bank0::Gpio1, hal::gpio::FunctionUart, hal::gpio::PullDown>,
    ),
>;

/// UART0 への出力。
struct Uart(Uart0);

impl Serial for Uart {
    fn write(&mut self, bytes: &[u8]) {
        self.0.write_full_blocking(bytes);
    }
}

#[hal::entry]
fn main() -> ! {
    let mut pac = hal::pac::Peripherals::take().unwrap();
    let mut watchdog = hal::Watchdog::new(pac.WATCHDOG);
    let clocks = hal::clocks::init_clocks_and_plls(
        XTAL_HZ,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .ok()
    .unwrap();

    let sio = hal::Sio::new(pac.SIO);
    let pins = hal::gpio::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );

    let uart_pins = (
        pins.gpio0.into_function::<hal::gpio::FunctionUart>(),
        pins.gpio1.into_function::<hal::gpio::FunctionUart>(),
    );
    let uart = hal::uart::UartPeripheral::new(pac.UART0, uart_pins, &mut pac.RESETS)
        .enable(
            hal::uart::UartConfig::new(
                115_200.Hz(),
                hal::uart::DataBits::Eight,
                None,
                hal::uart::StopBits::One,
            ),
            clocks.peripheral_clock.freq(),
        )
        .ok()
        .unwrap();

    // TIMER0 はリセットが掛かったまま起動するので、ここで解除する。
    // rp235x-hal はこの解除を Timer::new_timer0 の中でしか行わない。解除せずに
    // TIMELR を読むとバスフォルトか常時 0 になり、sleep が効かなくなる。
    let _timer = hal::Timer::new_timer0(pac.TIMER0, &mut pac.RESETS, &clocks);

    let mut serial = Uart(uart);
    serial.write(b"wasmicon rp2350\r\n");

    // SAFETY: Pico2Board がこれ以降 SIO / IO_BANK0 / PADS_BANK0 / TIMER0 /
    // SPI0 / I2C0 と、RESETS の SPI0 / I2C0 ビットを排他的に使う。上で取った
    // Pins は UART の GP0/GP1 だけで、役割名に割り当てた GPIO とも
    // SPI0 のピン (GP16/18/19) とも I2C0 のピン (GP4/GP5) とも重ねていない。
    //
    // クロックは 2 つ渡す。**SPI (PL022) は clk_peri、I2C (DW_apb_i2c) は
    // clk_sys** で動く。既定ではどちらも 150 MHz で一致するが、取り違えると
    // clk_peri を別に振った瞬間に I2C の SCL が狂うので分けてある。
    let board = unsafe {
        Pico2Board::new(
            serial,
            clocks.peripheral_clock.freq().to_Hz(),
            clocks.system_clock.freq().to_Hz(),
        )
    };
    let mut hal = Hal::new(board, cfg!(feature = "trace"));

    // SAFETY: シングルコアで割り込みからも触らないので、可変静的への参照は
    // ここでしか作らない。
    let arena_buf = unsafe { &mut *core::ptr::addr_of_mut!(ARENA) };
    let scratch_buf = unsafe { &mut *core::ptr::addr_of_mut!(SCRATCH) };

    let guest = pick_guest(hal.board_mut().serial());
    let outcome = run(&mut hal, guest, arena_buf, scratch_buf);

    // **理由を先に出す。** 下の release_all は無制限に待ちうる
    // （`spi_close` の `BSY` 待ちなど。docs/TODO.md §2.1）ので、掃除を
    // 先に回すとペリフェラルが固まったときに理由が出ないまま無言で止まる
    // ——  docs/handoff.md §3 #4「ログを出して停止する」が守れない。
    // abi-spec §6.6 は drop → ログの順で書いてあるが、順序を入れ替えても
    // 観測できるのは「理由が出る」ことだけ増える（docs/TODO.md §2 に記録）。
    if let Err(e) = outcome {
        let s = hal.board_mut().serial();
        s.write(b"wasmicon: ");
        s.write(e.reason().as_bytes());
        s.write(b" [");
        s.write(e.kind().name().as_bytes());
        s.write(b"]\r\n");
    }

    // abi-spec §5.2 / §6.6: `run` から戻ったら（トラップでも）残っている
    // ハンドルを全部 drop する。ゲストの Drop は走らないので、ここだけが
    // 片付ける。トレース行は出さない（`Hal::release_all` のコメント）。
    hal.release_all();
    loop {
        cortex_m::asm::wfi();
    }
}

/// スロットから走らせるアプリを選ぶ。
///
/// **スロットが優先、空なら内蔵アプリ。** 読めない理由はシリアルに出す
/// （`docs/app-workflow.md` §3.1。空は失敗ではないので、そのことも出す）。
///
/// スロットはフラッシュに memory-mapped で見えるので、**RAM に写さず
/// スライスのまま `decode` に渡す**（design-notes §4）。
fn pick_guest(serial: &mut impl Serial) -> &'static [u8] {
    // SAFETY: __flash_binary_end はリンカが置くシンボルで、読むのはアドレス
    // だけ（中身は見ない）。
    let fw_end = (&raw const __flash_binary_end) as usize;

    let mut line = [0u8; 96];
    let mut out = Buf::new(&mut line);
    // SAFETY: XIP は読み出し専用でマップされていて、4 MB のフラッシュに
    // 対して offset + len（1 MiB + 64 KiB）は収まる。ファームとの重なりは
    // read_xip が fw_end で弾く。
    match unsafe { slot::read_xip(XIP_BASE, SLOT, fw_end) } {
        Ok(wasm) => {
            out.str("wasmicon: slot ");
            out.u32(wasm.len() as u32);
            out.str(" B crc32=");
            out.hex(wasmicon_port::crc32(wasm), 8);
            serial.write(out.as_bytes());
            serial.write(b"\r\n");
            wasm
        }
        Err(e) => {
            out.str("wasmicon: ");
            out.str(e.reason());
            out.str(", running built-in");
            serial.write(out.as_bytes());
            serial.write(b"\r\n");
            BUILTIN
        }
    }
}

/// デコードから `run` の呼び出しまで。
fn run(
    hal: &mut Hal<Pico2Board<Uart>>,
    guest: &[u8],
    arena_buf: &'static mut [u8],
    scratch_buf: &'static mut [u8],
) -> Result<(), wasmicon_core::Error> {
    let mut arena = Arena::new(arena_buf);
    let mut scratch = Arena::new(scratch_buf);

    let m = decode::decode(guest, &mut arena)?;
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
