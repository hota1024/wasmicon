//! Wasm の型。対応機能セット（HANDOFF §2-7）の範囲だけを持つ。

use crate::error::{Error, Result};
use crate::reader::Reader;

/// 値型。reference-types は非対応なので数値型だけ。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ValType {
    I32,
    I64,
    F32,
    F64,
}

impl ValType {
    /// バイナリ表現から。
    pub const fn from_byte(b: u8) -> Result<ValType> {
        match b {
            0x7f => Ok(ValType::I32),
            0x7e => Ok(ValType::I64),
            0x7d => Ok(ValType::F32),
            0x7c => Ok(ValType::F64),
            // 0x63..=0x7b は現行仕様の参照型・GC 型・v128。Wasm としては正しいが
            // 対応機能セット外（HANDOFF §2-7）なので malformed と区別する。
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

    /// スタックスロット上の表現が 64bit かどうか（トレース用）。
    #[must_use]
    pub const fn is_64(self) -> bool {
        matches!(self, ValType::I64 | ValType::F64)
    }
}

/// 関数型。パラメータと結果はモジュールのバイト列を直接指す（ゼロコピー）。
/// 対応機能セットでは値型は 1 バイトなので、スライス長がそのまま個数になる。
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

    /// 引数型のバイト列（型の等値比較に使う）。
    #[must_use]
    pub const fn params_bytes(&self) -> &'m [u8] {
        self.params
    }

    /// 結果型のバイト列。
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

    /// i 番目のパラメータ型。範囲外なら `None`。
    #[must_use]
    pub fn param(&self, i: usize) -> Option<ValType> {
        self.params.get(i).and_then(|b| ValType::from_byte(*b).ok())
    }

    /// i 番目の結果型。
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

    /// 型が等しいか。import の照合とインダイレクト呼び出しで使う。
    #[must_use]
    pub fn matches(&self, other: &FuncType<'_>) -> bool {
        self.params == other.params && self.results == other.results
    }
}

/// メモリ・テーブルの上下限。
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Limits {
    pub min: u32,
    pub max: Option<u32>,
}

impl Limits {
    /// `limits` をデコードする。`upper` は仕様上の上限（メモリは 65536）。
    pub fn decode(r: &mut Reader<'_>, upper: u32, what: &'static str) -> Result<Limits> {
        // 上限超過は仕様上 invalid（malformed ではない）なので、u32 に収まらない
        // 値も一度 u64 で受けてから判定する。
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

/// グローバルの型。
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

/// ブロックの型。
#[derive(Clone, Copy, Default)]
pub enum BlockType {
    /// 結果なし。
    #[default]
    Empty,
    /// 単一の値型。
    Val(ValType),
    /// 型インデックス（multi-value）。
    Type(u32),
}
