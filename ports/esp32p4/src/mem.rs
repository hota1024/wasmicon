//! `memcpy` / `memmove` / `memset` / `memcmp` for silicon before v3.0.
//!
//! esp-rom-sys binds these names to ECO5 ROM addresses and has no pre-ECO5
//! ones, so on the v1.0 chip they would land in the middle of other ROM
//! functions. `pre-v3.x` points the names here instead. The symptom without
//! it was quiet: `decode` succeeded and `instantiate` returned a `Module` with
//! some slice lengths zeroed.
//!
//! The accesses are volatile so that LLVM cannot recognise a loop here as a
//! copy or fill and lower it to a call to `memcpy` / `memset`, which would be
//! this function calling itself. Aligned copies and fills go a word at a time,
//! since `instantiate` zeroes all of linear memory through `memset`.

use core::ptr::{read_volatile, write_volatile};

const WORD: usize = core::mem::size_of::<u32>();

/// # Safety
/// `dest` and `src` are valid for `n` bytes and do not overlap.
#[no_mangle]
pub unsafe extern "C" fn wasmicon_memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    let mut i = 0;
    if (dest as usize | src as usize).is_multiple_of(WORD) {
        while i + WORD <= n {
            // SAFETY: both pointers are word-aligned (checked above, and `i`
            // is a multiple of WORD), and [i, i + WORD) is inside the n bytes
            // the caller guarantees.
            unsafe {
                write_volatile(
                    dest.add(i).cast::<u32>(),
                    read_volatile(src.add(i).cast::<u32>()),
                );
            }
            i += WORD;
        }
    }
    while i < n {
        // SAFETY: i < n, inside the n bytes the caller guarantees.
        unsafe { write_volatile(dest.add(i), read_volatile(src.add(i))) };
        i += 1;
    }
    dest
}

/// # Safety
/// `dest` and `src` are valid for `n` bytes. They may overlap.
#[no_mangle]
pub unsafe extern "C" fn wasmicon_memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    if (dest as usize) < (src as usize) {
        let mut i = 0;
        while i < n {
            // SAFETY: i < n, inside the n bytes the caller guarantees. Going
            // forward reads every source byte before the copy overwrites it.
            unsafe { write_volatile(dest.add(i), read_volatile(src.add(i))) };
            i += 1;
        }
    } else {
        let mut i = n;
        while i > 0 {
            i -= 1;
            // SAFETY: i < n, inside the n bytes the caller guarantees. Going
            // backward reads every source byte before the copy overwrites it.
            unsafe { write_volatile(dest.add(i), read_volatile(src.add(i))) };
        }
    }
    dest
}

/// # Safety
/// `dest` is valid for `n` bytes.
#[no_mangle]
pub unsafe extern "C" fn wasmicon_memset(dest: *mut u8, c: i32, n: usize) -> *mut u8 {
    // C converts the fill value to `unsigned char`.
    let b = c as u8;
    let mut i = 0;
    if (dest as usize).is_multiple_of(WORD) {
        let w = u32::from_ne_bytes([b; WORD]);
        while i + WORD <= n {
            // SAFETY: `dest` is word-aligned (checked above, and `i` is a
            // multiple of WORD), and [i, i + WORD) is inside the n bytes the
            // caller guarantees.
            unsafe { write_volatile(dest.add(i).cast::<u32>(), w) };
            i += WORD;
        }
    }
    while i < n {
        // SAFETY: i < n, inside the n bytes the caller guarantees.
        unsafe { write_volatile(dest.add(i), b) };
        i += 1;
    }
    dest
}

/// # Safety
/// `a` and `b` are valid for `n` bytes.
#[no_mangle]
pub unsafe extern "C" fn wasmicon_memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    let mut i = 0;
    while i < n {
        // SAFETY: i < n, inside the n bytes the caller guarantees.
        let (x, y) = unsafe { (read_volatile(a.add(i)), read_volatile(b.add(i))) };
        if x != y {
            return i32::from(x) - i32::from(y);
        }
        i += 1;
    }
    0
}
