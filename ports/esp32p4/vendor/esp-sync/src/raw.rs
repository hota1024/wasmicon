//! TODO

use core::sync::atomic::{Ordering, compiler_fence};

use crate::RestoreState;

/// Trait for single-core locks.
pub trait RawLock {
    /// Acquires the raw lock
    ///
    /// # Safety
    ///
    /// The returned tokens must be released in reverse order, on the same thread that they were
    /// created on.
    unsafe fn enter(&self) -> RestoreState;

    /// Releases the raw lock
    ///
    /// # Safety
    ///
    /// - The `token` must be created by `self.enter()`
    /// - Tokens must be released in reverse order to their creation, on the same thread that they
    ///   were created on.
    unsafe fn exit(&self, token: RestoreState);
}

/// A lock that disables interrupts.
pub struct SingleCoreInterruptLock;

// Reserved bits in the PS register, these must be written as 0.
#[cfg(all(xtensa, debug_assertions))]
const RESERVED_MASK: u32 = 0b1111_1111_1111_1000_1111_0000_0000_0000;

impl RawLock for SingleCoreInterruptLock {
    #[inline]
    unsafe fn enter(&self) -> RestoreState {
        // WASMICON LOCAL PATCH: 上流の `esp32p4 =>` の腕を削除し、P4 を汎用の
        // `riscv` 経路へ落としている。上流は P4 v3.2/ECO7 の Zcmp ハードウェア
        // バグ回避として mintthresh (CSR 0x347) を書くが、**分岐条件が
        // `cfg(esp32p4)` だけでリビジョンも Zcmp の有無も見ていない**（上流自身の
        // `// TODO: any with zcmp` を参照）。M5Stack Tab5 の P4 **v1.0** では
        // この CSR が不正命令になり、`esp_hal::init` の中で例外 → RWDT リセットの
        // 無限ループになる（2026-09-21 に実機で mepc/mtval から特定）。
        // 加えて我々のターゲット `riscv32imafc` は Zcmp を含まないので、
        // この回避策はそもそも不要。詳細と削除条件は docs/TODO.md §1.1.5。
        cfg_select! {
            riscv => {
                let mut mstatus = 0u32;
                unsafe {
                    core::arch::asm!("csrrci {0}, mstatus, 8", inout(reg) mstatus);
                }
                let token = mstatus & 0b1000;
            }
            xtensa => {
                let token: u32;
                unsafe {
                    core::arch::asm!("rsil {0}, 5", out(reg) token);
                }
                #[cfg(debug_assertions)]
                let token = token & !RESERVED_MASK;
            }
            _ => {
                compile_error!("Unsupported architecture")
            }
        };

        // Ensure no subsequent memory accesses are reordered to before interrupts are
        // disabled.
        compiler_fence(Ordering::SeqCst);

        unsafe { RestoreState::new(token) }
    }

    #[inline]
    unsafe fn exit(&self, token: RestoreState) {
        // Ensure no preceeding memory accesses are reordered to after interrupts are
        // enabled.
        compiler_fence(Ordering::SeqCst);

        let token = token.inner();

        // WASMICON LOCAL PATCH: `enter` と対になる削除。理由は `enter` 側の注記。
        cfg_select! {
            riscv => {
                if token != 0 {
                    unsafe {
                        riscv::interrupt::enable();
                    }
                }
            }
            xtensa => {
                #[cfg(debug_assertions)]
                if token & RESERVED_MASK != 0 {
                    // We could do this transformation in fmt.rs automatically, but experiments
                    // show this is only worth it in terms of binary size for code inlined into many
                    // places.
                    #[cold]
                    #[inline(never)]
                    fn __assert_failed() {
                        panic!("Reserved bits in PS register must be written as 0");
                    }

                    __assert_failed();
                }

                unsafe {
                    core::arch::asm!(
                        "wsr.ps {0}",
                        "rsync", in(reg) token)
                }
            }
            _ => {
                compile_error!("Unsupported architecture")
            }
        }
    }
}
