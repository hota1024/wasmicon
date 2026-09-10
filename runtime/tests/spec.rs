//! WebAssembly spec testsuite ランナー（HANDOFF §5 Phase 2 の完了条件）。
//!
//! `third_party/testsuite`（`tools/fetch-testsuite.sh` が固定 SHA で取得）の
//! `.wast` を `wasm-tools json-from-wast` で JSON に変換し、コマンドを順に実行する。
//!
//! 「全通過」の定義: `FILES` に挙げた `.wast` の、対応済みコマンド種別が全て通ること。
//! 除外は `EXCLUDED` に理由つきで列挙する（HANDOFF §3 のデフォルトと同じ扱い）。
//!
//! 開発用に全ファイルを走らせて現状を一覧する調査モードがある:
//! `cargo test -p wasmicon-core --test spec -- --ignored --nocapture`

use std::path::{Path, PathBuf};
use std::process::Command;

use wasmicon_core::{Arena, ErrorKind, decode};

/// 実行対象の `.wast`。段階が進むごとに増やす。
const FILES: &[&str] = &[
    // --- 2a: デコードのみ（module コマンド）---
    "binary.wast",
    "custom.wast",
    "type.wast",
    "func.wast",
    "block.wast",
    "loop.wast",
    "br.wast",
    "br_if.wast",
    "br_table.wast",
    "call.wast",
    "i32.wast",
    "i64.wast",
    "f32.wast",
    "f64.wast",
    "memory.wast",
    "memory_copy.wast",
    "memory_fill.wast",
    "memory_init.wast",
    "global.wast",
    "start.wast",
    "endianness.wast",
    "forward.wast",
    "nop.wast",
    "local_get.wast",
    "local_set.wast",
    "local_tee.wast",
];

/// 除外したファイルと理由。
const EXCLUDED: &[(&str, &str)] = &[
    ("simd_*.wast", "SIMD 非対応（HANDOFF §2-7）"),
    ("*atomic*.wast", "threads 非対応"),
    ("ref_*.wast / select.wast", "reference-types 非対応"),
    ("table*.wast / elem.wast", "table.* 命令が非対応"),
    (
        "linking.wast / imports.wast",
        "複数モジュールのリンクは v0.1 の範囲外",
    ),
];

/// 対応済みコマンド種別。段階が進むごとに増やす。
/// 2a ではデコードだけなので `module` のみ。
const SUPPORTED: &[&str] = &["module"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("リポジトリルートが見つからない")
}

/// `.wast` を JSON + `.wasm` 群に変換し、(json, wasm ディレクトリ) を返す。
fn convert(root: &Path, wast: &str) -> Option<(serde_json::Value, PathBuf)> {
    let src = root.join("third_party/testsuite").join(wast);
    if !src.exists() {
        return None;
    }
    let out_dir = root
        .join("target/spec")
        .join(wast.trim_end_matches(".wast"));
    std::fs::create_dir_all(&out_dir).ok()?;
    let json_path = out_dir.join("cmds.json");

    let status = Command::new("wasm-tools")
        .arg("json-from-wast")
        .arg(&src)
        .arg("-o")
        .arg(&json_path)
        .arg("--wasm-dir")
        .arg(&out_dir)
        .stderr(std::process::Stdio::null())
        .status()
        .expect("wasm-tools を起動できない。`brew install wasm-tools` などで導入する");
    if !status.success() {
        return None;
    }
    let text = std::fs::read_to_string(&json_path).ok()?;
    Some((serde_json::from_str(&text).ok()?, out_dir))
}

#[derive(Default)]
struct Tally {
    ran: usize,
    skipped: usize,
    /// 対応機能セット外でスキップした件数（HANDOFF §2-7）。
    unsupported: usize,
    failures: Vec<String>,
}

fn arena_buf() -> Vec<u8> {
    vec![0u8; 8 << 20]
}

/// 1 ファイル分のコマンドを実行する。失敗は `tally.failures` に積む。
fn run_file(root: &Path, wast: &str, tally: &mut Tally) {
    let Some((json, dir)) = convert(root, wast) else {
        tally
            .failures
            .push(format!("{wast}: 変換できない（取得もれか対象外の構文）"));
        return;
    };
    let Some(commands) = json["commands"].as_array() else {
        return;
    };

    for cmd in commands {
        let ty = cmd["type"].as_str().unwrap_or("");
        let line = cmd["line"].as_u64().unwrap_or(0);
        if !SUPPORTED.contains(&ty) {
            tally.skipped += 1;
            continue;
        }
        match ty {
            "module" => {
                if cmd["module_type"].as_str() == Some("text") {
                    tally.skipped += 1;
                    continue;
                }
                let Some(file) = cmd["filename"].as_str() else {
                    tally.skipped += 1;
                    continue;
                };
                let bytes = std::fs::read(dir.join(file)).unwrap();
                let mut buf = arena_buf();
                let mut arena = Arena::new(&mut buf);
                match decode::decode(&bytes, &mut arena) {
                    Ok(_) => tally.ran += 1,
                    // 上流の testsuite は core の .wast にも post-MVP 機能を混ぜている。
                    // 対応機能セット外はスキップし、件数だけ表に出す。
                    Err(e) if e.kind() == ErrorKind::Unsupported => tally.unsupported += 1,
                    Err(e) => tally.failures.push(format!(
                        "{wast}:{line}: 正しいモジュールのデコードに失敗: {} [{}]",
                        e.reason(),
                        e.kind().name()
                    )),
                }
            }
            "assert_malformed" => {
                if cmd["module_type"].as_str() != Some("binary") {
                    tally.skipped += 1;
                    continue;
                }
                let Some(file) = cmd["filename"].as_str() else {
                    tally.skipped += 1;
                    continue;
                };
                let bytes = std::fs::read(dir.join(file)).unwrap();
                let mut buf = arena_buf();
                let mut arena = Arena::new(&mut buf);
                match decode::decode(&bytes, &mut arena) {
                    Err(e) if e.kind() == ErrorKind::Malformed => tally.ran += 1,
                    Err(e) => tally.failures.push(format!(
                        "{wast}:{line}: malformed を期待したが {} [{}]",
                        e.kind().name(),
                        e.reason()
                    )),
                    Ok(_) => tally.failures.push(format!(
                        "{wast}:{line}: malformed を期待したが成功した（{}）",
                        cmd["text"].as_str().unwrap_or("?")
                    )),
                }
            }
            _ => tally.skipped += 1,
        }
    }
}

#[test]
fn spec_testsuite() {
    let root = repo_root();
    if !root.join("third_party/testsuite/binary.wast").exists() {
        // CI では fetch-testsuite.sh を先に走らせている。手元で未取得なら明示して抜ける。
        println!(
            "spec: third_party/testsuite が無いので実行しない。\n                   `sh tools/fetch-testsuite.sh` で取得すること。"
        );
        return;
    }
    let mut tally = Tally::default();
    for wast in FILES {
        run_file(&root, wast, &mut tally);
    }
    println!(
        "spec: {} コマンド実行 / {} 機能セット外 / {} 未実装でスキップ / {} ファイル / 除外 {} 分類",
        tally.ran,
        tally.unsupported,
        tally.skipped,
        FILES.len(),
        EXCLUDED.len()
    );
    assert!(tally.ran > 0, "1 つもコマンドを実行していない");
    assert!(
        tally.failures.is_empty(),
        "{} 件失敗:\n{}",
        tally.failures.len(),
        tally.failures.join("\n")
    );
}

/// 開発用。testsuite 全ファイルを走らせて現状を一覧する。CI では回さない。
#[test]
#[ignore = "開発用の調査モード"]
fn spec_survey() {
    let root = repo_root();
    let dir = root.join("third_party/testsuite");
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .expect("testsuite が無い")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".wast"))
        .collect();
    files.sort();

    let mut total = Tally::default();
    let mut bad_files = 0;
    for f in &files {
        let mut t = Tally::default();
        run_file(&root, f, &mut t);
        if !t.failures.is_empty() {
            bad_files += 1;
            println!("--- {f}: {} 件", t.failures.len());
            for msg in t.failures.iter().take(3) {
                println!("    {msg}");
            }
        }
        total.ran += t.ran;
        total.skipped += t.skipped;
        total.unsupported += t.unsupported;
        total.failures.extend(t.failures);
    }
    println!(
        "\n調査: {} ファイル中 {} ファイルに失敗あり / 実行 {} / 機能セット外 {} / スキップ {} / 失敗 {}",
        files.len(),
        bad_files,
        total.ran,
        total.unsupported,
        total.skipped,
        total.failures.len()
    );
}
