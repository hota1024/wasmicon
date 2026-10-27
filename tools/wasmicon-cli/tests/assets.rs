//! Checks that what `wasmicon assets pack` writes can be read back through
//! the firmware's path (`assets::parse`), and the refusals before flashing
//! (docs/app-workflow.md §3.10).
//!
//! Calling the flasher needs hardware, so `assets write` is covered only up
//! to its first refusal.

use std::path::{Path, PathBuf};

use wasmicon_cli::{assets, manifest};
use wasmicon_port::assets as fmt;
use wasmicon_port::crc32;

fn tmp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("assets-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the work directory");
    dir
}

/// Writes a `wasmicon.toml` with an `[assets]` table and the files it names.
fn project(name: &str, files: &[(&str, &[u8])]) -> manifest::Manifest {
    let dir = tmp(name);
    let mut toml = String::from("version = 1\n[assets]\n");
    for (asset, body) in files {
        std::fs::write(dir.join(asset), body).expect("cannot write the asset");
        toml.push_str(&format!("\"{asset}\" = \"{asset}\"\n"));
    }
    std::fs::write(dir.join(manifest::FILE_NAME), toml).expect("cannot write the manifest");
    manifest::load(&dir.join(manifest::FILE_NAME)).unwrap_or_else(|e| panic!("{e:#}"))
}

#[test]
fn pack_writes_an_image_the_firmware_reads() {
    let pattern: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
    let m = project(
        "roundtrip",
        &[("hello.txt", b"Hello"), ("pattern.bin", &pattern)],
    );
    let dir = m.dir.clone();
    assets::pack(&assets::PackOptions {
        manifest: m,
        board: None,
        out: None,
    })
    .unwrap_or_else(|e| panic!("{e:#}"));

    let image = std::fs::read(dir.join("assets.bin")).expect("assets.bin next to the manifest");
    let read = fmt::parse(&image, image.len() as u32)
        .unwrap_or_else(|e| panic!("cannot parse: {}", e.reason()));
    assert_eq!(read.len(), 2);
    let h = read.entry(read.find(b"hello.txt").expect("hello.txt"));
    assert_eq!(h.len, 5);
    assert_eq!(h.crc, crc32(b"Hello"));
    assert_eq!(&image[h.offset as usize..][..5], b"Hello");
    let p = read.entry(read.find(b"pattern.bin").expect("pattern.bin"));
    assert_eq!(p.len, 5000);
    assert_eq!(p.crc, crc32(&pattern));
    assert_eq!(&image[p.offset as usize..][..5000], &pattern[..]);
    assert_eq!(image.len() % 4096, 0, "ends on an erase-unit boundary");
}

#[test]
fn pack_refuses_a_missing_file() {
    let m = project("missing", &[("present.txt", b"x")]);
    let dir = m.dir.clone();
    std::fs::remove_file(dir.join("present.txt")).expect("cannot remove");
    let e = assets::pack(&assets::PackOptions {
        manifest: m,
        board: None,
        out: None,
    })
    .expect_err("a missing file stops pack");
    let msg = format!("{e:#}");
    assert!(msg.contains("cannot read asset `present.txt`"), "{msg}");
    assert!(!dir.join("assets.bin").exists(), "nothing is written");
}

#[test]
fn pack_checks_the_fit_with_a_board() {
    // 1 MiB is more than the Pico WH's 978,944-byte slot and far less than
    // the ESP32-S3's 7 MiB.
    let big = vec![0xaa; 1 << 20];
    let m = project("fit", &[("big.bin", &big)]);
    let dir = m.dir.clone();
    let e = assets::pack(&assets::PackOptions {
        manifest: m,
        board: Some("rp2040".to_string()),
        out: None,
    })
    .expect_err("does not fit the rp2040 slot");
    assert!(format!("{e:#}").contains("do not fit"), "{e:#}");
    assert!(!dir.join("assets.bin").exists(), "rejected before writing");

    let m = manifest::load(&dir.join(manifest::FILE_NAME)).unwrap_or_else(|e| panic!("{e:#}"));
    assets::pack(&assets::PackOptions {
        manifest: m,
        board: Some("esp32s3".to_string()),
        out: Some(dir.join("out.bin")),
    })
    .unwrap_or_else(|e| panic!("fits the esp32s3 slot: {e:#}"));
    assert!(dir.join("out.bin").is_file(), "-o is honored");
}

#[test]
fn the_host_has_no_assets_slot() {
    let m = project("host", &[("a.txt", b"a")]);
    let e = assets::pack(&assets::PackOptions {
        manifest: m,
        board: Some("host".to_string()),
        out: None,
    })
    .expect_err("the host reads files");
    assert!(format!("{e:#}").contains("no assets slot"), "{e:#}");
}

#[test]
fn write_needs_a_board() {
    let m = project("write", &[("a.txt", b"a")]);
    let e = assets::write(&assets::WriteOptions {
        manifest: m,
        board: None,
        port: None,
        flash_baud: None,
    })
    .expect_err("no board, nothing to write to");
    assert!(format!("{e:#}").contains("no target board"), "{e:#}");
}

#[test]
fn an_empty_table_packs_a_directory_alone() {
    let m = project("empty", &[]);
    let dir = m.dir.clone();
    assets::pack(&assets::PackOptions {
        manifest: m,
        board: Some("rp2350".to_string()),
        out: None,
    })
    .unwrap_or_else(|e| panic!("{e:#}"));
    let image = std::fs::read(dir.join("assets.bin")).expect("written");
    assert_eq!(image.len(), fmt::BODY_START as usize);
    assert!(
        fmt::parse(&image, image.len() as u32)
            .unwrap_or_else(|e| panic!("{}", e.reason()))
            .is_empty()
    );
}
