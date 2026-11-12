//! Locks in the bytes of a line (`ports/common/src/serial.rs`).
//!
//! Every port writes its lines through `serial::write_parts`, so this is the
//! byte stream UART0 carries. A change would change every captured trace.

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
