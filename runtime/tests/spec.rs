//! WebAssembly spec testsuite runner.
//!
//! Converts the `.wast` files in `third_party/testsuite` (fetched at a pinned
//! SHA by `tools/fetch-testsuite.sh`) to JSON with `wasm-tools json-from-wast`
//! and runs the commands in order.
//!
//! "All passing" means: every supported command kind in every `.wast` listed in
//! `FILES` passes. Exclusions are listed in `EXCLUDED` with a reason; they are
//! adopted defaults, not frozen decisions.
//!
//! The skip breakdown is printed at run time (`-- --nocapture`). It is not
//! written here because a hand-maintained count always drifts, and it differs
//! from a static count of the JSON: once a chunk is judged outside the feature
//! set, its remaining commands are counted as outside the feature set too.
//!
//! A survey mode for development runs every file and lists the current state:
//! `cargo test -p wasmicon-core --test spec -- --ignored --nocapture`

use std::path::{Path, PathBuf};
use std::process::Command;

mod support;

use support::{SpecTest, is_trap, parse_arg, parse_expected};
use wasmicon_core::{Arena, Config, Error, ErrorKind, Exec, decode, instantiate, invoke, validate};

/// The `.wast` files that run. Grows as support grows.
const FILES: &[&str] = &[
    // The core spec files that fit the supported feature set (docs/decisions.md D-07).
    // Everything else is left out for the reasons in EXCLUDED.
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

/// Excluded files and why.
const EXCLUDED: &[(&str, &str)] = &[
    (
        "simd_*, i8x16_*, i16x8_*, i32x4_*, relaxed_*",
        "SIMD is not supported (docs/decisions.md D-07)",
    ),
    ("*atomic*", "threads are not supported"),
    (
        "ref*, br_on_*, call_ref, i31",
        "reference types and GC are not supported",
    ),
    ("table*", "the table.* instructions are not supported"),
    ("return_call*", "tail calls are not supported"),
    (
        "throw*, try_table, tag*",
        "exception handling is not supported",
    ),
    ("*64.wast, memory64*", "memory64 is not supported"),
    (
        "numbered files such as address0/1, load0/1/2, data0, exports0",
        "multi-memory is not supported",
    ),
    (
        "array*, struct, type-rec, type-canon and the like",
        "GC is not supported",
    ),
    (
        "linking*, imports*, instance",
        "linking several modules is out of scope for v0.1",
    ),
    ("memory_grow", "contains multi-memory modules"),
];

/// Supported command kinds. Grows as support grows.
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

/// Arena size. The remainder becomes linear memory (`Arena::alloc_rest`).
const ARENA: usize = 48 << 20;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("repository root not found")
}

/// Converts a `.wast` to JSON plus `.wasm` files and returns `(json, wasm directory)`.
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
        .expect("cannot run wasm-tools; install it, e.g. `brew install wasm-tools`");
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
    /// Commands skipped as outside the supported feature set (docs/decisions.md D-07).
    unsupported: usize,
    /// Skip breakdown: text-format modules (there is no WAT parser).
    skip_text: usize,
    /// Actions on named modules. Multiple instances are out of scope for v0.1.
    skip_named: usize,
    /// Unsupported command kinds such as `register`.
    skip_kind: usize,
    failures: Vec<String>,
}

impl Tally {
    fn fail(&mut self, wast: &str, line: u64, msg: String) {
        self.failures.push(format!("{wast}:{line}: {msg}"));
    }
}

fn arena_buf() -> Vec<u8> {
    // alloc_zeroed, so only the pages actually touched are materialized.
    vec![0u8; ARENA]
}

fn cfg() -> Config {
    Config::default()
}

/// Decodes and validates only (for `assert_malformed` / `assert_invalid`).
fn load(bytes: &[u8]) -> Result<(), Error> {
    let mut buf = arena_buf();
    let mut scratch_buf = vec![0u8; 8 << 20];
    let mut arena = Arena::new(&mut buf);
    let mut scratch = Arena::new(&mut scratch_buf);
    let m = decode::decode(bytes, &mut arena)?;
    validate::validate(&m, &cfg(), &mut arena, &mut scratch)?;
    Ok(())
}

/// Goes as far as instantiation (for `assert_unlinkable` / `assert_uninstantiable`).
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

/// Checks that loading fails with the given category.
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
        // A case that uses syntax outside the feature set cannot be judged either way.
        Err(e) if e.kind() == ErrorKind::Unsupported => tally.unsupported += 1,
        Err(e) => tally.fail(
            wast,
            line,
            format!(
                "expected {} but got {} [{}] (expected text: {})",
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
                "expected {} but it succeeded ({})",
                want.name(),
                cmd["text"].as_str().unwrap_or("?")
            ),
        ),
    }
}

/// Standalone checks (malformed / invalid / unlinkable / uninstantiable) that do
/// not act on the current module. Returns `true` when handled.
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

/// Runs one module and the commands that follow it.
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
                format!("decode failed: {} [{}]", e.reason(), e.kind().name()),
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
                format!("validation failed: {} [{}]", e.reason(), e.kind().name()),
            );
            return;
        }
    };
    // Exec must be allocated before linear memory (the rest of the arena).
    let mut exec = match Exec::new(&c, &mut arena) {
        Ok(e) => e,
        Err(e) => {
            tally.fail(
                wast,
                line,
                format!("Exec allocation failed: {}", e.reason()),
            );
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
                format!("instantiation failed: {} [{}]", e.reason(), e.kind().name()),
            );
            return;
        }
    };
    if let Some(start) = inst.module.start {
        if let Err(e) = invoke(&mut inst, &mut exec, &mut res, start, &[], &mut []) {
            tally.fail(wast, line, format!("start function failed: {}", e.reason()));
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
        // Actions on named modules are not handled (multiple instances are out of
        // scope for v0.1).
        if action["module"].is_string() {
            tally.skipped += 1;
            tally.skip_named += 1;
            continue;
        }
        match action["type"].as_str() {
            Some("invoke") => {
                let name = action["field"].as_str().unwrap_or("");
                let Some(func) = inst.export_func(name) else {
                    tally.fail(wast, line, format!("missing export {name}"));
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
                    tally.fail(wast, line, format!("missing global {name}"));
                    continue;
                };
                check(tally, wast, line, cmd, ty, Ok(()), &[v]);
            }
            _ => tally.skipped += 1,
        }
    }
}

/// Cross-checks the result against the expected values.
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
                        tally.fail(wast, line, format!("missing result {i}"));
                        return;
                    };
                    if !e.matches(got) {
                        tally.fail(
                            wast,
                            line,
                            format!(
                                "result {i} differs: {got:#x} != {}",
                                w["value"].as_str().unwrap_or("?")
                            ),
                        );
                        return;
                    }
                }
                tally.ran += 1;
            }
            Err(e) if e.kind() == ErrorKind::Unsupported => tally.unsupported += 1,
            Err(e) => tally.fail(wast, line, format!("execution failed: {}", e.reason())),
        },
        "assert_trap" => match r {
            Err(ref e) if is_trap(e) => tally.ran += 1,
            Err(e) if e.kind() == ErrorKind::Unsupported => tally.unsupported += 1,
            Err(e) => tally.fail(
                wast,
                line,
                format!(
                    "expected a trap but got {} [{}]",
                    e.kind().name(),
                    e.reason()
                ),
            ),
            Ok(()) => tally.fail(wast, line, "expected a trap but it succeeded".to_string()),
        },
        "assert_exhaustion" => match r {
            Err(e) if e.kind() == ErrorKind::Exhausted => tally.ran += 1,
            Err(e) => tally.fail(
                wast,
                line,
                format!(
                    "expected exhaustion but got {} [{}]",
                    e.kind().name(),
                    e.reason()
                ),
            ),
            Ok(()) => tally.fail(
                wast,
                line,
                "expected exhaustion but it succeeded".to_string(),
            ),
        },
        _ => tally.skipped += 1,
    }
}

/// Runs the commands of one file. Failures are pushed onto `tally.failures`.
fn run_file(root: &Path, wast: &str, tally: &mut Tally) {
    let Some((json, dir)) = convert(root, wast) else {
        tally.failures.push(format!(
            "{wast}: cannot convert (not fetched, or syntax outside the subset)"
        ));
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
        // CI runs fetch-testsuite.sh first. Locally, say so and return when it is missing.
        println!(
            "spec: third_party/testsuite is missing; skipping. Fetch it with `sh tools/fetch-testsuite.sh`."
        );
        return;
    }
    let mut tally = Tally::default();
    for wast in FILES {
        run_file(&root, wast, &mut tally);
    }
    println!(
        "spec: {} commands run / {} outside the feature set / {} skipped / {} files / {} exclusion groups",
        tally.ran,
        tally.unsupported,
        tally.skipped,
        FILES.len(),
        EXCLUDED.len()
    );
    println!(
        "      skipped: text format {} / named module {} / unsupported kind {} / other {}",
        tally.skip_text,
        tally.skip_named,
        tally.skip_kind,
        tally.skipped - tally.skip_text - tally.skip_named - tally.skip_kind
    );
    assert!(tally.ran > 0, "no command was run");
    assert!(
        tally.failures.is_empty(),
        "{} failures:\n{}",
        tally.failures.len(),
        tally.failures.join("\n")
    );
}

/// Development only: runs every testsuite file and lists the current state. Not run in CI.
#[test]
#[ignore = "survey mode for development"]
fn spec_survey() {
    let root = repo_root();
    let dir = root.join("third_party/testsuite");
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .expect("testsuite is missing")
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
            println!("--- {f}: {} failures", t.failures.len());
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
        "\nsurvey: {} files, {} with failures / run {} / outside the feature set {} / skipped {} / failed {}",
        files.len(),
        bad_files,
        total.ran,
        total.unsupported,
        total.skipped,
        total.failures.len()
    );
}
