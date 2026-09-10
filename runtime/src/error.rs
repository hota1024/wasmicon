//! エラー型。`core::fmt` を使わないので、理由は `&'static str` で持つ。
//!
//! 区分は Wasm 仕様のフェーズに対応する。spec テストの
//! `assert_malformed` / `assert_invalid` / `assert_trap` はこの区分で判定する。

/// 実行時トラップの理由。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Trap {
    Unreachable,
    MemoryOutOfBounds,
    TableOutOfBounds,
    IndirectCallTypeMismatch,
    UninitializedElement,
    IntegerDivideByZero,
    IntegerOverflow,
    InvalidConversionToInteger,
    UndefinedElement,
    UnreachableHostCall,
}

impl Trap {
    /// トレース用の短い名前。`core::fmt` を避けるため文字列を直接返す。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Trap::Unreachable => "unreachable",
            Trap::MemoryOutOfBounds => "out of bounds memory access",
            Trap::TableOutOfBounds => "out of bounds table access",
            Trap::IndirectCallTypeMismatch => "indirect call type mismatch",
            Trap::UninitializedElement => "uninitialized element",
            Trap::IntegerDivideByZero => "integer divide by zero",
            Trap::IntegerOverflow => "integer overflow",
            Trap::InvalidConversionToInteger => "invalid conversion to integer",
            Trap::UndefinedElement => "undefined element",
            Trap::UnreachableHostCall => "unreachable host call",
        }
    }
}

/// ランタイムのエラー。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// デコード時の不正（Wasm 仕様の malformed）。
    Malformed(&'static str),
    /// 検証時の不正（Wasm 仕様の invalid）。
    Invalid(&'static str),
    /// インスタンス化時の不正（Wasm 仕様の unlinkable）。
    Unlinkable(&'static str),
    /// Wasm としては正しいが、対応機能セット（HANDOFF §2-7）の外。
    /// 仕様上の invalid とは区別する。ゲストのバグではなくランタイムの守備範囲外。
    Unsupported(&'static str),
    /// 実行時トラップ。
    Trap(Trap),
    /// スタックまたは呼び出し深さの上限超過。
    Exhausted(&'static str),
    /// ポートから渡された arena が足りない。ホスト側の設定不足。
    OutOfArena,
}

impl Error {
    /// 区分。spec テストの assert_* の判定に使う。
    #[must_use]
    pub const fn kind(self) -> ErrorKind {
        match self {
            Error::Malformed(_) => ErrorKind::Malformed,
            Error::Invalid(_) => ErrorKind::Invalid,
            Error::Unlinkable(_) => ErrorKind::Unlinkable,
            Error::Unsupported(_) => ErrorKind::Unsupported,
            Error::Trap(_) => ErrorKind::Trap,
            Error::Exhausted(_) => ErrorKind::Exhausted,
            Error::OutOfArena => ErrorKind::OutOfArena,
        }
    }

    /// トレース用の短い理由。
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Error::Malformed(m)
            | Error::Invalid(m)
            | Error::Unlinkable(m)
            | Error::Unsupported(m)
            | Error::Exhausted(m) => m,
            Error::Trap(t) => t.name(),
            Error::OutOfArena => "arena が足りない",
        }
    }
}

/// エラーの区分。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Malformed,
    Invalid,
    Unlinkable,
    Unsupported,
    Trap,
    Exhausted,
    OutOfArena,
}

impl ErrorKind {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            ErrorKind::Malformed => "malformed",
            ErrorKind::Invalid => "invalid",
            ErrorKind::Unlinkable => "unlinkable",
            ErrorKind::Unsupported => "unsupported",
            ErrorKind::Trap => "trap",
            ErrorKind::Exhausted => "exhausted",
            ErrorKind::OutOfArena => "out-of-arena",
        }
    }
}

pub type Result<T> = core::result::Result<T, Error>;
