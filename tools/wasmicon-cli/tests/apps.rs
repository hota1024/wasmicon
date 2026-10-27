//! Checks that every app in this repo's `apps/` has its own `wasmicon.toml`
//! (dogfooding; docs/app-workflow.md §4.7).
//!
//! The firmware has no default role table (docs/app-workflow.md §3.9), so an
//! app that looks up roles gets `unsupported` on hardware without a wiring
//! table. This stops "added an app but forgot the manifest" and "declaration
//! and wiring table disagree". Zero apps is not a silent success (same
//! reasoning as `tests/flags.rs`).

use std::path::{Path, PathBuf};

use wasmicon_cli::{check, manifest};
use wasmicon_port::profile;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root not found")
}

/// Directories directly under `apps/` that are a Rust (`Cargo.toml`),
/// AssemblyScript (`asconfig.json`) or C (`src/main.c`) app.
fn app_dirs() -> Vec<PathBuf> {
    let apps = repo_root().join("apps");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&apps)
        .expect("cannot read apps/")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.join("Cargo.toml").is_file()
                || p.join("asconfig.json").is_file()
                || p.join("src/main.c").is_file()
        })
        .collect();
    dirs.sort();
    dirs
}

#[test]
fn every_app_has_a_manifest_whose_roles_are_wired_on_both_boards() {
    let dirs = app_dirs();
    assert!(dirs.len() >= 5, "no apps found in apps/: {dirs:?}");
    for dir in dirs {
        let path = dir.join(manifest::FILE_NAME);
        assert!(path.is_file(), "{} is missing", path.display());
        let m = manifest::load(&path).unwrap_or_else(|e| panic!("{e:#}"));
        // On the evaluation boards (README.md) every declared role is wired.
        for board in [&profile::RP2350, &profile::ESP32S3] {
            let provided = check::provided_roles(board, Some(&m));
            for role in &m.pin_roles {
                assert!(
                    provided.contains(role),
                    "{}: {role} is not in [board.{}.roles]",
                    path.display(),
                    board.name
                );
            }
            // The table can be written to the config slot.
            let map = m.role_map(board).unwrap_or_else(|e| panic!("{e:#}"));
            assert_eq!(map.len(), m.pin_roles.len(), "{}", path.display());
        }
    }
}

#[test]
fn every_asset_in_a_manifest_exists() {
    // `[assets]` names the files `run` serves and `assets write` flashes
    // (docs/app-workflow.md §3.10). A missing one would only show up as
    // `not-found` on the device, so it is caught here.
    let mut assets = 0;
    for dir in app_dirs() {
        let path = dir.join(manifest::FILE_NAME);
        let m = manifest::load(&path).unwrap_or_else(|e| panic!("{e:#}"));
        for (name, file) in &m.assets {
            assert!(
                file.is_file(),
                "{}: asset `{name}` is missing at {}",
                path.display(),
                file.display()
            );
            assets += 1;
        }
    }
    assert!(assets > 0, "no app ships an asset; apps/assets-rs should");
}

#[test]
fn a_display_entry_covers_both_evaluation_boards() {
    // An app that sets up a display for one evaluation board but not the
    // other would pass `check` on one and fail on the other
    // (docs/device-layer.md §6). The demo must have one.
    let mut seen = 0;
    for dir in app_dirs() {
        let path = dir.join(manifest::FILE_NAME);
        let m = manifest::load(&path).unwrap_or_else(|e| panic!("{e:#}"));
        if m.board_display.is_empty() {
            continue;
        }
        seen += 1;
        for board in [&profile::RP2350, &profile::ESP32S3] {
            assert!(
                m.display(board).is_some(),
                "{}: no [board.{}.display]",
                path.display(),
                board.name
            );
        }
    }
    assert!(seen >= 1, "apps/display-demo-rs has display entries");
}
