//! `wasmicon new` の雛形が**実際に使える**ことを固定する
//! （`docs/app-workflow.md` §4.6 の 3）。
//!
//! `apps/` とのバイト一致は `tests/flags.rs` が見るが、**一致していても
//! 動かない形はあり得る**（依存のパスが間違っている、`[workspace]` が
//! 無くて親の workspace に吸われる、`run` の export が出ない）。ここでは
//! 雛形を本当にビルドして `check` を通す。

use std::path::{Path, PathBuf};
use std::process::Command;

use wasmicon_cli::{check, manifest, new};
use wasmicon_port::profile;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("リポジトリルートが見つからない")
}

/// テストごとに別の作業ディレクトリ。**共有すると並列で壊れる**
/// （`tests/manifest.rs` で踏んだ）。
fn tmp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("new-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("作業ディレクトリを作れない");
    dir
}

fn opts(dir: PathBuf, lang: new::Lang) -> new::Options {
    new::Options {
        dir,
        lang,
        board: None,
        // **明示的に渡す。** 上に向かって探す経路も別のテストで見るが、
        // ここは雛形そのものを見たいので曖昧さを消す。
        hal: Some(repo_root().join("bindings")),
    }
}

fn created(o: &new::Options) -> new::Created {
    match new::create(o) {
        Ok(c) => c,
        Err(e) => panic!("雛形を作れなかった: {e:#}"),
    }
}

#[test]
fn the_rust_template_builds_and_passes_check_on_every_board() {
    // `apps/` のビルドと同じ理由で直列化する（`cargo` が同じ
    // レジストリロックを取る）。
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let made = created(&opts(tmp("rust").join("hello-app"), new::Lang::Rust));
    assert_eq!(made.name, "hello-app");
    assert!(made.hal.is_some(), "--hal を渡したのに見つかっていない");

    // **入れ子の cargo に rust-toolchain.toml が効かない場合がある**ので、
    // CI と同じく親の toolchain 指定を外す（`tests/deploy.rs` と同じ）。
    let status = Command::new("cargo")
        .current_dir(&made.dir)
        .env_remove("RUSTUP_TOOLCHAIN")
        .args(["build", "--release"])
        .status()
        .expect("cargo を起動できない");
    assert!(status.success(), "雛形のビルドに失敗した");

    let wasm = made
        .dir
        .join("target/wasm32-unknown-unknown/release/hello_app.wasm");
    let bytes =
        std::fs::read(&wasm).unwrap_or_else(|e| panic!("{} を読めない: {e}", wasm.display()));

    // 全ボード共通の不備が無いこと（import 名、export、memory）。
    let facts = match check::facts(&bytes) {
        Ok(f) => f,
        Err(e) => panic!("check が落ちた: {e:#}"),
    };
    assert_eq!(facts.failures(), 0, "雛形に全ボード共通の不備がある");

    // **4 ボードすべてで通ること。** 雛形は log だけを使うので、SPI / I2C が
    // 未実装の rp2040 でも通るのが正しい。
    for p in profile::PROFILES {
        let verdict = check::judge(&bytes, p, &facts, None);
        assert!(
            verdict.is_ok(),
            "雛形が {} で通らない（{} 件）",
            p.name,
            verdict.fails()
        );
    }
}

#[test]
fn the_generated_manifest_round_trips() {
    // `wasmicon.toml` は `deny_unknown_fields` なので、雛形が余計なキーを
    // 書いていたら読めない。**自分が書いたものを自分で読めない**のは
    // 一番ばかばかしい壊れ方なので固定する。
    let made = created(&opts(tmp("toml").join("round-trip"), new::Lang::Rust));
    let path = made.dir.join(manifest::FILE_NAME);
    let m = match manifest::load(&path) {
        Ok(m) => m,
        Err(e) => panic!("雛形の wasmicon.toml を読めない: {e:#}"),
    };
    // 雛形は役割を使わないので空。既定のボードも決め打ちしない。
    assert!(m.pin_roles.is_empty());
    assert!(m.default_board.is_none());

    // `--board` を渡したときは書かれて、読めること。
    let mut o = opts(tmp("toml-board").join("with-board"), new::Lang::Rust);
    o.board = Some("rp2350".to_string());
    let made = created(&o);
    let m = match manifest::load(&made.dir.join(manifest::FILE_NAME)) {
        Ok(m) => m,
        Err(e) => panic!("読めない: {e:#}"),
    };
    assert_eq!(m.default_board.as_deref(), Some("rp2350"));
}

#[test]
fn the_as_template_builds() {
    let root = repo_root();
    assert!(
        root.join("node_modules/assemblyscript").exists(),
        "AssemblyScript が入っていない。リポジトリルートで `npm install` を実行すること"
    );

    let made = created(&opts(tmp("as").join("hello-as"), new::Lang::AssemblyScript));
    // asc は雛形の中で動かす（`node_modules` はリポジトリルートのものを
    // 使うので、`npx` をそこから呼ぶ）。
    let status = Command::new("npx")
        .current_dir(&root)
        .args([
            "asc",
            made.dir
                .join("assembly/index.ts")
                .to_str()
                .expect("パスが utf-8 でない"),
            "--config",
            made.dir
                .join("asconfig.json")
                .to_str()
                .expect("パスが utf-8 でない"),
            "--target",
            "release",
        ])
        .status()
        .expect("npx を起動できない");
    assert!(status.success(), "AS の雛形のビルドに失敗した");

    let wasm = made.dir.join("build/hello_as.wasm");
    let bytes =
        std::fs::read(&wasm).unwrap_or_else(|e| panic!("{} を読めない: {e}", wasm.display()));
    let facts = match check::facts(&bytes) {
        Ok(f) => f,
        Err(e) => panic!("check が落ちた: {e:#}"),
    };
    assert_eq!(facts.failures(), 0, "AS の雛形に不備がある");
}

#[test]
fn the_written_hal_path_actually_resolves() {
    // 相対・絶対のどちらを選んでも、書いたパスが解決できなければ
    // `cargo build` が「見つからない」で止まる。root を素朴に連結して
    // スラッシュが 2 つ並ぶ形を一度踏んだので固定する。
    let made = created(&opts(tmp("resolve").join("resolve-app"), new::Lang::Rust));
    let cargo = std::fs::read_to_string(made.dir.join("Cargo.toml")).expect("読めない");
    let written = cargo
        .lines()
        .find_map(|l| l.strip_prefix("wasmicon-hal = { path = \""))
        .and_then(|l| l.split('"').next())
        .expect("依存の path を取れない");
    assert!(!written.contains("//"), "パスが二重スラッシュ: {written}");
    let resolved = made.dir.join(written).join("Cargo.toml");
    assert!(
        resolved.exists(),
        "書いた path が解決できない: {written}（{}）",
        resolved.display()
    );

    // AS 側も同じ。import の相対パスが解決できなければ asc が落ちる。
    let made = created(&opts(
        tmp("resolve-as").join("resolve-as"),
        new::Lang::AssemblyScript,
    ));
    let index = std::fs::read_to_string(made.dir.join("assembly/index.ts")).expect("読めない");
    let import = index
        .lines()
        .find_map(|l| l.split_once("from \"").map(|(_, r)| r))
        .and_then(|r| r.split('"').next())
        .expect("import を取れない");
    assert!(!import.contains("//"), "import が二重スラッシュ: {import}");
    let resolved = made.dir.join("assembly").join(format!("{import}.ts"));
    assert!(
        resolved.exists(),
        "import が解決できない: {import}（{}）",
        resolved.display()
    );
}

#[test]
fn it_refuses_to_write_into_a_non_empty_directory() {
    let dir = tmp("non-empty").join("taken");
    std::fs::create_dir_all(&dir).expect("作れない");
    std::fs::write(dir.join("README.md"), "先にあったもの").expect("書けない");

    let err = new::create(&opts(dir.clone(), new::Lang::Rust))
        .err()
        .expect("空でないのに通ってしまった");
    assert!(format!("{err:#}").contains("空でない"), "{err:#}");
    // **既にあったものを消さない。**
    assert!(dir.join("README.md").is_file());
}

#[test]
fn it_refuses_a_hal_path_without_bindings() {
    // **`--hal` も中身まで見る。** 探索の枝（`find_bindings`）は
    // `bindings/rust/Cargo.toml` があることを条件にしているのに、
    // `--hal` は存在するディレクトリなら何でも受けていた。すると
    // `<渡された所>/rust` が Cargo.toml に書かれ、`cargo build` が
    // 「no Cargo.toml」で落ちるまで分からない。
    let base = tmp("bad-hal");
    let empty = base.join("not-bindings");
    std::fs::create_dir_all(&empty).expect("作れない");

    let mut o = opts(base.join("app"), new::Lang::Rust);
    o.hal = Some(empty.clone());
    let err = new::create(&o).err().expect("bindings が無いのに通った");
    let msg = format!("{err:#}");
    assert!(msg.contains("bindings が無い"), "{msg}");
    // 何を探したかを出していること（渡す階層を間違えたときに一番欲しい）。
    assert!(msg.contains("rust/Cargo.toml"), "{msg}");

    // 無いパスは今までどおり弾く。
    let mut o = opts(base.join("app2"), new::Lang::Rust);
    o.hal = Some(base.join("nowhere"));
    let msg = format!("{:#}", new::create(&o).err().expect("通った"));
    assert!(msg.contains("--hal"), "{msg}");
}

#[test]
fn it_refuses_an_unknown_board() {
    // `manifest::load` は `[defaults] board` を検証しないので、ここで
    // 弾かないと `new` が通って**そのあと全部のコマンドが落ちる**。
    let mut o = opts(tmp("bad-board").join("bad-board"), new::Lang::Rust);
    o.board = Some("rp2040x".to_string());
    let err = new::create(&o)
        .err()
        .expect("知らないボードが通ってしまった");
    let msg = format!("{err:#}");
    assert!(msg.contains("知らないボード"), "{msg}");
    // 候補を出していること（打ち間違えたときに一番欲しい）。
    assert!(msg.contains("rp2040"), "{msg}");
}

#[test]
fn it_refuses_a_name_cargo_cannot_use() {
    let base = tmp("bad-name");
    for bad in ["1st-app", "my app", "app.rs"] {
        let err = new::create(&opts(base.join(bad), new::Lang::Rust))
            .err()
            .unwrap_or_else(|| panic!("{bad} が通ってしまった"));
        let msg = format!("{err:#}");
        assert!(msg.contains("アプリ名"), "{bad}: {msg}");
    }
}

#[test]
fn it_finds_the_bindings_by_walking_up() {
    // `--hal` を渡さない経路。リポジトリの中に作れば見つかるはず。
    // `CARGO_TARGET_TMPDIR` はリポジトリの `target/` の下なので、ここに
    // 作れば上に向かって `bindings/` が見つかる。
    let made = created(&new::Options {
        dir: tmp("walkup").join("found-app"),
        lang: new::Lang::Rust,
        board: None,
        hal: None,
    });
    let hal = made
        .hal
        .expect("リポジトリの中なのに bindings を見つけていない");
    assert!(hal.join("rust/Cargo.toml").is_file());
    // 依存はパスで書かれていること（版指定だと今は解決できない）。
    let cargo = std::fs::read_to_string(made.dir.join("Cargo.toml")).expect("読めない");
    assert!(
        cargo.contains("path = \""),
        "依存がパスで書かれていない:\n{cargo}"
    );
    // **リポジトリの workspace の中なので警告の対象**。
    assert!(made.inside_workspace);
}
