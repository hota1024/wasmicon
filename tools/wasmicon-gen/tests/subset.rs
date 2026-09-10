//! サブセット（`docs/abi-spec.md` §2.2）の外にある WIT を拒否することを検査する。
//!
//! 通してしまうと、ランタイム・Rust・AS の 3 箇所で lowering がずれる。

use std::path::PathBuf;

/// 一時ディレクトリに 1 ファイルの WIT を書いて lowering を試す。
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
        Ok(_) => panic!("{tag}: サブセット外の WIT が通ってしまった"),
        Err(e) => {
            let msg = format!("{e:#}");
            assert!(
                msg.contains("サブセット外") || msg.contains("未実装"),
                "{tag}: 拒否はされたが理由が不明瞭: {msg}"
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
    .expect("サブセット内の WIT が通らない");
}

/// abi-spec §2.1 は flags を許可している。ジェネレータもそれに追随する。
#[test]
fn accepts_flags_and_emits_masks() {
    let hal = try_load(
        "flags",
        "    flags perm { read, write, exec }\n             f: func(p: perm) -> result<_, error-code>;",
    )
    .expect("flags が通らない（abi-spec §2.1 は許可している）");

    let rs = wasmicon_gen::emit::runtime_rs(&hal);
    assert!(rs.contains("pub const Read: u32 = 1 << 0;"), "{rs}");
    assert!(rs.contains("pub const Exec: u32 = 1 << 2;"), "{rs}");
    assert!(rs.contains("pub const ALL: u32 = 0x7;"), "{rs}");

    let ts = wasmicon_gen::emit::bindings_ts(&hal);
    assert!(ts.contains("export const BadPermExec: u32 = 0x4;"), "{ts}");
    assert!(ts.contains("export const BadPermAll: u32 = 0x7;"), "{ts}");

    // flags 引数は i32 1 つに落ちる（abi-spec §4.1）。
    assert_eq!(
        wasmicon_gen::sig_dump(&hal)
            .trim_end()
            .split('\t')
            .next_back(),
        Some("i:i")
    );
}

/// abi-spec §3.3 は run を func() -> () と定めている。
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
    let e = r.expect_err("run の型違いが通ってしまった");
    assert!(
        format!("{e:#}").contains("func() -> ()"),
        "拒否理由が不明瞭: {e:#}"
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
