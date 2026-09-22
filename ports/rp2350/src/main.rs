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
//! **実機で動作確認していない**（docs/TODO.md §1.1 の配線とシリアル接続が未確認）。
//! ビルドが通ることまでを確認した段階。

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
use wasmicon_port::Hal;

use board::{Pico2Board, Serial};

/// Boot ROM に「これは実行イメージだ」と伝えるブロック。
/// フラッシュの先頭 4 KB（`.start_block`）に置く必要がある。
/// RP2040 の二段目ブートローダに相当するものは RP2350 には要らない。
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

/// Pico 2 の水晶振動子。
const XTAL_HZ: u32 = 12_000_000;

/// ゲスト。`cd apps && cargo build --release` を先に実行しておく。
static GUEST: &[u8] =
    include_bytes!("../../../apps/target/wasm32-unknown-unknown/release/blink_rs.wasm");

/// ランタイムの arena。残りが線形メモリになる（`Arena::alloc_rest`）。
/// RP2350 の SRAM は 520 KB（512 KB + 4 KB × 2）なので、線形メモリ
/// 4 ページ（256 KB）が収まる大きさにしてある。
static mut ARENA: [u8; 320 * 1024] = [0; 320 * 1024];

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

    // SAFETY: Pico2Board がこれ以降 SIO / IO_BANK0 / PADS_BANK0 / TIMER0 を
    // 排他的に使う。上で取った Pins は UART の GP0/GP1 だけで、役割名に
    // 割り当てた GPIO とは重ねていない。
    let board = unsafe { Pico2Board::new(serial) };
    let mut hal = Hal::new(board, cfg!(feature = "trace"));

    // SAFETY: シングルコアで割り込みからも触らないので、可変静的への参照は
    // ここでしか作らない。
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
        cortex_m::asm::wfi();
    }
}

/// デコードから `run` の呼び出しまで。
fn run(
    hal: &mut Hal<Pico2Board<Uart>>,
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
