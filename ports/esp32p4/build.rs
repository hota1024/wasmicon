//! Embeds `git describe` for the identity line the firmware prints at boot
//! (docs/app-workflow.md §3.8), lets the linker find `pre-v3.x`, and fails
//! the build when the local esp-sync copy is not the one in use.

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
    check_esp_sync_patch();
}

/// Fails if `Cargo.lock` takes esp-sync from crates.io.
///
/// `[patch.crates-io]` only applies while the local copy's version satisfies
/// every dependent. If an update made esp-hal ask for a newer esp-sync, cargo
/// would print "patch was not used" and quietly link the crates.io crate,
/// whose CSR 0x347 write makes `esp_hal::init` reset forever on silicon
/// before v3.0 (vendor/esp-sync/README.md). A patched package has no `source`
/// line in the lock file; one from the registry has.
fn check_esp_sync_patch() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let path = std::path::Path::new(&dir).join("Cargo.lock");
    println!("cargo:rerun-if-changed={}", path.display());
    let lock = std::fs::read_to_string(&path).expect("Cargo.lock is readable");
    let mut found = false;
    for package in lock.split("[[package]]") {
        if !package.contains("\nname = \"esp-sync\"\n") {
            continue;
        }
        found = true;
        assert!(
            !package.contains("\nsource = "),
            "esp-sync comes from crates.io, not vendor/esp-sync: \
             update the local copy (vendor/esp-sync/README.md)"
        );
    }
    assert!(found, "esp-sync is missing from Cargo.lock");
}
