# 残作業

最終更新: 2026-09-22

**全 6 フェーズのソフトウェア側は完了**し、CI も green。残っているものをここに集約する。
散らばると更新漏れで嘘になるので、**残作業はこのファイルだけに書く**。
決定済みの事項と経緯は `docs/handoff.md`、検証の現状は `docs/verification-report.md`。

---

## 1. 実機が要るもの

**Raspberry Pi Pico 2 W は手元にある**（2026-09-26 に `lcd-demo-rs` で動作確認済み）。
ESP32-S3 DevKitC-1 と Raspberry Pi Pico WH、および SHT31 センサーは未入手。

### 1.1 オーナーに聞くこと

- [ ] **実機の配線**。`docs/abi-spec.md` §8 の表（I2C/SPI のピン、役割名 → GPIO 番号）が実機と合っているか
      - RP2350 の LCD 側（`lcd-cs` / `lcd-dc` / `lcd-rst`、SCK=GP18 / MOSI=GP19）は
        2026-09-26 に実機で確認済み。**残るのは `led`、I2C (SDA=GP4 / SCL=GP5)、
        および RP2040 / ESP32-S3 の全て**
- [ ] **シリアルの接続方法**。RP2040 / RP2350 は UART0 (GP0/GP1)、ESP32-S3 は UART0 (GPIO43/44) を前提にしている
      - RP2350 は UART0 + USB シリアル変換 (CP2102N) で 2026-09-26 に確認済み
- [ ] **モジュールの型番**。ILI9341 は 3.3V ロジックの SPI 版、SHT31 は I2C アドレス 0x44 を前提にしている
- [ ] **役割名**。`led` / `lcd-cs` / `lcd-dc` / `lcd-rst` を既定のまま確定扱いで進めている。変えるなら 3 箇所（`wit/board.wit` のコメント、abi-spec §8 の表、各ポートの `ROLES`）
- [ ] **`led` に外付け LED を充てている**。どのボードもオンボード LED が素の GPIO ではないため（Pico W/WH と Pico 2 W は CYW43439、DevKitC-1 は WS2812）。Pico 2（無線なし）だけは GP25 が素の LED だが、Pico 2 W と揃えて外付けにしている
- [x] **RP2350 ボードの品種** → **Pico 2 W**（RP2350A、GP0..GP29）で確定。`NUM_GPIO` は 30 のままでよい
- [ ] **RP2350 を Arm だけで見るか**。`ports/rp2350` は Cortex-M33（`thumbv8m.main-none-eabihf`）のみ。RISC-V (Hazard3) でも同じトレースが出るかは v0.1 の検証範囲に入れていない

### 1.2 実装

- [ ] **`ports/rp2040` の I2C / SPI**。現在は `unsupported` を返す。これが無いと sensor-display は実機で動かない
- [x] **`ports/rp2350` の SPI**。SPI0 (PL022) をレジスタ直叩きで実装した。
      **2026-09-26 に実機で確認済み**（`docs/verification-report.md` §6）。
      rp2040 / esp32s3 に足すときは、周波数の丸め（要求値を超えない最大）と
      「送信後 `BSY` が落ちるまで戻らない」を揃えること
- [ ] **`ports/rp2350` の I2C**。まだ `unsupported`
- [ ] **`ports/esp32s3` の I2C / SPI**。同上

### 1.3 検証（Phase 4 / 5 / 6 の完了条件）

- [x] **`lcd-demo-rs` が Pico 2 W で表示される**（2026-09-26 達成。詳細は
      `docs/verification-report.md` §6）。トレースが host ポートと 14,352 行完全一致し、
      画面にも絵が出た。**残るのは I2C 側**
- [ ] 両ボードで `blink-rs` / `blink-as` が動き、シリアルのトレースが host 版と一致（Phase 4）
- [ ] 4 通り（Rust/AS × 2 ボード）で表示が出る（Phase 5）
- [ ] 同一 `.wasm` を両ボードで走らせ、`time` を除くトレースと SPI ピクセル CRC が完全一致（Phase 6）
  - 手順は `docs/verification-report.md` §5
  - 突き合わせは `sh verify/diff-traces.sh a.log b.log`
- [ ] **RP2350 も同じ 3 点を通す**。Phase 4/5/6 の完了条件そのものは ESP32-S3 と Pico WH の
  2 ボードで定義されている（`docs/handoff.md` §5）。`ports/rp2350` は 3 つ目のポートなので、
  完了条件は変えずに同じ検証を追加で回す
- [ ] **ボード間の浮動小数の一致**。sensor-display が唯一 f32 を使う温度バーの計算。RP2040 はソフトフロート、ESP32-S3 と RP2350 は f32 のみハード FPU（非正規化数の扱いに設定依存あり）。ここが Phase 6 の本来の実測対象
  - RP2350 は hard-float ABI（`thumbv8m.main-none-eabihf`）で組んでいる。FPU は `cortex-m-rt` が有効にし、FPSCR は既定のまま（最近接丸め、flush-to-zero 無効）なので IEEE 準拠のはず。実機で確かめる
  - RP2350 の DCP（f64 を速くする補助演算器）は使っていない。`rp235x-hal` の `dcp-fast-f64` を入れると `__aeabi_dadd` / `__aeabi_dmul` が差し替わる。速くはなるが結果の一致を確かめていないので、Phase 6 が通るまで入れない
- [ ] `verify/sht31-replay.txt` を**実機から記録した応答**に差し替える（現在は合成データ）
- [ ] 結果を `docs/verification-report.md` に反映する

### 1.4 実機で最初に疑うところ

**RP2350 は観測済み**（2026-09-26）。`lcd-demo-rs` を Pico 2 W で走らせ、GPIO
（`pin.open` / `pin.write`）と SPI0 が全て成功し、host call のトレースが host
ポートと完全一致した（14,352 行、`spi.write` の CRC-32 3,272 件を含む）。
以下の懸念は RP2350 では解消済み。**RP2040 と ESP32-S3 は未観測のまま**で、
「blink が光らない」を最初の期待値として想定すること。

- RP2040: SIO / IO_BANK0 / PADS_BANK0（FUNCSEL=5）
- RP2350: 同上。加えて **PADS_BANK0 の `ISO`（アイソレーションラッチ）のリセット値が 1**。
  落とし忘れるとパッドが切り離されたままで、レジスタは正しく見えるのに GPIO が無反応になる。
  `gpio_configure` は PADS へ書くたびに `iso().clear_bit()` している（`write()` はリセット値から
  始まるので、書き残すと再びアイソレートされる）
- ESP32-S3: GPIO / IO_MUX（MCU_SEL=1、GPIO マトリクスの out_sel=128）

---

## 2. 実機なしで判断できること

- [ ] **toolchain を固定するか**。`rust-toolchain.toml` は `channel = "stable"` の浮動。clippy の新しい lint や rustfmt の出力変化で CI が突然落ちる（初回 CI がまさにそれ: 手元 1.97.1 / CI 1.98.0）。特に「生成物 diff ゼロ」の検査は rustfmt の出力に依存するので、手元で通って CI で落ちる形で効く。固定すると手動でのバージョン上げが要る
- [ ] **`docs/abi-spec.md` §10 の未決 2〜5 を確定にするか**。いずれも既定のまま実装済みで動いている
  - #2 `sleep-ms` 中の挙動 → 各ポートの HAL に委ねる（実装済み）
  - #3 トラップ後の挙動 → ログを出して停止、再起動しない（実装済み）
  - #4 `log` の UTF-8 検証 → しない（実装済み）
  - #5 `spi.transfer` を v0.1 に残すか → 残している（実装済み）

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
