//! Checks that the committed generated files match `wit/`.
//!
//! Locks in that a WIT change is never committed without regenerating: the same check
//! as `cargo run -p wasmicon-gen -- --check`, run as a test.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root not found")
}

#[test]
fn committed_outputs_match_wit() {
    let root = repo_root();
    let outputs = wasmicon_gen::outputs(&root).expect("generation failed");
    assert_eq!(outputs.len(), 4, "expected 4 generated files");

    for o in &outputs {
        let on_disk = std::fs::read_to_string(&o.path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", o.path.display()));
        assert_eq!(
            on_disk,
            o.contents,
            "{} does not match wit/; run `cargo run -p wasmicon-gen`",
            o.path.display()
        );
    }
}
