//! `memcpy` / `memset` / `memmove` / `memcmp` の自前実装。
//!
//! **ROM の実装を使わない。** esp-rom-sys 0.1.5 は P4 のこれらを
//! `ld/esp32p4/rom/additional.ld` で ECO5 の ROM アドレスに固定している
//! （ファイル冒頭に `Ref: esp-idf esp32p4.rom.libc.ld (eco5)` とある）が、
//! M5Stack Tab5 の ROM は **esp32p4-eco2-20240710** で ECO5 ではない。
//! 表がずれているので別の関数に当たる。
//!
//! 実機で観測した症状（2026-09-21）: `decode` は完全に成功するのに
//! `instantiate` を通ると `Module` のスライス長が部分的に 0 へ潰れる
//! （`types=7 imports=7 funcs=3 mems=1 exports=2 code=3` →
//! `types=7 imports=0 funcs=0 mems=1 exports=0 code=0`）。
//! **クラッシュせず静かに壊す**ので、これだけを見て原因に辿り着くのは難しい。
//!
//! リンカスクリプト `rom-pre-eco5.x` が `memcpy = wasmicon_memcpy;` のように
//! ROM アドレスへの代入を上書きしてここへ向けている。
//!
//! **上流が ROM リビジョンを選べるようになったら、このファイルごと消すこと。**
//! 経緯と削除条件は docs/TODO.md §1.1.5。
//!
//! 読み書きに volatile を使うのは、LLVM が「バイトコピーのループ」を
//! `memcpy` 呼び出しへ畳み込んで**自分自身を無限再帰で呼ぶ**のを防ぐため。
//! 速度は落ちるが、ここは正しさが全て。

use core::ptr::{read_volatile, write_volatile};

/// # Safety
/// `dest` と `src` は `n` バイト有効で、領域が重ならないこと。
#[no_mangle]
pub unsafe extern "C" fn wasmicon_memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    let mut i = 0;
    while i < n {
        // SAFETY: 呼び出し側の契約により [0, n) は両方とも有効。
        unsafe { write_volatile(dest.add(i), read_volatile(src.add(i))) };
        i += 1;
    }
    dest
}

/// # Safety
/// `dest` と `src` は `n` バイト有効であること。重なっていてもよい。
#[no_mangle]
pub unsafe extern "C" fn wasmicon_memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    if (dest as usize) < (src as usize) {
        let mut i = 0;
        while i < n {
            // SAFETY: 契約により [0, n) は有効。前から詰めるので重なっても壊れない。
            unsafe { write_volatile(dest.add(i), read_volatile(src.add(i))) };
            i += 1;
        }
    } else {
        let mut i = n;
        while i > 0 {
            i -= 1;
            // SAFETY: 契約により [0, n) は有効。後ろから詰める。
            unsafe { write_volatile(dest.add(i), read_volatile(src.add(i))) };
        }
    }
    dest
}

/// # Safety
/// `dest` は `n` バイト有効であること。
#[no_mangle]
pub unsafe extern "C" fn wasmicon_memset(dest: *mut u8, c: i32, n: usize) -> *mut u8 {
    let b = c as u8;
    let mut i = 0;
    while i < n {
        // SAFETY: 契約により [0, n) は有効。
        unsafe { write_volatile(dest.add(i), b) };
        i += 1;
    }
    dest
}

/// # Safety
/// `a` と `b` は `n` バイト有効であること。
#[no_mangle]
pub unsafe extern "C" fn wasmicon_memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    let mut i = 0;
    while i < n {
        // SAFETY: 契約により [0, n) は両方とも有効。
        let (x, y) = unsafe { (read_volatile(a.add(i)), read_volatile(b.add(i))) };
        if x != y {
            return i32::from(x) - i32::from(y);
        }
        i += 1;
    }
    0
}
