//! Guest-side bindings of the Wasmicon HAL.
//!
//! `generated` is the output of `wasmicon-gen`; do not edit it by hand.
//! `hal` wraps the generated extern declarations in a safe API.

#![no_std]
#![warn(clippy::undocumented_unsafe_blocks)]

pub mod generated;
mod hal;

pub use generated::types::ErrorCode;
pub use hal::{Result, assets, board, gpio, i2c, log, spi, time};
