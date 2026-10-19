//! Embeds `git describe` for the identity line the firmware prints at boot
//! (docs/app-workflow.md §3.8).

include!("../common/build/git.rs");

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    emit_git_describe();
}
