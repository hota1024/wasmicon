//! Serial output of the ports, one line at a time.
//!
//! Every line the firmware prints goes through `Serial::write_line` whole: the
//! guest `log`, the trace, and the firmware's own messages (banner, slot lines,
//! trap reason). An output that may drop (the USB console of
//! docs/decisions.md A-11, which must not hang without a host) can then drop a
//! whole line and never cut one in two, which would glue the next trace line
//! to its tail (docs/TODO.md §5-11). The kind lets an output pick what it
//! carries. There is no method for bare bytes, so a caller cannot write half a
//! line.

/// What a line is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineKind {
    /// A guest `log` call (`[wasm] ...`).
    Log,
    /// A trace line (docs/abi-spec.md §9).
    Trace,
    /// The firmware's own output: banner, identity, slot lines, trap reason,
    /// probe output.
    Firmware,
}

/// End of every line.
pub const LINE_END: &[u8] = b"\r\n";

/// Serial output of a port.
pub trait Serial {
    /// Writes `parts` back to back as one line and ends it with `LINE_END`.
    /// `parts` hold no line end themselves.
    fn write_line(&mut self, kind: LineKind, parts: &[&[u8]]);
}

/// Hands `parts`, then `LINE_END`, to `write` in order. For a `Serial` impl
/// over a byte stream, so every port ends lines the same way.
pub fn write_parts<F: FnMut(&[u8])>(parts: &[&[u8]], mut write: F) {
    for part in parts {
        write(part);
    }
    write(LINE_END);
}
