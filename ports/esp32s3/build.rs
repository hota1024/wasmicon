//! `git describe` を埋める（ファームが起動時に名乗る。docs/app-workflow.md §3.8）。

include!("../common/build/git.rs");

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    emit_git_describe();
}
