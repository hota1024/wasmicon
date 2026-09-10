//! サブセット（`docs/abi-spec.md` §2.2）の外にある WIT を拒否することを検査する。
//!
//! 通してしまうと、ランタイム・Rust・AS の 3 箇所で lowering がずれる。

use std::path::PathBuf;

/// 一時ディレクトリに 1 ファイルの WIT を書いて lowering を試す。
fn try_load(tag: &str, body: &str) -> anyhow::Result<()> {
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
    let r = wasmicon_gen::lower::load(&dir).map(|_| ());
    let _ = std::fs::remove_dir_all(&dir);
    r
}

fn assert_rejected(tag: &str, body: &str) {
    match try_load(tag, body) {
        Ok(()) => panic!("{tag}: サブセット外の WIT が通ってしまった"),
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
