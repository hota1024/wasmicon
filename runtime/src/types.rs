//! Wasm types. Only what the supported feature set (docs/decisions.md D-07) needs.

use crate::error::{Error, Result};
use crate::reader::Reader;

/// Value type. Numeric types only, since reference types are not supported.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ValType {
    I32,
    I64,
    F32,
    F64,
}

impl ValType {
    /// Decodes the binary encoding.
    pub const fn from_byte(b: u8) -> Result<ValType> {
        match b {
            0x7f => Ok(ValType::I32),
            0x7e => Ok(ValType::I64),
            0x7d => Ok(ValType::F32),
            0x7c => Ok(ValType::F64),
            // 0x63..=0x7b are the current spec's reference, GC and v128 types. They
            // are valid Wasm but outside the supported feature set
            // (docs/decisions.md D-07), so they are kept distinct from malformed.
            0x7b => Err(Error::Unsupported("SIMD is not supported")),
            0x63..=0x7a => Err(Error::Unsupported("reference types are not supported")),
            _ => Err(Error::Malformed("malformed value type")),
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            ValType::I32 => "i32",
            ValType::I64 => "i64",
            ValType::F32 => "f32",
            ValType::F64 => "f64",
        }
    }

    /// Whether the stack slot representation is 64-bit (for traces).
    #[must_use]
    pub const fn is_64(self) -> bool {
        matches!(self, ValType::I64 | ValType::F64)
    }
}

/// Function type. Parameters and results point straight into the module bytes
/// (zero copy). In the supported feature set a value type is one byte, so the
/// slice length is the count.
#[derive(Clone, Copy, Default)]
pub struct FuncType<'m> {
    params: &'m [u8],
    results: &'m [u8],
}

impl<'m> FuncType<'m> {
    #[must_use]
    pub const fn new(params: &'m [u8], results: &'m [u8]) -> Self {
        FuncType { params, results }
    }

    /// Parameter type bytes (used for type equality).
    #[must_use]
    pub const fn params_bytes(&self) -> &'m [u8] {
        self.params
    }

    /// Result type bytes.
    #[must_use]
    pub const fn results_bytes(&self) -> &'m [u8] {
        self.results
    }

    #[must_use]
    pub const fn param_count(&self) -> usize {
        self.params.len()
    }

    #[must_use]
    pub const fn result_count(&self) -> usize {
        self.results.len()
    }

    /// The i-th parameter type, or `None` when out of range.
    #[must_use]
    pub fn param(&self, i: usize) -> Option<ValType> {
        self.params.get(i).and_then(|b| ValType::from_byte(*b).ok())
    }

    /// The i-th result type.
    #[must_use]
    pub fn result(&self, i: usize) -> Option<ValType> {
        self.results
            .get(i)
            .and_then(|b| ValType::from_byte(*b).ok())
    }

    pub fn params(&self) -> impl Iterator<Item = ValType> + '_ {
        self.params
            .iter()
            .filter_map(|b| ValType::from_byte(*b).ok())
    }

    pub fn results(&self) -> impl Iterator<Item = ValType> + '_ {
        self.results
            .iter()
            .filter_map(|b| ValType::from_byte(*b).ok())
    }

    /// Whether this type matches a signature in `params:results` notation
    /// (docs/abi-spec.md §7). Used for the exact-match link against the generated
    /// import table.
    #[must_use]
    pub fn sig_matches(&self, sig: &str) -> bool {
        let mut it = sig.chars();
        for t in self.params() {
            if it.next() != Some(sig_char(t)) {
                return false;
            }
        }
        if it.next() != Some(':') {
            return false;
        }
        for t in self.results() {
            if it.next() != Some(sig_char(t)) {
                return false;
            }
        }
        it.next().is_none()
    }

    /// Whether the types are equal. Used for import matching and indirect calls.
    #[must_use]
    pub fn matches(&self, other: &FuncType<'_>) -> bool {
        self.params == other.params && self.results == other.results
    }
}

/// One character of the signature notation (docs/abi-spec.md §7).
const fn sig_char(t: ValType) -> char {
    match t {
        ValType::I32 => 'i',
        ValType::I64 => 'I',
        ValType::F32 => 'f',
        ValType::F64 => 'F',
    }
}

/// Lower and upper bounds of a memory or table.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Limits {
    pub min: u32,
    pub max: Option<u32>,
}

impl Limits {
    /// Decodes `limits`. `upper` is the spec's upper bound (65536 for memories).
    pub fn decode(r: &mut Reader<'_>, upper: u32, what: &'static str) -> Result<Limits> {
        // Exceeding the bound is invalid per spec (not malformed), so values that do
        // not fit in u32 are read as u64 first and judged afterwards.
        let flag = r.u8()?;
        let (min, max) = match flag {
            0x00 => (r.u64_leb()?, None),
            0x01 => {
                let min = r.u64_leb()?;
                (min, Some(r.u64_leb()?))
            }
            0x04 | 0x05 => return Err(Error::Unsupported("memory64 is not supported")),
            _ => return Err(Error::Malformed("integer too large")),
        };
        let upper = u64::from(upper);
        if min > upper {
            return Err(Error::Invalid(what));
        }
        if let Some(m) = max {
            if m > upper {
                return Err(Error::Invalid(what));
            }
            if m < min {
                return Err(Error::Invalid(
                    "size minimum must not be greater than maximum",
                ));
            }
        }
        Ok(Limits {
            min: min as u32,
            max: max.map(|m| m as u32),
        })
    }
}

/// Global type.
#[derive(Clone, Copy)]
pub struct GlobalType {
    pub val: ValType,
    pub mutable: bool,
}

impl Default for GlobalType {
    fn default() -> Self {
        GlobalType {
            val: ValType::I32,
            mutable: false,
        }
    }
}

impl GlobalType {
    pub fn decode(r: &mut Reader<'_>) -> Result<GlobalType> {
        let val = ValType::from_byte(r.u8()?)?;
        let mutable = match r.u8()? {
            0x00 => false,
            0x01 => true,
            _ => return Err(Error::Malformed("malformed mutability")),
        };
        Ok(GlobalType { val, mutable })
    }
}

/// Block type.
#[derive(Clone, Copy, Default)]
pub enum BlockType {
    /// No result.
    #[default]
    Empty,
    /// A single value type.
    Val(ValType),
    /// A type index (multi-value).
    Type(u32),
}
