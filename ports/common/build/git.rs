// Included by each port's build.rs with `include!`. Embeds `git describe` of
// the built source as `WASMICON_GIT`, which the firmware reports at boot
// (docs/app-workflow.md §3.8).
//
// The `-dirty` suffix is kept on purpose. Firmware flashed from uncommitted
// changes must not report the same name as a committed build, or hardware
// records become ambiguous. So build.rs reruns whenever a source that goes
// into the firmware (the port itself, ports/common, runtime, wit) changes.
// Changes elsewhere (docs and so on) do not rerun it, so a dirty state that
// exists only there is not reflected until a source changes.

fn emit_git_describe() {
    use std::process::Command;
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let run = |args: &[&str]| -> Option<String> {
        let o = Command::new("git").args(args).current_dir(&dir).output().ok()?;
        o.status
            .success()
            .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    // Without git, or when building outside a repository, report "unknown"
    // rather than fail the build.
    let describe = run(&["describe", "--always", "--dirty", "--abbrev=7"])
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=WASMICON_GIT={describe}");

    if let Some(git_dir) = run(&["rev-parse", "--absolute-git-dir"]) {
        for f in ["HEAD", "index", "packed-refs", "refs"] {
            println!("cargo:rerun-if-changed={git_dir}/{f}");
        }
    }
    for d in ["src", "../common/src", "../../runtime/src", "../../wit"] {
        println!("cargo:rerun-if-changed={dir}/{d}");
    }
}
