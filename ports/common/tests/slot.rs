//! アプリスロットの形式を固定する（`docs/app-workflow.md` §3.4）。
//!
//! 書く側（CLI）と読む側（ファーム）が同じ形を使うので、ここが両方の契約。
//! **形を変えたら `FORMAT_VERSION` を上げる**（互換の軸の 1 つ。§3.6）。

use wasmicon_port::profile::Slot;
use wasmicon_port::slot::{self, SlotError};

/// 読めることを確かめて中身を返す。
///
/// `SlotError` は `Debug` を実装しない（`ports/common` はポートと同じ
/// 制約で書くので `core::fmt` を持ち込まない）。失敗は `reason()` で出す。
fn parsed(img: &[u8]) -> &[u8] {
    slot::parse(img).unwrap_or_else(|e| panic!("読めない: {}", e.reason()))
}

/// 読めないことを確かめて理由を返す。
fn parse_err(img: &[u8], what: &str) -> SlotError {
    match slot::parse(img) {
        Err(e) => e,
        Ok(_) => panic!("{what} を期待したが読めてしまった"),
    }
}

/// 書いて読む。
fn image(wasm: &[u8]) -> Vec<u8> {
    let mut v = slot::header(wasm).to_vec();
    v.extend_from_slice(wasm);
    assert_eq!(v.len(), slot::image_len(wasm.len()));
    v
}

#[test]
fn a_written_slot_reads_back_byte_for_byte() {
    // 中身は検査しないので、バイト列として往復することだけを見る。
    let wasm = b"\0asm\x01\0\0\0not-really-wasm-but-round-trips";
    let img = image(wasm);
    assert_eq!(parsed(&img), wasm);
}

#[test]
fn the_wasm_starts_at_a_16_byte_boundary() {
    // スライスをそのまま decode に渡せることの前提（§3.4）。
    assert_eq!(slot::HEADER_LEN, 16);
    let img = image(b"xyz");
    assert_eq!(&img[16..], b"xyz");
}

#[test]
fn an_empty_slot_is_not_an_error() {
    // 消去済みのフラッシュは 0xff、まだ書いていない RAM は 0x00。
    // **どちらも「空」**で、ファームはこれを見て idle に入る（§3.1）。
    for fill in [0xffu8, 0x00] {
        let e = parse_err(&[fill; 256], "空");
        assert!(e.is_empty(), "fill={fill:#x} は空として扱う");
        assert_eq!(e.reason(), "slot empty");
    }
    // ヘッダに届かない短さも空扱い（消去済みの先頭を見たのと同じ）。
    assert!(parse_err(&[], "空").is_empty());
    assert!(parse_err(&[0xff; 4], "空").is_empty());
}

#[test]
fn other_contents_are_distinguished_from_empty() {
    // 別のものが書かれているのは「空」ではない。ログの意味が変わる。
    let mut img = image(b"xyz");
    img[0] = b'X';
    let e = parse_err(&img, "magic 違い");
    assert!(!e.is_empty());
    assert!(matches!(e, SlotError::BadMagic), "{}", e.reason());
}

#[test]
fn a_newer_format_is_refused_with_its_version() {
    let mut img = image(b"xyz");
    img[4..6].copy_from_slice(&2u16.to_le_bytes());
    let e = parse_err(&img, "版違い");
    assert!(
        matches!(e, SlotError::UnsupportedVersion { found: 2 }),
        "{}",
        e.reason()
    );
}

#[test]
fn a_length_past_the_slot_is_refused() {
    let mut img = image(b"xyz");
    img[8..12].copy_from_slice(&999u32.to_le_bytes());
    let e = parse_err(&img, "長さが外を指す");
    let have = img.len();
    assert!(
        matches!(e, SlotError::Truncated { need, have: h } if need == 16 + 999 && h == have),
        "{}",
        e.reason()
    );
}

#[test]
fn a_corrupted_byte_is_caught_by_the_crc() {
    // **これが CRC を入れる理由。** 無いと壊れたバイト列を decode させて
    // 「対応外の命令」のような無関係な理由が出る。
    let wasm = b"\0asm\x01\0\0\0aaaaaaaaaaaaaaaa";
    let mut img = image(wasm);
    let last = img.len() - 1;
    img[last] ^= 0x01;
    match parse_err(&img, "CRC が合わない") {
        SlotError::BadCrc { want, got } => assert_ne!(want, got),
        other => panic!("CRC 不一致を期待した: {}", other.reason()),
    }
}

#[test]
fn trailing_bytes_in_the_slot_are_ignored() {
    // スロット領域全体を渡してよい（長さはヘッダが持つ）。4 KiB の
    // セクタをまるごと読んで渡す使い方を想定している。
    let wasm = b"\0asm\x01\0\0\0";
    let mut img = image(wasm);
    img.resize(4096, 0xff);
    assert_eq!(parsed(&img), wasm);
}

#[test]
fn an_overlapping_firmware_is_refused_without_reading() {
    // **重なっていたらスライスを作らない**（自分のコードを wasm として
    // 食わせてしまう）。アドレスを渡すだけなので、ここは host で試せる。
    let slot = Slot {
        offset: 0x10_0000,
        len: 64 * 1024,
    };
    let xip = 0x1000_0000;
    let slot_start = xip + slot.offset as usize;
    // SAFETY: 重なっている側に入るので、スライスは作られない。
    let e = match unsafe { slot::read_xip(xip, slot, slot_start + 1) } {
        Err(e) => e,
        Ok(_) => panic!("重なりを検出していない"),
    };
    assert_eq!(e.reason(), "firmware overlaps the app slot");
    assert!(matches!(e, SlotError::Overlap { .. }));
}

#[test]
fn a_mapped_slot_is_read_in_place() {
    // XIP の読み出しそのものは host でも試せる（static をフラッシュに
    // 見立てて、そのアドレスを xip_base として渡す）。
    let wasm = b"\0asm\x01\0\0\0mapped";
    let mut region = image(wasm);
    region.resize(4096, 0xff);
    let slot = Slot {
        offset: 0,
        len: u32::try_from(region.len()).expect("収まる"),
    };
    // SAFETY: region は生きていて、長さぶん読める。
    let read = match unsafe { slot::read_xip(region.as_ptr() as usize, slot, 0) } {
        Ok(w) => w,
        Err(e) => panic!("読めない: {}", e.reason()),
    };
    assert_eq!(read, wasm);
}

#[test]
fn an_empty_app_is_still_a_valid_slot() {
    // 長さ 0 の wasm は decode で弾かれる（ここの仕事ではない）。
    // スロットとしては成立することを決めておく。
    let img = image(b"");
    assert_eq!(parsed(&img), b"");
}
