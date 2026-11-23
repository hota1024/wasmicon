//! `wasmicon monitor --console`: reads the board's own USB console
//! (docs/decisions.md A-11, docs/TODO.md §5-11).
//!
//! The console carries the guest `log` and the firmware's lines (banner,
//! slot lines, trap reason), not the trace, which stays on UART0. So this is
//! a different read from `monitor`'s: it succeeds on lines, not on trace
//! lines, and it follows the board across resets.
//!
//! Three things differ from a USB-serial adapter, all seen on a Pico 2 W
//! from macOS (docs/verification-report.md §42):
//!
//! 1. The port goes away on every reset and comes back under the same
//!    identity, so the end of a read is not the end of the capture: it waits
//!    for the console to come back and reads on.
//! 2. The device node appears before the port delivers. A port opened at
//!    once read nothing while the firmware held its queue (it sends only
//!    while DTR is up), and one opened a second later read everything. So
//!    it settles before opening, and while nothing has arrived it drops and
//!    raises DTR again, which the firmware sees as the host opening the port.
//! 3. Opening must not flush. `serialport`'s own open raises DTR, configures
//!    the port and then, on macOS, flushes it: the lines the firmware sent in
//!    between were taken by the host and thrown away (the boot lines and 23
//!    `log` lines of the first try). The port is opened as `monitor` opens
//!    one, and the descriptor is handed to `serialport` only for timed reads
//!    and DTR.
//!
//! There is no baud rate: the CDC device takes bytes at USB speed.

use anyhow::{Context, Result, bail};
use std::io::{Read, Write};
use std::os::fd::{FromRawFd, IntoRawFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serialport::{SerialPort, TTYPort};

use crate::monitor::{self, Own, PortInfo, Sink};

pub struct Options {
    /// `--port`. `None` picks the console by USB identity.
    pub port: Option<PathBuf>,
    /// `-o`. Also written to this file, besides stdout.
    pub out: Option<PathBuf>,
    /// Silence that ends the capture, counted only while the port is open.
    /// `None` never ends (stop with Ctrl-C).
    pub idle: Option<Duration>,
    /// Overall limit. `None` = none.
    pub timeout: Option<Duration>,
}

/// Wait after the console appears before opening it (item 2).
const SETTLE: Duration = Duration::from_millis(300);

/// With nothing read for this long since the port opened, DTR is dropped
/// and raised again (item 2).
const KICK_AFTER: Duration = Duration::from_secs(1);

/// At most this many DTR kicks per opening; an app that prints nothing must
/// not be kicked forever.
const MAX_KICKS: u32 = 5;

/// How long one read waits, so the limits are checked in between.
const READ_WAIT: Duration = Duration::from_millis(100);

/// How often the port list is checked while the console is away.
const POLL: Duration = Duration::from_millis(100);

/// Picks the console to read among `candidates`.
///
/// `Ok(None)` means "not there now": the caller waits. With `explicit`, that
/// path is the console. Otherwise the firmware's console ports; `follow`, the
/// USB serial number of the board read so far, keeps a capture on one board
/// across its resets.
///
/// # Errors
/// When several consoles qualify.
pub fn pick(
    candidates: &[PortInfo],
    explicit: Option<&Path>,
    follow: Option<&str>,
) -> Result<Option<PortInfo>> {
    if let Some(path) = explicit {
        return Ok(candidates.iter().find(|p| p.path == path).cloned());
    }
    let consoles: Vec<&PortInfo> = candidates
        .iter()
        .filter(|p| p.own() == Some(Own::Console))
        .filter(|p| follow.is_none() || serial(p) == follow)
        .collect();
    match consoles.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some((*one).clone())),
        many => {
            let lines: Vec<String> = many.iter().map(|p| p.describe()).collect();
            bail!(
                "{} board consoles found; pick one with --port:\n  {}",
                many.len(),
                lines.join("\n  ")
            )
        }
    }
}

fn serial(p: &PortInfo) -> Option<&str> {
    p.usb.as_ref()?.serial.as_deref()
}

/// Why a console capture stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stopped {
    /// Silence while the port was open (the app finished).
    Idle,
    /// The overall limit was reached.
    Timeout,
}

/// Reads the console until silence or the limit, across resets.
///
/// # Errors
/// When several consoles qualify, the port cannot be opened, or writing
/// fails.
pub fn run(opts: &Options) -> Result<bool> {
    let started = Instant::now();
    let left = |now: Instant| opts.timeout.map(|t| t.saturating_sub(now - started));
    let mut sink = Sink::new(opts.out.as_deref())?;
    let mut follow: Option<String> = None;
    let mut bytes = 0usize;
    let mut lines = 0usize;
    let mut waiting_said = false;
    if let Some(idle) = opts.idle {
        eprintln!("(stops after {} s of silence)", idle.as_secs());
    }

    let stopped = 'capture: loop {
        // Wait for the console (item 1).
        let port = loop {
            if left(Instant::now()) == Some(Duration::ZERO) {
                break 'capture Stopped::Timeout;
            }
            if let Some(p) = pick(&monitor::ports(), opts.port.as_deref(), follow.as_deref())? {
                break p;
            }
            if !waiting_said {
                eprintln!("waiting for the board's USB console");
                waiting_said = true;
            }
            std::thread::sleep(POLL);
        };
        if follow.is_none() {
            follow = serial(&port).map(str::to_string);
        }
        std::thread::sleep(SETTLE);
        // Raw mode without a flush (item 3). The baud rate means nothing to
        // the CDC device.
        let Ok(session) = monitor::open(&port.path, monitor::DEFAULT_BAUD) else {
            // Gone again between the list and the open: wait for it.
            continue;
        };
        let fd = session.into_file().into_raw_fd();
        // SAFETY: `fd` was just taken out of an open `File` with
        // `into_raw_fd`, so it is a valid descriptor that nothing else owns;
        // `TTYPort` takes it over and closes it when dropped.
        let mut dev = unsafe { TTYPort::from_raw_fd(fd) };
        dev.set_timeout(READ_WAIT)
            .with_context(|| format!("cannot set a read timeout on `{}`", port.path.display()))?;
        eprintln!("opened {}", port.describe());
        waiting_said = false;

        let mut last = Instant::now();
        let mut got = false;
        let mut kicks = 0;
        let mut buf = [0u8; 4096];
        loop {
            if left(Instant::now()) == Some(Duration::ZERO) {
                break 'capture Stopped::Timeout;
            }
            match dev.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    sink.write_all(&buf[..n])
                        .and_then(|()| sink.flush())
                        .context("cannot write the capture")?;
                    bytes += n;
                    lines += buf[..n].iter().filter(|b| **b == b'\n').count();
                    last = Instant::now();
                    got = true;
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    if !got && kicks < MAX_KICKS && last.elapsed() >= KICK_AFTER {
                        let _ = dev.write_data_terminal_ready(false);
                        std::thread::sleep(Duration::from_millis(50));
                        let _ = dev.write_data_terminal_ready(true);
                        kicks += 1;
                        last = Instant::now();
                    }
                    if opts.idle.is_some_and(|idle| last.elapsed() >= idle) {
                        break 'capture Stopped::Idle;
                    }
                }
                // The board reset or the cable went (item 1).
                Err(_) => break,
            }
        }
        eprintln!("console gone (the board reset?); waiting for it to come back");
    };
    sink.finish()?;

    let reason = match stopped {
        Stopped::Idle => "went silent",
        Stopped::Timeout => "hit the time limit",
    };
    eprintln!("{reason}: {bytes} B / {lines} lines");
    if lines == 0 {
        eprintln!(
            "  no line at all. Suspect one of: firmware without the USB console\n  \
             (ports/rp2350 has it), or a board that is not running"
        );
        return Ok(false);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::Usb;

    fn port(path: &str, product: &str, interface: u8, serial: &str) -> PortInfo {
        PortInfo {
            path: PathBuf::from(path),
            usb: Some(Usb {
                vid: 0x2E8A,
                pid: 0x0009,
                product: Some(product.to_string()),
                serial: Some(serial.to_string()),
                interface: Some(interface),
            }),
        }
    }

    fn board(n: u8, serial: &str) -> [PortInfo; 2] {
        [
            port(
                &format!("/dev/cu.usbmodem1{n}01"),
                "wasmicon rp2350",
                1,
                serial,
            ),
            port(
                &format!("/dev/cu.usbmodem1{n}03"),
                "wasmicon rp2350",
                3,
                serial,
            ),
        ]
    }

    #[test]
    fn the_console_is_picked_by_identity() {
        let c = board(1, "A");
        let p = pick(&c, None, None).expect("one").expect("there");
        assert_eq!(p.path, PathBuf::from("/dev/cu.usbmodem1101"));
    }

    #[test]
    fn no_console_means_wait() {
        let c = [port("/dev/cu.usbmodem1101", "Pico", 0, "A")];
        assert!(pick(&c, None, None).expect("no error").is_none());
    }

    #[test]
    fn two_boards_are_not_guessed() {
        let c: Vec<PortInfo> = board(1, "A").into_iter().chain(board(2, "B")).collect();
        let msg = format!("{:#}", pick(&c, None, None).expect_err("two"));
        assert!(msg.contains("--port"), "{msg}");
    }

    #[test]
    fn a_capture_follows_its_board() {
        // After a reset the first board's console comes back; the second
        // board's console must not take its place.
        let c: Vec<PortInfo> = board(1, "A").into_iter().chain(board(2, "B")).collect();
        let p = pick(&c, None, Some("B")).expect("one").expect("there");
        assert_eq!(p.path, PathBuf::from("/dev/cu.usbmodem1201"));
        assert!(
            pick(&board(1, "A"), None, Some("B"))
                .expect("no error")
                .is_none()
        );
    }

    #[test]
    fn an_explicit_port_waits_for_that_path() {
        let c = board(1, "A");
        let want = PathBuf::from("/dev/cu.usbmodem1103");
        let p = pick(&c, Some(&want), None).expect("ok").expect("there");
        assert_eq!(p.path, want);
        let gone = PathBuf::from("/dev/cu.usbmodem9");
        assert!(pick(&c, Some(&gone), None).expect("ok").is_none());
    }
}
