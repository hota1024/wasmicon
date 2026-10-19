//! Error types. Reasons are `&'static str` because the core does not use `core::fmt`.
//!
//! The categories follow the phases of the Wasm spec. The spec tests'
//! `assert_malformed` / `assert_invalid` / `assert_trap` are judged by category.

/// Reason for a run-time trap.
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
    /// Short name for traces. Returns a string directly to avoid `core::fmt`.
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

/// Runtime error.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Rejected while decoding (malformed in the Wasm spec).
    Malformed(&'static str),
    /// Rejected while validating (invalid in the Wasm spec).
    Invalid(&'static str),
    /// Rejected while instantiating (unlinkable in the Wasm spec).
    Unlinkable(&'static str),
    /// Valid Wasm, but outside the supported feature set (docs/decisions.md D-07).
    /// Kept distinct from invalid: not a guest bug, but outside what the runtime covers.
    Unsupported(&'static str),
    /// Run-time trap.
    Trap(Trap),
    /// Stack or call depth limit exceeded.
    Exhausted(&'static str),
    /// The arena the port handed in is too small. A host-side configuration problem.
    OutOfArena,
}

impl Error {
    /// Category. Used to judge the spec tests' `assert_*`.
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

    /// Short reason for traces.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Error::Malformed(m)
            | Error::Invalid(m)
            | Error::Unlinkable(m)
            | Error::Unsupported(m)
            | Error::Exhausted(m) => m,
            Error::Trap(t) => t.name(),
            Error::OutOfArena => "out of arena",
        }
    }
}

/// Error category.
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
