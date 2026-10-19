//! Core Wasm interpreter for Wasmicon.
//!
//! The rules for this crate live in `runtime/AGENTS.md`:
//!
//! - `no_std`, zero dependency crates, no `alloc`; the port injects the arena
//!   (docs/decisions.md D-11)
//! - Wasm arithmetic wraps: use `wrapping_*` / `rotate_*` / `checked_*` explicitly
//! - no `core::fmt`, which bloats the binary (docs/decisions.md P-09)
//! - `unsafe` is kept to a minimum, and every block carries a `// SAFETY:` comment

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
