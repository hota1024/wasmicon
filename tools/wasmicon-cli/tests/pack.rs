//! `wasmicon pack` が書いたものを、ファームが読む経路（`slot::parse`）で
//! 読み直せることを確かめる。
//!
//! 書く側（CLI）と読む側（ファーム）は同じ `ports/common` の `slot` を使うが、
//! **実際に往復させておかないと「どちらも同じ間違いをしている」を検出できない。**
//! CRC が標準の CRC-32 であることも、ここで外から確かめる。

use std::path::{Path, PathBuf};

use wasmicon_cli::pack;
use wasmicon_port::slot;

/// 読めないことを確かめて理由を返す（`SlotError` は `Debug` を実装しない）。
fn parse_err(img: &[u8], what: &str) -> slot::SlotError {
    match slot::parse(img) {
        Err(e) => e,
        Ok(_) => panic!("{what} を期待したが読めてしまった"),
    }
}

fn tmp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("pack-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("作業ディレクトリを作れない");
    dir
}

/// `pack` を走らせて書かれた画像を返す。
fn pack_bytes(name: &str, wasm: &[u8]) -> Vec<u8> {
    let dir = tmp(name);
    let input = dir.join("app.wasm");
    std::fs::write(&input, wasm).expect("書けない");
    let out = dir.join("app.slot");
    pack::run(&pack::Options {
        path: input,
        out: Some(out.clone()),
    })
    .expect("pack できる");
    std::fs::read(&out).expect("読めない")
}

#[test]
fn what_pack_writes_is_what_the_firmware_reads() {
    let wasm = b"\0asm\x01\0\0\0some-bytes-standing-in-for-a-guest";
    let image = pack_bytes("roundtrip", wasm);
    let read = slot::parse(&image).unwrap_or_else(|e| panic!("読めない: {}", e.reason()));
    assert_eq!(read, wasm, "バイト単位で往復する");
}

#[test]
fn the_default_output_sits_next_to_the_input() {
    let dir = tmp("default-out");
    let input = dir.join("app.wasm");
    std::fs::write(&input, b"\0asm\x01\0\0\0").expect("書けない");
    pack::run(&pack::Options {
        path: input,
        out: None,
    })
    .expect("pack できる");
    assert!(dir.join("app.slot").is_file(), "<入力>.slot に書く");
}

#[test]
fn the_header_is_the_documented_layout() {
    // docs/app-workflow.md §3.4 の表。**ここが書く側と読む側の契約**なので、
    // 配置を変えたら FORMAT_VERSION を上げる。
    let wasm = b"abcd";
    let image = pack_bytes("layout", wasm);

    assert_eq!(&image[0..4], b"WMCA", "magic");
    assert_eq!(
        u16::from_le_bytes([image[4], image[5]]),
        1,
        "フォーマット版"
    );
    assert_eq!(u16::from_le_bytes([image[6], image[7]]), 0, "予約");
    assert_eq!(
        u32::from_le_bytes([image[8], image[9], image[10], image[11]]),
        4,
        "wasm の長さ"
    );
    assert_eq!(&image[16..], wasm, "wasm は 16 バイト目から");

    // CRC-32 (IEEE) であることを、既知の値で外から確かめる。
    // "abcd" の CRC-32 は 0xed82cd11。
    let crc = u32::from_le_bytes([image[12], image[13], image[14], image[15]]);
    assert_eq!(crc, 0xed82_cd11, "標準の CRC-32");
}

#[test]
fn a_flipped_bit_in_the_image_is_caught() {
    // スロットに CRC を置いた理由。これが無いと、壊れたバイト列を decode
    // させて「対応外の命令」のような無関係な理由が出る。
    let mut image = pack_bytes("corrupt", b"\0asm\x01\0\0\0aaaaaaaa");
    let last = image.len() - 1;
    image[last] ^= 0x01;
    let e = parse_err(&image, "CRC が合わない");
    assert_eq!(e.reason(), "slot crc mismatch");
}
