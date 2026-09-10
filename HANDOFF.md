# Wasmicon 引き継ぎドキュメント（Claude Code 向け）

作成日: 2026-09-10
作成者: Claude (Cowork セッション) / オーナー: hota
同梱: `wit/`（WIT 7 ファイル）, `docs/abi-spec.md`, `docs/design-notes.md`, `tools/wit2sig.py`, `CLAUDE.md`
更新: 2026-09-10 — §3 の #1（ランタイム実装言語）と #7（実装分担）をオーナーが決定。ランタイムは **Rust `no_std`**。§2-11 / §2-12 に確定事項として移動

このドキュメントは、設計フェーズを終えた Wasmicon プロジェクトを実装フェーズへ引き継ぐためのものです。**ここに書いてある決定事項は覆さないでください。** 変更が必要だと判断したら、変更せずに理由を書いてオーナーに確認してください。

---

## 0. 30 秒で分かる Wasmicon

マイコン（ESP32-S3, RP2040）向けの WebAssembly 実行環境。

- **Runtime**: 自作の Core Wasm インタプリタ（既存の wasm3 / WAMR / wasmi は使わない）
- **HAL**: GPIO / I2C / SPI / time / log を WIT で定義。**Component Model は使わない**。WIT は IDL としてのみ使い、`abi-spec.md` の規則で Core Wasm の import に落とす
- **Bindings**: Rust と AssemblyScript。WIT から生成する
- **ゴール**: I2C 温湿度センサー (SHT31) を読んで SPI ディスプレイ (ILI9341) に描くアプリを Rust と AS で書き、**同一 Wasm バイナリ**を ESP32-S3 と Pico WH で動かして host call トレースが一致することを検証する

---

## 1. 現在の状態

| 成果物 | 状態 | 場所 |
|---|---|---|
| 設計メモ（背景・方針・ロードマップ） | 完了 | `design-notes.md` |
| ABI 仕様書 v0.1 | 完了（draft、未決事項 5 件） | `abi-spec.md` |
| WIT 定義 `wasmicon:hal@0.1.0` | 完了、`wasm-tools component wit` で検証済み | `wit/*.wit` |
| シグネチャ導出スクリプト（ジェネレータの種） | 完了、abi-spec §7 と一致確認済み | `tools/wit2sig.py` |
| ランタイム | 完了（Phase 2）。spec テストのコア 74 ファイルが通る | `runtime/` |
| ジェネレータ `wasmicon-gen` | 完了（Phase 1）。3 出力を生成、abi-spec §7 との一致をテストで検査 | `tools/wasmicon-gen/` |
| バインディング | 生成物のみ（安全ラッパは Phase 3） | `bindings/rust/`, `bindings/assemblyscript/` |
| ポート層 (host / rp2040 / esp32s3) | host は完了（mock HAL + トレース）。実機は Phase 4（§3 #8） | `ports/host/` |
| サンプルアプリ | **未着手** | — |

Phase 2（ランタイムコア + host ポート）まで完了。実機ポートとアプリは未着手。

---

## 2. 確定している決定事項（変更禁止）

1. **WIT のサブセット**は `abi-spec.md` §2 の通り。`record` / `variant` / `option` / `tuple` は使わない。
2. **ハンドル表現**は WIT の `resource` 構文。import 名は Canonical ABI と同じ `[static]r.f` / `[method]r.f` / `[resource-drop]r`。
3. **エラーコード**は HAL 共通の `types.error-code` 1 つ。関数は単一の i32 ステータスを返す（0 = 成功、n>0 = discriminant+1）。
4. **`result<T,_>` の T は末尾 out ポインタ**、`list<u8>` の戻り値は `(buf, cap, len-out)` の呼び出し側バッファ。`cabi_realloc` は不要。
5. **import モジュール名**は `wasmicon:hal/<iface>@0.1.0`。バージョン必須、完全一致。
6. **ゲスト export** は `run: func()` と `memory`。`_start` / `_initialize` は使わない。
7. **対応機能セット**: MVP + sign-ext + nontrapping-fptoint + bulk-memory（memory.* のみ）+ multi-value + mutable-globals。SIMD / threads / EH / tail-call / GC / reference-types 拡張は非対応。
8. **SPI の CS/DC はゲストが gpio で制御**する。SPI インターフェースは CS を扱わない。
9. **センサーは SHT31/SHT30**、ディスプレイは ILI9341。
10. **決定性検証の定義**: `time` を除く全 host call とその結果のトレース（abi-spec §9 の形式）が両ボードで一致すること。
11. **ランタイム実装言語は Rust `no_std`**（2026-09-10 オーナー決定）。依存クレートゼロ、`alloc` 不使用、arena はポートから注入。design-notes §4 の C11 推奨を上書きする。ABI・WIT・フェーズ構成はこの変更の影響を受けない。
12. **フルスクラッチ実装**。wasm3 / WAMR / wasmi のコードは取り込まない（アルゴリズムを参考にするのは可、コード流用は不可）。コアは Claude Code が書く（2026-09-10 オーナー決定）。

---

## 3. 未決事項と、Claude Code が採用してよいデフォルト

オーナーが未回答の項目。**下記デフォルトで進めてよい**が、着手時に「このデフォルトで進めます」と明示し、後から差し替え可能な構造にすること。

| # | 項目 | デフォルト | 根拠 |
|---|---|---|---|
| 1 | ランタイム実装言語 | **決定済み → §2-11（Rust `no_std`）** | オーナー判断（2026-09-10）「組込みを安全に書けるから」。軽量化の工夫は Phase 2 の条件に入れた |
| 2 | ボード間の GPIO 番号差の吸収 | **(b) `wasmicon:hal/board@0.1.0` を追加**: `pin-by-role: func(role: string) -> result<u32, error-code>`。役割名は `"lcd-cs"`, `"lcd-dc"`, `"lcd-rst"` など | 同一バイナリで検証するため必須。abi-spec §8 |
| 3 | `sleep-ms` 中の挙動 | ポート層の HAL に委ねる。host: `std::thread::sleep`、rp2040 / esp32s3: 各 HAL クレートの `Delay`（#8 に依存） | 自然な選択 |
| 4 | トラップ後の挙動 | ログ出力して停止（無限ループ / abort）。再起動しない | デバッグしやすい |
| 5 | `log` の UTF-8 検証 | しない | 仕様通り |
| 6 | `spi.transfer` | v0.1 に残す | 実装コストが低い |
| 7 | コアの実装分担 | **決定済み → §2-12（フルスクラッチ、Claude Code が書く）** | オーナー判断（2026-09-10） |
| 8 | **ポート層の実装方式**（Rust 化に伴い新規） | **全て Rust。host = `std`、rp2040 = `rp-hal` + `cortex-m-rt`、esp32s3 = `esp-hal`（`no_std`）。ESP-IDF / Pico SDK は使わない** | コアが Rust である以上 Xtensa のフォークツールチェーンは必須で、C SDK を混ぜても軽くならない。単一言語・単一ツールチェーンの方が軽く安全。**Phase 4 着手前にオーナー確認**（HANDOFF §8） |
| 9 | **Cargo workspace の分割**（Rust 化に伴い新規） | **承認済み（2026-09-10）**。ターゲットごとに別 workspace。root = `runtime` + `tools/wasmicon-gen` + `ports/host`。guest workspace の root は `bindings/rust`（Phase 3 で `apps/*-rs` を members に足す）。`ports/rp2040` / `ports/esp32s3` は Phase 4 で作る。`.cargo/config.toml` は **カレントディレクトリ基準**で探索されるので、`apps/*-rs` には `apps/.cargo/config.toml` が別途要る | 単一 workspace ではターゲット・profile・toolchain が衝突する |

デフォルト 2 を採用した場合、`wit/board.wit` を追加し `world app` に `import board;` を足し、`abi-spec.md` §7 と §10 を更新すること。

デフォルト 8 を却下する（C SDK を使う）場合、Phase 1 の出力に C ヘッダ (`runtime/include/wasmicon/hal.h`) を戻し、コアを `staticlib` として `extern "C"` API を生やす必要がある。

---

## 4. リポジトリ構成（提案）

```
wasmicon/
├── CLAUDE.md                  # Claude Code 向けの短い規約（同梱）
├── HANDOFF.md                 # 本書
├── Cargo.toml                 # workspace: runtime, tools/wasmicon-gen, ports/host
├── rust-toolchain.toml        # stable
├── docs/
│   ├── design-notes.md
│   └── abi-spec.md
├── wit/                       # 唯一の真実
│   ├── types.wit gpio.wit i2c.wit spi.wit time.wit log.wit world.wit
├── tools/
│   ├── wit2sig.py             # 種。ジェネレータの回帰テストの基準として残す
│   └── wasmicon-gen/          # Rust (wit-parser) 製ジェネレータ
├── runtime/                   # wasmicon-core: no_std, 依存クレートゼロ, alloc 不使用
│   ├── src/                   # lib.rs decode.rs validate.rs interp.rs module.rs generated.rs(生成)
│   └── tests/                 # spec テストランナー（dev-dependencies は可）
├── ports/
│   ├── host/                  # PC 用。std。mock HAL + trace。CI はここで回す
│   ├── esp32s3/               # 別 workspace。esp-hal、toolchain = esp (espup)
│   └── rp2040/                # 別 workspace。rp-hal + cortex-m-rt、thumbv6m-none-eabi
├── bindings/
│   ├── rust/                  # wasmicon-hal (no_std)。guest workspace の root。生成物 + 手書きラッパ
│   └── assemblyscript/        # @wasmicon/hal パッケージ
├── apps/                      # *-rs は bindings/rust と同じ guest workspace (wasm32-unknown-unknown)
│   ├── blink-rs/ blink-as/
│   └── sensor-display-rs/ sensor-display-as/
└── verify/                    # トレース diff スクリプト、記録済み I2C 応答
```

`ports/rp2040`、`ports/esp32s3`、guest（`bindings/rust` + `apps/*-rs`）は root workspace に入れず、`Cargo.toml` の `exclude` に列挙する（§3 #9）。

---

## 5. フェーズと完了条件

順番は守ること。各フェーズの完了条件を満たしてから次へ進む。

### Phase 1: ジェネレータ `wasmicon-gen`

- Rust + `wit-parser` クレートで実装。`tools/wit2sig.py` と同じ規則を Rust に移植し、まず `wit2sig.py` の出力と一致することをテストにする。
- 出力: (a) `runtime/src/generated.rs` — import 表（module, name, sig 文字列, ホスト関数のスロット）とエラーコード enum、(b) `bindings/rust/src/generated.rs` — `#[link(wasm_import_module=...)]` extern 宣言と enum、(c) `bindings/assemblyscript/assembly/generated.ts` — `@external` 宣言と enum。
- サブセット外の WIT 構文はエラーで拒否する。
- `wit2sig.py` との突き合わせは `tools/check-sigs.sh`（`wasmicon-gen --sigs` の出力と diff）。abi-spec §7 の表そのものは `tools/wasmicon-gen/tests/abi_spec.rs` にハードコードして検査する（§7 が正なので、期待値は WIT からではなく仕様書から取る）。
- 生成した Rust は rustfmt に通してから書き出す。そうしないと `cargo fmt --check` と diff ゼロ検査が両立しない。
- **完了条件**: 3 出力が abi-spec §7 と一致。CI で WIT 変更時に再生成して diff がゼロであることを検査。→ **達成（2026-09-10）**。`.github/workflows/ci.yml` の root ジョブが `cargo test`（`tests/generated_up_to_date.rs`）と `--check` で検査する。
- `flags` は abi-spec §2.1 が許可しているので生成に対応しているが、v0.1 の `wit/` には無いため実際の出力では使われていない（テストのみ）。
- 残: AssemblyScript の出力は `asc` でのコンパイル検証をしていない（Phase 3 で AS ツールチェーンを入れたときに行う）。

### Phase 2: ランタイムコア（ホスト PC 上）

- Rust `no_std`、**依存クレートゼロ**、`alloc` 不使用。arena（`&'a mut [u8]`）をポートから受け取り、内部構造は生ポインタではなく arena 内のインデックス/オフセットで持つ。
- lint: CI で `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`。`unsafe` は最小限に閉じ込め、必ず直前に `// SAFETY:` を書く。
- 軽量化（オーナー要求）: root workspace の `[profile.release]` は `opt-level = "z"`, `lto = "fat"`, `codegen-units = 1`（サイズ計測用）。`panic = "abort"` と `strip = true` はここに置かず、guest / ポートの workspace 側の `[profile.release]` に置く（ホストツールと `cargo test` は unwind が要る）。コアで `core::fmt` を使わない。ジェネリクスの単相化でコードが膨らまないよう型は具体型で書く。サイズは Phase 2 完了時点で計測して記録する。
- 構造: `decode`（セクション解析、コードはフラッシュ上のスライス `&'static [u8]` を保持）→ `validate`（型検査 + br のジャンプ先 side table 構築）→ `interp`（スタックマシン、`match` ディスパッチ）。
- **Rust に computed goto は無い**。`match` ループから始める。明示的テールコール（`become`）は unstable なので当てにしない。最適化は spec テスト通過後にプロファイルを取ってから。
- Wasm の算術は wrapping。`wrapping_*` / `rotate_*` / `checked_*` を明示的に使う。debug の `overflow-checks` は on のままにし、引っかかった箇所は仕様どおりの wrapping に直す。
- `memory.grow` はポートが指定する最大ページ数まで。
- import 解決: 生成された `generated.rs` の表を module 名 + name + sig で完全一致リンク。
- spec テストランナーは `runtime/tests/` に置く。ここは dev-dependencies を使ってよい（`wast` クレートで `.wast` を直接読む、または `wasm-tools json-from-wast` を使う）。コア本体の依存ゼロは崩さない。
- **完了条件**: WebAssembly spec testsuite（対応機能セット分）を全通過。`ports/host` で `apps/blink-rs` の Wasm が動き、mock GPIO のトレースが出る。→ **達成（2026-09-10）**。
  - 「全通過」の定義は `runtime/tests/spec.rs` の `FILES`（コア 74 ファイル）。除外は同ファイルの `EXCLUDED` に理由つきで列挙した。22507 コマンドが通る
  - `apps/blink-rs` は Phase 3 で作るので、同じ host call 列を出す手書きの `ports/host/tests/blink.wat` で先に検証している。Phase 3 で本物に差し替える
- 最適化は spec テスト通過後、プロファイルを取ってから。RP2040 で ILI9341 のテキスト描画が目視で 1 秒以内に終わる程度を目標。

### Phase 3: Rust / AS バインディング + blink

- Rust: `no_std` クレート、`wasm32-unknown-unknown`。生成 extern を `Pin` / `I2cBus` / `SpiBus` の安全ラッパ（`Drop` で `[resource-drop]`）で包む。ビルドフラグ: `-C target-feature=-reference-types,+sign-ext,+nontrapping-fptoint,+bulk-memory,+mutable-globals,+multivalue`、`--initial-memory=65536`、`-z stack-size=8192`、`panic=abort`、`opt-level=z`。
- AS: `--runtime stub`、`--initialMemory 1`、`--disable simd,threads,exception-handling`、`--enable sign-extension,nontrapping-f2i,bulk-memory,mutable-globals`。`env.abort` は AS 用に小さな shim をホストへ用意（トラップに変換）。
- **完了条件**: `blink-rs` と `blink-as` の Wasm を `wasm-tools validate --features=...` で対応機能セット内であることを確認し、`ports/host` で同じ GPIO トレースを出す。

### Phase 4: ポート `rp2040` → `esp32s3`

- **Pico を先に**やる（制約が厳しい方から）。
- ポート層の実装方式は §3 #8（全 Rust）。着手前にオーナー確認。
- Pico: `rp-hal` + `cortex-m-rt`、`thumbv6m-none-eabi`、`memory` 上限 2 ページ。Wasm バイナリは `include_bytes!` で `.rodata`（flash, XIP）に置き `&'static [u8]` としてランタイムに渡す。
- ESP32-S3: `esp-hal`（`no_std`）、espup が入れる Xtensa ツールチェーン（`rust-toolchain.toml` の `channel = "esp"`）、上限 4 ページ（PSRAM なし）。
- トレースは cargo feature `trace` を有効にしたビルドで abi-spec §9 形式をシリアル（UART / USB-CDC）に出す。
- ボード設定（abi-spec §8 の表）は `ports/<board>/src/board.rs` に集約し、`board.pin-by-role` を実装。
- **完了条件**: 両ボードで `blink-rs` / `blink-as` が動き、シリアルのトレースが host 版と一致。

### Phase 5: センサー + ディスプレイアプリ

- SHT31: 0x44、単発計測コマンド `0x2400`（高精度・クロックストレッチなし）、15 ms 待ち、6 バイト読み（T MSB, T LSB, CRC, RH MSB, RH LSB, CRC）。CRC-8（poly 0x31, init 0xFF）をゲストで検証。温度 = -45 + 175·raw/65535、湿度 = 100·raw/65535。**表示は固定小数（×100 の整数）で計算し、浮動小数点を使うのはあえて 1 箇所（f32 変換）に限定**して決定性検証の題材にする。
- ILI9341: 初期化シーケンスは一般的なもの（SWRESET, SLPOUT, PIXFMT=0x55 (RGB565), MADCTL, DISPON）。描画は「矩形塗り」と「8×8 ビットマップフォントでの文字列描画」の 2 プリミティブのみ。行単位（最大 320×8×2 = 5 KB）のバッファを `spi.write` で送る。全画面フレームバッファは持たない（RP2040 に載らない）。
- Rust 版と AS 版は**同じ描画結果**になるよう、フォントと座標を共通仕様にする（`apps/README.md` に書く）。
- **完了条件**: 4 通り（Rust/AS × ESP32-S3/Pico）で表示が出る。

### Phase 6: クロスボード検証

- `verify/` に、(1) 記録済み SHT31 応答を返す mock I2C モード（ポート層の `WASMICON_I2C_REPLAY`）、(2) 2 ボードのトレースを diff するスクリプト、(3) `ports/host` + wasmtime での差分テスト（インタプリタの正しさ）。
- **完了条件**: 同一 `.wasm`（Rust 版、AS 版それぞれ）を両ボードで走らせ、`time` を除くトレースと SPI ピクセル CRC が完全一致。結果を `docs/verification-report.md` にまとめる。

---

## 6. 落とし穴・注意点

### ゲスト側

- **Rust の wasm32 デフォルト機能**: 最近の rustc は `reference-types` / `multivalue` を有効にする。`call_indirect` のテーブルインデックスは LEB128 で読み、0 以外は拒否する実装にしておく（abi-spec §6.1 注）。
- **AS は `env.abort` を import する**: `--noAssert` にしても残ることがある。ホストに `env.abort(msg, file, line, col)` を用意しトラップに変換する。これは `world app` には無い例外的 import として明記する。

### ランタイム側（Rust）

- **`no_std` に無い浮動小数メソッド**: `floor` / `ceil` / `trunc` / `round` / `sqrt` などは `core` に無い（`std` か `libm`）。依存クレートゼロを守るなら自前実装する。`f32.sqrt` は Wasm の必須命令なので避けて通れない。`f32.nearest` は偶数丸め、`min`/`max` の NaN 伝播、`copysign`、`trunc_sat` も仕様どおりに自前で書く。spec テストの `float_misc.wast`, `conversions.wast` が鬼門。
- **RP2040 は FPU 無し・ハード除算無し**: f32/f64 は `compiler_builtins` のソフトフロートになる。IEEE 準拠は保たれるが遅い。
- **ESP32-S3 の FPU は f32 のみ**: f64 はソフト。Xtensa の FPU は非正規化数を扱えない設定があるので、spec テストの denormal ケースを ESP32-S3 実機でも回して確認する。
- **XIP 上のバイトコードを直接読む**: RP2040 (Cortex-M0+) は非アライメントのワードアクセスで HardFault する。スライスからの読み出しは必ず `u32::from_le_bytes(bytes[i..i+4].try_into().unwrap())` 形式にし、`ptr::read` / `align_to` / 参照のキャストを使わない（HardFault かつ UB）。LEB128 はバイト単位で読む。
- **`memory` の範囲検査**: `list<u8>` 引数と out ポインタは host call のたびに範囲検査する。`checked_add` と `slice::get` を使い、`as` キャストでの暗黙の切り詰めを作らない。
- **`panic = "abort"` でもパニックメッセージは残る**: `panic!` とスライスの暗黙の境界チェックが文字列をバイナリに残す。コアは `Result` を返す設計にし、ホットパスは `get()` で明示的に扱う。`panic_immediate_abort` は build-std が要るので当てにしない。
- **`core::fmt` はサイズの主犯**: コアでは `write!` / `{:?}` / `Display` を使わない。トレース出力はポート層で、バイト列を直接書く形にする。

### ツールチェーン

- **Xtensa は upstream Rust 非対応**: ESP32-S3 は espup で入れるフォーク（`channel = "esp"`）が要る。CI で ESP32-S3 をビルドするなら espup のインストール手順を含める。RP2040 は `rustup target add thumbv6m-none-eabi` だけで済む。

### 共通

- **ハンドル 0 は無効**（abi-spec §5.1）。テーブルのインデックス 0 を使わない。
- **エラーコード = discriminant + 1** を忘れない。生成された定数だけを使い、手書きの数値を書かない。
- **決定性**: `time` 以外の host call の順序・引数・結果が両ボードで一致しなければならない。ポート層で「たまたま成功する」挙動（例: 未初期化 GPIO の read）を作らない。

---

## 7. コマンド早見

```bash
# WIT 検証
wasm-tools component wit wit/
# 生成物の更新と検査（Phase 1）
cargo run -p wasmicon-gen                # 3 ファイルを再生成
cargo run -p wasmicon-gen -- --check     # 生成物が wit/ と一致するか
sh tools/check-sigs.sh                   # 参照実装 tools/wit2sig.py と突き合わせ
# Rust の常用チェック
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
# spec テスト変換（Phase 2）
wasm-tools json-from-wast <file>.wast -o out/<file>.json   # 旧 wast2json 相当
# ゲスト Wasm の機能検査（Phase 3）
wasm-tools validate --features=mvp,sign-extension,saturating-float-to-int,bulk-memory,multi-value,mutable-global app.wasm
# ターゲット準備（Phase 4）
rustup target add thumbv6m-none-eabi   # RP2040
espup install                          # ESP32-S3 (Xtensa フォーク)
```

---

## 8. オーナー（hota）への確認が必要なタイミング

- ~~Phase 1 着手前: §3 のデフォルト #1（実装言語）と #7（分担）の承認~~ → **2026-09-10 に回答済み。§2-11 / §2-12**
- ~~Phase 1 着手前: §3 #9（Cargo workspace の分割）~~ → **2026-09-10 に承認済み**
- Phase 4 着手前: §3 #8（ポート層を全 Rust にする）の承認、実機の配線（abi-spec §8 の表）とシリアルの接続方法
- Phase 5: 手元にある SHT31 / ILI9341 モジュールの型番（ILI9341 は 3.3V ロジックの SPI 版、SHT31 は I2C アドレス 0x44 前提）

---

## 9. 参照

- Component Model Canonical ABI（import 命名と flat 型の出典）: https://github.com/WebAssembly/component-model/blob/main/design/mvp/CanonicalABI.md
- wit-parser: https://crates.io/crates/wit-parser
- WebAssembly spec testsuite: https://github.com/WebAssembly/testsuite
- 先行事例: Wasefire (Google), WAMR, wasm3, wasmi（design-notes §1）
