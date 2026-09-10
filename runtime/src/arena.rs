//! ポートから渡された 1 枚のバッファを前から切り出すだけのアロケータ。
//!
//! `alloc` を使わない代わりに、モジュールの解決に必要な表（型表、関数表、
//! side table、線形メモリなど）をここから取る。解放はしない。
//! モジュールを捨てるときはポートが arena ごと作り直す。

use crate::error::{Error, Result};

/// 前方に伸びるだけのバンプアロケータ。
pub struct Arena<'a> {
    buf: &'a mut [u8],
    used: usize,
}

impl<'a> Arena<'a> {
    /// ポートから受け取ったバッファで初期化する。
    #[must_use]
    pub fn new(buf: &'a mut [u8]) -> Self {
        Arena { buf, used: 0 }
    }

    /// 使用済みバイト数。
    #[must_use]
    pub fn used(&self) -> usize {
        self.used
    }

    /// 総容量。
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// `n` 個の `T` を確保し、全要素を `init` で埋めて返す。
    ///
    /// 未初期化のまま外へ出さないので、呼び出し側は安全に読める。
    /// `T: Copy` なので `Drop` は無く、解放しなくても漏れない。
    pub fn alloc<T: Copy>(&mut self, n: usize, init: T) -> Result<&'a mut [T]> {
        let align = align_of::<T>();
        let size = size_of::<T>();

        let base = self.buf.as_mut_ptr();
        // アラインは「バッファ先頭のアドレス + used」を基準に取る。
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

        // SAFETY: `aligned` は buf の範囲内（end <= buf.len() を確認済み）で、
        // T のアラインに合わせてある。この領域は used を進めることで二度と
        // 配られない。書き込んでから参照を作るので未初期化は読ませない。
        // 返す参照の寿命 'a は buf 自体の寿命であり、arena が生きている間
        // この領域が再利用されないことは used の単調増加が保証する。
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

    /// `n` バイトを 0 で埋めて確保する（線形メモリなど）。
    pub fn alloc_bytes(&mut self, n: usize) -> Result<&'a mut [u8]> {
        self.alloc(n, 0u8)
    }
}
