//! spec テストランナーの補助（`spectest` シムと値の突き合わせ）。
//!
//! `runtime/tests/spec.rs` から `include!` して使う。単独のテストではない。

use wasmicon_core::error::{Error, Result};
use wasmicon_core::instance::{Extern, ExternType, Resolver};

/// spec testsuite が import する `spectest` モジュールの最小実装。
pub struct SpecTest;

impl Resolver for SpecTest {
    fn resolve(&mut self, module: &str, name: &str, ty: &ExternType<'_>) -> Option<Extern> {
        if module != "spectest" {
            return None;
        }
        match ty {
            ExternType::Func(_) if name.starts_with("print") => Some(Extern::Func(0)),
            ExternType::Global(g) => {
                // spectest のグローバルは全て 666 相当。
                let v = match (name, g.val) {
                    ("global_i32", _) => 666u64,
                    ("global_i64", _) => 666u64,
                    ("global_f32", _) => u64::from(666.6f32.to_bits()),
                    ("global_f64", _) => 666.6f64.to_bits(),
                    _ => return None,
                };
                Some(Extern::Global(v))
            }
            _ => None,
        }
    }

    fn call(&mut self, _host: u32, _args: &[u64], _results: &mut [u64]) -> Result<()> {
        // print* は何もしない。戻り値は無い。
        Ok(())
    }
}

/// 期待値。NaN は「どの NaN でもよい」種別がある。
pub enum Expected {
    Bits32(u32),
    Bits64(u64),
    NanCanonical32,
    NanArithmetic32,
    NanCanonical64,
    NanArithmetic64,
    /// 対応機能セット外の値（v128 や参照型）。
    Unsupported,
}

/// JSON の値をスタックスロットへ。
pub fn parse_arg(v: &serde_json::Value) -> Option<u64> {
    let t = v["type"].as_str()?;
    let s = v["value"].as_str()?;
    match t {
        "i32" | "f32" => s.parse::<u32>().ok().map(u64::from),
        "i64" | "f64" => s.parse::<u64>().ok(),
        _ => None,
    }
}

/// JSON の期待値を解釈する。
pub fn parse_expected(v: &serde_json::Value) -> Expected {
    let Some(t) = v["type"].as_str() else {
        return Expected::Unsupported;
    };
    let s = v["value"].as_str().unwrap_or("");
    match (t, s) {
        ("f32", "nan:canonical") => Expected::NanCanonical32,
        ("f32", "nan:arithmetic") => Expected::NanArithmetic32,
        ("f64", "nan:canonical") => Expected::NanCanonical64,
        ("f64", "nan:arithmetic") => Expected::NanArithmetic64,
        ("i32" | "f32", _) => s
            .parse::<u32>()
            .map_or(Expected::Unsupported, Expected::Bits32),
        ("i64" | "f64", _) => s
            .parse::<u64>()
            .map_or(Expected::Unsupported, Expected::Bits64),
        _ => Expected::Unsupported,
    }
}

impl Expected {
    /// 実際の値と一致するか。
    pub fn matches(&self, actual: u64) -> bool {
        match self {
            Expected::Bits32(b) => actual as u32 == *b,
            Expected::Bits64(b) => actual == *b,
            Expected::NanCanonical32 => actual as u32 & 0x7fff_ffff == 0x7fc0_0000,
            Expected::NanArithmetic32 => {
                let a = actual as u32;
                a & 0x7f80_0000 == 0x7f80_0000 && a & 0x007f_ffff != 0 && a & 0x0040_0000 != 0
            }
            Expected::NanCanonical64 => actual & 0x7fff_ffff_ffff_ffff == 0x7ff8_0000_0000_0000,
            Expected::NanArithmetic64 => {
                actual & 0x7ff0_0000_0000_0000 == 0x7ff0_0000_0000_0000
                    && actual & 0x000f_ffff_ffff_ffff != 0
                    && actual & 0x0008_0000_0000_0000 != 0
            }
            Expected::Unsupported => false,
        }
    }

    pub fn is_unsupported(&self) -> bool {
        matches!(self, Expected::Unsupported)
    }
}

/// エラーがトラップか（`assert_trap` の判定）。
pub fn is_trap(e: &Error) -> bool {
    matches!(e, Error::Trap(_))
}
