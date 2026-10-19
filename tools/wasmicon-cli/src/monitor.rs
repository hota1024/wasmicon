//! `wasmicon monitor`: opens the serial port and captures the trace
//! (docs/app-workflow.md §4.2).
//!
//! Three things go wrong in this procedure. All three happened on hardware
//! (docs/verification-report.md §9), so they are contained here.
//!
//! 1. Apply `stty` while the port is open. On macOS, `stty -f` opens the
//!    device, configures it and closes it, so the settings do not persist
//!    (the next open is back at 9600). One capture came out as pure noise
//!    that way. Open the port first and keep the handle while applying it.
//! 2. Apply it before the data starts. Configuring while bytes flow mixes up
//!    the lines and drops bytes. So `monitor` opens and configures first and
//!    the caller resets the board afterwards (`deploy --monitor` does it in
//!    that order).
//! 3. Treat the data as bytes. NUL and line noise arrive at the moment of
//!    reset. Converting to `String` mangles non-UTF-8 bytes, so a line is a
//!    `Vec<u8>` (same reason as in `trace`).
//!
//! The capture ends on silence. When the app returns from `run`, the firmware
//! parks and nothing more arrives, so "N seconds without data" is the natural
//! end. Waiting for Ctrl-C alone cannot be automated.
//!
//! `monitor` stamps nothing on the capture. The record of origin is the
//! `wasmicon id ...` line the firmware prints after its banner
//! (docs/app-workflow.md §3.8); `trace diff` warns when two logs disagree
//! on it.

use anyhow::{Context, Result, bail};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Default baud rate (matches UART0 in `ports/*`).
pub const DEFAULT_BAUD: u32 = 115_200;

/// Default silence that ends the capture.
pub const DEFAULT_IDLE: Duration = Duration::from_secs(3);

pub struct Options {
    /// `--port`. `None` picks from `/dev/cu.usb*`.
    pub port: Option<PathBuf>,
    pub baud: u32,
    /// `-o`. Also written to this file, besides stdout.
    pub out: Option<PathBuf>,
    /// Silence that ends the capture. `None` never ends (stop with Ctrl-C).
    pub idle: Option<Duration>,
    /// Overall limit. `None` = none.
    pub timeout: Option<Duration>,
}

/// Result of a capture.
pub struct Stats {
    pub bytes: usize,
    /// Lines starting with `>` / `<` (trace lines, docs/abi-spec.md §9).
    pub trace_lines: usize,
    /// Why it stopped.
    pub stopped: Stopped,
}

#[derive(PartialEq, Eq)]
pub enum Stopped {
    /// Silence (the app finished and the firmware parked).
    Idle,
    /// The overall limit was reached.
    Timeout,
    /// The other end closed (USB unplugged).
    Closed,
}

impl Stopped {
    #[must_use]
    pub fn reason(&self) -> &'static str {
        match self {
            Stopped::Idle => "went silent",
            Stopped::Timeout => "hit the time limit",
            Stopped::Closed => "port closed (USB unplugged)",
        }
    }
}

/// An open serial port, after `stty` has been applied.
pub struct Session {
    port: PathBuf,
    file: std::fs::File,
}

impl Session {
    #[must_use]
    pub fn port(&self) -> &Path {
        &self.port
    }
}

/// Lists the candidate serial ports.
///
/// Not hard-coded to `usbserial`: the name depends on the bridge chip (the
/// ESP32-S3 at hand has a CH343 and shows up as `usbmodem`;
/// docs/TODO.md §1.1).
#[must_use]
pub fn ports() -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir("/dev") else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = dir
        .filter_map(|e| {
            let name = e.ok()?.file_name().to_string_lossy().into_owned();
            PREFIXES
                .iter()
                .any(|p| name.starts_with(p))
                .then(|| PathBuf::from("/dev").join(name))
        })
        .collect();
    found.sort();
    found
}

/// Prefixes of port names.
///
/// Not macOS only. `open` has a branch for Linux's `stty -F`, and with only
/// `cu.usb` here Linux would find zero candidates and blame the wiring
/// ("plug in a USB-serial adapter"); there the names are `ttyUSB*` /
/// `ttyACM*`.
const PREFIXES: &[&str] = &["cu.usb", "ttyUSB", "ttyACM"];

/// Decides which port to use.
///
/// One candidate is used as is. With several and no explicit choice, none is
/// picked (opening the wrong port silently produces "nothing comes out" and
/// sends people to check the wiring).
///
/// # Errors
/// When there is no candidate, or several and no explicit choice.
pub fn choose(candidates: &[PathBuf], explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    match candidates {
        [] => bail!(
            "no serial port found (no /dev/cu.usb*).\n  \
             plug in a USB-serial adapter and connect the board's TX"
        ),
        [one] => Ok(one.clone()),
        many => {
            let list: Vec<String> = many.iter().map(|p| p.display().to_string()).collect();
            bail!(
                "{} serial ports found; pick one with --port:\n  {}",
                many.len(),
                list.join("\n  ")
            )
        }
    }
}

/// Opens the port and sets the baud rate.
///
/// The order matters: open first, then apply `stty` while holding the handle
/// (closing resets the settings; item 1 in the module docs).
///
/// # Errors
/// When the port cannot be opened, or `stty` is missing or fails.
pub fn open(port: &Path, baud: u32) -> Result<Session> {
    let file =
        std::fs::File::open(port).with_context(|| format!("cannot open `{}`", port.display()))?;

    // The handle stays open here. `stty` is applied to the same device.
    let flag = if cfg!(target_os = "linux") {
        "-F"
    } else {
        "-f"
    };
    let status = Command::new("stty")
        .args([
            flag,
            &port.to_string_lossy(),
            &baud.to_string(),
            "raw",
            "-echo",
        ])
        .status()
        .context("cannot start stty")?;
    if !status.success() {
        bail!("cannot set `{}` to {baud} baud", port.display());
    }
    Ok(Session {
        port: port.to_path_buf(),
        file,
    })
}

/// Reads continuously and forwards to `sink`.
///
/// Lines are kept as bytes (item 3 in the module docs). Only lines starting
/// with `>` / `<` are counted.
///
/// # Errors
/// When writing fails.
pub fn stream<R: Read + Send + 'static>(
    src: R,
    sink: &mut dyn Write,
    idle: Option<Duration>,
    timeout: Option<Duration>,
) -> Result<Stats> {
    // Reading happens on another thread and arrives through a channel so
    // that it can be timed (a serial read blocks, so a timer alone cannot
    // break out of it).
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut src = src;
        let mut buf = [0u8; 4096];
        loop {
            match src.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let started = Instant::now();
    let mut bytes = 0usize;
    let mut trace_lines = 0usize;
    let mut partial: Vec<u8> = Vec::new();
    let stopped;

    loop {
        if let Some(limit) = timeout
            && started.elapsed() >= limit
        {
            stopped = Stopped::Timeout;
            break;
        }
        // The wait used to detect silence, capped by what is left of the
        // overall limit.
        let wait = idle.unwrap_or(Duration::from_secs(3600));
        let wait = match timeout {
            Some(limit) => wait.min(limit.saturating_sub(started.elapsed())),
            None => wait,
        };
        match rx.recv_timeout(wait) {
            Ok(chunk) => {
                bytes += chunk.len();
                sink.write_all(&chunk).context("cannot write the capture")?;
                sink.flush().context("cannot write the capture")?;
                trace_lines += count_trace_lines(&chunk, &mut partial);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Hitting the overall limit must not be reported as silence.
                // The wait is capped by what is left of the limit, so a
                // `recv_timeout` expiry alone does not say which one it was.
                // The limit is checked first. Otherwise a capture cut off
                // mid-stream reports "went silent" and looks like a clean
                // end (and since `--idle` is on by default, `Timeout` was in
                // practice never reported).
                if timeout.is_some_and(|limit| started.elapsed() >= limit) {
                    stopped = Stopped::Timeout;
                    break;
                }
                if idle.is_some() {
                    stopped = Stopped::Idle;
                    break;
                }
                // Silence does not end this capture. Back to the limit check.
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                stopped = Stopped::Closed;
                break;
            }
        }
    }

    Ok(Stats {
        bytes,
        trace_lines,
        stopped,
    })
}

/// Counts trace lines, joining lines that span chunks through `partial`.
fn count_trace_lines(chunk: &[u8], partial: &mut Vec<u8>) -> usize {
    let mut n = 0;
    for b in chunk {
        if *b == b'\n' {
            // CR and NUL are dropped (same rule as `trace::normalize`).
            let line: Vec<u8> = partial
                .iter()
                .copied()
                .filter(|c| *c != b'\r' && *c != 0)
                .collect();
            if matches!(line.first(), Some(b'>' | b'<')) {
                n += 1;
            }
            partial.clear();
        } else {
            partial.push(*b);
        }
    }
    n
}

/// Captures.
///
/// # Errors
/// When no port can be chosen or opened, or writing fails.
pub fn run(opts: &Options) -> Result<bool> {
    let port = choose(&ports(), opts.port.as_deref())?;
    let session = open(&port, opts.baud)?;
    eprintln!(
        "opened {} at {} baud; reset the board to start the output",
        session.port().display(),
        opts.baud
    );
    capture(session, opts)
}

/// Captures from an already open port.
///
/// `deploy --monitor` must open the port before the reset, so opening is
/// separate from capturing (item 2 in the module docs).
///
/// # Errors
/// When writing fails.
pub fn capture(session: Session, opts: &Options) -> Result<bool> {
    if let Some(idle) = opts.idle {
        eprintln!("(stops after {} s of silence)", idle.as_secs());
    }

    let mut sink = Sink::new(opts.out.as_deref())?;
    let stats = stream(session.file, &mut sink, opts.idle, opts.timeout)?;
    sink.finish()?;
    report(&stats)
}

/// Lets a tool open the port and captures from that tool's stdout.
///
/// This exists for ESP32-S3. That board flashes, resets and traces through
/// one port, and the reset needs DTR/RTS (`monitor` only applies `stty`, so
/// it cannot reset by itself). The whole port is handed to a tool that does
/// both (`espflash monitor`) and its output is counted here
/// (docs/verification-report.md §10).
///
/// # Errors
/// When the tool cannot be started or writing fails.
pub fn capture_cmd(cmd: &str, args: &[&str], opts: &Options) -> Result<bool> {
    if let Some(idle) = opts.idle {
        eprintln!("(stops after {} s of silence)", idle.as_secs());
    }

    // stderr is passed through (the tool's progress and failure reasons go
    // there).
    let mut child = Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!("cannot start {cmd} (run `wasmicon doctor` to see what is installed)")
        })?;
    let out = child
        .stdout
        .take()
        .with_context(|| format!("cannot take the stdout of {cmd}"))?;

    let mut sink = Sink::new(opts.out.as_deref())?;
    let stats = stream(out, &mut sink, opts.idle, opts.timeout)?;
    sink.finish()?;

    // Check first whether the tool has already died. When `espflash` exits at
    // once because it cannot open the port (`Resource busy`, wrong `--port`),
    // `stream` sees 0 bytes and `Disconnected`, and `report` returns
    // `Ok(false)` as "no trace line at all". The caller then cannot tell "the
    // tool failed" from "it ran but produced no trace" (`deploy` uses
    // `is_err()` to print the "board is parked in the bootloader" hint, which
    // is exactly the case that would be lost).
    let failed = match child.try_wait() {
        Ok(Some(status)) => !status.success(),
        // Still alive = the capture ended, so stop it.
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            false
        }
        Err(_) => false,
    };
    if failed {
        bail!("{cmd} failed (see the output above)");
    }

    report(&stats)
}

/// Prints the capture result and returns whether the capture is usable.
fn report(stats: &Stats) -> Result<bool> {
    eprintln!(
        "{}: {} B / {} trace lines",
        stats.stopped.reason(),
        stats.bytes,
        stats.trace_lines
    );
    if stats.trace_lines == 0 {
        // "Nothing captured" is never a silent success
        // (docs/verification-report.md §9).
        eprintln!(
            "  no trace line at all. Suspect one of: wrong baud rate (everything is noise),\n  \
             wrong port, board not reset, or firmware flashed with the trace feature off"
        );
        return Ok(false);
    }
    Ok(true)
}

/// Writes to stdout and, when given, to a file as well.
struct Sink {
    file: Option<std::fs::File>,
}

impl Sink {
    fn new(path: Option<&Path>) -> Result<Self> {
        let file = match path {
            None => None,
            Some(p) => Some(
                std::fs::File::create(p)
                    .with_context(|| format!("cannot write `{}`", p.display()))?,
            ),
        };
        Ok(Sink { file })
    }

    fn finish(&mut self) -> Result<()> {
        if let Some(f) = &mut self.file {
            f.flush().context("cannot write the capture")?;
        }
        Ok(())
    }
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        std::io::stdout().write_all(buf)?;
        if let Some(f) = &mut self.file {
            f.write_all(buf)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stdout().flush()?;
        if let Some(f) = &mut self.file {
            f.flush()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_candidate_needs_no_choice() {
        let c = [PathBuf::from("/dev/cu.usbserial-1")];
        assert_eq!(choose(&c, None).expect("chooses"), c[0]);
    }

    #[test]
    fn several_candidates_are_not_guessed() {
        // Opening the wrong port silently produces "nothing comes out" and
        // sends people to check the wiring.
        let c = [
            PathBuf::from("/dev/cu.usbserial-1"),
            PathBuf::from("/dev/cu.usbmodem2"),
        ];
        let e = choose(&c, None).expect_err("does not choose");
        let msg = format!("{e:#}");
        assert!(msg.contains("--port"), "{msg}");
        assert!(msg.contains("usbmodem2"), "lists the candidates: {msg}");
    }

    #[test]
    fn an_explicit_port_wins() {
        let c = [PathBuf::from("/dev/cu.usbserial-1")];
        let want = PathBuf::from("/dev/cu.usbmodem9");
        assert_eq!(choose(&c, Some(&want)).expect("chooses"), want);
    }

    #[test]
    fn no_candidate_says_what_to_do() {
        let e = choose(&[], None).expect_err("none");
        assert!(format!("{e:#}").contains("USB-serial adapter"), "{e:#}");
    }

    #[test]
    fn trace_lines_are_counted_across_chunks() {
        // A line split across chunks is still counted.
        let mut partial = Vec::new();
        let mut n = count_trace_lines(b"wasmicon rp2350\n> a/b(1)\n< 0 ", &mut partial);
        assert_eq!(n, 1, "the banner is not counted");
        n += count_trace_lines(b"[1]\n", &mut partial);
        assert_eq!(n, 2, "a line spanning chunks is counted");
    }

    #[test]
    fn noise_does_not_break_the_count() {
        // NUL and CR from the moment of reset are dropped before counting
        // (docs/verification-report.md §9).
        let mut partial = Vec::new();
        let n = count_trace_lines(b"\0\0> a/b(1)\r\n\0< 0\r\n", &mut partial);
        assert_eq!(n, 2);
    }

    #[test]
    fn silence_ends_the_capture() {
        // Silence ends it (the app finished and the firmware parked).
        let (_tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        // A `Read` that stays empty and never closes, standing in for a pipe
        // that sends nothing.
        struct Quiet(std::sync::mpsc::Receiver<Vec<u8>>);
        impl Read for Quiet {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                // Nothing is ever sent, so this blocks forever (like a
                // serial port).
                match self.0.recv() {
                    Ok(_) | Err(_) => Ok(0),
                }
            }
        }
        let mut out = Vec::new();
        let stats = stream(
            Quiet(rx),
            &mut out,
            Some(Duration::from_millis(50)),
            Some(Duration::from_secs(5)),
        )
        .expect("reads");
        assert!(stats.stopped == Stopped::Idle, "{}", stats.stopped.reason());
        assert_eq!(stats.bytes, 0);
    }

    #[test]
    fn what_comes_in_is_written_out() {
        let data = b"wasmicon rp2350\n> a/b(1)\n< 0 [1]\n".to_vec();
        let mut out = Vec::new();
        let stats = stream(
            std::io::Cursor::new(data.clone()),
            &mut out,
            Some(Duration::from_millis(50)),
            None,
        )
        .expect("reads");
        assert_eq!(out, data, "bytes are forwarded unchanged");
        assert_eq!(stats.trace_lines, 2);
        assert_eq!(stats.bytes, data.len());
    }
}
