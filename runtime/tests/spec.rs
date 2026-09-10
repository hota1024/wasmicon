//! WebAssembly spec testsuite ランナー（docs/handoff.md §5 Phase 2 の完了条件）。
//!
//! `third_party/testsuite`（`tools/fetch-testsuite.sh` が固定 SHA で取得）の
//! `.wast` を `wasm-tools json-from-wast` で JSON に変換し、コマンドを順に実行する。
//!
//! 「全通過」の定義: `FILES` に挙げた `.wast` の、対応済みコマンド種別が全て通ること。
//! 除外は `EXCLUDED` に理由つきで列挙する（docs/handoff.md §3 のデフォルトと同じ扱い）。
//!
//! スキップの内訳は実行時に出る（`-- --nocapture`）。手で書くと必ずずれるので
//! ここには書かない。JSON を静的に数えた値ともずれる: 機能セット外と判定した
//! チャンクは後続のコマンドまで「機能セット外」に数えるため。
//!
//! 開発用に全ファイルを走らせて現状を一覧する調査モードがある:
//! `cargo test -p wasmicon-core --test spec -- --ignored --nocapture`

use std::path::{Path, PathBuf};
use std::process::Command;

mod support;

use support::{SpecTest, is_trap, parse_arg, parse_expected};
use wasmicon_core::{Arena, Config, Error, ErrorKind, Exec, decode, instantiate, invoke, validate};

/// 実行対象の `.wast`。段階が進むごとに増やす。
const FILES: &[&str] = &[
    // Wasm コア仕様のうち、対応機能セット（docs/handoff.md §2-7）に収まるファイル。
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
        "SIMD 非対応（docs/handoff.md §2-7）",
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
const SUPPORTED: &[&str] = &[
    "module",
    "assert_malformed",
    "assert_invalid",
    "assert_unlinkable",
    "assert_uninstantiable",
    "assert_return",
    "assert_trap",
    "assert_exhaustion",
    "action",
];

/// arena の大きさ。残りが線形メモリになる（`Arena::alloc_rest`）。
const ARENA: usize = 48 << 20;

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
    /// 対応機能セット外でスキップした件数（docs/handoff.md §2-7）。
    unsupported: usize,
    /// スキップの内訳。テキスト形式のモジュール（WAT パーサを持たない）。
    skip_text: usize,
    /// 名前つきモジュールへの操作。複数インスタンスは v0.1 の範囲外。
    skip_named: usize,
    /// `register` など、対応していないコマンド種別。
    skip_kind: usize,
    failures: Vec<String>,
}

impl Tally {
    fn fail(&mut self, wast: &str, line: u64, msg: String) {
        self.failures.push(format!("{wast}:{line}: {msg}"));
    }
}

fn arena_buf() -> Vec<u8> {
    // alloc_zeroed なので実際に触ったページしか実体化しない。
    vec![0u8; ARENA]
}

fn cfg() -> Config {
    Config::default()
}

/// デコード + 検証だけ行う（`assert_malformed` / `assert_invalid` 用）。
fn load(bytes: &[u8]) -> Result<(), Error> {
    let mut buf = arena_buf();
    let mut scratch_buf = vec![0u8; 8 << 20];
    let mut arena = Arena::new(&mut buf);
    let mut scratch = Arena::new(&mut scratch_buf);
    let m = decode::decode(bytes, &mut arena)?;
    validate::validate(&m, &cfg(), &mut arena, &mut scratch)?;
    Ok(())
}

/// インスタンス化まで行う（`assert_unlinkable` / `assert_uninstantiable` 用）。
fn load_and_instantiate(bytes: &[u8]) -> Result<(), Error> {
    let mut buf = arena_buf();
    let mut scratch_buf = vec![0u8; 8 << 20];
    let mut arena = Arena::new(&mut buf);
    let mut scratch = Arena::new(&mut scratch_buf);
    let m = decode::decode(bytes, &mut arena)?;
    let v = validate::validate(&m, &cfg(), &mut arena, &mut scratch)?;
    let mut exec = Exec::new(&cfg(), &mut arena)?;
    let mut r = SpecTest;
    let mut inst = instantiate(m, v, &cfg(), &mut arena, &mut r)?;
    if let Some(start) = inst.module.start {
        invoke(&mut inst, &mut exec, &mut r, start, &[], &mut [])?;
    }
    Ok(())
}

/// 読み込みが指定した区分で失敗することを確かめる。
fn expect(
    tally: &mut Tally,
    wast: &str,
    line: u64,
    cmd: &serde_json::Value,
    bytes: &[u8],
    want: ErrorKind,
    instantiate_too: bool,
) {
    let r = if instantiate_too {
        load_and_instantiate(bytes)
    } else {
        load(bytes)
    };
    match r {
        Err(e) if e.kind() == want => tally.ran += 1,
        // 対応機能セット外の構文を含むケースは、期待どおりに落ちたかを判定できない。
        Err(e) if e.kind() == ErrorKind::Unsupported => tally.unsupported += 1,
        Err(e) => tally.fail(
            wast,
            line,
            format!(
                "{} を期待したが {} [{}]（期待: {}）",
                want.name(),
                e.kind().name(),
                e.reason(),
                cmd["text"].as_str().unwrap_or("?")
            ),
        ),
        Ok(()) => tally.fail(
            wast,
            line,
            format!(
                "{} を期待したが成功した（{}）",
                want.name(),
                cmd["text"].as_str().unwrap_or("?")
            ),
        ),
    }
}

/// モジュールを伴わない検査（malformed / invalid / unlinkable / uninstantiable）。
/// 扱えたら `true`。
fn standalone(tally: &mut Tally, wast: &str, dir: &Path, cmd: &serde_json::Value) -> bool {
    let ty = cmd["type"].as_str().unwrap_or("");
    let line = cmd["line"].as_u64().unwrap_or(0);
    let want = match ty {
        "assert_malformed" => ErrorKind::Malformed,
        "assert_invalid" => ErrorKind::Invalid,
        "assert_unlinkable" => ErrorKind::Unlinkable,
        "assert_uninstantiable" => ErrorKind::Trap,
        _ => return false,
    };
    if cmd["module_type"].as_str() != Some("binary") {
        tally.skipped += 1;
        tally.skip_text += 1;
        return true;
    }
    let Some(file) = cmd["filename"].as_str() else {
        tally.skipped += 1;
        return true;
    };
    let bytes = std::fs::read(dir.join(file)).unwrap();
    let with_inst = matches!(ty, "assert_unlinkable" | "assert_uninstantiable");
    expect(tally, wast, line, cmd, &bytes, want, with_inst);
    true
}

/// 1 つのモジュールと、それに続くコマンド列を実行する。
fn run_chunk(
    tally: &mut Tally,
    wast: &str,
    dir: &Path,
    module_cmd: &serde_json::Value,
    rest: &[serde_json::Value],
) {
    let line = module_cmd["line"].as_u64().unwrap_or(0);
    if module_cmd["module_type"].as_str() == Some("text") {
        tally.skipped += 1 + rest.len();
        tally.skip_text += 1 + rest.len();
        return;
    }
    let Some(file) = module_cmd["filename"].as_str() else {
        tally.skipped += 1 + rest.len();
        return;
    };
    let wasm = std::fs::read(dir.join(file)).unwrap();

    let mut buf = arena_buf();
    let mut scratch_buf = vec![0u8; 8 << 20];
    let mut arena = Arena::new(&mut buf);
    let mut scratch = Arena::new(&mut scratch_buf);
    let c = cfg();

    let m = match decode::decode(&wasm, &mut arena) {
        Ok(m) => m,
        Err(e) if e.kind() == ErrorKind::Unsupported => {
            tally.unsupported += 1 + rest.len();
            return;
        }
        Err(e) => {
            tally.fail(
                wast,
                line,
                format!("デコード失敗: {} [{}]", e.reason(), e.kind().name()),
            );
            return;
        }
    };
    let v = match validate::validate(&m, &c, &mut arena, &mut scratch) {
        Ok(v) => v,
        Err(e) if e.kind() == ErrorKind::Unsupported => {
            tally.unsupported += 1 + rest.len();
            return;
        }
        Err(e) => {
            tally.fail(
                wast,
                line,
                format!("検証失敗: {} [{}]", e.reason(), e.kind().name()),
            );
            return;
        }
    };
    // Exec は線形メモリ（arena の残り全部）より先に確保する。
    let mut exec = match Exec::new(&c, &mut arena) {
        Ok(e) => e,
        Err(e) => {
            tally.fail(wast, line, format!("Exec 確保失敗: {}", e.reason()));
            return;
        }
    };
    let mut res = SpecTest;
    let mut inst = match instantiate(m, v, &c, &mut arena, &mut res) {
        Ok(i) => i,
        Err(e) if e.kind() == ErrorKind::Unsupported => {
            tally.unsupported += 1 + rest.len();
            return;
        }
        Err(e) => {
            tally.fail(
                wast,
                line,
                format!("インスタンス化失敗: {} [{}]", e.reason(), e.kind().name()),
            );
            return;
        }
    };
    if let Some(start) = inst.module.start {
        if let Err(e) = invoke(&mut inst, &mut exec, &mut res, start, &[], &mut []) {
            tally.fail(wast, line, format!("start 関数が失敗: {}", e.reason()));
            return;
        }
    }
    tally.ran += 1;

    for cmd in rest {
        let ty = cmd["type"].as_str().unwrap_or("");
        let line = cmd["line"].as_u64().unwrap_or(0);
        if standalone(tally, wast, dir, cmd) {
            continue;
        }
        if !SUPPORTED.contains(&ty) {
            tally.skipped += 1;
            tally.skip_kind += 1;
            continue;
        }
        let action = &cmd["action"];
        // 名前つきモジュールへの操作は扱わない（複数インスタンスは v0.1 の範囲外）。
        if action["module"].is_string() {
            tally.skipped += 1;
            tally.skip_named += 1;
            continue;
        }
        match action["type"].as_str() {
            Some("invoke") => {
                let name = action["field"].as_str().unwrap_or("");
                let Some(func) = inst.export_func(name) else {
                    tally.fail(wast, line, format!("export {name} が無い"));
                    continue;
                };
                let raw = action["args"].as_array().cloned().unwrap_or_default();
                let mut args = Vec::new();
                let mut bad = false;
                for a in &raw {
                    match parse_arg(a) {
                        Some(v) => args.push(v),
                        None => bad = true,
                    }
                }
                if bad {
                    tally.unsupported += 1;
                    continue;
                }
                let mut out = [0u64; 16];
                let n = inst.func_type(func).map_or(0, |t| t.result_count());
                let r = invoke(&mut inst, &mut exec, &mut res, func, &args, &mut out[..n]);
                check(tally, wast, line, cmd, ty, r, &out[..n]);
            }
            Some("get") => {
                let name = action["field"].as_str().unwrap_or("");
                let Some(v) = inst.export_global(name) else {
                    tally.fail(wast, line, format!("global {name} が無い"));
                    continue;
                };
                check(tally, wast, line, cmd, ty, Ok(()), &[v]);
            }
            _ => tally.skipped += 1,
        }
    }
}

/// 実行結果を期待値と突き合わせる。
fn check(
    tally: &mut Tally,
    wast: &str,
    line: u64,
    cmd: &serde_json::Value,
    ty: &str,
    r: Result<(), Error>,
    out: &[u64],
) {
    match ty {
        "assert_return" | "action" => match r {
            Ok(()) => {
                let want = cmd["expected"].as_array().cloned().unwrap_or_default();
                for (i, w) in want.iter().enumerate() {
                    let e = parse_expected(w);
                    if e.is_unsupported() {
                        tally.unsupported += 1;
                        return;
                    }
                    let Some(&got) = out.get(i) else {
                        tally.fail(wast, line, format!("戻り値が足りない（{i} 番目）"));
                        return;
                    };
                    if !e.matches(got) {
                        tally.fail(
                            wast,
                            line,
                            format!(
                                "{i} 番目の戻り値が違う: {got:#x} != {}",
                                w["value"].as_str().unwrap_or("?")
                            ),
                        );
                        return;
                    }
                }
                tally.ran += 1;
            }
            Err(e) if e.kind() == ErrorKind::Unsupported => tally.unsupported += 1,
            Err(e) => tally.fail(wast, line, format!("実行に失敗: {}", e.reason())),
        },
        "assert_trap" => match r {
            Err(ref e) if is_trap(e) => tally.ran += 1,
            Err(e) if e.kind() == ErrorKind::Unsupported => tally.unsupported += 1,
            Err(e) => tally.fail(
                wast,
                line,
                format!("trap を期待したが {} [{}]", e.kind().name(), e.reason()),
            ),
            Ok(()) => tally.fail(wast, line, "trap を期待したが成功した".to_string()),
        },
        "assert_exhaustion" => match r {
            Err(e) if e.kind() == ErrorKind::Exhausted => tally.ran += 1,
            Err(e) => tally.fail(
                wast,
                line,
                format!(
                    "exhaustion を期待したが {} [{}]",
                    e.kind().name(),
                    e.reason()
                ),
            ),
            Ok(()) => tally.fail(wast, line, "exhaustion を期待したが成功した".to_string()),
        },
        _ => tally.skipped += 1,
    }
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

    let mut i = 0;
    while i < commands.len() {
        let cmd = &commands[i];
        if cmd["type"].as_str() == Some("module") {
            let mut j = i + 1;
            while j < commands.len() && commands[j]["type"].as_str() != Some("module") {
                j += 1;
            }
            run_chunk(tally, wast, &dir, cmd, &commands[i + 1..j]);
            i = j;
        } else {
            if !standalone(tally, wast, &dir, cmd) {
                tally.skipped += 1;
                tally.skip_kind += 1;
            }
            i += 1;
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
        "spec: {} コマンド実行 / {} 機能セット外 / {} スキップ / {} ファイル / 除外 {} 分類",
        tally.ran,
        tally.unsupported,
        tally.skipped,
        FILES.len(),
        EXCLUDED.len()
    );
    println!(
        "      スキップの内訳: テキスト形式 {} / 名前つきモジュール {} / 未対応の種別 {} / その他 {}",
        tally.skip_text,
        tally.skip_named,
        tally.skip_kind,
        tally.skipped - tally.skip_text - tally.skip_named - tally.skip_kind
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
        total.skip_text += t.skip_text;
        total.skip_named += t.skip_named;
        total.skip_kind += t.skip_kind;
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
