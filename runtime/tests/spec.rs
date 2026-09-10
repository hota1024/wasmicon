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

use wasmicon_core::{Arena, Config, Error, ErrorKind, decode, validate};

/// 実行対象の `.wast`。段階が進むごとに増やす。
const FILES: &[&str] = &[
    // Wasm コア仕様のうち、対応機能セット（HANDOFF §2-7）に収まるファイル。
    // ここに無いものは EXCLUDED の理由で外している。
    "address.wast",
    "align.wast",
    "binary.wast",
    "binary-leb128.wast",
    "block.wast",
    "br.wast",
    "br_if.wast",
    "br_table.wast",
    "bulk.wast",
    "call.wast",
    "call_indirect.wast",
    "comments.wast",
    "const.wast",
    "conversions.wast",
    "custom.wast",
    "data.wast",
    "elem.wast",
    "endianness.wast",
    "exports.wast",
    "f32.wast",
    "f32_bitwise.wast",
    "f32_cmp.wast",
    "f64.wast",
    "f64_bitwise.wast",
    "f64_cmp.wast",
    "fac.wast",
    "float_exprs.wast",
    "float_literals.wast",
    "float_memory.wast",
    "float_misc.wast",
    "forward.wast",
    "func.wast",
    "func_ptrs.wast",
    "global.wast",
    "i32.wast",
    "i64.wast",
    "if.wast",
    "inline-module.wast",
    "int_exprs.wast",
    "int_literals.wast",
    "labels.wast",
    "left-to-right.wast",
    "load.wast",
    "local_get.wast",
    "local_set.wast",
    "local_tee.wast",
    "loop.wast",
    "memory.wast",
    "memory_copy.wast",
    "memory_fill.wast",
    "memory_init.wast",
    "memory_redundancy.wast",
    "memory_size.wast",
    "memory_trap.wast",
    "names.wast",
    "nop.wast",
    "return.wast",
    "select.wast",
    "skip-stack-guard-page.wast",
    "stack.wast",
    "start.wast",
    "store.wast",
    "switch.wast",
    "token.wast",
    "traps.wast",
    "type.wast",
    "unreachable.wast",
    "unreached-invalid.wast",
    "unreached-valid.wast",
    "unwind.wast",
    "utf8-custom-section-id.wast",
    "utf8-import-field.wast",
    "utf8-import-module.wast",
    "utf8-invalid-encoding.wast",
];

/// 除外したファイルと理由。
const EXCLUDED: &[(&str, &str)] = &[
    (
        "simd_*, i8x16_*, i16x8_*, i32x4_*, relaxed_*",
        "SIMD 非対応（HANDOFF §2-7）",
    ),
    ("*atomic*", "threads 非対応"),
    (
        "ref*, br_on_*, call_ref, i31",
        "reference-types / GC 非対応",
    ),
    ("table*", "table.* 命令が非対応"),
    ("return_call*", "tail-call 非対応"),
    ("throw*, try_table, tag 系", "exception-handling 非対応"),
    ("*64.wast, memory64*", "memory64 非対応"),
    (
        "address0/1, load0/1/2, data0, exports0 など数字付き",
        "multi-memory 非対応",
    ),
    ("array*, struct, type-rec, type-canon など", "GC 非対応"),
    (
        "linking*, imports*, instance",
        "複数モジュールのリンクは v0.1 の範囲外",
    ),
    ("memory_grow", "multi-memory のモジュールを含む"),
];

/// 対応済みコマンド種別。段階が進むごとに増やす。
/// 2b でデコード + 検証まで。実行系は 2c で足す。
const SUPPORTED: &[&str] = &["module", "assert_malformed", "assert_invalid"];

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

/// デコード + 検証。実行はまだしない。
fn load(bytes: &[u8]) -> Result<(), Error> {
    let mut buf = arena_buf();
    let mut scratch_buf = arena_buf();
    let mut arena = Arena::new(&mut buf);
    let mut scratch = Arena::new(&mut scratch_buf);
    let m = decode::decode(bytes, &mut arena)?;
    validate::validate(&m, &Config::default(), &mut arena, &mut scratch)?;
    Ok(())
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
                match load(&bytes) {
                    Ok(()) => tally.ran += 1,
                    // 上流の testsuite は core の .wast にも post-MVP 機能を混ぜている。
                    // 対応機能セット外はスキップし、件数だけ表に出す。
                    Err(e) if e.kind() == ErrorKind::Unsupported => tally.unsupported += 1,
                    Err(e) => tally.failures.push(format!(
                        "{wast}:{line}: 正しいモジュールの読み込みに失敗: {} [{}]",
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
                expect(tally, wast, line, cmd, &bytes, ErrorKind::Malformed);
            }
            "assert_invalid" => {
                if cmd["module_type"].as_str() != Some("binary") {
                    tally.skipped += 1;
                    continue;
                }
                let Some(file) = cmd["filename"].as_str() else {
                    tally.skipped += 1;
                    continue;
                };
                let bytes = std::fs::read(dir.join(file)).unwrap();
                expect(tally, wast, line, cmd, &bytes, ErrorKind::Invalid);
            }
            _ => tally.skipped += 1,
        }
    }
}

/// 読み込みが指定した区分で失敗することを確かめる。
fn expect(
    tally: &mut Tally,
    wast: &str,
    line: u64,
    cmd: &serde_json::Value,
    bytes: &[u8],
    want: ErrorKind,
) {
    match load(bytes) {
        Err(e) if e.kind() == want => tally.ran += 1,
        // 対応機能セット外の構文を含むケースは、期待どおりに落ちたかを判定できない。
        Err(e) if e.kind() == ErrorKind::Unsupported => tally.unsupported += 1,
        Err(e) => tally.failures.push(format!(
            "{wast}:{line}: {} を期待したが {} [{}]（期待: {}）",
            want.name(),
            e.kind().name(),
            e.reason(),
            cmd["text"].as_str().unwrap_or("?")
        )),
        Ok(()) => tally.failures.push(format!(
            "{wast}:{line}: {} を期待したが成功した（{}）",
            want.name(),
            cmd["text"].as_str().unwrap_or("?")
        )),
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
