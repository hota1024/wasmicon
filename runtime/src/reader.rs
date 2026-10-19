//! Byte reader: LEB128 and immediate decoding.
//!
//! Cortex-M0+ HardFaults on unaligned word access, so multi-byte values are
//! always assembled byte by byte with `from_le_bytes`, never through pointer
//! casts or `align_to` (docs/decisions.md P-06).

use crate::error::{Error, Result};

/// Cursor that reads a slice in flash from front to back.
pub struct Reader<'m> {
    bytes: &'m [u8],
    pos: usize,
}

const END: Error = Error::Malformed("unexpected end");

impl<'m> Reader<'m> {
    #[must_use]
    pub fn new(bytes: &'m [u8]) -> Self {
        Reader { bytes, pos: 0 }
    }

    #[must_use]
    pub fn pos(&self) -> usize {
        self.pos
    }

    #[must_use]
    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    /// Returns everything that is left.
    #[must_use]
    pub fn rest(&self) -> &'m [u8] {
        &self.bytes[self.pos..]
    }

    /// Returns [from, to) of the underlying bytes. Used to slice out constant expressions.
    #[must_use]
    pub fn slice(&self, from: usize, to: usize) -> &'m [u8] {
        &self.bytes[from..to]
    }

    pub fn u8(&mut self) -> Result<u8> {
        let b = *self.bytes.get(self.pos).ok_or(END)?;
        self.pos += 1;
        Ok(b)
    }

    /// Returns the next byte without consuming it.
    pub fn peek_u8(&self) -> Result<u8> {
        self.bytes.get(self.pos).copied().ok_or(END)
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'m [u8]> {
        let end = self.pos.checked_add(n).ok_or(END)?;
        let s = self.bytes.get(self.pos..end).ok_or(END)?;
        self.pos = end;
        Ok(s)
    }

    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }

    /// Unsigned LEB128 (32-bit). Rejects over-long encodings and out-of-range values.
    pub fn u32_leb(&mut self) -> Result<u32> {
        let mut result: u32 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = self.u8()?;
            if shift == 28 {
                if byte & 0x80 != 0 {
                    return Err(Error::Malformed("integer representation too long"));
                }
                if byte & 0x70 != 0 {
                    return Err(Error::Malformed("integer too large"));
                }
                return Ok(result | (u32::from(byte) << 28));
            }
            result |= u32::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
        }
    }

    /// Unsigned LEB128 (64-bit).
    pub fn u64_leb(&mut self) -> Result<u64> {
        let mut result: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = self.u8()?;
            if shift == 63 {
                if byte & 0x80 != 0 {
                    return Err(Error::Malformed("integer representation too long"));
                }
                if byte & 0x7e != 0 {
                    return Err(Error::Malformed("integer too large"));
                }
                return Ok(result | (u64::from(byte) << 63));
            }
            result |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
        }
    }

    /// Signed LEB128 (32-bit).
    pub fn i32_leb(&mut self) -> Result<i32> {
        let mut result: u32 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = self.u8()?;
            if shift == 28 {
                if byte & 0x80 != 0 {
                    return Err(Error::Malformed("integer representation too long"));
                }
                // 4 bits remain. The top 3 must be the sign extension.
                let b = byte & 0x7f;
                let sign = (b >> 3) & 1;
                let top = b >> 4;
                if (sign == 1 && top != 0b111) || (sign == 0 && top != 0) {
                    return Err(Error::Malformed("integer too large"));
                }
                return Ok((result | (u32::from(b) << 28)) as i32);
            }
            result |= u32::from(byte & 0x7f) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                if byte & 0x40 != 0 && shift < 32 {
                    result |= u32::MAX << shift;
                }
                return Ok(result as i32);
            }
        }
    }

    /// Signed LEB128 (64-bit).
    pub fn i64_leb(&mut self) -> Result<i64> {
        let mut result: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = self.u8()?;
            if shift == 63 {
                if byte & 0x80 != 0 {
                    return Err(Error::Malformed("integer representation too long"));
                }
                // 1 bit remains. The top 6 must be the sign extension.
                let b = byte & 0x7f;
                let sign = b & 1;
                let top = b >> 1;
                if (sign == 1 && top != 0b111_111) || (sign == 0 && top != 0) {
                    return Err(Error::Malformed("integer too large"));
                }
                return Ok((result | (u64::from(b) << 63)) as i64);
            }
            result |= u64::from(byte & 0x7f) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                if byte & 0x40 != 0 && shift < 64 {
                    result |= u64::MAX << shift;
                }
                return Ok(result as i64);
            }
        }
    }

    /// 4-byte little-endian. Safe on unaligned input.
    pub fn f32_bits(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// 8-byte little-endian.
    pub fn f64_bits(&mut self) -> Result<u64> {
        let b = self.bytes(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    /// Length-prefixed UTF-8 name. Validity is checked with `core::str::from_utf8`.
    pub fn name(&mut self) -> Result<&'m str> {
        let len = self.u32_leb()? as usize;
        let b = self.bytes(len)?;
        core::str::from_utf8(b).map_err(|_| Error::Malformed("malformed UTF-8 encoding"))
    }
}
