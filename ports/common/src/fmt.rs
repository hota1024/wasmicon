//! Minimal formatting without `core::fmt`.
//!
//! Traces are emitted on microcontrollers too, and `core::fmt` would bloat the
//! binary (docs/decisions.md P-09). Only decimal and hex are needed.

/// Fixed-size byte buffer that never grows.
pub struct Buf<'a> {
    buf: &'a mut [u8],
    len: usize,
    /// When `false`, nothing is written (`off`).
    on: bool,
}

impl<'a> Buf<'a> {
    #[must_use]
    pub fn new(buf: &'a mut [u8]) -> Self {
        Buf {
            buf,
            len: 0,
            on: true,
        }
    }

    /// A buffer that writes nothing. Used while tracing is off.
    ///
    /// Callers need not branch on every write (every method returns at its
    /// entry). Only arguments that are expensive to compute (CRC-32, hex
    /// dumps) are skipped by the caller after checking `is_on`.
    #[must_use]
    pub fn off(buf: &'a mut [u8]) -> Self {
        Buf {
            buf,
            len: 0,
            on: false,
        }
    }

    /// Whether this buffer writes.
    #[must_use]
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// What was written. Truncated if the buffer overflowed.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// Whether the buffer overflowed.
    #[must_use]
    pub fn overflowed(&self) -> bool {
        self.len == self.buf.len()
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn byte(&mut self, b: u8) {
        if self.on && self.len < self.buf.len() {
            self.buf[self.len] = b;
            self.len += 1;
        }
    }

    pub fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }

    pub fn bytes(&mut self, s: &[u8]) {
        if !self.on {
            return;
        }
        for &b in s {
            self.byte(b);
        }
    }

    /// A string embedded in a trace. Emitting a newline or `"` as is would
    /// break the one-record-per-line form of docs/abi-spec.md §9 and shift
    /// the diff between the two boards by whole lines.
    pub fn escaped(&mut self, s: &[u8]) {
        if !self.on {
            return;
        }
        for &b in s {
            match b {
                b'\n' => self.str("\\n"),
                b'\r' => self.str("\\r"),
                b'\t' => self.str("\\t"),
                b'"' => self.str("\\\""),
                b'\\' => self.str("\\\\"),
                0x20..=0x7e => self.byte(b),
                // Control and non-ASCII bytes are emitted as hex, so a cut in the
                // middle of a UTF-8 sequence does no harm.
                _ => {
                    self.str("\\x");
                    self.hex(u32::from(b), 2);
                }
            }
        }
    }

    /// Decimal.
    pub fn u64(&mut self, mut v: u64) {
        if !self.on {
            return;
        }
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

    /// Fixed-width hex (`width` digits, zero-padded).
    pub fn hex(&mut self, v: u32, width: usize) {
        if !self.on {
            return;
        }
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
