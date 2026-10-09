//! ファームが名乗る 1 行の形を固定する（`docs/app-workflow.md` §3.8）。
//!
//! `trace diff` はこの行を `identity::PREFIX` で見つけて 2 つのログを比べるので、
//! 形が変わると照合が黙って効かなくなる。

use wasmicon_port::fmt::Buf;
use wasmicon_port::identity::{self, Identity};
use wasmicon_port::profile;

fn line(p: &'static profile::Profile, trace: bool) -> String {
    let mut buf = [0u8; 192];
    let mut out = Buf::new(&mut buf);
    identity::describe(
        &Identity {
            profile: p,
            fw: "0.1.0",
            git: "ba2ce81-dirty",
            trace,
        },
        &mut out,
    );
    String::from_utf8(out.as_bytes().to_vec()).expect("ASCII")
}

#[test]
fn every_axis_is_named() {
    assert_eq!(
        line(&profile::RP2350, true),
        "wasmicon id rp2350 fw=0.1.0 git=ba2ce81-dirty abi=wasmicon:hal@0.1.0 \
         trace=on pages=4 if=gpio,i2c,spi,time,log,board slot=v1/65536 roles=v1/4096"
    );
}

#[test]
fn unimplemented_interfaces_and_trace_off_are_visible() {
    // rp2040 は I2C / SPI が未実装。trace を切ったファームは monitor に何も
    // 出さないので、名乗りで分かるようにする。
    let l = line(&profile::RP2040, false);
    assert!(l.contains(" trace=off "), "{l}");
    assert!(l.contains(" if=gpio,time,log,board "), "{l}");
    assert!(l.contains(" pages=2 "), "{l}");
}

#[test]
fn the_longest_line_fits_the_ports_buffer() {
    // ポートは 192 バイトのバッファで組む。溢れると黙って切り詰められる。
    for p in profile::PROFILES {
        let l = line(p, false);
        assert!(l.len() < 160, "{}: {} バイト", p.name, l.len());
        assert!(l.starts_with(identity::PREFIX));
    }
}
