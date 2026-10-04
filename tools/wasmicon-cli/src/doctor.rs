//! `wasmicon doctor` — 道具が揃っているかを見る（`docs/app-workflow.md` §4.6）。
//!
//! **ファームはリリース成果物**なので（§4.5）、アプリ作者に要るのは
//! 「アプリをビルドする道具」と「焼く道具」だけ。espup と Xtensa の
//! ツールチェーンは要らない。
//!
//! 報告するだけで、終了コードは常に 0 にする。何が「必要」かは、これから
//! どのボードを触るかで変わるので、ここで決めない。

use anyhow::Result;
use std::process::Command;

/// 版の文字列を取る。見つからなければ `None`。
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

/// 版の出力は複数行だったり前後に空白が付いたりする。1 行目だけ使う。
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

/// rustup が入れているターゲットに wasm32 があるか。
fn has_wasm32() -> bool {
    probe("rustup", &["target", "list", "--installed"]).is_some_and(|_| {
        Command::new("rustup")
            .args(["target", "list", "--installed"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("wasm32-unknown-unknown"))
            .unwrap_or(false)
    })
}

pub fn run() -> Result<bool> {
    let rustc = probe("rustc", &["--version"]);
    let wasm32 = has_wasm32();
    let asc = probe("asc", &["--version"])
        .or_else(|| probe("npx", &["--no-install", "asc", "--version"]));
    let picotool = probe("picotool", &["version"]);
    let espflash = probe("espflash", &["--version"]);
    let probe_rs = probe("probe-rs", &["--version"]);

    // Rust のアプリ
    match &rustc {
        Some(v) if wasm32 => line("rustc", &format!("{v} + wasm32-unknown-unknown"), true),
        Some(v) => line(
            "rustc",
            &format!("{v}（wasm32 が無い: rustup target add wasm32-unknown-unknown）"),
            false,
        ),
        None => line("rustc", "無い", false),
    }

    // AssemblyScript のアプリ
    match &asc {
        Some(v) => line("asc", v, true),
        None => line(
            "asc",
            "無い（AssemblyScript で書くなら npm i -D assemblyscript）",
            false,
        ),
    }

    // 焼く道具
    match &picotool {
        Some(v) => line("picotool", v, true),
        None => line("picotool", "無い（rp2040 / rp2350 を焼くなら要る）", false),
    }
    match &espflash {
        Some(v) => line("espflash", v, true),
        None => line("espflash", "無い（esp32s3 を焼くなら要る）", false),
    }
    match &probe_rs {
        Some(v) => line("probe-rs", v, true),
        None => line("probe-rs", "無い（任意。デバッガを使うなら）", false),
    }

    println!();
    let mut boards: Vec<&str> = Vec::new();
    if picotool.is_some() {
        boards.push("rp2040");
        boards.push("rp2350");
    }
    if espflash.is_some() {
        boards.push("esp32s3");
    }
    if boards.is_empty() {
        println!("→ ファームを焼けるボードは無い。アプリの build / check / run はできる");
    } else {
        println!("→ ファームを焼けるボード: {}", boards.join(" / "));
    }
    if rustc.is_some() && wasm32 {
        println!("  Rust のアプリはビルドできる");
    }
    if asc.is_some() {
        println!("  AssemblyScript のアプリはビルドできる");
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
    fn version_output_is_reduced_to_one_line() {
        assert_eq!(
            first_line("rustc 1.98.0 (abc 2026-01-01)\n"),
            "rustc 1.98.0 (abc 2026-01-01)"
        );
        // picotool は複数行出す。
        assert_eq!(
            first_line("picotool v2.1.1 (macOS)\nSOME NOTICE\n"),
            "picotool v2.1.1 (macOS)"
        );
        assert_eq!(first_line("  0.28.8  \n"), "0.28.8");
        assert_eq!(first_line(""), "");
    }

    #[test]
    fn rustc_is_present_in_this_repo() {
        // この repo を触っている環境には必ずある。probe() が動くことの確認。
        assert!(probe("rustc", &["--version"]).is_some());
        assert!(probe("絶対に無いコマンド", &["--version"]).is_none());
    }
}
