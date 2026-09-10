# Wasmicon 設計メモ

作成日: 2026-09-10（同日、ABI 仕様書 v0.1 作成に伴い §8 を更新）
状態: 初期構想。コンセプトの整理と、実装前に決めるべき判断事項をまとめたもの。

## 1. コンセプト

- 従来の MCU 開発環境はハードウェアごとに異なり、管理が面倒
- WebAssembly は多言語からコンパイル可能で、高速かつ安全な命令セット。MCU でも実行可能
- しかし WebAssembly には MCU 周辺機器へアクセスする標準 API が無い
  - Bytecode Alliance embedded WG の wasi-i2c / wasi-spi / wasi-gpio 提案は Phase 1 のまま更新が止まっている
  - それらは Component Model 前提。Canonical ABI は cabi_realloc や resource テーブルを要求し、RP2040 クラスには重い
- よって Core Wasm ランタイムから直接周辺機器にアクセスできる軽量な API 層が必要

### 構成要素

- **Wasmicon Runtime**: 自作の Core Wasm インタプリタ。ESP32-S3 / RP2040 で動作する軽量さ
- **Wasmicon HAL**: WIT で GPIO / SPI / I2C（＋ time / log）を定義。Component Model は使わず、WIT は IDL としてのみ使用。限られた型とシンタックスのサブセットで定義
- **Wasmicon Bindings**: Rust、AssemblyScript

### 目標

I2C 接続の温湿度センサーから値を読み、SPI 接続の ILI9341 に描画するアプリを Rust と AssemblyScript で実装。ESP32-S3 と Raspberry Pi Pico WH で実行し、同じ結果が出るかを検証する。

### 比較対象・先行事例

- **Wasefire (Google)**: Core Wasm の import として applet API を定義。Component Model 不使用という同じ判断
- **WAMR**: ESP-IDF 対応あり。既存ランタイムの代表
- **wasm3**: 軽量インタプリタ。クロージャコンパイル方式（メンテ停滞気味）
- **wasmi**: Rust 製、no_std 可。レジスタマシン変換方式

Wasmicon の差別化: WIT を IDL に使いつつ独自の軽量 ABI を定義 / ランタイム自作 / Rust と AS の両バインディング / 複数 MCU での決定性検証、の組み合わせ。

## 2. Wasmicon ABI（最初に決めるべきこと）

WIT の各型を Core Wasm の関数シグネチャへどう落とすかを明文化する。曖昧だと Rust バインディング、AS バインディング、ランタイムのホスト関数テーブルがずれてバグの温床になる。

→ 具体化した仕様は `abi-spec.md` を参照。以下は方針の要約。

### 使用する WIT の型（サブセット）

- bool, u8, u16, u32, s32, u64, f32, f64
- enum, flags
- list<u8>
- resource（handle として扱う）

### 落とし込み規則

| WIT | Core Wasm |
|---|---|
| bool, u8, u16, u32, s32, enum, flags | i32 |
| u64 | i64 |
| f32 / f64 | f32 / f64 |
| list<u8> | (ptr: i32, len: i32) の 2 引数。Canonical ABI と同じ |
| resource | u32 handle（i32）。[method] 関数は import 名 `[method]bus.write` のように機械的に生成 |
| result<T, error-code> | 単一の i32 エラーコードを返し、T は out ポインタ経由。Canonical ABI（retptr 経由）からの意図的な逸脱。逸脱はドキュメントに明記 |

設計方針: 可能な限り Canonical ABI のサブセットに留め、将来 Component Model へ橋渡しする際に機械的に変換できるようにする。逸脱する箇所は明示する。

### ジェネレータ

WIT を唯一の真実（single source of truth）とし、`wit-parser` クレート（wit-bindgen 内の WIT パーサ、単体利用可）で小さなジェネレータを作り、以下を一括生成する。

- ランタイム用の import 表（Rust。`runtime/src/generated.rs`）
- Rust の `extern "C"` 宣言（`#[link(wasm_import_module = "wasmicon:hal/i2c@0.1.0")]`）
- AssemblyScript の `@external` 宣言

## 3. HAL インターフェース設計

必要なインターフェース: gpio, spi, i2c に加えて **time**（sleep-ms, now-us）と **log**（デバッグ出力）。ILI9341 には DC/RST/CS の GPIO と SPI バルク書き込みが必要で、センサーは計測待ちが要るため time は必須。

API 粒度の原則: インタプリタのオーバーヘッドは host call ごとに乗る。SPI 書き込みは「1 ピクセルずつ host call」ではなく「線形メモリ上に組み立てたバッファを `spi.write(list<u8>)` で一括転送」にする。API の粒度がそのまま性能を決める。

フレームバッファ: 320x240x2 = 150KB は RP2040 に載らない。行単位・矩形単位の小バッファで描画し、ゲスト側に小さなフォントと fill-rect 程度のミニ描画ライブラリを持つ。

## 4. ランタイム実装方針

### 実装言語

**決定（2026-09-10、オーナー判断）: Rust `no_std`**。理由は「組込み開発を安全に行えるから」。条件は **なるべく軽量に保つこと**。

- 依存クレートゼロ、`alloc` 不使用、arena はポート層から注入（malloc を使わない方針は変わらない）
- `panic = "abort"` / `opt-level = "z"` / `lto` / `codegen-units = 1`。コアで `core::fmt` を使わない
- ディスパッチは `match` ループから始める（Rust に computed goto は無い）。最適化はプロファイル後

却下した案: **C11、依存ゼロ**。両プラットフォームの公式 SDK がそのまま使え、GCC の computed goto でディスパッチが速いという利点があったが、安全性を優先して見送った。

Rust を選んだコスト（織り込み済み）:
- ESP32-S3（Xtensa）は upstream Rust 非対応。espup 経由のフォークツールチェーンが要る。RP2040 は upstream の `thumbv6m-none-eabi` でそのまま
- computed goto が使えないぶん、ディスパッチの速度は素の `match` から始まる
- `no_std` では浮動小数の一部メソッド（`floor` / `ceil` / `trunc` / `sqrt` など）が `core` に無い。依存ゼロを守るなら自前実装する

### インタプリタ設計

- ロード時に完全な validation（仕様上必須。安全性の根拠）
- validation 時に br のジャンプ先を事前計算した side table を作る「検証済みバイトコード直接実行」型から始める
- wasm3 のクロージャコンパイルや wasmi のレジスタマシン変換は速いが複雑。まず spec テスト全通過を優先し、最適化はプロファイル後
- コードセクションは RAM にコピーせず、フラッシュ（両ボードとも XIP でメモリマップ）上のポインタを直接使う。RAM に置くのは side table、globals、table、線形メモリのみ

### メモリ制約

- RP2040: 264KB SRAM、Cortex-M0+ 133MHz、FPU 無し、ハード除算無し
- ESP32-S3: 512KB SRAM（＋PSRAM オプション）、Xtensa LX7 240MHz、f32 のみハード FPU
- 線形メモリの 64KB ページは RP2040 では 1〜2 枚が限度
  - Rust: `--initial-memory`、`-z stack-size` を絞る
  - AS: `--initialMemory`、`--runtime stub`

### 対応機能セット

基準: **MVP + sign-extension + non-trapping float-to-int + bulk-memory + multi-value + mutable-globals**

理由: 最近の Rust の wasm32-unknown-unknown ターゲットはこれらをデフォルト有効で出力する。MVP のみだと Rust の出力が動かない（`-C target-cpu=mvp` で抑制可能だが、素直にサポートする方が健全）。

不要: SIMD, threads, exceptions, GC, tail-call

## 5. 決定性の検証方法

RP2040 は FPU 無し、ESP32-S3 は f32 のみハード FPU。Wasm の浮動小数点は IEEE 準拠で決定的でなければならないため、f32.nearest、min/max の NaN 扱い、trunc_sat、i64 演算の 32bit 実装などがまさに差の出やすい検証対象。

### トレース比較

LCD の見た目ではなく、ホスト呼び出しのトレースを比較する。

- HAL ポート層に **trace モード** を入れ、I2C/SPI の全トランザクション（送信バイト列、受信バイト列。タイムスタンプは除く）をシリアルに出力
- SPI に流したピクセルデータの CRC を含めて両ボードで diff
- センサーの生値は物理的に異なるので、検証時は I2C を固定値応答（または記録したトランザクションの再生）に差し替え、モジュールバイナリ・入力・出力の完全一致を機械的に確認

### ホスト PC ポート

mock HAL 付きのホストポートを作り、wasmtime を参照実装として差分テスト。CI も回せる。

## 6. ロードマップ

1. ABI 仕様と WIT を書く（**完了**: `abi-spec.md`, `wit/`）
2. ランタイムをホスト PC 上で spec テスト（wast2json で変換）全通過まで仕上げる
3. Pico へ先に移植（制約が厳しい方を先にやると設計が引き締まる）
4. ESP32-S3 へ移植
5. ジェネレータとバインディング（Rust / AS）
6. アプリ: Lチカ → I2C センサー → SPI ディスプレイ
7. クロスボード検証（trace 比較）

※ 実装フェーズの詳細な順序と完了条件は `HANDOFF.md` §5 が正（ジェネレータを最初に置く構成に組み替えた）。

## 7. 部品の選定

- 温湿度センサー推奨: **SHT31 / SHT30**。初期化シーケンス不要、コマンド 1 つで 6 バイト読める、CRC-8 付きで決定性検証の題材として素直。両ボード 3.3V で使える
- ディスプレイ: ILI9341（SPI）

## 8. 決定済み・未決事項

決定済み（2026-09-10）:
- resource は WIT の `resource` 構文で書く。import 名は Canonical ABI 準拠
- エラーコードは HAL 共通の `types.error-code` 1 つ
- **ランタイム実装言語は Rust `no_std`**（オーナー決定。§4 参照）
- **フルスクラッチ実装、コアは Claude Code が書く**（オーナー決定）

未決（`HANDOFF.md` §3 にデフォルト案あり）:
- ボード間の GPIO 番号差の吸収方法（`abi-spec.md` §8）
- ポート層の実装方式（全 Rust か、C SDK + FFI か）
