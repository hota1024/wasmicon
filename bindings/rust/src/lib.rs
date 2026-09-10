//! Wasmicon HAL のゲスト向けバインディング（HANDOFF §5 Phase 3）。
//!
//! `generated.rs` は `wasmicon-gen` の出力（Phase 1）。手で編集しない。
//! ここには生成された extern 宣言を包む安全ラッパ
//! （`Pin` / `I2cBus` / `SpiBus`、`Drop` で `[resource-drop]` を呼ぶ）を書く。

#![no_std]
