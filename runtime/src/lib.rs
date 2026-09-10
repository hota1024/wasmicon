//! Wasmicon の Core Wasm インタプリタ。
//!
//! 規約は `CLAUDE.md` と `docs/handoff.md` §5 Phase 2 が正。
//!
//! - `no_std`、依存クレートゼロ、`alloc` 不使用。arena はポート層から注入する
//! - Wasm の算術は wrapping。`wrapping_*` / `rotate_*` / `checked_*` を明示的に使う
//! - `core::fmt` を使わない（バイナリが肥大する）
//! - `unsafe` は最小限。書くときは直前に `// SAFETY:` を添える

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(clippy::undocumented_unsafe_blocks)]

pub mod arena;
pub mod config;
pub mod decode;
pub mod error;
pub mod float;
pub mod generated;
pub mod instance;
pub mod interp;
pub mod module;
pub mod reader;
pub mod types;
pub mod validate;

pub use arena::Arena;
pub use config::Config;
pub use error::{Error, ErrorKind, Result, Trap};
pub use instance::{Extern, ExternType, Instance, Resolver, instantiate};
pub use interp::{Exec, invoke};
pub use module::Module;
