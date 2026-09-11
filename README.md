# Wasmicon

マイコン（ESP32-S3 / ESP32-P4 / RP2040）向けの WebAssembly 実行環境。

- **Runtime**: 自作の Core Wasm インタプリタ。`no_std`、依存クレートゼロ、`alloc` 不使用
- **HAL**: GPIO / I2C / SPI / time / log / board を WIT で定義。**Component Model は使わない**。
  WIT は IDL としてのみ使い、`docs/abi-spec.md` の規則で Core Wasm の import に落とす
- **Bindings**: Rust と AssemblyScript。WIT から生成する
- **ゴール**: I2C 温湿度センサー（SHT31）を読んで SPI ディスプレイ（ILI9341）に描くアプリを
  Rust と AS で書き、**同一の Wasm バイナリ**を ESP32-S3 と Pico WH で動かして
  host call トレースが一致することを検証する

## 現在地

全 6 フェーズのソフトウェア側が完了し、CI は 5 ジョブとも green。
**残っているのは実機が要る部分**（→ [`docs/TODO.md`](docs/TODO.md)）。

ボードは ESP32-S3 / RP2040 に加えて **ESP32-P4** を足してある（`ports/esp32p4`）。
GPIO / time / log までは 3 ポートとも同じ形で、I2C / SPI は 3 ポートとも未実装。

実機は 2026-09-11 に M5Stack Tab5 へ書き込みまで到達したが、**その個体が
ESP32-P4 v1.0 でブートローダに弾かれ、ランタイムはまだ一度も実機で動いていない**
（→ [`docs/TODO.md`](docs/TODO.md) §1.1.5）。

P4 ポートだけは**チップ層とボード定義を分けてある**。同じ P4 でもボードごとに
GPIO の意味が入れ替わる（`G23` は P4-EYE ではオンボード LED、Tab5 では TP_INT、
Function-EV-Board では LCD バックライト）ため、予約ピンと役割名は共有できない。
ボード定義は `src/boards/` に置き、Cargo feature でちょうど 1 つ選ぶ（既定 = `tab5`）。

| 検証 | 状態 |
|---|---|
| Wasm 仕様適合（spec testsuite コア 74 ファイル / 22507 コマンド） | 達成 |
| インタプリタの正しさ（wasmtime との差分、4 ゲスト） | 達成 |
| Rust 版と AS 版が同じ host call 列を出す（成功経路 + 失敗経路） | 達成 |
| 同一バイナリが 2 ボードで同じトレースを出す | **未達（実機が必要）** |

詳細は [`docs/verification-report.md`](docs/verification-report.md)。

コアのコードサイズは thumbv6m-none-eabi 向けで **49.4 KiB**（`sh tools/measure-size.sh`）。

## 構成

```
wit/                  HAL 定義。唯一の真実。生成物を手で編集しない
tools/wasmicon-gen/   wit/ から 3 つの生成物を出すジェネレータ
runtime/              wasmicon-core。no_std / 依存ゼロ / alloc 不使用のインタプリタ
ports/common/         ポート共通の HAL。ボード固有の操作だけ Board トレイトに切り出す
ports/host/           PC 用。mock HAL + トレース。CI はここで回す
ports/rp2040/         Raspberry Pi Pico WH（別 workspace）
ports/esp32s3/        ESP32-S3 DevKitC-1（別 workspace、esp toolchain）
ports/esp32p4/        ESP32-P4（別 workspace、RISC-V なので upstream stable）
  src/chip.rs           P4 のレジスタ操作。ボードに依存しない
  src/boards/           どのピンが何に繋がっているか。feature でちょうど 1 つ選ぶ
  src/board.rs          上の 2 つを繋いで Board トレイトを実装する
bindings/rust/        ゲスト向け Rust バインディング
bindings/assemblyscript/  同 AssemblyScript
apps/                 ゲスト（別 workspace）。blink と sensor-display の Rust / AS 版
verify/               検証の道具。wasmtime との差分テスト、トレース diff
docs/                 仕様・設計・検証レポート・残作業
```

`ports/rp2040` / `ports/esp32s3` / `ports/esp32p4` / `apps` / `verify/differential` は
ターゲットも toolchain も profile も違うので**別 workspace**にしてある。

## ビルドと検証

上から順にそのまま貼れる。**`cargo test` の前に testsuite を取ること。**
無いと spec テストは「取得されていない」と出して丸ごとスキップし、
それでも `test result: ok` になる。

```bash
# spec テストに要る testsuite（固定 SHA）。cargo test より先に。
sh tools/fetch-testsuite.sh

# ホスト側（ランタイム、ジェネレータ、host ポート）
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test

# 生成物が wit/ と一致しているか
cargo run -p wasmicon-gen -- --check
sh tools/check-sigs.sh          # 参照実装 tools/wit2sig.py と突き合わせ

# ゲスト（wasm32-unknown-unknown と AssemblyScript）
npm ci
(cd apps && cargo build --release)
(cd apps/sensor-display-as && npx asc assembly/index.ts --config asconfig.json --target release)

# 実機向け
(cd ports/rp2040 && cargo build --release)
sh ports/esp32s3/build.sh build --release   # ~/export-esp.sh を読んでから cargo を呼ぶ
(cd ports/esp32p4 && cargo build --release) # P4 は RISC-V なので espup 不要
                                            # ボードは feature で選ぶ（既定 = tab5）

# wasmtime との差分テスト（インタプリタの正しさ）
(cd verify/differential && cargo test)

# 実機のトレースを突き合わせる（2 ボード）
sh verify/diff-traces.sh pico.log esp32s3.log

# その他
wasm-tools component wit wit/                              # WIT の構文検証
wasm-tools json-from-wast <file>.wast -o out/<file>.json   # spec テストの変換（旧 wast2json）
wasm-tools validate --features=mvp,sign-extension,saturating-float-to-int,bulk-memory,multi-value,mutable-global app.wasm
rustup target add thumbv6m-none-eabi                       # RP2040
espup install                                              # ESP32-S3（Xtensa フォーク）
rustup target add riscv32imafc-unknown-none-elf            # ESP32-P4
sh tools/measure-size.sh                                   # コアのコードサイズ
```

## ドキュメント

| ファイル | 内容 |
|---|---|
| [`docs/TODO.md`](docs/TODO.md) | **残作業。ここだけ見れば何が残っているか分かる** |
| [`docs/abi-spec.md`](docs/abi-spec.md) | WIT → Core Wasm の lowering 規則。**この仕様が正** |
| [`docs/verification-report.md`](docs/verification-report.md) | 何がどこまで検証されたか、何がされていないか |
| [`docs/design-notes.md`](docs/design-notes.md) | 背景・方針・技術選定の理由 |
| [`docs/handoff.md`](docs/handoff.md) | 確定した決定事項、フェーズと完了条件、落とし穴。**コードのコメントが節番号で参照している** |
| [`apps/README.md`](apps/README.md) | Rust 版と AS 版で描画を揃えるための共通仕様 |

ブランチは `v2`。`main` は旧実装（wasm decoder / llvm 試行）で、潰さずに残してある。
