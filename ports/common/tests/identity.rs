//! Locks in the one-line identity the firmware reports (docs/app-workflow.md §3.8).
//!
//! `trace diff` finds this line by `identity::PREFIX` to compare two logs, so a
//! change of shape would silently disable that check.

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
    // rp2040 has no I2C / SPI yet. A firmware built without trace prints
    // nothing to monitor, so the identity line has to say so.
    let l = line(&profile::RP2040, false);
    assert!(l.contains(" trace=off "), "{l}");
    assert!(l.contains(" if=gpio,time,log,board "), "{l}");
    assert!(l.contains(" pages=2 "), "{l}");
}

#[test]
fn the_longest_line_fits_the_ports_buffer() {
    // The port builds the line in a 192-byte buffer; an overflow is silently truncated.
    for p in profile::PROFILES {
        let l = line(p, false);
        assert!(l.len() < 160, "{}: {} bytes", p.name, l.len());
        assert!(l.starts_with(identity::PREFIX));
    }
}
