//! Locks in the bytes of a line (`ports/common/src/serial.rs`).
//!
//! The five embedded ports write their UART0 lines through
//! `serial::write_parts`, so this is the byte stream UART0 carries there. A
//! change would change every captured trace. The host port prints through
//! `println!` and is not covered.

use wasmicon_port::serial;

fn bytes(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    serial::write_parts(parts, |b| out.extend_from_slice(b));
    out
}

#[test]
fn parts_are_joined_and_end_in_crlf() {
    assert_eq!(bytes(&[b"[wasm] ", b"hello"]), b"[wasm] hello\r\n");
}

#[test]
fn no_parts_is_an_empty_line() {
    assert_eq!(bytes(&[]), b"\r\n");
}
