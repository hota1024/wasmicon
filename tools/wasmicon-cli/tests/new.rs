//! Locks in that the `wasmicon new` scaffolds actually work
//! (docs/app-workflow.md §4.6, step 3).
//!
//! `tests/flags.rs` checks the byte match with `apps/`, but a scaffold can
//! match and still not work (a wrong dependency path, no `[workspace]` so
//! the parent workspace absorbs it, no `run` export). Here the scaffold is
//! really built and run through `check`.

use std::path::{Path, PathBuf};
use std::process::Command;

use wasmicon_cli::{check, manifest, new};
use wasmicon_port::profile;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root not found")
}

/// One work directory per test. A shared one breaks under parallel runs
/// (seen in `tests/manifest.rs`).
fn tmp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("new-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the work directory");
    dir
}

fn opts(dir: PathBuf, lang: new::Lang) -> new::Options {
    new::Options {
        dir,
        lang,
        board: None,
        // Passed explicitly. The upward search has its own test; here the
        // scaffold itself is under test, so the ambiguity is removed.
        hal: Some(repo_root().join("bindings")),
    }
}

fn created(o: &new::Options) -> new::Created {
    match new::create(o) {
        Ok(c) => c,
        Err(e) => panic!("cannot create the scaffold: {e:#}"),
    }
}

#[test]
fn the_rust_template_builds_and_passes_check_on_every_board() {
    // Serialized for the same reason as the `apps/` builds (`cargo` takes
    // the same registry lock).
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let made = created(&opts(tmp("rust").join("hello-app"), new::Lang::Rust));
    assert_eq!(made.name, "hello-app");
    assert!(made.hal.is_some(), "--hal was passed but not found");

    // A nested cargo may ignore rust-toolchain.toml, so the parent's
    // toolchain setting is removed, as in CI (same as `tests/deploy.rs`).
    let status = Command::new("cargo")
        .current_dir(&made.dir)
        .env_remove("RUSTUP_TOOLCHAIN")
        .args(["build", "--release"])
        .status()
        .expect("cannot start cargo");
    assert!(status.success(), "building the scaffold failed");

    let wasm = made
        .dir
        .join("target/wasm32-unknown-unknown/release/hello_app.wasm");
    let bytes =
        std::fs::read(&wasm).unwrap_or_else(|e| panic!("cannot read {}: {e}", wasm.display()));

    // No board-independent defects (import names, exports, memory).
    let facts = match check::facts(&bytes) {
        Ok(f) => f,
        Err(e) => panic!("check failed: {e:#}"),
    };
    assert_eq!(
        facts.failures(),
        0,
        "the scaffold has a board-independent defect"
    );

    // Passes on all four boards. The scaffold uses only log, so it must pass
    // even on rp2040, which has no SPI / I2C.
    for p in profile::PROFILES {
        let verdict = check::judge(&bytes, p, &facts, None, &[], None);
        assert!(
            verdict.is_ok(),
            "the scaffold fails on {} ({} failures)",
            p.name,
            verdict.fails()
        );
    }
}

#[test]
fn the_generated_manifest_round_trips() {
    // `wasmicon.toml` is `deny_unknown_fields`, so a scaffold with an extra
    // key could not be loaded. Not being able to read what it wrote itself
    // is the most absurd way to break, so it is locked in.
    let made = created(&opts(tmp("toml").join("round-trip"), new::Lang::Rust));
    let path = made.dir.join(manifest::FILE_NAME);
    let m = match manifest::load(&path) {
        Ok(m) => m,
        Err(e) => panic!("cannot load the scaffold's wasmicon.toml: {e:#}"),
    };
    // The scaffold uses no roles, so the list is empty. No default board is
    // chosen either.
    assert!(m.pin_roles.is_empty());
    assert!(m.default_board.is_none());

    // With `--board`, it is written and loads.
    let mut o = opts(tmp("toml-board").join("with-board"), new::Lang::Rust);
    o.board = Some("rp2350".to_string());
    let made = created(&o);
    let m = match manifest::load(&made.dir.join(manifest::FILE_NAME)) {
        Ok(m) => m,
        Err(e) => panic!("cannot load: {e:#}"),
    };
    assert_eq!(m.default_board.as_deref(), Some("rp2350"));
}

#[test]
fn the_as_template_builds() {
    let root = repo_root();
    assert!(
        root.join("node_modules/assemblyscript").exists(),
        "AssemblyScript is not installed; run `npm install` in the repository root"
    );

    let made = created(&opts(tmp("as").join("hello-as"), new::Lang::AssemblyScript));
    // asc runs on the scaffold, but with the repository root's
    // `node_modules` (so `npx` is run from there).
    let status = Command::new("npx")
        .current_dir(&root)
        .args([
            "asc",
            made.dir
                .join("assembly/index.ts")
                .to_str()
                .expect("path is not UTF-8"),
            "--config",
            made.dir
                .join("asconfig.json")
                .to_str()
                .expect("path is not UTF-8"),
            "--target",
            "release",
        ])
        .status()
        .expect("cannot start npx");
    assert!(status.success(), "building the AS scaffold failed");

    let wasm = made.dir.join("build/hello_as.wasm");
    let bytes =
        std::fs::read(&wasm).unwrap_or_else(|e| panic!("cannot read {}: {e}", wasm.display()));
    let facts = match check::facts(&bytes) {
        Ok(f) => f,
        Err(e) => panic!("check failed: {e:#}"),
    };
    assert_eq!(facts.failures(), 0, "the AS scaffold has a defect");
}

#[test]
fn the_written_hal_path_actually_resolves() {
    // Whether relative or absolute was chosen, a path that does not resolve
    // stops `cargo build` with "not found". A naive join of the root once
    // produced a double slash, so this is locked in.
    let made = created(&opts(tmp("resolve").join("resolve-app"), new::Lang::Rust));
    let cargo = std::fs::read_to_string(made.dir.join("Cargo.toml")).expect("cannot read");
    let written = cargo
        .lines()
        .find_map(|l| l.strip_prefix("wasmicon-hal = { path = \""))
        .and_then(|l| l.split('"').next())
        .expect("cannot find the dependency path");
    assert!(
        !written.contains("//"),
        "double slash in the path: {written}"
    );
    let resolved = made.dir.join(written).join("Cargo.toml");
    assert!(
        resolved.exists(),
        "the written path does not resolve: {written} ({})",
        resolved.display()
    );

    // Same on the AS side. If the relative import does not resolve, asc
    // fails.
    let made = created(&opts(
        tmp("resolve-as").join("resolve-as"),
        new::Lang::AssemblyScript,
    ));
    let index = std::fs::read_to_string(made.dir.join("assembly/index.ts")).expect("cannot read");
    let import = index
        .lines()
        .find_map(|l| l.split_once("from \"").map(|(_, r)| r))
        .and_then(|r| r.split('"').next())
        .expect("cannot find the import");
    assert!(
        !import.contains("//"),
        "double slash in the import: {import}"
    );
    let resolved = made.dir.join("assembly").join(format!("{import}.ts"));
    assert!(
        resolved.exists(),
        "the import does not resolve: {import} ({})",
        resolved.display()
    );
}

#[test]
fn it_refuses_to_write_into_a_non_empty_directory() {
    let dir = tmp("non-empty").join("taken");
    std::fs::create_dir_all(&dir).expect("cannot create");
    std::fs::write(dir.join("README.md"), "already here").expect("cannot write");

    let err = new::create(&opts(dir.clone(), new::Lang::Rust))
        .err()
        .expect("passed although the directory is not empty");
    assert!(format!("{err:#}").contains("is not empty"), "{err:#}");
    // What was there is not deleted.
    assert!(dir.join("README.md").is_file());
}

#[test]
fn it_refuses_a_hal_path_without_bindings() {
    // `--hal` is inspected too. The search branch (`find_bindings`) requires
    // `bindings/rust/Cargo.toml`, but `--hal` used to accept any existing
    // directory. Then `<given>/rust` went into Cargo.toml and nothing was
    // noticed until `cargo build` failed with "no Cargo.toml".
    let base = tmp("bad-hal");
    let empty = base.join("not-bindings");
    std::fs::create_dir_all(&empty).expect("cannot create");

    let mut o = opts(base.join("app"), new::Lang::Rust);
    o.hal = Some(empty.clone());
    let err = new::create(&o)
        .err()
        .expect("passed although there are no bindings");
    let msg = format!("{err:#}");
    assert!(msg.contains("no bindings"), "{msg}");
    // Says what it looked for (the most useful thing when the wrong level
    // was passed).
    assert!(msg.contains("rust/Cargo.toml"), "{msg}");

    // A path that does not exist is rejected as before.
    let mut o = opts(base.join("app2"), new::Lang::Rust);
    o.hal = Some(base.join("nowhere"));
    let msg = format!("{:#}", new::create(&o).err().expect("passed"));
    assert!(msg.contains("--hal"), "{msg}");
}

#[test]
fn it_refuses_an_unknown_board() {
    // `manifest::load` does not validate `[defaults] board`, so unless
    // rejected here, `new` passes and every later command fails.
    let mut o = opts(tmp("bad-board").join("bad-board"), new::Lang::Rust);
    o.board = Some("rp2040x".to_string());
    let err = new::create(&o).err().expect("an unknown board passed");
    let msg = format!("{err:#}");
    assert!(msg.contains("unknown board"), "{msg}");
    // Lists the candidates (the most useful thing after a typo).
    assert!(msg.contains("rp2040"), "{msg}");
}

#[test]
fn it_refuses_a_name_cargo_cannot_use() {
    let base = tmp("bad-name");
    for bad in ["1st-app", "my app", "app.rs"] {
        let err = new::create(&opts(base.join(bad), new::Lang::Rust))
            .err()
            .unwrap_or_else(|| panic!("{bad} passed"));
        let msg = format!("{err:#}");
        assert!(msg.contains("app name"), "{bad}: {msg}");
    }
}

#[test]
fn it_finds_the_bindings_by_walking_up() {
    // The path without `--hal`. Created inside the repository, the bindings
    // must be found. `CARGO_TARGET_TMPDIR` is under the repository's
    // `target/`, so a scaffold there finds `bindings/` by walking up.
    let made = created(&new::Options {
        dir: tmp("walkup").join("found-app"),
        lang: new::Lang::Rust,
        board: None,
        hal: None,
    });
    let hal = made
        .hal
        .expect("inside the repository, but the bindings were not found");
    assert!(hal.join("rust/Cargo.toml").is_file());
    // The dependency is written as a path (a version cannot be resolved
    // yet).
    let cargo = std::fs::read_to_string(made.dir.join("Cargo.toml")).expect("cannot read");
    assert!(
        cargo.contains("path = \""),
        "the dependency is not written as a path:\n{cargo}"
    );
    // Inside the repository's workspace, so the note applies.
    assert!(made.inside_workspace);
}
