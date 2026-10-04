//! ゲストのビルドフラグの**単一の真実**（`docs/app-workflow.md` §4.4）。
//!
//! アプリの契約の実体は 2 つのファイル —— `.cargo/config.toml` と
//! `asconfig.json` —— で、これが `apps/` と `wasmicon new` の雛形で
//! **3 重化**する。食い違っても**ビルドは通る**ので、実機で
//! 「call_indirect がテーブル索引で弾かれる」「RP2040 の 2 ページ上限を
//! 超える」といった形で初めて出る。
//!
//! そこで**値の表ではなくファイルの中身そのもの**をここに置き、
//! `wasmicon new` はこれを書き出すだけにした。`apps/` のファイルとの
//! 一致は `tools/wasmicon-cli/tests/flags.rs` が**バイトで**見る
//! （`wasmicon-gen --check` と同じ形。検査は既に CI にある `cargo test` に
//! 乗るので、新しいジョブは要らない）。
//!
//! **コメント 1 文字の差でも落ちる。** 緩めると「値は合っているがどちらが
//! 真実か分からない」状態に戻るので、意図的にそうしてある。
//!
//! ここに入れないもの: `[profile.release]`（`opt-level = "z"` など）は
//! サイズにしか効かず、食い違ってもアプリが**大きくなるだけ**で静かには
//! 壊れない。§4.4 が名指しする 2 つに絞ってある。

/// 線形メモリの初期サイズ（バイト）。**最も厳しいボードの上限に収める**
/// （`ports/rp2040` は 2 ページ）。`tests/flags.rs` が
/// `profile::PROFILES` と突き合わせる。
pub const INITIAL_MEMORY: u32 = 65_536;

/// ゲストのシャドースタック（バイト）。
pub const STACK_SIZE: u32 = 8_192;

/// Wasm の 1 ページ。
pub const PAGE_SIZE: u32 = 65_536;

/// AssemblyScript 側の初期メモリ（**ページ数**で指定する）。
///
/// `INITIAL_MEMORY` とページ単位で一致していないと、**同じアプリを
/// Rust 版と AS 版で書いたときに線形メモリの初期サイズが変わる**。
/// §4.4 が本当に気にしているのはここ。
pub const AS_INITIAL_MEMORY_PAGES: u32 = INITIAL_MEMORY / PAGE_SIZE;

/// `.cargo/config.toml` の中身。`apps/.cargo/config.toml` と**バイト一致**。
pub const CARGO_CONFIG: &str = r#"# ゲストのビルドフラグ（Wasmicon の契約。docs/app-workflow.md §4.4）。
#
# **このファイルは `wasmicon new` が書き出すものと同一**で、一致は
# `tools/wasmicon-cli/tests/flags.rs` が見る。真実は
# `tools/wasmicon-cli/src/flags.rs` の `CARGO_CONFIG` で、ここを手で変えると
# テストが落ちる（コメント 1 文字でも落ちる。それが狙い）。
#
# 値を変えると実機で静かに壊れる: reference-types が混ざれば
# call_indirect のテーブル索引で弾かれ（abi-spec §6.1 の注）、
# --initial-memory を増やすと RP2040 の 2 ページ上限を超える。
#
# cargo は実行したディレクトリから上に向かって .cargo/config.toml を探すので、
# この設定はこのディレクトリの中で cargo を実行したときに効く。
[build]
target = "wasm32-unknown-unknown"

[target.wasm32-unknown-unknown]
rustflags = [
    # rustc 1.97 の wasm32 は reference-types をデフォルトで有効にする。
    # 対応機能セット（docs/handoff.md §2-7）から外れるので明示的に切る。
    # 他の 5 つ（bulk-memory / multivalue / mutable-globals /
    # nontrapping-fptoint / sign-ext）は既定で有効かつ対応済み。
    "-Ctarget-feature=-reference-types",
    # 線形メモリ 1 ページ（64 KiB）。RP2040 の 2 ページ上限に収める。
    "-Clink-arg=--initial-memory=65536",
    # ゲストのシャドースタック。
    "-Clink-arg=-zstack-size=8192",
]
"#;

/// `asconfig.json` の中身。`outFile` だけがプロジェクトごとに変わる。
///
/// `out_file` は `asconfig.json` から見た相対パス（例: `build/blink_as.wasm`）。
#[must_use]
pub fn asconfig(out_file: &str) -> String {
    ASCONFIG
        .replace("{OUT_FILE}", out_file)
        .replace("{PAGES}", &AS_INITIAL_MEMORY_PAGES.to_string())
}

/// `asconfig` の雛形。`{OUT_FILE}` と `{PAGES}` を差し替える。
///
/// JSON はコメントを持てないので、「ここが真実」という案内を
/// ファイル自身に書けない。`apps/*-as/asconfig.json` を手で変えると
/// `tests/flags.rs` が落ちることで気付く形にしてある。
const ASCONFIG: &str = r#"{
  "targets": {
    "release": {
      "outFile": "{OUT_FILE}",
      "optimizeLevel": 3,
      "shrinkLevel": 2,
      "converge": false,
      "noAssert": true
    }
  },
  "options": {
    "runtime": "stub",
    "initialMemory": {PAGES},
    "exportStart": false,
    "disable": ["simd", "threads", "exception-handling", "reference-types"],
    "enable": ["sign-extension", "nontrapping-f2i", "bulk-memory", "mutable-globals"]
  }
}
"#;
