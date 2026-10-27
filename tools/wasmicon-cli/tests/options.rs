//! Option errors of the `wasmicon` binary that are decided while parsing,
//! before any file or board is touched.

use std::process::Command;

/// Runs the binary and returns its stderr; the command must fail.
fn refused(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_wasmicon"))
        .args(args)
        .current_dir(env!("CARGO_TARGET_TMPDIR"))
        .output()
        .expect("cannot run wasmicon");
    assert!(!out.status.success(), "{args:?} was accepted");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn capture_options_need_a_capture() {
    for opt in [
        &["--idle", "30"][..],
        &["--timeout", "600"],
        &["-o", "x.log"],
    ] {
        let mut args = vec!["deploy", "app.wasm"];
        args.extend_from_slice(opt);
        let e = refused(&args);
        assert!(
            e.contains("needs --monitor without --no-run"),
            "{opt:?}: {e}"
        );

        let mut args = vec!["deploy", "app.wasm", "--monitor", "--no-run"];
        args.extend_from_slice(opt);
        let e = refused(&args);
        assert!(
            e.contains("needs --monitor without --no-run"),
            "{opt:?}: {e}"
        );
    }
}

#[test]
fn a_flashing_speed_must_be_positive() {
    for cmd in [&["deploy", "app.wasm"][..], &["assets", "write"]] {
        for v in ["0", "fast"] {
            let mut args = cmd.to_vec();
            args.extend_from_slice(&["--flash-baud", v]);
            let e = refused(&args);
            assert!(e.contains("not a positive number"), "{args:?}: {e}");
        }
    }
}

#[test]
fn an_empty_ignore_text_is_refused() {
    let e = refused(&["trace", "diff", "a.log", "b.log", "--ignore", ""]);
    assert!(e.contains("non-empty"), "{e}");
}
