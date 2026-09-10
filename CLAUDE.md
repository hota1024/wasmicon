# Wasmicon

マイコン (ESP32-S3 / RP2040) 向け WebAssembly 実行環境。自作 Core Wasm ランタイム + WIT 定義の HAL + Rust/AssemblyScript バインディング。

## まず読むもの

1. `HANDOFF.md` — 現状、確定事項、未決事項のデフォルト、フェーズと完了条件
2. `docs/abi-spec.md` — WIT → Core Wasm の lowering 規則。**この仕様が正**
3. `wit/` — HAL 定義。**唯一の真実**。生成物を手で編集しない

## 絶対に守ること

- `docs/abi-spec.md` §2 のサブセット外の WIT 構文を使わない
- ABI（import 名、シグネチャ、エラーコード = discriminant + 1、ハンドル 0 = 無効）を変更しない。変更が必要なら実装せず理由を書いてオーナーに確認
- ランタイムコア (`runtime/`) は Rust `no_std`、**依存クレートゼロ**、`alloc` 不使用（arena をポートから注入）、警告ゼロ
- Wasm の算術は wrapping。`wrapping_*` / `rotate_*` / `checked_*` を明示的に使う
- `unsafe` は最小限。書くときは直前に `// SAFETY:` を必ず添える
- コアで `core::fmt`（`format!` / `write!` / `{:?}`）を使わない。バイナリが肥大する
- 生成物 (`runtime/src/generated.rs`, `bindings/*/generated.*`) はジェネレータ出力のみ。CI で再生成 diff ゼロを検査
- フェーズは `HANDOFF.md` §5 の順に進め、完了条件を満たしてから次へ

## 検証

```bash
wasm-tools component wit wit/            # WIT 構文
cargo run -p wasmicon-gen                # 生成物を更新
cargo run -p wasmicon-gen -- --check     # 生成物が wit/ と一致するか
sh tools/check-sigs.sh                   # 参照実装 tools/wit2sig.py と突き合わせ
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

## 未決事項のデフォルト

`HANDOFF.md` §3 の表に従う。採用したデフォルトはコミットメッセージと PR に明記する。
