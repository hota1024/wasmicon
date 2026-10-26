//! Embeds `git describe` for the identity line the firmware prints at boot
//! (docs/app-workflow.md §3.8), and lets the linker find `pre-v3.x`.

include!("../common/build/git.rs");

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=pre-v3.x");
    // The linker runs in another directory, so `-Tpre-v3.x` in
    // .cargo/config.toml needs this crate's directory on the search path.
    println!(
        "cargo:rustc-link-search={}",
        std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR")
    );
    emit_git_describe();
}
