//! Checks that WIT outside the subset of docs/abi-spec.md §2.2 is rejected.
//!
//! Letting it through would make the runtime, Rust and AS lowerings drift apart.

use std::path::PathBuf;

/// Writes a one-file WIT package to a temporary directory and tries to lower it.
fn try_load(tag: &str, body: &str) -> anyhow::Result<wasmicon_gen::model::Hal> {
    let dir: PathBuf = std::env::temp_dir().join(format!("wasmicon-gen-subset-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let wit = format!(
        "package wasmicon:hal@0.1.0;\n\
         \n\
         interface types {{\n\
         \x20   enum error-code {{ io }}\n\
         }}\n\
         \n\
         interface bad {{\n\
         \x20   use types.{{error-code}};\n\
         {body}\n\
         }}\n\
         \n\
         world app {{\n\
         \x20   import types;\n\
         \x20   import bad;\n\
         \x20   export run: func();\n\
         }}\n"
    );
    std::fs::write(dir.join("bad.wit"), wit)?;
    let r = wasmicon_gen::lower::load(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    r
}

fn assert_rejected(tag: &str, body: &str) {
    match try_load(tag, body) {
        Ok(_) => panic!("{tag}: WIT outside the subset was accepted"),
        Err(e) => {
            let msg = format!("{e:#}");
            assert!(
                msg.contains("outside the WIT subset") || msg.contains("not implemented"),
                "{tag}: rejected, but the reason is unclear: {msg}"
            );
        }
    }
}

#[test]
fn accepts_the_subset() {
    try_load(
        "ok",
        "    resource thing {\n\
         \x20       open: static func(index: u32) -> result<thing, error-code>;\n\
         \x20       send: func(data: list<u8>) -> result<_, error-code>;\n\
         \x20       recv: func(len: u32) -> result<list<u8>, error-code>;\n\
         \x20   }",
    )
    .expect("WIT inside the subset was rejected");
}

/// docs/abi-spec.md §2.1 allows flags; the generator follows.
#[test]
fn accepts_flags_and_emits_masks() {
    let hal = try_load(
        "flags",
        "    flags perm { read, write, exec }\n             f: func(p: perm) -> result<_, error-code>;",
    )
    .expect("flags were rejected (docs/abi-spec.md §2.1 allows them)");

    let rs = wasmicon_gen::emit::runtime_rs(&hal);
    assert!(rs.contains("pub const Read: u32 = 1 << 0;"), "{rs}");
    assert!(rs.contains("pub const Exec: u32 = 1 << 2;"), "{rs}");
    assert!(rs.contains("pub const ALL: u32 = 0x7;"), "{rs}");

    let ts = wasmicon_gen::emit::bindings_ts(&hal);
    assert!(ts.contains("export const BadPermExec: u32 = 0x4;"), "{ts}");
    assert!(ts.contains("export const BadPermAll: u32 = 0x7;"), "{ts}");

    // A flags parameter lowers to a single i32 (docs/abi-spec.md §4.1).
    assert_eq!(
        wasmicon_gen::sig_dump(&hal)
            .trim_end()
            .split('\t')
            .next_back(),
        Some("i:i")
    );
}

/// docs/abi-spec.md §3.3 defines run as func() -> ().
#[test]
fn rejects_run_with_wrong_signature() {
    let dir = std::env::temp_dir().join("wasmicon-gen-subset-run");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("bad.wit"),
        "package wasmicon:hal@0.1.0;\n\
         interface types { enum error-code { io } }\n\
         world app {\n\
         \x20   import types;\n\
         \x20   export run: func(x: u32) -> u32;\n\
         }\n",
    )
    .unwrap();
    let r = wasmicon_gen::lower::load(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    let e = r.expect_err("run with the wrong signature was accepted");
    assert!(
        format!("{e:#}").contains("func() -> ()"),
        "the reason for rejection is unclear: {e:#}"
    );
}

#[test]
fn rejects_record() {
    assert_rejected(
        "record",
        "    record point { x: u32 }\n    f: func(p: point);",
    );
}

#[test]
fn rejects_variant() {
    assert_rejected("variant", "    variant v { a, b(u32) }\n    f: func(x: v);");
}

#[test]
fn rejects_tuple() {
    assert_rejected("tuple", "    f: func(x: tuple<u32, u32>);");
}

#[test]
fn rejects_option() {
    assert_rejected("option", "    f: func(x: option<u32>);");
}

#[test]
fn rejects_non_u8_list() {
    assert_rejected("list", "    f: func(x: list<u32>);");
}

#[test]
fn rejects_string_result() {
    assert_rejected("string-ret", "    f: func() -> result<string, error-code>;");
}

#[test]
fn rejects_char() {
    assert_rejected("char", "    f: func(c: char);");
}
