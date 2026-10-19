//! Locks in when `wasmicon monitor` stops.
//!
//! The serial port itself needs hardware, so `stream` is fed an arbitrary
//! `Read` and only "when, and for which reason, it stopped" is checked.
//!
//! The reason appears in the report. Getting it wrong makes a capture cut
//! off mid-stream say "went silent" and look like a clean end; the trace
//! comparison then notices only when the line counts differ.

use std::io::Read;
use std::time::Duration;

use wasmicon_cli::monitor::{self, Stopped};

/// A serial port that returns once and then stays silent.
struct OnceThenSilent {
    sent: bool,
}

impl Read for OnceThenSilent {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if !self.sent {
            self.sent = true;
            let line = b"> wasmicon:hal/log@0.1.0/log(2, \"x\")\n";
            buf[..line.len()].copy_from_slice(line);
            return Ok(line.len());
        }
        // Never closes (no `Ok(0)`). A board just goes quiet; the port stays
        // open.
        std::thread::sleep(Duration::from_secs(3600));
        Ok(0)
    }
}

/// A serial port that closes at once (USB unplugged).
struct Closed;

impl Read for Closed {
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        Ok(0)
    }
}

#[test]
fn hitting_the_overall_limit_is_not_reported_as_silence() {
    // With both `--idle` and `--timeout`, the silence wait is capped by what
    // is left of the limit, so a `recv_timeout` expiry alone does not say
    // which one it was. Unless the limit is checked first, a capture cut off
    // mid-stream becomes "went silent" and `Stopped::Timeout` is in practice
    // never reported (`--idle` is on by default).
    let mut sink: Vec<u8> = Vec::new();
    let stats = monitor::stream(
        OnceThenSilent { sent: false },
        &mut sink,
        Some(Duration::from_secs(5)),
        Some(Duration::from_millis(600)),
    )
    .expect("writes");

    assert!(
        stats.stopped == Stopped::Timeout,
        "hit the limit but says \"{}\"",
        stats.stopped.reason()
    );
    assert_eq!(stats.trace_lines, 1);
}

#[test]
fn silence_without_a_limit_is_reported_as_silence() {
    // Without a limit, silence is silence (the app finished and the
    // firmware parked).
    let mut sink: Vec<u8> = Vec::new();
    let stats = monitor::stream(
        OnceThenSilent { sent: false },
        &mut sink,
        Some(Duration::from_millis(200)),
        None,
    )
    .expect("writes");

    assert!(stats.stopped == Stopped::Idle, "{}", stats.stopped.reason());
    assert_eq!(stats.trace_lines, 1);
    assert!(
        String::from_utf8_lossy(&sink).contains("log(2"),
        "the received bytes were not written out"
    );
}

#[test]
fn a_closed_port_is_distinguished_from_silence() {
    // Calling an unplugged USB "silence" makes it look like the app finished.
    let mut sink: Vec<u8> = Vec::new();
    let stats =
        monitor::stream(Closed, &mut sink, Some(Duration::from_secs(5)), None).expect("writes");

    assert!(
        stats.stopped == Stopped::Closed,
        "{}",
        stats.stopped.reason()
    );
    assert_eq!(stats.bytes, 0);
}
