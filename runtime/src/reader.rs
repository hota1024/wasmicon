//! バイト列リーダ。LEB128 と即値の読み出し。
//!
//! Cortex-M0+ は非アライメントのワードアクセスで HardFault するので、
//! 多バイト値は必ず `from_le_bytes` でバイト単位に組み立てる
//! （ポインタキャストや `align_to` は使わない）。

use crate::error::{Error, Result};

/// フラッシュ上のスライスを前から読むカーソル。
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

    /// 残り全部を切り出す。
    #[must_use]
    pub fn rest(&self) -> &'m [u8] {
        &self.bytes[self.pos..]
    }

    /// 元のバイト列の [from, to) を切り出す。定数式の範囲取りに使う。
    #[must_use]
    pub fn slice(&self, from: usize, to: usize) -> &'m [u8] {
        &self.bytes[from..to]
    }

    pub fn u8(&mut self) -> Result<u8> {
        let b = *self.bytes.get(self.pos).ok_or(END)?;
        self.pos += 1;
        Ok(b)
    }

    /// 次の 1 バイトを消費せずに見る。
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

    /// 符号なし LEB128（32bit）。冗長な長さと範囲外を拒否する。
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

    /// 符号なし LEB128（64bit）。
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

    /// 符号つき LEB128（32bit）。
    pub fn i32_leb(&mut self) -> Result<i32> {
        let mut result: u32 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = self.u8()?;
            if shift == 28 {
                if byte & 0x80 != 0 {
                    return Err(Error::Malformed("integer representation too long"));
                }
                // 残り 4 ビット。上位 3 ビットは符号拡張でなければならない。
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

    /// 符号つき LEB128（64bit）。
    pub fn i64_leb(&mut self) -> Result<i64> {
        let mut result: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = self.u8()?;
            if shift == 63 {
                if byte & 0x80 != 0 {
                    return Err(Error::Malformed("integer representation too long"));
                }
                // 残り 1 ビット。上位 6 ビットは符号拡張でなければならない。
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

    /// 4 バイト little-endian。非アライメントでも安全。
    pub fn f32_bits(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// 8 バイト little-endian。
    pub fn f64_bits(&mut self) -> Result<u64> {
        let b = self.bytes(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    /// 長さつき UTF-8 名。妥当性は `core::str::from_utf8` で検査する。
    pub fn name(&mut self) -> Result<&'m str> {
        let len = self.u32_leb()? as usize;
        let b = self.bytes(len)?;
        core::str::from_utf8(b).map_err(|_| Error::Malformed("malformed UTF-8 encoding"))
    }
}
