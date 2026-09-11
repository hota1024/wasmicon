//! ESP32-P4 のチップ層。**ボードに依存しない**。
//!
//! GPIO は番号で動的に触る必要があるので、型付きピンではなく GPIO / IO_MUX の
//! レジスタを直接叩く。ここにあるのは「P4 というチップならどのボードでも同じ」
//! ものだけで、どのピンが何に繋がっているかは `crate::boards` が持つ。
//!
//! ESP32-S3 版と同じ形だが、チップ固有の定数が 2 つ違う（`NUM_GPIO` と
//! `SIG_GPIO_OUT`）。レジスタとフィールドの名前は S3 と同じ。

use esp_hal::peripherals::{GPIO, IO_MUX};
use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::ErrorCode;
use wasmicon_port::BoardResult;

/// ESP32-P4 の GPIO は 0..=54 で欠番が無い（S3 と違う点）。
pub const NUM_GPIO: u32 = 55;

/// IO_MUX の MCU_SEL に入れる「GPIO 機能」。ESP32-P4 では 1（S3 と同じ）。
const MCU_SEL_GPIO: u8 = 1;

/// GPIO マトリクスの「GPIO 出力」信号。**ESP32-P4 では 256**（S3 は 128）。
/// FUNC_OUT_SEL_CFG.OUT_SEL は 9 bit で、256 が GPIO_OUT_REG 直結を意味する。
const SIG_GPIO_OUT: u16 = 256;

// P4 のフラッシュと PSRAM は GPIO 空間の外にある専用の MSPI ピン
// （PAC の `iomux_mspi_pin`）に出ているので、**チップの都合で塞ぐ番号は無い**。
// S3 が 22..=25（欠番）と 26..=32（フラッシュ / PSRAM）を塞いでいたのに対して、
// P4 で塞ぐものはすべてボード由来になる（`crate::boards`）。
//
// ただし **GPIO 32..=38 は strapping ピン**で、起動時の状態がブートに効く。
// チップ層では塞がないが、ここを役割名に充てるボード定義を書くときは注意すること。

/// GPIO / IO_MUX レジスタへの排他アクセス。
///
/// `steal` を一度だけ呼び、以降は安全なメソッドで触る。`unsafe` をここ 1 箇所に
/// 閉じ込めるための型で、値そのものは持たない。
pub struct Gpio(());

impl Gpio {
    /// # Safety
    /// GPIO と IO_MUX のレジスタを、このインスタンスだけが触ること。
    #[must_use]
    pub unsafe fn steal() -> Self {
        Gpio(())
    }

    pub fn configure(&mut self, index: u32, mode: PinMode) -> BoardResult<()> {
        if index >= NUM_GPIO {
            return Err(ErrorCode::InvalidArgument);
        }
        let n = index as usize;
        // SAFETY: Gpio::steal の契約により GPIO / IO_MUX はこの型だけが触る。
        let (gpio, io_mux) = unsafe { (GPIO::steal(), IO_MUX::steal()) };
        let g = gpio.register_block();
        let m = io_mux.register_block();

        // IO_MUX: GPIO 機能にして、入力バッファとプルを設定する。
        m.gpio(n).write(|w| {
            unsafe { w.mcu_sel().bits(MCU_SEL_GPIO) };
            w.fun_ie()
                .bit(!matches!(mode, PinMode::Output | PinMode::OutputOpenDrain));
            w.fun_wpu().bit(matches!(mode, PinMode::InputPullUp));
            w.fun_wpd().bit(matches!(mode, PinMode::InputPullDown));
            w
        });

        // GPIO マトリクス: 出力は GPIO_OUT 信号に繋ぐ。
        g.func_out_sel_cfg(n)
            .write(|w| unsafe { w.out_sel().bits(SIG_GPIO_OUT) });

        // オープンドレインは PIN_PAD_DRIVER で切り替える。
        g.pin(n)
            .modify(|_, w| w.pad_driver().bit(matches!(mode, PinMode::OutputOpenDrain)));

        let (high, mask) = bank(index);
        match mode {
            PinMode::Output | PinMode::OutputOpenDrain => {
                if high {
                    g.enable1_w1ts().write(|w| unsafe { w.bits(mask) });
                } else {
                    g.enable_w1ts().write(|w| unsafe { w.bits(mask) });
                }
            }
            _ => {
                if high {
                    g.enable1_w1tc().write(|w| unsafe { w.bits(mask) });
                } else {
                    g.enable_w1tc().write(|w| unsafe { w.bits(mask) });
                }
            }
        }
        Ok(())
    }

    pub fn write(&mut self, index: u32, level: Level) -> BoardResult<()> {
        if index >= NUM_GPIO {
            return Err(ErrorCode::InvalidArgument);
        }
        // SAFETY: 同上。
        let gpio = unsafe { GPIO::steal() };
        let g = gpio.register_block();
        let (high, mask) = bank(index);
        let enabled = if high {
            g.enable1().read().bits() & mask
        } else {
            g.enable().read().bits() & mask
        };
        if enabled == 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        match (level, high) {
            (Level::High, false) => g.out_w1ts().write(|w| unsafe { w.bits(mask) }),
            (Level::Low, false) => g.out_w1tc().write(|w| unsafe { w.bits(mask) }),
            (Level::High, true) => g.out1_w1ts().write(|w| unsafe { w.bits(mask) }),
            (Level::Low, true) => g.out1_w1tc().write(|w| unsafe { w.bits(mask) }),
        };
        Ok(())
    }

    pub fn read(&mut self, index: u32) -> BoardResult<Level> {
        if index >= NUM_GPIO {
            return Err(ErrorCode::InvalidArgument);
        }
        // SAFETY: 同上。
        let gpio = unsafe { GPIO::steal() };
        let g = gpio.register_block();
        let (high, mask) = bank(index);
        let (enable, out, input) = if high {
            (
                g.enable1().read().bits(),
                g.out1().read().bits(),
                g.in1().read().bits(),
            )
        } else {
            (
                g.enable().read().bits(),
                g.out().read().bits(),
                g.in_().read().bits(),
            )
        };
        // 出力モードなら出力値、入力モードなら入力値（WIT の read の定義）。
        let bits = if enable & mask != 0 { out } else { input };
        Ok(if bits & mask != 0 {
            Level::High
        } else {
            Level::Low
        })
    }
}

/// 32 本ずつに分かれたレジスタのどちらを触るかを決める。
fn bank(index: u32) -> (bool, u32) {
    if index < 32 {
        (false, 1u32 << index)
    } else {
        (true, 1u32 << (index - 32))
    }
}

/// 起動からの経過時間。
pub fn now_us() -> u64 {
    esp_hal::time::Instant::now()
        .duration_since_epoch()
        .as_micros()
}
