# 残作業

最終更新: 2026-09-26

**全 6 フェーズのソフトウェア側は完了**し、CI も green。残っているものをここに集約する。
散らばると更新漏れで嘘になるので、**残作業はこのファイルだけに書く**。
決定済みの事項と経緯は `docs/handoff.md`、検証の現状は `docs/verification-report.md`。

---

## 1. 実機が要るもの

**ボードは 3 枚とも手元にある**（2026-09-26 時点）。Raspberry Pi Pico 2 W /
ESP32-S3 DevKitC-1 / Raspberry Pi Pico WH。Pico 2 W は `lcd-demo-rs` で
GPIO と SPI の実機動作まで確認済み（`docs/verification-report.md` §6）。
**センサーは SHT40**（2026-09-26 にオーナー承認で SHT31 から差し替え、
`docs/handoff.md` §2-9）。手元にはもう 1 つ VEML7700（環境光）もある。

つまり**この節の大半はもう実機待ちではない**。残りは実装と、実機を触る作業。
特に、2 ボードでのトレース一致（Phase 6）は `lcd-demo-rs` を使えば
**センサー抜きで回せる**（SPI と GPIO だけで足りる）。要るのは
`ports/rp2040` か `ports/esp32s3` の SPI 実装（§1.2）。

### 1.1 オーナーに聞くこと

- [ ] **実機の配線**。`docs/abi-spec.md` §8 の表（I2C/SPI のピン、役割名 → GPIO 番号）が実機と合っているか
      - RP2350 の LCD 側（`lcd-cs` / `lcd-dc` / `lcd-rst`、SCK=GP18 / MOSI=GP19）は
        2026-09-26 に実機で確認済み。**残るのは `led`、I2C (SDA=GP4 / SCL=GP5)、
        および RP2040 / ESP32-S3 の全て**
- [ ] **シリアルの接続方法**。RP2040 / RP2350 は UART0 (GP0/GP1)、ESP32-S3 は UART0 (GPIO43/44) を前提にしている
      - RP2350 は UART0 + USB シリアル変換 (CP2102N) で 2026-09-26 に確認済み
- [ ] **モジュールの型番**。ILI9341 は 3.3V ロジックの SPI 版を前提にしている。
      Pico 2 W に繋いだものは条件を満たし、2026-09-26 に動作確認済み
      - [ ] **ILI9341 は何枚あるか。** 2 ボード同時にトレースを取るなら 2 枚欲しい
        （1 枚でも順番に差し替えれば取れる）
- [x] **センサーの型番** → **SHT40**（SHT4x）で確定。2026-09-26 にオーナー承認のうえ
      `docs/handoff.md` §2-9 を差し替え、`apps/README.md` §1 と Rust / AS の両実装を
      対応させた。**VEML7700 は v0.1 では使わない**（§4 の候補）
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
- [ ] `verify/sht4x-replay.txt` を**実機から記録した応答**に差し替える（現在は合成データ）。SHT40 実機が要る
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

### 2.1 SPI 実装から出た未決（`/code-review xhigh` の指摘、2026-09-26）

いずれも `ports/rp2350` に SPI を入れたときに生まれたもの。**1 と 2 は import の
エラーステータスに関わるので、ABI の変更にあたる**（`docs/handoff.md` §2-3）。
実装せずここに置いてある。

- [ ] **`spi.bus.open` の失敗条件を全ポートで揃えるか。** `ports/rp2350` は
      `frequency-hz == 0` で `invalid-argument`、`index == 1` で `unsupported` を
      返すが、`ports/host` の mock はどちらも成功を返し、`ports/common` は
      周波数を検査しない。**同じ `.wasm` が host と実機で違うトレースを出す**ので、
      「同一バイナリが 2 ボードで同じトレースを出す」（handoff §5 Phase 6）に
      抵触する。`lcd-demo-rs` も `sensor-display` も踏まないが、踏めば食い違う
      - 揃えるなら検査は `ports/common` に置く（全ポートが同じ判定になる）
      - `wit/spi.wit` は「frequency-hz はホストが対応できる最も近い値に丸められる」
        と書いていて 0 を失敗と定めていない。`docs/abi-spec.md` も長さ 0 の転送
        だけを `invalid-argument` としている。**どちらに寄せるかはオーナーの判断**
- [ ] **SPI の待ちループに上限を設けるか。** `ports/rp2350` の `spi_drain` の
      `BSY` 待ち、RESETS 完了待ち、`spi_write` / `spi_transfer` の `TNF` / `RNE`
      待ちはいずれも無制限に回る。クロックが止まる・ペリフェラルが固まると、
      UART に最後のトレース 1 行を残して無言で停止する。これは
      `ports/rp2350/src/main.rs` と §1.4 が「診断可能にする」と言っている
      失敗の形そのもの
      - `types.error-code` に `timeout` は既にあるので型は足りている。
        ただし**今まで返らなかった状態を返すようになる**ので ABI の変更
- [ ] **ゲストの `ili9341.rs` の重複を解消するか。** `apps/sensor-display-rs` と
      `apps/lcd-demo-rs` に 179 行の写しがある（元は module コメント以外同一）。
      意図的に分けたが、**実際に挙動が分岐した**: 境界検査の u16 折り返しバグ
      （`fix: 境界検査の u16 折り返しを直し…` = `321fe3d` で `lcd-demo-rs` 側だけ
      修正）は両方にあり、今は振る舞いが違う
      - `sensor-display` 側も直せる。画面内の座標では送るバイト列が変わらない
        ので CRC は動かない。ただし `apps/README.md` §4 の規則どおり
        **AssemblyScript 版も同時に直す**必要がある（片方だけだと
        `sensor_display_rs_and_as_agree` が落ちる）
      - 共有クレートに切り出すならフォント表をパラメータにする。
        `apps/` の workspace メンバーが 1 つ増える
      - 分けたままにするなら、片方を直したらもう片方も見ることを
        `apps/README.md` に書く（AS 版も含めて 3 箇所になる）

### 2.2 センサーを SHT40 にした（2026-09-26、決定済み）

**決着済み。記録として残す。** 手元にあるのは SHT40（温湿度）と VEML7700（環境光）で、
SHT31 は入手しない。`docs/handoff.md` §2-9 は SHT31/SHT30 を変更禁止としていたが、
オーナー承認のうえ SHT4x に差し替えた。

差分は小さかった。**アドレス・フレーム・CRC・温度換算はすべて同じ**で、
変わったのはコマンド 1 バイトと湿度の式だけ。

| | SHT31（旧） | SHT40（現行） |
|---|---|---|
| I2C アドレス | `0x44` | `0x44`（同じ） |
| 計測コマンド | `0x2400`（2 バイト） | **`0xFD`（1 バイト）** |
| 計測待ち | 15 ms | **10 ms**（データシート最大 8.3 ms） |
| 読み出し | 6 バイト `T,T,CRC,RH,RH,CRC` | 同じ（並びも同じ） |
| CRC-8 | poly `0x31` / init `0xFF` / 反転なし | 同じ |
| 温度換算 | `-45 + 175·raw/65535` | **同じ** |
| 湿度換算 | `100·raw/65535` | **`-6 + 125·raw/65535`**（0..100% にクランプ） |

**Phase 6 の本来の対象は無傷。** f32 の温度バーは温度 centi 値からしか計算せず、
温度の式が同じなので `apps/README.md` §1 の f32 部分は変わっていない。
`sleep-ms` はトレースに出ない（`ports/common` が `Time*` を除外している）ので、
15→10 ms もトレースを変えない。

合成の応答データ（`verify/sht4x-replay.txt`）はバイト列をそのまま流用した
（フレームも CRC-8 も同じなので有効）。湿度の式が変わったので表示値だけ
45.66% → 51.08% に動いた。

#### VEML7700 は v0.1 では使わない

環境光センサーなので温湿度の代わりにならず、主センサーにすると f32 の題材
（温度バー）を定義し直すことになる。ただし追加のデモとしては価値がある（§4）:

- I2C の**別のアクセス形**を通る。SHT は「コマンドを書く → 後で 6 バイト読む」だが、
  VEML7700 は 16 bit レジスタをリトルエンディアンで読み書きする
  （アドレス `0x10`、ALS データはレジスタ `0x04`）。`i2c.write-read` の
  実装を別の角度から検査できる
- CRC が無いので、CRC 検証の経路は通らない
- lux 換算に分解能の係数が掛かるので、f32 の題材にもなる

---

## 3. 分かっている制限（今は困っていない）

直す必要が出たときのために書いておく。

- **`spi.transfer` と `i2c.write-read` は 128 バイトまで**（`ports/common` の `SCRATCH`）。送信元と受信先がどちらもゲストメモリにあり範囲が重なりうるので、送信側を一度写している。超えると `unsupported`。v0.1 の用途（SHT40 の 6 バイト、ILI9341 の ID 読み）には十分
- **`draw_text` は 12 文字まで**（`apps/README.md` §2）。超えると描かずに失敗を返す
- **`fill_rect` は行ごとに `dc` を high に上げ直している。** CS low の 1 トランザクション
  内で `dc` は RAMWR の後に 1 回上げれば足りるので、2 回目以降は無駄な host call。
  `lcd-demo-rs` では 7,176 回のうち約 2,659 回がこれで、トレースを出すビルドの
  実行時間がほぼ倍になっている。**直すとトレースが変わる**ので、
  `docs/verification-report.md` §6 と `README.md` に記録した 14,352 行という
  数値も取り直しになる（`apps/README.md` §2 の送信手順そのものは変わらない）
- **テキスト形式の Wasm を読めない**。spec テストの `(module quote ...)` 538 件はこれでスキップしている（スキップ 548 件の内訳はランナーが実行時に出す）
- **複数モジュールのリンクをしない**。`linking.wast` / `imports.wast` 系は対象外
- **`panic` の理由を実機のシリアルに出せない**。シリアルはボードが持っていて panic handler から届かない。ランタイム由来の失敗は `main` が捕まえて出すので、ここに来るのはポート自身のバグに限られる

---

## 4. 今後やるとしたら（MVP 後）

`docs/design-notes.md` §6 のロードマップにある、v0.1 の範囲外のもの。

- **VEML7700（環境光）のデモ**。手元にある。SHT4x と違って 16 bit レジスタを
  リトルエンディアンで読み書きするので、`i2c.write-read` を別の形で検査できる
  （§2.2）。v0.1 の主センサーにはしない
- AoT コンパイル（`wasmicon_compiler`）
- HTTP での動的ロード
- Component Model の完全採用（resource type / async / component binary）
- インタプリタの最適化。今は `match` ループのまま。RP2040 で ILI9341 のテキスト描画が
  1 秒以内という目標は未計測（`docs/handoff.md` §5 Phase 2）
