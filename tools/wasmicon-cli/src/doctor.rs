//! `wasmicon doctor`: reports which tools are installed (docs/app-workflow.md §4.6).
//!
//! The firmware is a release build (docs/app-workflow.md §4.5), so an app
//! author only needs the tools to build an app and the tools to flash it.
//! espup and the Xtensa toolchain are not needed.
//!
//! It only reports; the exit code is always 0. What counts as "required"
//! depends on which board comes next, so it is not decided here.

use anyhow::Result;
use std::process::Command;

/// Returns the version string, or `None` when the tool is not found.
fn probe(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = if out.stdout.is_empty() {
        String::from_utf8_lossy(&out.stderr).into_owned()
    } else {
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    Some(first_line(&text))
}

/// Version output can span several lines or carry whitespace. Only the first
/// line is used.
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

/// Strips the tool's name from the front of its version string.
///
/// `espflash --version` prints `espflash 4.5.0` and `picotool version` prints
/// `picotool v2.3.1 (...)`, so next to the label the name would appear twice.
fn strip_name(name: &str, version: &str) -> String {
    version
        .strip_prefix(name)
        .map_or(version, |rest| rest.trim_start())
        .trim()
        .to_string()
}

/// Whether wasm32 is among the installed targets.
///
/// Decided in the current directory. rustup looks for a `rust-toolchain.toml`
/// override from the current directory, so running inside an app project
/// looks at that project's toolchain. If it lists `targets = [...]`, a
/// missing target is installed automatically at `cargo build` (this repo's
/// `apps/rust-toolchain.toml` does that).
fn has_wasm32() -> bool {
    let Ok(out) = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
    else {
        return false;
    };
    out.status.success() && String::from_utf8_lossy(&out.stdout).contains("wasm32-unknown-unknown")
}

pub fn run() -> Result<bool> {
    let rustc = probe("rustc", &["--version"]);
    let wasm32 = has_wasm32();
    let asc = probe("asc", &["--version"])
        .or_else(|| probe("npx", &["--no-install", "asc", "--version"]));
    let picotool = probe("picotool", &["version"]);
    let espflash = probe("espflash", &["--version"]);
    let probe_rs = probe("probe-rs", &["--version"]);

    // Rust apps
    match &rustc {
        Some(v) if wasm32 => line(
            "rustc",
            &format!("{} + wasm32-unknown-unknown", strip_name("rustc", v)),
            true,
        ),
        Some(v) => line(
            "rustc",
            &format!(
                "{} (no wasm32 target; installed automatically when the project's \
                 rust-toolchain.toml lists it in targets)",
                strip_name("rustc", v)
            ),
            false,
        ),
        None => line("rustc", "missing", false),
    }

    // AssemblyScript apps
    match &asc {
        Some(v) => line("asc", &strip_name("asc", v), true),
        None => line(
            "asc",
            "missing (for AssemblyScript apps: npm i -D assemblyscript)",
            false,
        ),
    }

    // Flashing tools
    match &picotool {
        Some(v) => line("picotool", &strip_name("picotool", v), true),
        None => line(
            "picotool",
            "missing (needed to flash rp2040 / rp2350)",
            false,
        ),
    }
    match &espflash {
        Some(v) => line("espflash", &strip_name("espflash", v), true),
        None => line(
            "espflash",
            "missing (needed to flash esp32s3 / esp32c5 / esp32p4)",
            false,
        ),
    }
    match &probe_rs {
        Some(v) => line("probe-rs", &strip_name("probe-rs", v), true),
        None => line("probe-rs", "missing (optional; for a debugger)", false),
    }

    println!();
    let mut boards: Vec<&str> = Vec::new();
    if picotool.is_some() {
        boards.push("rp2040");
        boards.push("rp2350");
    }
    if espflash.is_some() {
        boards.push("esp32s3");
        boards.push("esp32s3-psram");
        boards.push("esp32c5");
        boards.push("esp32p4");
    }
    if boards.is_empty() {
        println!("-> no board can be flashed from here; apps can still be built, checked and run");
    } else {
        println!("-> boards that can be flashed: {}", boards.join(" / "));
    }
    if rustc.is_some() && wasm32 {
        println!("  Rust apps can be built");
    }
    if asc.is_some() {
        println!("  AssemblyScript apps can be built");
    }
    Ok(true)
}

fn line(label: &str, value: &str, ok: bool) {
    let mark = if ok { "ok  " } else { "--  " };
    println!("{mark}{label:<10}{value}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tool_name_is_not_repeated() {
        // Version strings usually start with the tool's name. It is shown
        // next to the label, so the name is stripped.
        assert_eq!(strip_name("espflash", "espflash 4.5.0"), "4.5.0");
        assert_eq!(
            strip_name("picotool", "picotool v2.3.1 (Darwin)"),
            "v2.3.1 (Darwin)"
        );
        assert_eq!(strip_name("rustc", "rustc 1.97.1 (abc)"), "1.97.1 (abc)");
        // A string that does not start with the name is left alone (asc
        // prints `Version 0.28.20`).
        assert_eq!(strip_name("asc", "Version 0.28.20"), "Version 0.28.20");
    }

    #[test]
    fn version_output_is_reduced_to_one_line() {
        assert_eq!(
            first_line("rustc 1.98.0 (abc 2026-01-01)\n"),
            "rustc 1.98.0 (abc 2026-01-01)"
        );
        // picotool prints several lines.
        assert_eq!(
            first_line("picotool v2.1.1 (macOS)\nSOME NOTICE\n"),
            "picotool v2.1.1 (macOS)"
        );
        assert_eq!(first_line("  0.28.8  \n"), "0.28.8");
        assert_eq!(first_line(""), "");
    }

    #[test]
    fn rustc_is_present_in_this_repo() {
        // Any environment that works on this repo has rustc. This checks that
        // probe() works at all.
        assert!(probe("rustc", &["--version"]).is_some());
        assert!(probe("wasmicon-no-such-command", &["--version"]).is_none());
    }
}
