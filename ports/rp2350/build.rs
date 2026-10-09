//! `memory.x` をリンカが見つけられる場所へ置き、`git describe` を埋める。

use std::io::Write;

include!("../common/build/git.rs");

fn main() {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    std::fs::File::create(out.join("memory.x"))
        .unwrap()
        .write_all(include_bytes!("memory.x"))
        .unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=build.rs");
    emit_git_describe();
}
