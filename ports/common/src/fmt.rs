//! `core::fmt` を使わない最小の整形。
//!
//! トレースはマイコンでも出すので、`core::fmt` を持ち込みたくない
//! （バイナリが肥大する。CLAUDE.md）。必要なのは 10 進と 16 進だけ。

/// 固定長のバイト列にだけ書ける、伸びないバッファ。
pub struct Buf<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl<'a> Buf<'a> {
    #[must_use]
    pub fn new(buf: &'a mut [u8]) -> Self {
        Buf { buf, len: 0 }
    }

    /// 書けた分。溢れた場合は切り詰められている。
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// 溢れたか。
    #[must_use]
    pub fn overflowed(&self) -> bool {
        self.len == self.buf.len()
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn byte(&mut self, b: u8) {
        if self.len < self.buf.len() {
            self.buf[self.len] = b;
            self.len += 1;
        }
    }

    pub fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }

    pub fn bytes(&mut self, s: &[u8]) {
        for &b in s {
            self.byte(b);
        }
    }

    /// 10 進。
    pub fn u64(&mut self, mut v: u64) {
        let mut digits = [0u8; 20];
        let mut n = 0;
        loop {
            digits[n] = b'0' + (v % 10) as u8;
            n += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        while n > 0 {
            n -= 1;
            self.byte(digits[n]);
        }
    }

    pub fn u32(&mut self, v: u32) {
        self.u64(u64::from(v));
    }

    /// 幅を固定した 16 進（`width` 桁、0 埋め）。
    pub fn hex(&mut self, v: u32, width: usize) {
        const D: &[u8; 16] = b"0123456789abcdef";
        let mut i = width;
        while i > 0 {
            i -= 1;
            let shift = (i * 4) as u32;
            let nibble = if shift >= 32 { 0 } else { (v >> shift) & 0xf };
            self.byte(D[nibble as usize]);
        }
    }
}
