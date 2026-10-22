//! Locks in what `wasmicon check` says about `[assets]` (docs/app-workflow.md §3.10).
//!
//! The shareware DOOM IWAD fits the ESP boards and not the Pico family, and a
//! file the manifest names but that does not exist would only show up as
//! `not-found` on the device. Both are decided before anything is flashed.

use std::path::{Path, PathBuf};

use wasmicon_cli::check::{self, AssetCheck};
use wasmicon_cli::manifest;
use wasmicon_port::profile;

fn tmp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("check-assets-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the work directory");
    dir
}

/// `(module (memory 1) (export "memory" ...) (func (export "run")))`, built
/// by hand: the smallest guest that passes every board.
fn minimal_wasm() -> Vec<u8> {
    let mut w = Vec::new();
    w.extend_from_slice(b"\0asm");
    w.extend_from_slice(&1u32.to_le_bytes());
    w.extend_from_slice(&[0x01, 0x04, 0x01, 0x60, 0x00, 0x00]);
    w.extend_from_slice(&[0x03, 0x02, 0x01, 0x00]);
    w.extend_from_slice(&[0x05, 0x03, 0x01, 0x00, 1]);
    let exports: &[u8] = &[
        0x02, //
        0x03, b'r', b'u', b'n', 0x00, 0x00, //
        0x06, b'm', b'e', b'm', b'o', b'r', b'y', 0x02, 0x00,
    ];
    w.push(0x07);
    w.push(u8::try_from(exports.len()).expect("export section too large"));
    w.extend_from_slice(exports);
    w.extend_from_slice(&[0x0a, 0x04, 0x01, 0x02, 0x00, 0x0b]);
    w
}

/// A project with one asset of `len` bytes (or none when `len` is `None`,
/// the file being named but absent).
fn project(name: &str, len: Option<usize>) -> (PathBuf, manifest::Manifest) {
    let dir = tmp(name);
    let wasm = dir.join("app.wasm");
    std::fs::write(&wasm, minimal_wasm()).expect("cannot write");
    if let Some(len) = len {
        std::fs::write(dir.join("big.bin"), vec![0xaa; len]).expect("cannot write");
    }
    std::fs::write(
        dir.join(manifest::FILE_NAME),
        "version = 1\n[assets]\n\"big.bin\" = \"big.bin\"\n",
    )
    .expect("cannot write");
    let m = manifest::load(&dir.join(manifest::FILE_NAME)).unwrap_or_else(|e| panic!("{e:#}"));
    (wasm, m)
}

fn run(wasm: PathBuf, m: manifest::Manifest, board: &str) -> bool {
    check::run(&check::Options {
        path: wasm,
        board: Some(board.to_string()),
        manifest: Some(m),
    })
    .unwrap_or_else(|e| panic!("{e:#}"))
}

#[test]
fn assets_fit_is_per_board() {
    // The shareware IWAD: 4,196,020 bytes.
    let checks = [AssetCheck {
        name: "doom1.wad".to_string(),
        path: PathBuf::from("doom1.wad"),
        len: Some(4_196_020),
    }];
    let image = check::assets_image_len(&checks).expect("sized");
    // The directory sector, then the body rounded up to 4 KiB.
    assert_eq!(image, 4096 + 4_198_400);
    assert_eq!(check::assets_fit(&profile::ESP32S3, image), Some(true));
    assert_eq!(check::assets_fit(&profile::ESP32C5, image), Some(true));
    assert_eq!(check::assets_fit(&profile::RP2350, image), Some(false));
    assert_eq!(check::assets_fit(&profile::RP2040, image), Some(false));
    assert_eq!(
        check::assets_fit(&profile::HOST, image),
        None,
        "the host serves files"
    );

    // A missing file cannot be sized.
    let missing = [AssetCheck {
        name: "doom1.wad".to_string(),
        path: PathBuf::from("doom1.wad"),
        len: None,
    }];
    assert!(check::assets_image_len(&missing).is_none());
    assert!(
        check::assets_image_len(&[]).is_some(),
        "no assets, just the directory"
    );
}

#[test]
fn a_missing_asset_file_fails_check() {
    let (wasm, m) = project("missing", None);
    let checks = check::asset_checks(Some(&m));
    assert_eq!(checks.len(), 1);
    assert!(checks[0].len.is_none(), "the file is not there");
    assert!(!run(wasm, m, "rp2350"), "fails before anything is flashed");

    let (wasm, m) = project("present", Some(1000));
    assert!(run(wasm, m, "rp2350"));

    // A directory where the file should be is "no file" too: `run` and
    // `assets pack` cannot read it, so `check` must not pass it.
    let (wasm, m) = project("directory", None);
    std::fs::create_dir(wasm.with_file_name("big.bin")).expect("cannot create");
    let checks = check::asset_checks(Some(&m));
    assert!(checks[0].len.is_none(), "a directory is not a file");
    assert!(!run(wasm, m, "rp2350"));
}

#[test]
fn an_image_too_big_for_the_board_fails_check() {
    // 1 MiB fits the ESP32-S3's slot and not the Pico WH's 978,944 bytes.
    let (wasm, m) = project("too-big", Some(1 << 20));
    assert!(!run(wasm.clone(), m, "rp2040"));
    let m = manifest::load(&wasm.with_file_name(manifest::FILE_NAME))
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert!(run(wasm, m, "esp32s3"));
}
