# Wasmicon ABI 仕様書 v0.1 (draft)

作成日: 2026-09-10
対象: `wasmicon:hal@0.1.0`（`wit/` 配下の WIT 定義）
関連: `wasmicon/design-notes.md`

本書は、WIT で記述された Wasmicon HAL を **Core WebAssembly モジュールの import / export** にどう落とし込むかを定める。Component Model は使わない。WIT は IDL としてのみ用い、ここで定める規則によって Rust バインディング、AssemblyScript バインディング、ランタイムのホスト関数テーブルを機械的に生成する。

用語: 「ゲスト」= Wasm モジュール、「ホスト」= Wasmicon Runtime とそのポート層。

---

## 1. 設計原則

1. **WIT が唯一の真実。** 手書きで 3 箇所を同期しない。生成物は WIT から導出される。
2. **Canonical ABI のサブセットに留める。** 可能な限り Component Model の Canonical ABI と同じ lowering を使い、将来コンポーネントへ橋渡しできるようにする。逸脱する箇所は §4 で明示する。
3. **ゲスト側にアロケータを要求しない。** `cabi_realloc` は不要。ホストがゲストメモリを確保することはない。可変長データは常に呼び出し側（ゲスト）が用意したバッファに書く。
4. **host call は粗い粒度で。** インタプリタのオーバーヘッドは呼び出しごとに乗るため、バルク転送 API を基本とする。
5. **決定性。** 同一モジュール・同一入力に対し、どのプラットフォームでも同一の host call 列と同一の結果を出す。

---

## 2. 使用する WIT のサブセット

Wasmicon の WIT は以下の構文・型のみを使う。ジェネレータはこれ以外を検出したらエラーにする。

### 2.1 使ってよい型

| 分類 | 型 |
|---|---|
| スカラー | `bool`, `u8`, `u16`, `u32`, `s32`, `u64`, `s64`, `f32`, `f64` |
| 列挙 | `enum`（ケース数 ≤ 256） |
| ビット集合 | `flags`（ケース数 ≤ 32） |
| バイト列 | `list<u8>` |
| 文字列 | `string`（**引数位置のみ**） |
| ハンドル | `resource`（メソッド、static 関数、暗黙の drop） |
| 結果 | `result<T, error-code>`, `result<_, error-code>`（`T` は上記のスカラー・enum・resource ハンドル・`list<u8>` のいずれか） |

### 2.2 使わない型・構文

`record`, `variant`, `option`, `tuple`, `list<T>`（`u8` 以外）, `string` の戻り値, `own<T>` / `borrow<T>` の明示、`constructor`（`result` を返せないため static `open` を使う）、`char`, `s8`, `s16`, ネストした `result`。

これらは v0.1 では不要と判断した。必要になった時点で lowering 規則を追加する。

### 2.3 構文上の制約

- 1 パッケージ `wasmicon:hal@X.Y.Z` に全インターフェースを置く。
- `world app` がゲストの import/export 全体を記述する。
- 共通型は `interface types` に置き、各インターフェースから `use types.{...}` する。
- ドキュメントコメント（`///`）は生成物のコメントにそのまま転記される。

---

## 3. 名前の規則

### 3.1 import のモジュール名

WIT のインターフェースごとに 1 つの Core Wasm import モジュール名を使う。形式は Canonical ABI と同じ:

```
<namespace>:<package>/<interface>@<version>
```

例: `wasmicon:hal/gpio@0.1.0`, `wasmicon:hal/i2c@0.1.0`

バージョンは **常に含める**。ランタイムは完全一致で解決し、未知のモジュール名・関数名を import するモジュールのインスタンス化を拒否する（リンクエラー）。

### 3.2 import のフィールド名

| WIT | Core Wasm import 名 |
|---|---|
| 自由関数 `foo: func(...)` | `foo` |
| resource `r` の static 関数 `open` | `[static]r.open` |
| resource `r` のメソッド `write` | `[method]r.write` |
| resource `r` の drop（暗黙） | `[resource-drop]r` |

これも Canonical ABI と同一の命名である。

### 3.3 export

`world app` の export はゲストモジュールがそのままの名前で export する。v0.1 では:

| export 名 | 型 | 説明 |
|---|---|---|
| `run` | `func() -> ()` | エントリポイント。ホストがインスタンス化直後に 1 回呼ぶ |
| `memory` | `memory` | 線形メモリ。**必須**。ホストは `list<u8>` / `string` / out ポインタの読み書きにこれを使う |

`_start`, `_initialize`, `cabi_realloc` は使わない（export されていても無視する）。

---

## 4. 型の lowering 規則

### 4.1 スカラー・enum・flags（引数）

| WIT | Core Wasm | 値の表現 |
|---|---|---|
| `bool` | `i32` | 0 = false, それ以外 = true（ホスト→ゲストは常に 0/1） |
| `u8`, `u16`, `u32` | `i32` | ゼロ拡張。`u8`/`u16` の範囲外の上位ビットはホストが**無視**する（マスクする） |
| `s32` | `i32` | そのまま |
| `u64`, `s64` | `i64` | そのまま |
| `f32`, `f64` | `f32`, `f64` | そのまま |
| `enum` | `i32` | ケースの宣言順の discriminant（0 始まり）。範囲外はホストが `invalid-argument` を返す |
| `flags` | `i32` | 宣言順にビット 0 から割り当て。未定義ビットが立っていたら `invalid-argument` |
| resource ハンドル | `i32` | §5 参照 |

Canonical ABI と同一。

### 4.2 `list<u8>` と `string`（引数）

`(ptr: i32, len: i32)` の **2 つの i32** に展開する。`ptr` はゲストの `memory` 内のバイトオフセット。Canonical ABI と同一。

- `string` は UTF-8 のバイト列。ホストは UTF-8 の妥当性を検証**しない**（ログ出力にしか使わないため）。
- ホストは `[ptr, ptr+len)` が `memory` の範囲内であることを検証し、範囲外なら **トラップ**する（エラーコードではない。これはゲストのバグであり、Wasm のメモリ安全性の一部として扱う）。
- `len = 0` は許容し、ホストは `ptr` を参照しない。ただし I2C/SPI の転送長 0 は `invalid-argument` を返す。
- ホストは引数バッファを**読むだけ**で書き換えない。
- ホストは host call から戻った後、`ptr` を保持しない。

### 4.3 戻り値（`result` を含まない関数）

| WIT | Core Wasm |
|---|---|
| 戻り値なし | 結果なし |
| スカラー / enum / flags | 対応する 1 つの値（§4.1 と同じ表現） |

例: `now-us: func() -> u64` → `(func (result i64))`

### 4.4 `result<T, error-code>`（**Canonical ABI からの逸脱**）

> Canonical ABI では `result<T, E>` は flat 化されて複数値になり、2 値以上は retptr 経由になる。MCU での扱いやすさのため Wasmicon は以下に統一する。

関数は **常に単一の `i32` を返し**、これをステータスコードとする。

- `0` = 成功 (`Ok`)
- `n > 0` = エラー。`n - 1` が `error-code` の discriminant（例: `timeout` は宣言順 3 番目 → `4`）

`T` が存在する場合（`result<_, ...>` でない場合）、`T` は **引数リストの末尾に追加される out ポインタ** 経由でゲストに渡す。

| `T` | 追加される引数 | ホストの書き込み |
|---|---|---|
| `bool`, `u8`, `u16`, `u32`, `s32`, `enum`, `flags` | `out: i32` | `out` に **u32 (4 バイト, little-endian)** で書く。`bool` は 0/1、`u8`/`u16` もゼロ拡張して 4 バイト |
| `u64`, `s64` | `out: i32` | 8 バイト little-endian |
| `f32` / `f64` | `out: i32` | IEEE 754 4 / 8 バイト little-endian |
| resource ハンドル | `out: i32` | u32 ハンドル 4 バイト |
| `list<u8>` | `buf: i32, cap: i32, len-out: i32` | `[buf, buf+cap)` に最大 `cap` バイト書き、実際に書いたバイト数を `len-out` に u32 で書く |

規則:
- 成功時のみ out に書く。エラー時に out の内容は未定義（ホストは書かなくてよい）。
- out ポインタと `[buf, buf+cap)` は `memory` 範囲内でなければならず、範囲外はトラップ。アライメントは要求しない（ホストはバイト単位で書く）。
- `list<u8>` を返す関数で要求長（`len` 引数など）が `cap` を超える場合、ホストは `invalid-argument` を返す。ゲストは常に十分な `cap` を渡す責任がある。
- v0.1 の `i2c.read` / `i2c.write-read` / `spi.transfer` では、成功時 `len-out` は常に要求長に等しい。将来の可変長 API（UART 受信など）のために `len-out` を設けている。

### 4.5 逸脱の要約

| 項目 | Canonical ABI | Wasmicon ABI |
|---|---|---|
| `result<T, E>` | flat 化、複数値は retptr | 単一 `i32` ステータス + 末尾 out ポインタ |
| `list<u8>` 戻り値 | ホストが `cabi_realloc` で確保して (ptr,len) を返す | 呼び出し側バッファ `(buf, cap, len-out)` |
| `string` 戻り値 | 同上 | 使用しない |
| `cabi_realloc` | 必須（可変長戻り値がある場合） | 不要 |
| その他（スカラー、enum、flags、list 引数、ハンドル、import 名） | — | **同一** |

将来 Component Model へ橋渡しする場合、`result` と `list<u8>` 戻り値のアダプタを書くだけで済む設計にしてある。

---

## 5. resource とハンドル

### 5.1 表現

resource ハンドルは `i32`（符号なし u32 として解釈）。値の意味はホスト実装依存だが、以下を保証する。

- `0` は**無効ハンドル**として予約する。ホストは有効なハンドルとして `0` を返さない。ゲストは `0` を「未取得」の番兵として使える。
- ハンドルの名前空間は **resource 型ごとに独立**。`gpio.pin` の `1` と `i2c.bus` の `1` は無関係。
- ホストは型ごとにハンドルテーブルを持ち、不一致・範囲外・drop 済みのハンドルに対して `invalid-handle` を返す（トラップしない）。

### 5.2 ライフサイクル

- static `open` が成功すると新しいハンドルを返す。ゲストが所有する。
- `[resource-drop]r` `(func (param i32))` を呼ぶとホストはペリフェラルを解放する（GPIO は入力・プル無しに戻す、バスは無効化する）。無効ハンドルの drop は**無視**する（エラーもトラップもしない）。
- `run` から戻ったとき、ホストは残っている全ハンドルを drop する。
- 同じペリフェラル・ピンを二重に `open` すると `busy`。

### 5.3 ハンドル上限

ホストは型ごとに少なくとも次の同時ハンドル数を保証する: `gpio.pin` 16、`i2c.bus` 2、`spi.bus` 2。超えると `out-of-memory`。

---

## 6. ゲストモジュールの要件

### 6.1 対応する Wasm 機能セット

ランタイムは以下を実装し、これ以外の命令・セクションを含むモジュールを検証時に拒否する。

MVP + `sign-extension` + `nontrapping-float-to-int` + `bulk-memory`（`memory.copy`, `memory.fill`, `data.drop`; `table.*` 命令は除く）+ `multi-value` + `mutable-globals`。

対象外: SIMD, threads/atomics, exception-handling, tail-call, GC, reference-types の拡張命令（`ref.null`, `ref.func`, `table.get/set/grow/size/fill/copy/init`）、multi-memory, memory64。

> 注: Rust の `wasm32-unknown-unknown` は近年 `reference-types` と `multivalue` をデフォルト有効にしている。`reference-types` が有効だと `call_indirect` のテーブルインデックスが LEB128 でエンコードされる（テーブルが 1 つなら値は 0 で従来と同じバイト列）。ランタイムは `call_indirect` のテーブルインデックスを LEB128 で読み、0 以外を拒否する。Rust 側では `-C target-feature=-reference-types` または `-C target-cpu=mvp` に加えて必要機能を個別に `+` する運用を推奨する（バインディング側の build 手順で固定する）。

### 6.2 メモリ

- `memory` を 1 つ定義し `"memory"` として export する。import memory は不可。
- 最小ページ数はプラットフォームの上限以下でなければならない。上限は **ポートが決める**。参考値: RP2040 = 2 ページ (128 KiB)、ESP32-S3 (PSRAM なし) = 4 ページ (256 KiB)。
- `memory.grow` は上限までは成功し、超えると `-1` を返す（トラップしない）。
- データセグメントは受動・能動とも可。

### 6.3 テーブル

- `funcref` テーブル 0 個または 1 個。要素数上限はポートが決める（参考: 256）。

### 6.4 import

- `world app` に列挙されたインターフェース由来の import のみ。未知の import はリンクエラー。
- 全ての HAL 関数を import する必要はない（使う分だけでよい）。
- 各 import の型シグネチャは §7 の表と**完全一致**しなければならない。不一致はリンクエラー。

### 6.5 start 関数

`start` セクションは許可する。ホストは `start` → `run` の順に呼ぶ。

### 6.6 トラップ

ゲストがトラップした（範囲外アクセス、`unreachable`、整数ゼロ除算、スタックオーバーフローなど）場合、ホストはインスタンスを破棄し、全ハンドルを drop し、ログにトラップ理由を出力する。その後の挙動（再起動・停止）はポートが決める。

---

## 7. v0.1 import 一覧（正規表）

ジェネレータの出力はこの表と一致しなければならない。`ec` は §4.4 のステータス i32。

### `wasmicon:hal/gpio@0.1.0`

| import 名 | Core Wasm 型 | 備考 |
|---|---|---|
| `[static]pin.open` | `(param i32 i32 i32) (result i32)` | `(index, mode, out-handle) -> ec` |
| `[method]pin.set-mode` | `(param i32 i32) (result i32)` | `(self, mode) -> ec` |
| `[method]pin.read` | `(param i32 i32) (result i32)` | `(self, out-level) -> ec` |
| `[method]pin.write` | `(param i32 i32) (result i32)` | `(self, level) -> ec` |
| `[method]pin.toggle` | `(param i32) (result i32)` | `(self) -> ec` |
| `[resource-drop]pin` | `(param i32)` | `(self)` |

### `wasmicon:hal/i2c@0.1.0`

| import 名 | Core Wasm 型 | 備考 |
|---|---|---|
| `[static]bus.open` | `(param i32 i32 i32) (result i32)` | `(index, speed, out-handle) -> ec` |
| `[method]bus.write` | `(param i32 i32 i32 i32) (result i32)` | `(self, address, data-ptr, data-len) -> ec` |
| `[method]bus.read` | `(param i32 i32 i32 i32 i32 i32) (result i32)` | `(self, address, len, buf, cap, len-out) -> ec` |
| `[method]bus.write-read` | `(param i32 i32 i32 i32 i32 i32 i32 i32) (result i32)` | `(self, address, data-ptr, data-len, len, buf, cap, len-out) -> ec` |
| `[resource-drop]bus` | `(param i32)` | `(self)` |

### `wasmicon:hal/spi@0.1.0`

| import 名 | Core Wasm 型 | 備考 |
|---|---|---|
| `[static]bus.open` | `(param i32 i32 i32 i32) (result i32)` | `(index, frequency-hz, mode, out-handle) -> ec` |
| `[method]bus.write` | `(param i32 i32 i32) (result i32)` | `(self, data-ptr, data-len) -> ec` |
| `[method]bus.transfer` | `(param i32 i32 i32 i32 i32 i32) (result i32)` | `(self, data-ptr, data-len, buf, cap, len-out) -> ec`。`cap >= data-len` が必要 |
| `[resource-drop]bus` | `(param i32)` | `(self)` |

### `wasmicon:hal/time@0.1.0`

| import 名 | Core Wasm 型 | 備考 |
|---|---|---|
| `now-us` | `(result i64)` | |
| `sleep-ms` | `(param i32)` | |
| `sleep-us` | `(param i32)` | |

### `wasmicon:hal/log@0.1.0`

| import 名 | Core Wasm 型 | 備考 |
|---|---|---|
| `log` | `(param i32 i32 i32)` | `(level, msg-ptr, msg-len)` |

### `wasmicon:hal/board@0.1.0`

| import 名 | Core Wasm 型 | 備考 |
|---|---|---|
| `pin-by-role` | `(param i32 i32 i32) (result i32)` | `(role-ptr, role-len, out-index) -> ec` |

役割名は小文字の kebab-case。v0.1 で定めるもの: `led`, `lcd-cs`, `lcd-dc`, `lcd-rst`。
そのボードに割り当てが無ければ `unsupported` を返す。

### `wasmicon:hal/types@0.1.0`

関数なし。`error-code` の discriminant（§4.4 のステータスは `+1`）:

| discriminant | ケース | ステータス値 |
|---|---|---|
| 0 | `invalid-argument` | 1 |
| 1 | `invalid-handle` | 2 |
| 2 | `busy` | 3 |
| 3 | `timeout` | 4 |
| 4 | `nack` | 5 |
| 5 | `io` | 6 |
| 6 | `unsupported` | 7 |
| 7 | `out-of-memory` | 8 |

---

## 8. ボード設定（ポート層の責務）

`index` の解釈と SDA/SCL/SCK/MOSI/MISO のピン割り当てはポート層のボード設定で決める。ゲストからは `index` だけが見える。目標アプリ向けの初期割り当て案:

| 用途 | WIT 上の指定 | ESP32-S3 (DevKitC-1) | Raspberry Pi Pico WH |
|---|---|---|---|
| I2C バス (SHT31) | `i2c.bus` index 0 | I2C0: SDA=GPIO8, SCL=GPIO9 | i2c0: SDA=GP4, SCL=GP5 |
| SPI バス (ILI9341) | `spi.bus` index 0 | SPI2: SCK=GPIO12, MOSI=GPIO11, MISO=GPIO13 | spi0: SCK=GP18, MOSI=GP19, MISO=GP16 |
| ILI9341 CS | `gpio.pin` | GPIO10 | GP17 |
| ILI9341 DC | `gpio.pin` | GPIO14 | GP20 |
| ILI9341 RST | `gpio.pin` | GPIO15 | GP21 |

`board.pin-by-role`（§7）が返す役割名と GPIO 番号の対応。ポート層の `board` 設定に置く:

| 役割名 | ESP32-S3 (DevKitC-1) | Raspberry Pi Pico WH | ホスト (mock) |
|---|---|---|---|
| `led` | GPIO2（外付け） | GP15（外付け） | 2 |
| `lcd-cs` | GPIO10 | GP17 | 10 |
| `lcd-dc` | GPIO14 | GP20 | 11 |
| `lcd-rst` | GPIO15 | GP21 | 12 |

`led` に外付けを充てるのは、どちらのボードもオンボード LED が素の GPIO ではないため
（Pico W/WH は CYW43439 側、ESP32-S3 DevKitC-1 は WS2812）。実機の配線は
Phase 4 でオーナーに確認する。

**GPIO 番号がボードごとに異なる**ため、目標アプリの「同一バイナリで同一結果」を実現するには、ゲストがピン番号をハードコードしない仕組みが要る。v0.1 では次のいずれかとする（未決、§10 参照）:

- (a) ゲストがボードごとに別ビルドする（`cfg` / 定数で切り替え）
- (b) `wasmicon:hal/board` インターフェースを追加し、`pin-by-role: func(role: string) -> result<u32, error-code>` のような「役割名 → GPIO 番号」の問い合わせをホストに委ねる

決定性検証の観点では (b) が望ましい（バイナリが完全に同一になる）。

---

## 9. 決定性・トレース

ポート層は `WASMICON_TRACE` を有効にしてビルドすると、全 host call をシリアルに次の形式で出力する。

```
> <module>/<name>(<args...>)
< <status> [<out values...>]
```

- `list<u8>` 引数は 16 進ダンプ、長さ 32 バイト超は先頭 16 バイト + CRC-32。
- `time` インターフェースの呼び出しはトレースに**含めない**（実時間に依存するため）。
- `spi.write` の `data` は CRC-32 のみ（ピクセルデータの一致確認用）。
- **`board.pin-by-role` が返した GPIO 番号は役割名に置き換えて出す。** ボードごとに
  番号が違うため、そのまま出すと同一バイナリでもトレースが一致しない。
  ポートは `pin-by-role` で配った番号と役割名の対応を覚えておき、`gpio.pin.open` の
  `index` 引数を `role:led` の形で出す。`pin-by-role` 自身の戻り値も同様。
  ホストは引数の**数値からしか判断できない**ので、ゲストがピン番号をハードコード
  していて、それがそのボードの役割割り当てとたまたま一致した場合も役割名に
  置き換わる。ピン番号のハードコードはそもそもボード間の同一性を壊すため、
  決定性検証（§2-10）の対象となるゲストは必ず `pin-by-role` を使うこと。

2 つのボードでこのトレースを diff し、`time` を除く全 host call 列と結果が一致することを「同じ結果が出る」の定義とする。

---

## 10. 未決事項

2〜5 はいずれも既定のまま実装済みで動いている。確定にするかの判断は `docs/TODO.md` §2 に集約した。

1. ~~ボードごとの GPIO 番号差の吸収方法（§8 の (a) か (b)）~~ → **(b) を採用（2026-09-10）**。`wasmicon:hal/board@0.1.0` の `pin-by-role`。役割名は docs/handoff.md §3 #2 のデフォルト。
2. `sleep-ms` 中のホストの挙動（他タスクへ譲るか、単純ビジーウェイトか）。ポート層の HAL に委ねる（`docs/handoff.md` §3 #3）。
3. トラップ後の挙動（再起動 / 停止 / `run` 再呼び出し）。
4. `log` の `string` に UTF-8 検証を入れるか（現状: 入れない）。
5. `spi.transfer` を v0.1 に残すか（目標アプリでは不要。ILI9341 の ID 読み出しに使える程度）。

---

## 付録 A. ゲスト側コード例（Rust, 生成物のイメージ）

```rust
// 生成: bindings/rust/src/generated.rs
// インターフェースごとに mod を作る（i2c.bus.open と spi.bus.open の衝突を避けるため）
pub mod gpio {
    #[link(wasm_import_module = "wasmicon:hal/gpio@0.1.0")]
    unsafe extern "C" {
        #[link_name = "[static]pin.open"]
        pub fn pin_open(index: u32, mode: u32, out: *mut u32) -> u32;
        #[link_name = "[method]pin.write"]
        pub fn pin_write(this: u32, level: u32) -> u32;
        #[link_name = "[resource-drop]pin"]
        pub fn pin_drop(this: u32);
    }
}

// 手書き: 安全なラッパ（Phase 3）
pub struct Pin(u32);
impl Pin {
    pub fn open(index: u32, mode: PinMode) -> Result<Pin, ErrorCode> {
        let mut h = 0u32;
        match unsafe { pin_open(index, mode as u32, &mut h) } {
            0 => Ok(Pin(h)),
            n => Err(ErrorCode::from_status(n)),
        }
    }
    pub fn write(&self, level: Level) -> Result<(), ErrorCode> {
        match unsafe { pin_write(self.0, level as u32) } { 0 => Ok(()), n => Err(ErrorCode::from_status(n)) }
    }
}
impl Drop for Pin { fn drop(&mut self) { unsafe { pin_drop(self.0) } } }
```

## 付録 B. ゲスト側コード例（AssemblyScript, 生成物のイメージ）

```ts
// 生成: wasmicon:hal/i2c@0.1.0
@external("wasmicon:hal/i2c@0.1.0", "[static]bus.open")
declare function i2c_bus_open(index: u32, speed: u32, out: usize): u32;
@external("wasmicon:hal/i2c@0.1.0", "[method]bus.write-read")
declare function i2c_bus_write_read(self: u32, address: u32, dataPtr: usize, dataLen: u32,
                                     len: u32, buf: usize, cap: u32, lenOut: usize): u32;
@external("wasmicon:hal/i2c@0.1.0", "[resource-drop]bus")
declare function i2c_bus_drop(self: u32): void;
```

## 付録 C. ホスト側 import 表（Rust, 生成物のイメージ）

```rust
// 生成: runtime/src/generated.rs
pub struct ImportDesc {
    pub module: &'static str,  // "wasmicon:hal/gpio@0.1.0"
    pub name: &'static str,    // "[static]pin.open"
    pub sig: &'static str,     // "iii:i"  params:results, i=i32 I=i64 f=f32 F=f64
    pub host_fn: HostFn,       // ポート層がディスパッチに使うスロット
}

#[repr(u16)]
pub enum HostFn { GpioPinOpen, GpioPinSetMode, /* ... */ LogLog }

pub static IMPORTS: [ImportDesc; 19] = [ /* ... */ ];

pub fn resolve(module: &str, name: &str) -> Option<&'static ImportDesc>;
```

ランタイムは module + name で `ImportDesc` を引き、`sig` がゲストの型と完全一致することを確かめてから `host_fn` をリンクする。ホスト関数の実体はポート層が供給する。
