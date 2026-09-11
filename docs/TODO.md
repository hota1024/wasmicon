# 残作業

最終更新: 2026-09-11

**全 6 フェーズのソフトウェア側は完了**し、CI も green。残っているものをここに集約する。
散らばると更新漏れで嘘になるので、**残作業はこのファイルだけに書く**。
決定済みの事項と経緯は `docs/handoff.md`、検証の現状は `docs/verification-report.md`。

---

## 1. 実機が要るもの

実機（ESP32-S3 DevKitC-1 / M5Stack Tab5 / Raspberry Pi Pico WH）が
手元に来るまで進められない。

### 1.1 オーナーに聞くこと

- [ ] **実機の配線**。`docs/abi-spec.md` §8 の表（I2C/SPI のピン、役割名 → GPIO 番号）が実機と合っているか
- [ ] **シリアルの接続方法**。RP2040 は UART0 (GP0/GP1)、ESP32-S3 は UART0 (GPIO43/44)、Tab5 は UART0 (G37/G38 = M5-Bus 13/14) を前提にしている
- [ ] **モジュールの型番**。ILI9341 は 3.3V ロジックの SPI 版、SHT31 は I2C アドレス 0x44 を前提にしている。
      **Tab5 の内部 I2C には PI4IOE5V6408-2 が 0x44 で載っている**ので、SHT31 を内部バスに
      繋ぐならアドレスを 0x45 にする必要がある（現在は PORT.A に出しているので衝突しない）
- [ ] **役割名**。`led` / `lcd-cs` / `lcd-dc` / `lcd-rst` を既定のまま確定扱いで進めている。変えるなら 3 箇所（`wit/board.wit` のコメント、abi-spec §8 の表、各ポートの `ROLES`）
- [ ] **`led` に外付け LED を充てている**。どのボードもオンボード LED が素の GPIO ではないため（Pico W/WH は CYW43439、DevKitC-1 は WS2812、**Tab5 はユーザーが振れる LED を持たない**）
- [ ] **PORT.A の SDA/SCL（ESP32-P4 / Tab5）**。Tab5 の PinMap は色と GPIO
      （Yellow=G53, White=G54）までで、どちらが SDA かを書いていない。
      M5Stack の通例（Yellow=SCL, White=SDA）に従って **SCL=G53 / SDA=G54** と
      しているが、**逆だとする二次情報もある**。実機かテスタで確定させる。
      I2C は未実装なので、今のところ間違っていても表 1 行の問題で済む
- [ ] **Tab5 のトレース取り込み方法**。UART0 (G37/G38) は M5-Bus の 13/14 番ピンに
      出ているだけで USB には繋がっていない。取り込みに USB シリアル変換が要る。
      USB-Serial-JTAG に移せば変換なしで取れるが、**移すかどうかは未決**
      （移すと 3 ポートで出力経路が揃わなくなる）
- [ ] **M5-Bus のラベル付きピンが何か**。PB_IN (G17) / PB_OUT (G52) は名前から
      電源ボタン系と推定して `reserved` に入れてある。違うならゲストに開けてよい。
      PC_RX (G7) / PC_TX (G6) は逆に**開けたまま**にしてあるが、書き込み経路の
      UART なら塞ぐべき
- [ ] **Tab5 の PinMap に出てこない GPIO（0, 1, 24, 25, 33, 46, 49, 50）の扱い**。
      周辺の表にも M5-Bus にも PORT.A にも出てこない。塞いでいないが、未接続なのか
      内部で使われているのかが分からない。なお 33 と 35 は P4 の strapping ピン
      (32..=38) なので、開けてはいるが出力に使うと起動に影響しうる

### 1.2 実装

- [ ] **`ports/rp2040` の I2C / SPI**。現在は `unsupported` を返す。これが無いと sensor-display は実機で動かない
- [ ] **`ports/esp32s3` の I2C / SPI**。同上
- [ ] **`ports/esp32p4` の I2C / SPI**。同上
- [ ] **`ports/esp32p4` の 2 つ目のボード定義**。チップ層 (`src/chip.rs`) と
      ボード定義 (`src/boards/`) は分けてあるが、**定義は Tab5 の 1 つだけ**。
      M5Stamp ESP32P4 はオンボードの GPIO 割り当てが非公開で書けない。
      Function-EV-Board と P4-EYE なら esp-bsp のヘッダから今すぐ書ける
      （実機がある場合）。**2 つ目を足すときは `src/boards/mod.rs` の
      「複数選択」ガードを一緒に足すこと**（コメントに書いてある）

### 1.3 検証（Phase 4 / 5 / 6 の完了条件）

Phase 4 / 5 / 6 の完了条件は **ESP32-S3 と Pico WH の 2 ボード**で定義されている
（`docs/handoff.md` §5）。ESP32-P4 は後から足したボードなので完了条件は変えていない。
P4 の分は下に別項として置く。

- [ ] 両ボードで `blink-rs` / `blink-as` が動き、シリアルのトレースが host 版と一致（Phase 4）
- [ ] 4 通り（Rust/AS × 2 ボード）で表示が出る（Phase 5）
- [ ] 同一 `.wasm` を両ボードで走らせ、`time` を除くトレースと SPI ピクセル CRC が完全一致（Phase 6）
  - 手順は `docs/verification-report.md` §5
  - 突き合わせは `sh verify/diff-traces.sh a.log b.log`
- [ ] **ボード間の浮動小数の一致**。sensor-display が唯一 f32 を使う温度バーの計算。RP2040 はソフトフロート、ESP32-S3 は f32 のみハード FPU（非正規化数の扱いに設定依存あり）。ここが Phase 6 の本来の実測対象
- [ ] `verify/sht31-replay.txt` を**実機から記録した応答**に差し替える（現在は合成データ）
- [ ] 結果を `docs/verification-report.md` に反映する

ESP32-P4 の分（Phase 4 / 5 / 6 と同じことを 3 ボード目にも通す）:

- [ ] ESP32-P4 で `blink-rs` / `blink-as` が動き、トレースが host 版と一致
- [ ] ESP32-P4 で sensor-display の表示が出る（Rust / AS）
- [ ] 同一 `.wasm` を 3 ボードで走らせ、`time` を除くトレースが完全一致
  （`sh verify/diff-traces.sh esp32s3.log esp32p4.log` を追加で回す）
- [ ] **P4 の f32 の一致**。P4 の HP コアは RV32IMA**F**C でハード FPU（単精度）。
      RP2040 のソフトフロートと ESP32-S3 の Xtensa FPU に加えて 3 つ目の実装になるので、
      温度バーの計算がここでも一致するかは実測対象

### 1.4 実機で最初に疑うところ

**3 ポートとも GPIO はレジスタ直叩きで、一度も観測していない。**
「blink が光らない」を最初の期待値として想定すること。

- RP2040: SIO / IO_BANK0 / PADS_BANK0（FUNCSEL=5）
- ESP32-S3: GPIO / IO_MUX（MCU_SEL=1、GPIO マトリクスの out_sel=128）
- ESP32-P4: GPIO / IO_MUX（MCU_SEL=1、GPIO マトリクスの **out_sel=256**）。
  S3 と定数が違うのはここと GPIO 本数（0..=54）だけで、レジスタ名は同じ

---

## 2. 実機なしで判断できること

- [ ] **toolchain を固定するか**。`rust-toolchain.toml` は `channel = "stable"` の浮動。clippy の新しい lint や rustfmt の出力変化で CI が突然落ちる（初回 CI がまさにそれ: 手元 1.97.1 / CI 1.98.0）。特に「生成物 diff ゼロ」の検査は rustfmt の出力に依存するので、手元で通って CI で落ちる形で効く。固定すると手動でのバージョン上げが要る
- [ ] **`docs/abi-spec.md` §10 の未決 2〜5 を確定にするか**。いずれも既定のまま実装済みで動いている
  - #2 `sleep-ms` 中の挙動 → 各ポートの HAL に委ねる（実装済み）
  - #3 トラップ後の挙動 → ログを出して停止、再起動しない（実装済み）
  - #4 `log` の UTF-8 検証 → しない（実装済み）
  - #5 `spi.transfer` を v0.1 に残すか → 残している（実装済み）
- [ ] **旧 `main` ブランチをどうするか**。リモートの `main` は旧実装（wasm decoder / llvm 試行）のまま。`v2` を `main` にするか、`main` を残すか

---

## 3. 分かっている制限（今は困っていない）

直す必要が出たときのために書いておく。

- **`spi.transfer` と `i2c.write-read` は 128 バイトまで**（`ports/common` の `SCRATCH`）。送信元と受信先がどちらもゲストメモリにあり範囲が重なりうるので、送信側を一度写している。超えると `unsupported`。v0.1 の用途（SHT31 の 6 バイト、ILI9341 の ID 読み）には十分
- **`draw_text` は 12 文字まで**（`apps/README.md` §2）。超えると描かずに失敗を返す
- **テキスト形式の Wasm を読めない**。spec テストの `(module quote ...)` 538 件はこれでスキップしている（スキップ 548 件の内訳はランナーが実行時に出す）
- **複数モジュールのリンクをしない**。`linking.wast` / `imports.wast` 系は対象外
- **`panic` の理由を実機のシリアルに出せない**。シリアルはボードが持っていて panic handler から届かない。ランタイム由来の失敗は `main` が捕まえて出すので、ここに来るのはポート自身のバグに限られる

---

## 4. 今後やるとしたら（MVP 後）

`docs/design-notes.md` §6 のロードマップにある、v0.1 の範囲外のもの。

- AoT コンパイル（`wasmicon_compiler`）
- HTTP での動的ロード
- Component Model の完全採用（resource type / async / component binary）
- インタプリタの最適化。今は `match` ループのまま。RP2040 で ILI9341 のテキスト描画が
  1 秒以内という目標は未計測（`docs/handoff.md` §5 Phase 2）
