//! `wasmicon trace diff`: extracts the traces from two serial captures and
//! compares them (the two-board determinism check, docs/decisions.md D-10).
//! `wasmicon trace replay`: extracts the I2C responses from a hardware trace
//! in the form `run --i2c-replay` reads (to replace `verify/sht4x-replay.txt`).
//!
//! The normalization rules are identical to `verify/diff-traces.sh`. CI keeps
//! running that script's `--self-test`, and `tests/trace.rs` checks that the
//! two give the same verdict (the same shape as `tools/check-sigs.sh`
//! cross-checking `wit2sig.py`). Fixing one side alone fails.
//!
//! There are only two rules:
//!
//! - Drop CR and NUL. A board's serial port emits NUL as line noise at reset
//!   or power-on. Left in, `grep` treats the file as binary and returns one
//!   line (docs/verification-report.md §8).
//! - Keep only lines starting with `>` or `<`. Banners and the guest's
//!   `[wasm] ...` lines differ per board, so they are dropped. That is safe
//!   because the same content appears in the trace as the `log` host call.
//!
//! The firmware identity (`wasmicon id ...`; docs/app-workflow.md §3.8) is
//! not compared, but a warning is printed when the two logs disagree. The
//! match / mismatch verdict does not change (it stays the same as
//! `verify/diff-traces.sh`). This says up front whether "same `.wasm`,
//! different trace" might come from different firmware.
//!
//! `--ignore <text>` is the one addition the script does not have: it drops
//! the trace lines containing the text from both sides after normalizing,
//! for an app that logs timings (docs/verification-report.md §27). Without
//! it the verdict is the script's, and `tests/trace.rs` cross-checks only
//! that case.
//!
//! Zero trace lines is a failure. Two empty traces compare equal as strings,
//! but that is "nothing was compared", not "identical". This verdict is used
//! as a completion criterion, so a silent success is the worst way to break.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

pub struct Options {
    pub a: PathBuf,
    pub b: PathBuf,
    /// `--ignore`: drop the lines containing any of these from both traces.
    pub ignore: Vec<String>,
}

pub struct ReplayOptions {
    pub log: PathBuf,
    /// Output path. `None` = stdout.
    pub out: Option<PathBuf>,
}

/// A normalized trace.
///
/// Lines are `Vec<u8>`. Converting to `String` turns every non-UTF-8 byte
/// into `U+FFFD`, so two captures with different noise would compare as
/// identical (noise on the serial port is the premise of this code; that is
/// why NUL is dropped). `from_utf8_lossy` is used only for display.
pub struct Trace {
    pub lines: Vec<Vec<u8>>,
}

/// Extracts the trace lines from a serial capture.
///
/// Same as `normalize()` in `verify/diff-traces.sh`: drop CR and NUL, keep
/// lines starting with `>` / `<`.
#[must_use]
pub fn normalize(bytes: &[u8]) -> Trace {
    let cleaned: Vec<u8> = bytes
        .iter()
        .copied()
        .filter(|b| *b != b'\r' && *b != 0)
        .collect();
    let lines = cleaned
        .split(|b| *b == b'\n')
        .filter(|l| matches!(l.first(), Some(b'>' | b'<')))
        .map(<[u8]>::to_vec)
        .collect();
    Trace { lines }
}

/// Drops the lines containing any of `ignore`. Returns how many were dropped.
///
/// A plain byte substring, not a pattern: what differs by design is usually
/// a fixed label in the guest's log text (`"frame ms="`), and a regex would
/// be the CLI's first dependency for it.
pub fn drop_lines(t: &mut Trace, ignore: &[String]) -> usize {
    let before = t.lines.len();
    t.lines.retain(|l| {
        !ignore
            .iter()
            .filter(|text| !text.is_empty())
            .any(|text| l.windows(text.len()).any(|w| w == text.as_bytes()))
    });
    before - t.lines.len()
}

/// The first position (0-based) where the traces differ. `None` when
/// identical.
#[must_use]
pub fn first_divergence(a: &Trace, b: &Trace) -> Option<usize> {
    for (i, (x, y)) in a.lines.iter().zip(&b.lines).enumerate() {
        if x != y {
            return Some(i);
        }
    }
    if a.lines.len() == b.lines.len() {
        None
    } else {
        Some(a.lines.len().min(b.lines.len()))
    }
}

/// Compares two captures. `true` when the traces match.
///
/// # Errors
/// When a file cannot be read, or one of them yields no trace line.
pub fn diff(opts: &Options) -> Result<bool> {
    let mut a = read(&opts.a)?;
    let mut b = read(&opts.b)?;
    warn_on_firmware_mismatch(opts)?;

    if !opts.ignore.is_empty() {
        let (da, db) = (
            drop_lines(&mut a, &opts.ignore),
            drop_lines(&mut b, &opts.ignore),
        );
        // Printed even when equal: a count that differs is itself a finding
        // (one side logged more timings), and the line numbers below are
        // of what is left.
        eprintln!(
            "ignored: {da} lines of {}, {db} lines of {}",
            short(&opts.a),
            short(&opts.b)
        );
        if a.lines.is_empty() || b.lines.is_empty() {
            bail!("--ignore dropped every trace line of one log; nothing is left to compare");
        }
    }

    match first_divergence(&a, &b) {
        None => {
            println!(
                "match: {} lines ({} and {})",
                a.lines.len(),
                opts.a.display(),
                opts.b.display()
            );
            Ok(true)
        }
        Some(i) => {
            eprintln!(
                "mismatch: {} has {} lines, {} has {} lines",
                opts.a.display(),
                a.lines.len(),
                opts.b.display(),
                b.lines.len()
            );
            // A trace is append-only, so the first divergence is closest to
            // the cause.
            if opts.ignore.is_empty() {
                eprintln!("  first divergence: line {}", i + 1);
            } else {
                eprintln!("  first divergence: line {} (of the lines left)", i + 1);
            }
            eprintln!("    {}  {}", short(&opts.a), at(&a, i));
            eprintln!("    {}  {}", short(&opts.b), at(&b, i));
            Ok(false)
        }
    }
}

/// The firmware identity in a log (the last one). `None` when there is none.
///
/// One log can contain several boots (the automatic reset after flashing,
/// then RST), so the last one, closest to the trace, is taken. The host's
/// `run --trace` has none.
#[must_use]
pub fn firmware_identity(bytes: &[u8]) -> Option<String> {
    let prefix = wasmicon_port::identity::PREFIX.as_bytes();
    bytes
        .split(|b| *b == b'\n')
        .rev()
        .map(|l| {
            l.iter()
                .copied()
                .filter(|b| *b != b'\r' && *b != 0)
                .collect::<Vec<u8>>()
        })
        .find(|l| l.starts_with(prefix))
        .map(|l| String::from_utf8_lossy(&l).into_owned())
}

/// Warns when the two logs name different firmware (the verdict is unchanged).
fn warn_on_firmware_mismatch(opts: &Options) -> Result<()> {
    let ida = firmware_identity(&std::fs::read(&opts.a)?);
    let idb = firmware_identity(&std::fs::read(&opts.b)?);
    if let (Some(x), Some(y)) = (&ida, &idb)
        && x != y
    {
        eprintln!(
            "warning: the two logs come from different firmware (a possible cause of trace \
             differences)"
        );
        eprintln!("    {}  {x}", short(&opts.a));
        eprintln!("    {}  {y}", short(&opts.b));
    }
    Ok(())
}

/// Detail lines show only the file name (the full path is in the heading).
fn short(p: &Path) -> String {
    p.file_name()
        .unwrap_or(p.as_os_str())
        .to_string_lossy()
        .into_owned()
}

fn at(t: &Trace, i: usize) -> String {
    t.lines.get(i).map_or_else(
        || "(ends here)".to_string(),
        |l| String::from_utf8_lossy(l).into_owned(),
    )
}

/// Reads and normalizes. Zero trace lines is an error.
fn read(path: &Path) -> Result<Trace> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot read `{}`", path.display()))?;
    let t = normalize(&bytes);
    if t.lines.is_empty() {
        bail!(
            "no trace line found in `{}`.\n  \
             Lines starting with '> ' / '< ' (docs/abi-spec.md §9) are required. Check that\n  \
             the capture was not cut short and that no timestamp prefixes each line",
            path.display()
        );
    }
    Ok(t)
}

/// Extracts the bytes received by each I2C read, in trace order.
///
/// Picks the `data=0x...` of the result line that follows each `i2c.read` /
/// `i2c.write-read` call line (docs/abi-spec.md §9). The host consumes one
/// replay line per read (`ports/host/src/hal.rs`), so the same order is the
/// replay as is. A failed read (non-zero status) is skipped, since the host
/// does not consume a line for it either.
///
/// # Errors
/// When a read longer than 32 bytes was folded into a CRC (the original
/// bytes are not in the trace).
pub fn i2c_reads(t: &Trace) -> Result<Vec<Vec<u8>>> {
    let mut out = Vec::new();
    let mut lines = t.lines.iter();
    while let Some(line) = lines.next() {
        let is_read = line.starts_with(b"> wasmicon:hal/i2c@")
            && (contains(line, b"/[method]bus.read(")
                || contains(line, b"/[method]bus.write-read("));
        if !is_read {
            continue;
        }
        let Some(result) = lines.next() else { break };
        if !result.starts_with(b"< 0 ") {
            continue;
        }
        let Some(hex) = after(result, b"data=0x") else {
            bail!(
                "a read result has no data=: {}\n  \
                 check whether the trace came from firmware that predates data=",
                String::from_utf8_lossy(result)
            );
        };
        let hex: Vec<u8> = hex
            .iter()
            .copied()
            .take_while(u8::is_ascii_hexdigit)
            .collect();
        if hex.len() % 2 != 0 || after(result, b"..len=").is_some() {
            bail!(
                "the read's bytes are not fully recorded (reads over 32 bytes are folded into \
                 a CRC): {}",
                String::from_utf8_lossy(result)
            );
        }
        let bytes = hex
            .chunks(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap_or("zz"), 16))
            .collect::<Result<Vec<u8>, _>>()
            .context("cannot parse data= as hex")?;
        out.push(bytes);
    }
    Ok(out)
}

/// The form `run --i2c-replay` reads (one line per read, hex bytes separated
/// by spaces).
#[must_use]
pub fn format_replay(reads: &[Vec<u8>], source: &str) -> String {
    let mut s = format!("# I2C responses extracted from {source} by wasmicon trace replay.\n");
    s.push_str("# One line per i2c read: the bytes to return, in hex, separated by spaces.\n");
    for r in reads {
        let line: Vec<String> = r.iter().map(|b| format!("{b:02x}")).collect();
        s.push_str(&line.join(" "));
        s.push('\n');
    }
    s
}

/// Writes the replay extracted from a trace.
///
/// # Errors
/// When a file cannot be read or written, the log has no trace line, or it
/// has no successful read.
pub fn replay(opts: &ReplayOptions) -> Result<bool> {
    let t = read(&opts.log)?;
    let reads = i2c_reads(&t)?;
    // Zero reads never become an empty replay silently. Run on the host, it
    // would mean "no sensor" and a capture failure would show up as a
    // different symptom.
    if reads.is_empty() {
        bail!("no successful I2C read in `{}`", opts.log.display());
    }
    let text = format_replay(&reads, &short(&opts.log));
    match &opts.out {
        Some(p) => {
            std::fs::write(p, &text).with_context(|| format!("cannot write `{}`", p.display()))?;
            eprintln!("wrote {} reads to {}", reads.len(), p.display());
        }
        None => print!("{text}"),
    }
    Ok(true)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Everything after `needle` up to the end of the line.
fn after<'a>(hay: &'a [u8], needle: &[u8]) -> Option<&'a [u8]> {
    hay.windows(needle.len())
        .position(|w| w == needle)
        .map(|i| &hay[i + needle.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Converts to text for easier comparison (tests only).
    fn text(t: &Trace) -> Vec<String> {
        t.lines
            .iter()
            .map(|l| String::from_utf8_lossy(l).into_owned())
            .collect()
    }

    #[test]
    fn drops_banners_and_guest_logs() {
        let raw = b"wasmicon rp2040\n[wasm] blink start\n> a/b(1)\n< 0 [1]\n";
        assert_eq!(text(&normalize(raw)), ["> a/b(1)", "< 0 [1]"]);
    }

    #[test]
    fn line_noise_is_not_folded_into_one_character() {
        // Two captures that differ in a non-UTF-8 byte must not match. As
        // Strings both would become U+FFFD and compare equal.
        let a = normalize(b"> a/b(1)\xff\n< 0 [1]\n");
        let b = normalize(b"> a/b(1)\xfe\n< 0 [1]\n");
        assert_eq!(a.lines.len(), 2);
        assert_eq!(first_divergence(&a, &b), Some(0), "differs on line 1");
    }

    #[test]
    fn drops_cr_and_nul() {
        // A board's serial port emits NUL (docs/verification-report.md §8).
        let raw = b"wasmicon esp32s3\r\n\0\0[wasm] x\r\n\0> a/b(1)\r\n< 0 [1]\r\n";
        assert_eq!(text(&normalize(raw)), ["> a/b(1)", "< 0 [1]"]);
    }

    #[test]
    fn finds_the_first_divergence() {
        let a = normalize(b"> x\n< 0\n> y\n");
        let b = normalize(b"> x\n< 0\n> z\n");
        assert_eq!(first_divergence(&a, &b), Some(2));
        assert_eq!(first_divergence(&a, &a), None);
    }

    #[test]
    fn a_shorter_trace_diverges_at_its_end() {
        let a = normalize(b"> x\n< 0\n> y\n");
        let b = normalize(b"> x\n< 0\n");
        assert_eq!(first_divergence(&a, &b), Some(2));
    }

    #[test]
    fn extracts_i2c_reads_in_order() {
        let t = normalize(
            b"> wasmicon:hal/i2c@0.1.0/[method]bus.write(1, 68, 0xfd)\n< 0\n\
              > wasmicon:hal/i2c@0.1.0/[method]bus.read(1, 68, 6)\n< 0 [len=6 data=0x6421e074e970]\n\
              > wasmicon:hal/i2c@0.1.0/[method]bus.read(1, 68, 2)\n< 3\n\
              > wasmicon:hal/i2c@0.1.0/[method]bus.write-read(1, 68, 0x89, 2)\n< 0 [len=2 data=0xabcd]\n",
        );
        let reads = i2c_reads(&t).expect("parses");
        // The failed read (< 3) is skipped; the host does not consume a
        // line for it either.
        assert_eq!(
            reads,
            [vec![0x64, 0x21, 0xe0, 0x74, 0xe9, 0x70], vec![0xab, 0xcd]]
        );
        assert_eq!(
            format_replay(&reads, "x.log")
                .lines()
                .filter(|l| !l.starts_with('#'))
                .collect::<Vec<_>>(),
            ["64 21 e0 74 e9 70", "ab cd"]
        );
    }

    #[test]
    fn a_trace_without_data_is_an_error() {
        // A trace from firmware that predates data=. Never silently empty.
        let t = normalize(b"> wasmicon:hal/i2c@0.1.0/[method]bus.read(1, 68, 6)\n< 0 [len=6]\n");
        assert!(i2c_reads(&t).is_err());
    }

    #[test]
    fn a_folded_read_is_an_error() {
        let t = normalize(
            b"> wasmicon:hal/i2c@0.1.0/[method]bus.read(1, 68, 40)\n\
              < 0 [len=40 data=0x00112233445566778899aabbccddeeff..len=40 crc32=01234567]\n",
        );
        assert!(i2c_reads(&t).is_err());
    }

    #[test]
    fn the_last_firmware_identity_is_taken() {
        // A log with two boots: the automatic reset after flashing, then RST.
        let log = b"wasmicon esp32s3\r\nwasmicon id esp32s3 fw=0.1.0 git=aaaaaaa\r\n\0\
                    wasmicon esp32s3\r\nwasmicon id esp32s3 fw=0.1.0 git=bbbbbbb-dirty\r\n> a/b(1)\r\n";
        assert_eq!(
            firmware_identity(log).as_deref(),
            Some("wasmicon id esp32s3 fw=0.1.0 git=bbbbbbb-dirty")
        );
        // The host's run --trace has no identity.
        assert_eq!(firmware_identity(b"> a/b(1)\n< 0\n"), None);
    }
}
