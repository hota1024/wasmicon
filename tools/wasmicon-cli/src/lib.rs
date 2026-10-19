//! Implementation of the `wasmicon` CLI (docs/app-workflow.md §4).
//!
//! The decisions live in this library rather than in `main` so that tests can
//! call them (the same layout as `wasmicon-gen`).

pub mod check;
pub mod deploy;
pub mod doctor;
pub mod flags;
pub mod manifest;
pub mod monitor;
pub mod new;
pub mod pack;
pub mod run;
pub mod trace;
