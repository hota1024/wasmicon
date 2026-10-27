//! Cross-checks the `wasmicon trace diff` verdict against `verify/diff-traces.sh`.
//!
//! The same normalization exists twice, so fixing one side alone must fail.
//! The same shape as `tools/check-sigs.sh` cross-checking `wit2sig.py` and
//! `wasmicon-gen`.
//!
//! The properties the script's `--self-test` protects (banners and CR are
//! absorbed, a real difference is detected, two empty traces are not a
//! match, NUL does not eat lines) are demanded of the Rust side with the
//! same inputs.

use std::path::{Path, PathBuf};
use std::process::Command;

use wasmicon_cli::trace;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root not found")
}

/// The script's verdict.
struct ScriptVerdict {
    matched: bool,
    /// The line count the script reports on a match.
    lines: Option<usize>,
}

fn script_verdict(a: &Path, b: &Path) -> ScriptVerdict {
    let root = repo_root();
    let out = Command::new("sh")
        .current_dir(&root)
        .arg("verify/diff-traces.sh")
        .arg(a)
        .arg(b)
        .output()
        .expect("cannot start diff-traces.sh");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    // Takes N from `match: N lines (...)`.
    let lines = stdout
        .split_once("match: ")
        .and_then(|(_, rest)| rest.split_once(" lines"))
        .and_then(|(n, _)| n.trim().parse().ok());
    ScriptVerdict {
        matched: out.status.success(),
        lines,
    }
}

/// The Rust side's verdict. `Ok(true)` = match, `Err` = no trace lines.
fn cli_says_match(a: &Path, b: &Path) -> Result<bool, String> {
    trace::diff(&trace::Options {
        a: a.to_path_buf(),
        b: b.to_path_buf(),
        ignore: Vec::new(),
    })
    .map_err(|e| format!("{e:#}"))
}

/// Asserts that both verdicts agree.
///
/// The line count is compared too, not just the verdict. With the verdict
/// alone, a normalization mismatch that yields different line counts would
/// still pass as "both match".
fn assert_agree(a: &Path, b: &Path, expect_match: bool, label: &str) {
    let script = script_verdict(a, b);
    let cli = cli_says_match(a, b);

    assert_eq!(
        script.matched, expect_match,
        "{label}: the diff-traces.sh verdict differs from the expectation"
    );
    match cli {
        Ok(m) => assert_eq!(
            m, expect_match,
            "{label}: the CLI verdict differs from the expectation"
        ),
        Err(e) => panic!("{label}: the CLI failed (expected {expect_match}): {e}"),
    }

    if expect_match {
        let bytes = std::fs::read(a).expect("cannot read");
        let n = trace::normalize(&bytes).lines.len();
        assert_eq!(
            Some(n),
            script.lines,
            "{label}: line counts differ (CLI {n} lines, script {:?} lines)",
            script.lines
        );
    }
}

struct Fixtures {
    dir: PathBuf,
}

impl Fixtures {
    /// Creates the fixtures under `CARGO_TARGET_TMPDIR`.
    ///
    /// A fixed name under the shared `/tmp` makes worktrees running
    /// `cargo test` side by side overwrite each other's `a.log` (this repo
    /// is worked on in several worktrees). Under target the directory is per
    /// worktree and `cargo clean` removes it.
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("trace-{name}"));
        // Remove what the previous run left before creating.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("cannot create the work directory");
        Fixtures { dir }
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.dir.join(name);
        std::fs::write(&p, bytes).expect("cannot write");
        p
    }
}

/// The same trace body as the script's self-test.
const TRACE: &str = "> wasmicon:hal/log@0.1.0/log(2, \"blink start\")\n\
                     <\n\
                     > wasmicon:hal/gpio@0.1.0/[static]pin.open(role:led, 3)\n\
                     < 0 [1]\n";

#[test]
fn banners_and_cr_are_absorbed() {
    let f = Fixtures::new("banner");
    let a = f.write(
        "a.log",
        format!("wasmicon rp2040\n[wasm] blink start\n{TRACE}").as_bytes(),
    );
    // The same trace with a different banner and log line, and CRLF line
    // endings.
    let b_text = format!("wasmicon esp32s3\r\n[wasm] blink start\r\n{TRACE}").replace('\n', "\r\n");
    let b = f.write("b.log", b_text.as_bytes());

    assert_agree(&a, &b, true, "banner and CR");
}

#[test]
fn a_real_difference_is_detected() {
    let f = Fixtures::new("diff");
    let a = f.write("a.log", format!("wasmicon rp2040\n{TRACE}").as_bytes());
    let c = f.write(
        "c.log",
        format!("wasmicon rp2040\n{TRACE}")
            .replace("role:led", "role:lcd-cs")
            .as_bytes(),
    );

    assert_agree(&a, &c, false, "different role name");
}

#[test]
fn nul_bytes_do_not_eat_lines() {
    // Seen on hardware (docs/verification-report.md §8). Unless NUL is
    // dropped, grep treats the file as binary and returns one line (the same
    // property the script's self-test protects).
    let f = Fixtures::new("nul");
    let a = f.write("a.log", format!("wasmicon rp2040\n{TRACE}").as_bytes());

    let mut nul = Vec::new();
    nul.extend_from_slice(b"wasmicon esp32s3\r\n\0\0[wasm] blink start\r\n");
    nul.extend_from_slice(b"> wasmicon:hal/log@0.1.0/log(2, \"blink start\")\r\n<\r\n");
    nul.extend_from_slice(b"\0> wasmicon:hal/gpio@0.1.0/[static]pin.open(role:led, 3)\r\n");
    nul.extend_from_slice(b"< 0 [1]\r\n");
    let b = f.write("nul.log", &nul);

    assert_agree(&a, &b, true, "log with NUL mixed in");
}

#[test]
fn line_noise_is_compared_byte_for_byte() {
    // Two captures that differ in a non-UTF-8 byte are a mismatch. As
    // Strings both would become U+FFFD and match (CLI side). With a UTF-8
    // locale, macOS tr stops midway (script side). Both were fixed, so the
    // two agree here.
    let f = Fixtures::new("noise");
    let mut a_bytes = Vec::new();
    a_bytes.extend_from_slice(b"wasmicon rp2040\n> a/b(1)\xff\n< 0 [1]\n");
    let mut b_bytes = Vec::new();
    b_bytes.extend_from_slice(b"wasmicon esp32s3\n> a/b(1)\xfe\n< 0 [1]\n");
    let a = f.write("noise-a.log", &a_bytes);
    let b = f.write("noise-b.log", &b_bytes);

    assert_agree(&a, &b, false, "different non-UTF-8 noise");
}

#[test]
fn empty_traces_are_not_a_match() {
    // Two empty traces compare equal as strings, but that is "nothing was
    // compared". This verdict is used as a completion criterion, so a silent
    // success is the worst way to break.
    let f = Fixtures::new("empty");
    let a = f.write("e1.log", b"wasmicon rp2040\n[wasm] blink start\n");
    let b = f.write("e2.log", b"wasmicon esp32s3\n[wasm] blink start\n");

    assert!(!script_verdict(&a, &b).matched, "the script fails");
    assert!(
        cli_says_match(&a, &b).is_err(),
        "the CLI fails too (does not say match)"
    );
}

#[test]
fn line_counts_exclude_banners() {
    let f = Fixtures::new("count");
    let a = f.write(
        "a.log",
        format!("wasmicon rp2040\n[wasm] blink start\n{TRACE}").as_bytes(),
    );
    let bytes = std::fs::read(&a).expect("cannot read");
    // 4 lines without the banner and the log line.
    assert_eq!(trace::normalize(&bytes).lines.len(), 4);
}

/// A trace with one timing log per frame: `frames` gives each frame's
/// milliseconds, and `last` is the final log text.
fn timed(frames: &[u32], last: &str) -> Vec<u8> {
    let mut t = String::from("> wasmicon:hal/log@0.1.0/log(2, \"start\")\n<\n");
    for ms in frames {
        t.push_str(&format!(
            "> wasmicon:hal/log@0.1.0/log(2, \"frame ms={ms}\")\n<\n"
        ));
    }
    t.push_str(&format!("> wasmicon:hal/log@0.1.0/log(2, \"{last}\")\n<\n"));
    t.into_bytes()
}

fn diff_ignoring(a: &Path, b: &Path, ignore: &[&str]) -> Result<bool, String> {
    trace::diff(&trace::Options {
        a: a.to_path_buf(),
        b: b.to_path_buf(),
        ignore: ignore.iter().map(|s| (*s).to_string()).collect(),
    })
    .map_err(|e| format!("{e:#}"))
}

#[test]
fn ignore_drops_the_calls_that_differ_by_design() {
    let f = Fixtures::new("ignore");
    let host = f.write("host.log", &timed(&[180, 175], "done"));
    let board = f.write("board.log", &timed(&[990, 1010], "done"));

    assert_eq!(
        diff_ignoring(&host, &board, &[]),
        Ok(false),
        "the timings differ"
    );
    assert_eq!(diff_ignoring(&host, &board, &["frame ms="]), Ok(true));

    // A difference elsewhere is still caught.
    let other = f.write("other.log", &timed(&[990, 1010], "dome"));
    assert_eq!(diff_ignoring(&host, &other, &["frame ms="]), Ok(false));

    // Dropping everything is not a match.
    let e = diff_ignoring(&host, &board, &["wasmicon:hal"]).expect_err("nothing left");
    assert!(e.contains("nothing is left"), "{e}");
}

#[test]
fn ignore_takes_a_call_and_its_result_together() {
    let mut t = trace::normalize(&timed(&[180], "done"));
    let kept = trace::drop_calls(&mut t, &["frame ms=".to_string()]);
    // The timing call (lines 3 and 4) goes with its `<`; no orphan is left.
    assert_eq!(kept, [1, 2, 5, 6]);
    assert!(
        t.lines
            .iter()
            .all(|l| !l.starts_with(b"> wasmicon:hal/log@0.1.0/log(2, \"frame"))
    );
    assert_eq!(t.lines.iter().filter(|l| l.as_slice() == b"<").count(), 2);

    // Naming a call drops its result line too, which has no name of its own.
    let mut t = trace::normalize(
        b"> wasmicon:hal/assets@0.1.0/[method]blob.read(1, 0, 4)\n< 0 [len=4 data=0x01020304]\n\
          > wasmicon:hal/log@0.1.0/log(2, \"x\")\n<\n",
    );
    assert_eq!(
        trace::drop_calls(&mut t, &["blob.read".to_string()]),
        [3, 4]
    );
}

#[test]
fn ignore_does_not_hide_a_call_one_side_made_more_often() {
    // Three timing logs against two: the rest lines up, but one side made a
    // call the other did not.
    let f = Fixtures::new("ignore-count");
    let a = f.write("a.log", &timed(&[1, 2, 3], "done"));
    let b = f.write("b.log", &timed(&[1, 2], "done"));
    assert_eq!(diff_ignoring(&a, &b, &["frame ms="]), Ok(false));
}
