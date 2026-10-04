//! ゲストのビルドフラグが 1 箇所にしか無いことを固定する
//! （`docs/app-workflow.md` §4.4）。
//!
//! **`wasmicon-gen --check` と同じ形**: 真実（`flags.rs`）が書き出すものと
//! リポジトリの実物を**バイトで**突き合わせる。コメント 1 文字でも落ちる。
//!
//! これが無いと、`apps/` を直したのに `wasmicon new` の雛形が古いまま、
//! という形で静かに壊れる（ビルドは通り、実機で初めて出る）。
//!
//! 検査は `cargo test` に乗るので CI のジョブは要らない
//! （`.github/workflows/ci.yml` の `- run: cargo test`）。

use std::path::{Path, PathBuf};

use wasmicon_cli::flags;
use wasmicon_port::profile;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("リポジトリルートが見つからない")
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{} を読めない: {e}", p.display()))
}

#[test]
fn apps_cargo_config_is_the_canonical_one() {
    assert_eq!(
        read("apps/.cargo/config.toml"),
        flags::CARGO_CONFIG,
        "apps/.cargo/config.toml が tools/wasmicon-cli/src/flags.rs の \
         CARGO_CONFIG と違う。**真実は flags.rs の側**（wasmicon new が \
         同じものを書き出す）。apps/ を直したいなら flags.rs を直すこと"
    );
}

#[test]
fn every_asconfig_is_the_canonical_one() {
    let apps = repo_root().join("apps");
    let mut seen = 0;
    for entry in std::fs::read_dir(&apps).expect("apps/ を読めない") {
        let dir = entry.expect("apps/ の要素を読めない").path();
        let conf = dir.join("asconfig.json");
        if !conf.is_file() {
            continue;
        }
        let name = dir
            .file_name()
            .and_then(|s| s.to_str())
            .expect("ディレクトリ名")
            .to_string();
        // `outFile` だけがプロジェクトごとに変わる。ディレクトリ名から導く
        // ので、名前と中身の食い違いもここで落ちる。
        let want = flags::asconfig(&format!("build/{}.wasm", name.replace('-', "_")));
        let got = std::fs::read_to_string(&conf).expect("asconfig.json を読めない");
        assert_eq!(got, want, "{}/asconfig.json が flags.rs の雛形と違う", name);
        seen += 1;
    }
    // **0 件を黙って成功にしない**（apps/ の構成が変わって拾えなくなった
    // のに「一致した」と言う形が最悪）。
    assert!(seen >= 2, "asconfig.json が {seen} 件しか見つからない");
}

#[test]
fn the_two_languages_ask_for_the_same_initial_memory() {
    // **§4.4 が本当に気にしているのはここ。** Rust 版と AS 版で初期メモリが
    // 違うと、同じアプリのはずなのにボードの上限の当たり方が変わる。
    assert_eq!(
        flags::AS_INITIAL_MEMORY_PAGES * flags::PAGE_SIZE,
        flags::INITIAL_MEMORY,
        "AS 側のページ数と Rust 側のバイト数が食い違っている"
    );
    // 文字列の中の数値が定数と一致していること（片方だけ直す事故を防ぐ）。
    assert!(
        flags::CARGO_CONFIG.contains(&format!("--initial-memory={}", flags::INITIAL_MEMORY)),
        "CARGO_CONFIG の --initial-memory が INITIAL_MEMORY と違う"
    );
    assert!(
        flags::CARGO_CONFIG.contains(&format!("-zstack-size={}", flags::STACK_SIZE)),
        "CARGO_CONFIG の -zstack-size が STACK_SIZE と違う"
    );
    assert!(
        flags::asconfig("x.wasm").contains(&format!(
            "\"initialMemory\": {}",
            flags::AS_INITIAL_MEMORY_PAGES
        )),
        "asconfig の initialMemory が定数と違う"
    );
}

#[test]
fn the_initial_memory_fits_the_tightest_board() {
    // 一番きついボード（`ports/rp2040` は 2 ページ）に収まっていること。
    // 上げるとそのボードで instantiate が落ちる（`docs/TODO.md` §5）。
    let tightest = profile::PROFILES
        .iter()
        .filter(|p| p.slot.is_some()) // host は上限が緩いので除く
        .map(|p| p.config.max_memory_pages)
        .min()
        .expect("プロファイルが無い");
    let pages = flags::INITIAL_MEMORY / flags::PAGE_SIZE;
    assert!(
        pages <= tightest,
        "初期 {pages} ページは一番きついボードの {tightest} ページを超える"
    );
}

#[test]
fn reference_types_stays_off() {
    // 混ざると `call_indirect` のテーブル索引で弾かれる（abi-spec §6.1 の注）。
    // 既定で有効になる機能なので、**明示的に切ったままである**ことを見る。
    assert!(
        flags::CARGO_CONFIG.contains("-Ctarget-feature=-reference-types"),
        "reference-types を切る指定が無い"
    );
    assert!(
        flags::asconfig("x.wasm").contains("\"reference-types\""),
        "asconfig の disable に reference-types が無い"
    );
}
