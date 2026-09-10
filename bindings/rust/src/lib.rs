//! Wasmicon HAL のゲスト向けバインディング。
//!
//! `generated` は `wasmicon-gen` の出力（Phase 1）。手で編集しない。
//! `hal` は生成された extern 宣言を包む安全なラッパ（docs/handoff.md §5 Phase 3）。

#![no_std]

pub mod generated;
mod hal;

pub use generated::types::ErrorCode;
pub use hal::{Result, board, gpio, i2c, log, spi, time};
