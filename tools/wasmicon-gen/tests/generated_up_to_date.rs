//! コミット済みの生成物が `wit/` と一致することを検査する。
//!
//! docs/handoff.md §5 Phase 1 の完了条件「CI で WIT 変更時に再生成して diff がゼロ」に対応する。
//! `cargo run -p wasmicon-gen -- --check` と同じ検査をテストとして回す。

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("リポジトリルートが見つからない")
}

#[test]
fn committed_outputs_match_wit() {
    let root = repo_root();
    let outputs = wasmicon_gen::outputs(&root).expect("生成に失敗した");
    assert_eq!(outputs.len(), 3, "生成物は 3 ファイル");

    for o in &outputs {
        let on_disk = std::fs::read_to_string(&o.path)
            .unwrap_or_else(|e| panic!("{} を読めない: {e}", o.path.display()));
        assert_eq!(
            on_disk,
            o.contents,
            "{} が wit/ と一致しない。`cargo run -p wasmicon-gen` で再生成すること",
            o.path.display()
        );
    }
}
