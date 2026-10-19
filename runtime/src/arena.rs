//! Bump allocator over the single buffer the port hands in.
//!
//! Instead of `alloc`, the tables a module needs (types, functions, the side
//! table, linear memory and so on) are carved out of this buffer. Nothing is
//! ever freed: to discard a module, the port rebuilds the whole arena.

use crate::error::{Error, Result};

/// Bump allocator that only grows forward.
pub struct Arena<'a> {
    buf: &'a mut [u8],
    used: usize,
}

impl<'a> Arena<'a> {
    /// Wraps the buffer received from the port.
    #[must_use]
    pub fn new(buf: &'a mut [u8]) -> Self {
        Arena { buf, used: 0 }
    }

    /// Bytes handed out so far.
    #[must_use]
    pub fn used(&self) -> usize {
        self.used
    }

    /// Total capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// Allocates `n` values of `T`, fills every element with `init` and returns the slice.
    ///
    /// Nothing leaves this function uninitialized, so callers can read the slice
    /// safely. `T: Copy` means there is no `Drop`, so never freeing cannot leak.
    pub fn alloc<T: Copy>(&mut self, n: usize, init: T) -> Result<&'a mut [T]> {
        let align = align_of::<T>();
        let size = size_of::<T>();

        let base = self.buf.as_mut_ptr();
        // Alignment is computed on the absolute address (buffer start + used).
        let start = base as usize + self.used;
        let aligned = start
            .checked_next_multiple_of(align)
            .ok_or(Error::OutOfArena)?;
        let pad = aligned - start;
        let bytes = size.checked_mul(n).ok_or(Error::OutOfArena)?;
        let end = self
            .used
            .checked_add(pad)
            .and_then(|u| u.checked_add(bytes))
            .ok_or(Error::OutOfArena)?;
        if end > self.buf.len() {
            return Err(Error::OutOfArena);
        }

        // SAFETY: `aligned` lies inside `buf` (end <= buf.len() was checked above)
        // and is aligned for `T`. Advancing `used` past this region means it is
        // never handed out again. Every element is written before the reference
        // is created, so no uninitialized memory is exposed. The returned
        // lifetime 'a is the lifetime of `buf` itself, and `used` only grows, so
        // the region is not reused while the arena lives.
        let slice = unsafe {
            let ptr = aligned as *mut T;
            for i in 0..n {
                ptr.add(i).write(init);
            }
            core::slice::from_raw_parts_mut(ptr, n)
        };
        self.used = end;
        Ok(slice)
    }

    /// Allocates `n` bytes filled with 0.
    pub fn alloc_bytes(&mut self, n: usize) -> Result<&'a mut [u8]> {
        self.alloc(n, 0u8)
    }

    /// Hands out everything that is left. The contents are NOT initialized.
    ///
    /// Used for linear memory. A bump allocator cannot grow an allocation, so
    /// the remainder is handed out last and `memory.grow` only moves the current
    /// page count within it. No further allocation is possible after this call.
    ///
    /// The bytes are not zeroed here because touching the whole arena would
    /// materialize pages that are never used. `instantiate` and `Memory::grow`
    /// zero only the pages that are actually used.
    pub fn alloc_rest(&mut self) -> &'a mut [u8] {
        let n = self.buf.len() - self.used;
        // SAFETY: [used, len) has not been handed out to anyone and lies inside
        // `buf`. `used` is advanced to the end, so the region is never handed
        // out again. `u8` needs no alignment.
        let slice =
            unsafe { core::slice::from_raw_parts_mut(self.buf.as_mut_ptr().add(self.used), n) };
        self.used = self.buf.len();
        slice
    }
}
