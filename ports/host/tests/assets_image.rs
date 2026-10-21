//! The host builds assets slot images in the firmware's layout (docs/app-workflow.md §3.10).

use wasmicon_host::assets;
use wasmicon_port::assets::{AssetsError, BODY_START, DIR_LEN};
use wasmicon_port::crc32;

fn entries(list: &[(&str, &[u8])]) -> Vec<(String, Vec<u8>)> {
    list.iter()
        .map(|(n, b)| ((*n).to_string(), b.to_vec()))
        .collect()
}

#[test]
fn an_image_round_trips_through_the_firmware_parser() {
    let pattern: Vec<u8> = (0..4100u32).map(|i| (i * 7) as u8).collect();
    let list = entries(&[("hello.txt", b"Hello"), ("pattern.bin", &pattern)]);
    let img = assets::image(&list).unwrap_or_else(|e| panic!("builds: {}", e.reason()));
    assert_eq!(&img[..4], b"WMCB");
    // Directory sector, one sector for "Hello", two for the 4,100-byte pattern.
    assert_eq!(img.len(), 4 * 4096);

    let dir = assets::directory(&img).unwrap_or_else(|e| panic!("parses: {}", e.reason()));
    assert_eq!(dir.len(), 2);
    let h = dir.entry(0);
    assert_eq!(h.name(), b"hello.txt");
    assert_eq!(h.offset, BODY_START);
    assert_eq!(h.len, 5);
    assert_eq!(h.crc, crc32(b"Hello"));
    assert_eq!(&img[4096..4101], b"Hello");
    assert_eq!(img[4101], 0xff, "padding reads as erased flash");
    let p = dir.entry(1);
    assert_eq!(p.name(), b"pattern.bin");
    assert_eq!(p.offset, 8192);
    assert_eq!(p.len, 4100);
    assert_eq!(&img[8192..8192 + 4100], &pattern[..]);
    assert!(img[DIR_LEN..4096].iter().all(|b| *b == 0xff));
}

#[test]
fn no_assets_is_a_directory_alone() {
    let img = assets::image(&[]).unwrap_or_else(|e| panic!("builds: {}", e.reason()));
    assert_eq!(img.len(), BODY_START as usize);
    let dir = assets::directory(&img).unwrap_or_else(|e| panic!("parses: {}", e.reason()));
    assert!(dir.is_empty());
}

#[test]
fn the_builder_rejects_what_the_firmware_would() {
    let err = |list: Vec<(String, Vec<u8>)>| match assets::image(&list) {
        Err(e) => e,
        Ok(_) => panic!("built an image the firmware would reject"),
    };
    assert!(matches!(
        err(entries(&[("Bad", b"")])),
        AssetsError::BadName
    ));
    assert!(matches!(
        err(entries(&[("a", b""), ("a", b"")])),
        AssetsError::DuplicateName
    ));
    let nine: Vec<(String, Vec<u8>)> = (0..9).map(|i| (format!("a{i}"), Vec::new())).collect();
    assert!(matches!(err(nine), AssetsError::TooMany));
}
