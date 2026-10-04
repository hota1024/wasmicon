//! `wasmicon` CLI の中身（`docs/app-workflow.md` §4）。
//!
//! 判定を `main` から分けて lib に置いてあるのは、テストから呼べるようにする
//! ため（`wasmicon-gen` と同じ構成）。

pub mod check;
pub mod doctor;
pub mod run;
pub mod trace;
