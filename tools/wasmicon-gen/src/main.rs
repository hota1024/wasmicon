//! `wit/` を読み、`docs/abi-spec.md` の規則で生成物を出力する（HANDOFF §5 Phase 1）。
//!
//! 出力:
//! - `runtime/src/generated.rs`
//! - `bindings/rust/src/generated.rs`
//! - `bindings/assemblyscript/assembly/generated.ts`
//!
//! サブセット外の WIT 構文はエラーで拒否する。

fn main() {
    eprintln!("wasmicon-gen: 未実装（HANDOFF §5 Phase 1）");
    std::process::exit(1);
}
