# 残作業

最終更新: 2026-10-07

**当初の目標は達成した**（2026-10-07 オーナー判断。sensor-display が ESP32-S3 と Pico 2 W で
動き、トレースの一致を確かめた。`docs/handoff.md` §0）。ここに残っているのは、目標の
外にある改善・未決・MVP 後の作業。残っているものをここに集約する。
散らばると更新漏れで嘘になるので、**残作業はこのファイルだけに書く**。
決定済みの事項と経緯は `docs/handoff.md`、検証の現状は `docs/verification-report.md`。

---

## 1. 実機が要るもの

**Raspberry Pi Pico 2 W と ESP32-S3 DevKitC-1 は手元にある**（どちらも 2026-09-26 に
`lcd-demo-rs` で動作確認済み。`docs/verification-report.md` §6 / §7）。
**温湿度センサーは SHT40 が手元にある**（2026-09-29。`docs/handoff.md` §2 の
決定 9 を SHT31/SHT30 から SHT4x に変更し、ゲストのドライバを直した）。
**RP2040（Pico WH）は評価対象から外した**（2026-10-07 オーナー決定。`docs/handoff.md` §0）。
Phase 4/5/6 の完了条件の 2 ボードは **ESP32-S3 と Pico 2 W（RP2350）**。
`ports/rp2040` のコードと CI のビルドは残すが、実機検証と I2C / SPI の実装はしない。
I2C と sensor-display は 2026-10-05 / 07 に 2 ボードで動作を確認した
（`docs/verification-report.md` §11 / §12）。

### 1.1 オーナーに聞くこと

- [x] **実機の配線**（2026-10-07 に閉じた。`led` 以外は 2 ボードとも実機で確認済み。`led` は下の項目）。`docs/abi-spec.md` §8 の表（I2C/SPI のピン、役割名 → GPIO 番号）が実機と合っているか
      - RP2350 の LCD 側（`lcd-cs` / `lcd-dc` / `lcd-rst`、SCK=GP18 / MOSI=GP19）は
        2026-09-26 に、I2C (SDA=GP4 / SCL=GP5) は 2026-10-05 に実機で確認済み
        （`docs/verification-report.md` §11）。**残るのは `led`**（RP2040 は評価対象外）
      - ESP32-S3 の LCD 側（CS=GPIO10 / DC=GPIO14 / RST=GPIO15、SCK=GPIO12 /
        MOSI=GPIO11）は 2026-09-26 に実機で確認済み。配線表は
        `apps/lcd-demo-rs/README.md`。I2C (SDA=GPIO8 / SCL=GPIO9) も
        2026-10-05 に確認済み。**残るのは `led`**
      - **I2C は SDA / SCL に外部 10 kΩ のプルアップが要る。** 両ポートとも
        内部プルアップを有効にしているが（esp-hal の `connect_pin` が必ず
        `Pull::Up` を掛けるので rp2350 もそれに揃えた）、RP2350 は 50..80 kΩ、
        ESP32-S3 は約 45 kΩ と弱く、`standard` (100 kHz) の短い配線で
        かろうじてという程度。モジュール側に載っていることが多いので、
        **載っているなら足さなくてよい**
- [x] **シリアルの接続方法**（2026-10-07 に閉じた。2 ボードとも UART0 で確認済み）。RP2350 は UART0 (GP0/GP1)、ESP32-S3 は UART0 (GPIO43/44) を前提にしている
      - RP2350 は UART0 + USB シリアル変換 (CP2102N) で 2026-09-26 に確認済み
      - ESP32-S3 も UART0 (GPIO43/44) で 2026-09-26 に確認済み。**手元のボードは
        `USB-UART` 側のブリッジが CH343 (VID 0x1A86 / PID 0x55D3) で、macOS では
        `/dev/cu.usbmodem*` として見える**（CP2102N ではないので `usbserial` を
        探すと見つからない）。書き込みは `USB-OTG` 側（USB-Serial-JTAG）からも通る
      - **ESP32-S3 で `USB-UART` 側を使うときは、外付けの USB シリアル変換を
        UART0 に繋がない。** CH343 の TX が基板上で GPIO44 に直結しているので
        引っ張り合い、書き込めなくなる（2026-10-07 に踏んだ。
        `docs/verification-report.md` §12）
- [x] **モジュールの型番**（2026-10-07 に閉じた。ILI9341 は 3.3V の SPI 版で 2 ボードとも表示が出た。SHT40 はアドレス 0x44 で読めた）。ILI9341 は 3.3V ロジックの SPI 版を前提にしている
      - センサーは **SHT40 で確定**（2026-09-29）。ただし **I2C アドレスは
        サフィックスで変わる**（-AD1B が 0x44）。ドライバは `0x44` のままなので、
        **手元の品種の刻印かバススキャンで確認すること**。違っていれば **3 箇所**:
        `apps/sensor-display-rs/src/sht4x.rs` と
        `apps/sensor-display-as/assembly/sht4x.ts` の `ADDRESS`、および
        `ports/host/tests/apps.rs` の `SHT4X_ADDR`（host テストが期待する
        トレースの値。直さないと「計測コマンドが違う」という紛らわしい
        メッセージで落ちる）
- [x] **役割名**（2026-10-07 オーナー決定: 既定のまま確定）。`led` / `lcd-cs` / `lcd-dc` / `lcd-rst`。変えるなら 3 箇所（`wit/board.wit` のコメント、abi-spec §8 の表、`ports/common` の `profile`）。**番号を変えるだけなら `profile` の 1 箇所**で、語彙（名前）を増やすときは `ROLE_NAMES` にも足す（`assert_role_names` がコンパイル時に弾く）
- [x] **`led` に外付け LED を充てている**（2026-10-07 に閉じた。**`led` はファームの役割ではなくなった**。役割名は固定の語彙を持たず、配線はアプリの `wasmicon.toml` に書く（§5-9）。blink の `apps/blink-*/wasmicon.toml` が `led` を GP15 / GPIO2 に割り当てている。外付け LED を実際に光らせる確認は Phase 4 の blink の項目に残る）。どのボードもオンボード LED が素の GPIO ではないため（Pico W/WH と Pico 2 W は CYW43439、DevKitC-1 は WS2812）。Pico 2（無線なし）だけは GP25 が素の LED だが、Pico 2 W と揃えて外付けにしている
- [x] **RP2350 ボードの品種** → **Pico 2 W**（RP2350A、GP0..GP29）で確定。`NUM_GPIO` は 30 のままでよい
- [x] **RP2350 を Arm だけで見るか**（2026-10-07 オーナー決定: Arm（Cortex-M33）だけで確定。RISC-V の Hazard3 は対象外）。`ports/rp2350` は Cortex-M33（`thumbv8m.main-none-eabihf`）のみ。RISC-V (Hazard3) でも同じトレースが出るかは v0.1 の検証範囲に入れていない

### 1.2 実装

- ~~`ports/rp2040` の I2C / SPI~~ → **評価対象外**（2026-10-07）。`unsupported` を
      返したまま残す。戻すときのための記録: 足すなら rp2350 の実装をそのまま持って
      これる（RP2040 と RP2350 の I2C は同じ DW_apb_i2c で、SPI も同じ PL022。
      違いは PADS の `ISO` が無いことと FUNCSEL の綴りだけ）
- [x] **`ports/rp2350` の SPI**。SPI0 (PL022) をレジスタ直叩きで実装した。
      **2026-09-26 に実機で確認済み**（`docs/verification-report.md` §6）。
      rp2040 に足すときは、周波数の丸め（要求値を超えない最大）と
      「送信後 `BSY` が落ちるまで戻らない」を揃えること
      （esp32s3 はこの 2 点を `esp-hal` のドライバが満たしている）
- [x] **`ports/rp2350` の I2C**。I2C0（DW_apb_i2c）をレジスタ直叩きで実装した
      （2026-10-04）。初期化と転送の手順は `rp235x-hal` の
      `i2c/controller.rs` に合わせたが、`assert!` ではなくエラーコードを返す
      （パニックハンドラは理由を出せない → §1.4）。待ちは TIMER0 の実時間で
      上限を付け、**タイムアウト時は `IC_ENABLE.ABORT` で転送を畳んでから**
      `timeout` を返す（畳まないとバスが握られたまま残る）。
      **SCL カウンタは `clk_sys` から計算する**（SPI の PL022 は `clk_peri`
      だが DW_apb_i2c は `clk_sys`。既定では両方 150 MHz で一致するので
      取り違えても動いてしまう）。**実機では未検証**（→ §1.3）
- [x] **`ports/esp32s3` の SPI**。SPI2 (FSPI) を `esp-hal` の `spi::master`
      ドライバで実装した（GPIO と違いレジスタ直叩きにしていない）。rp2350 と
      揃えた点は `ports/esp32s3/src/board.rs` の module コメント。
      **2026-09-26 に実機で確認済み**（トレース 14,352 行完全一致 + ILI9341 に
      絵が出た。`docs/verification-report.md` §7）
- [x] **`ports/esp32s3` の I2C**。I2C0 を `esp-hal` の `i2c::master`
      ドライバで実装した（2026-10-04。SPI と同じくレジスタ直叩きにしていない）。
      エラーの振り分けは rp2350 と揃えてある（NACK → `nack`、調停負け → `io`、
      `esp-hal` の `Error::Timeout` → `timeout`）。**実機では未検証**（→ §1.3）

### 1.3 検証（Phase 4 / 5 / 6 の完了条件）

- [x] **`lcd-demo-rs` が Pico 2 W で表示される**（2026-09-26 達成。詳細は
      `docs/verification-report.md` §6）。トレースが host ポートと 14,352 行完全一致し、
      画面にも絵が出た。**残るのは I2C 側**
- [ ] **両ボード（ESP32-S3 と Pico 2 W）で `blink-rs` / `blink-as` が動き、シリアルのトレースが host 版と一致（Phase 4）**。
  GPIO 自体は 2 ボードとも `lcd-demo-rs` / `sensor-display` で動いているが、blink は
  まだ焼いていない。`led` 役の外付け LED（§1.1）が要る。**Phase 4 で残っているのはこれだけ**。
  2026-10-07 にオーナーが当初の目標を達成と判断したので、**ゴールの判定には含めない**
  （やるなら任意の追加検証）
- [x] **`lcd-demo-rs` が ESP32-S3 で表示される**（2026-09-26 達成。
      `docs/verification-report.md` §7）。トレースが host と 14,352 行完全一致し、
      画面にも絵が出た。**残るのは I2C 側**
- [x] **4 通り（Rust/AS × ESP32-S3/Pico 2 W）で表示が出る（Phase 5）**。2026-10-07 達成
  （`docs/verification-report.md` §12。下の AS 版の項目）
- [x] 同一 `.wasm` を両ボードで走らせ、`time` を除くトレースと SPI ピクセル CRC が完全一致（Phase 6）
      → **2026-09-26 に RP2350 と ESP32-S3 で達成**（どちらも host リファレンスと
      14,352 行完全一致、`spi.write` の CRC-32 3,272 件を含む。
      `docs/verification-report.md` §6 / §7）。**表示の一致は別途**（上の項目）
  - **sensor-display は「センサーを読むまで」の全行が 2 ボードで一致**。読んだ後は
    値がボードごとに違うので、各ボード ≡ host（そのボードの応答を食わせた host）を
    4 通りで確かめた（§11 / §12）。**同じ応答を 2 ボードに注入する手段は無い**
  - [x] **これで Phase 6 を完了とみなす**（2026-10-07 オーナー判断。当初の目標を達成
    したとして完了）。厳密に「sensor-display の全文が 2 ボードで一致」まで見たく
    なったら、ポートに I2C の replay モードを足して同じ応答を両ボードに食わせる
  - 手順は `docs/verification-report.md` §5
  - 突き合わせは `sh verify/diff-traces.sh a.log b.log`
- [ ] **ボード間の浮動小数の一致**。sensor-display が唯一 f32 を使う温度バーの計算。ESP32-S3 と RP2350 は f32 のみハード FPU（非正規化数の扱いに設定依存あり）。ここが Phase 6 の本来の実測対象（RP2040 のソフトフロートは評価対象外になった）
  - RP2350 は hard-float ABI（`thumbv8m.main-none-eabihf`）で組んでいる。FPU は `cortex-m-rt` が有効にし、FPSCR は既定のまま（最近接丸め、flush-to-zero 無効）なので IEEE 準拠のはず。実機で確かめる
  - **RP2350 は 2026-10-07 に 1 点測れた**（`docs/verification-report.md` §12）。
    Pico 2 W が読んだ応答を host に食わせると、温度バーを含むトレースが実機と
    全文一致した（Rust 版・AS 版とも）。**ESP32-S3 も同日に同じ結果**（4 通りとも）。
    **同じ入力を 2 ボードに食わせる手段はまだ無い**ので、言えるのは
    「各ボード ≡ host」まで
  - RP2350 の DCP（f64 を速くする補助演算器）は使っていない。`rp235x-hal` の `dcp-fast-f64` を入れると `__aeabi_dadd` / `__aeabi_dmul` が差し替わる。速くはなるが結果の一致を確かめていないので、Phase 6 が通るまで入れない
- [x] **SHT40 を実機で読む**（2026-10-05 達成。`docs/verification-report.md` §11）。
  sensor-display-rs を ESP32-S3 と Pico 2 W で走らせ、どちらも 1 回目で読めて
  画面に出た。**センサーを読むまでの 3,756 行は host・2 ボードの 3 者で完全一致**。
  アドレスは 0x44 で合っていた
- [x] **AS 版（`sensor-display-as`）を実機で走らせる**（2026-10-07、Pico 2 W と
  ESP32-S3 の両方で達成。実機が読んだ応答を host の Rust 版・AS 版に食わせた
  トレースと全文一致。`docs/verification-report.md` §12）
- [ ] **ESP32-S3 で USB からのリセットが効かなくなった原因を突き止める**（2026-10-07）。
  **切り分けが進んだ**（`docs/verification-report.md` §14）: `USB-OTG` 側が Mac に
  繋がっていると `RST` でも書き込み待ち（`boot:0x20`）に入る。`USB-UART` 側だけなら
  `RST` で普通に起動し、CP2102N を外せば CH343 からの書き込みも通る。**残るのは
  `deploy --monitor`（`espflash monitor` のリセット）でアプリが走らないことだけ**。
  以下は当初の記録:
  USB-OTG から焼くと書き込み待ち（`boot:0x21`）で止まり、CH343 の自動リセットは
  EN だけ効いて GPIO0 が効かない。§10（2026-10-04）では `deploy --monitor` が
  1 コマンドで通っていたので、何かが変わっている。今は `RST` ボタンで起動して
  回避している（`docs/verification-report.md` §12「ESP32-S3 で詰まったこと」）。
  直るまで ESP32-S3 の `deploy` は「焼く → `RST` を押す」の 2 手になる
- [x] `verify/sht4x-replay.txt` を**実機から記録した応答**に差し替える（2026-10-07。
  Pico 2 W の SHT40、T=25.63C / RH=74.49%）。オーナー判断でトレースに
  読み出しのバイト列（`data=`）を出すことにし（abi-spec §9）、
  `wasmicon trace replay` で取り出した
- [x] 結果を `docs/verification-report.md` に反映する（§11 / §12）

### 1.4 実機で最初に疑うところ

**RP2350 と ESP32-S3 は観測済み**（どちらも 2026-09-26）。`lcd-demo-rs` を Pico 2 W と
ESP32-S3 DevKitC-1 で走らせ、host call のトレースが host ポートと完全一致し
（どちらも 14,352 行、`spi.write` の CRC-32 3,272 件を含む）、画面にも絵が出た
（`docs/verification-report.md` §6 / §7）。以下の懸念のうち **GPIO / SPI のレジスタ
設定に関するもの**は RP2350 / ESP32-S3 では解消済み（`ARENA` とネイティブスタックの
項目は解消ではなく、今も有効な注意書き）。ただし **ESP32-S3 は「トレースが完全一致
するのにピンが動かない」を実際に踏んでいる**（`out_sel`。下の項目）ので、
**トレースの一致だけでは GPIO が動いた証拠にならない**。**RP2040 は未観測のまま**
（評価対象外。2026-10-07）。戻すなら「blink が光らない」を最初の期待値として想定すること。

- RP2040: SIO / IO_BANK0 / PADS_BANK0（FUNCSEL=5）
- RP2350: 同上。加えて **PADS_BANK0 の `ISO`（アイソレーションラッチ）のリセット値が 1**。
  落とし忘れるとパッドが切り離されたままで、レジスタは正しく見えるのに GPIO が無反応になる。
  `gpio_configure` は PADS へ書くたびに `iso().clear_bit()` している（`write()` はリセット値から
  始まるので、書き残すと再びアイソレートされる）
- ESP32-S3: GPIO / IO_MUX（MCU_SEL=1、GPIO マトリクスの **out_sel=256**）。
  **ここは 2026-09-26 に実機で踏んだ。** `out_sel` を 128（C3 / C6 など
  RISC-V 勢の値。ESP32 / S2 / S3 は 256）にしていたため、
  `GPIO_OUT` / `GPIO_ENABLE` は正しく読めるのにピンが
  一切動かなかった。症状は「host call のトレースは host と 14,352 行完全一致
  するのに ILI9341 が真白」。`docs/verification-report.md` §7
- ESP32-S3: SPI2 は `esp-hal` のドライバ任せなので信号番号を自前で持たない。
  上のような取り違えは起きない
- **I2C は 2026-10-05 に ESP32-S3 と RP2350 の両方で 1 回目から通った**
  （`docs/verification-report.md` §11）。RP2040 に持って行くときや配線を
  変えたときのために残しておく。失敗の最初の期待値は
  「`i2c.bus.open` は成功するのに `write` が `nack` を返す」。見る順番:
  1. **外部プルアップ**（§1.1）。無いと SDA/SCL が high に戻れず、
     アドレスの ACK が取れない。内部プルだけでは弱い
  2. **アドレス**（§1.1）。SHT4x はサフィックスで変わる。`nack` が出たら
     まずここ。品種が違えば 3 箇所直す
  3. **SDA / SCL の取り違え**。入れ替わっていても `open` は成功する
  4. RP2350 のみ: **PADS の `ISO`**。GPIO / SPI と同じ落とし穴で、
     `i2c_open` でも `pue` と一緒に落としている。レジスタは正しく読めるのに
     波形が出ないならここ
  5. ESP32-S3 のみ: `esp-hal` のドライバ任せなので GPIO マトリクスの
     信号番号を自前で持たない。§7 の `out_sel` のような取り違えは起きない
  - **`timeout` が返ったらバスが握られている**（SCL が low に張り付く）。
    rp2350 は TIMER0 の実時間で 1 バイトあたり 25 ms で諦める。無言で
    止まらないようにしてあるので、トレースの最後の行が理由を示す

- ESP32-S3: **ネイティブスタックは `ARENA` の残り**。`esp-hal` の
  リンカスクリプトは `.stack` を dram_seg の余りに置くので、`ARENA` を
  300 KB にしている今は 17.4 KiB しかない（`.bss` 315,496 B の直後、
  dram_seg の端 0x3FCDB700 まで）。インタプリタは呼び出しフレームを arena の
  配列に積むのでネイティブ再帰はしないが、`ARENA` を増やすとここが削れる。
  溢れると panic handler（理由を出せない）に入って無言で止まるので、
  「トレースが途中で切れて何も出ない」はこれを疑う
  - **この 17.4 KiB のうち約 4 KiB は `esp-storage` が使う。** あちらの
    `read` は**呼ぶたびに 4096 バイトのセクタバッファをスタックに作る**
    （`FlashSectorBuffer`。あちらのドキュメントに書いてある）。
    スロットの読み出しで何度も呼ぶが順に呼ぶので山は 1 つ分。
    **`ARENA` を増やすときは、この 4 KiB を引いた残りで考えること**
    （17.4 KiB の数字にはまだ含まれていない計算）

---

## 2. 実機なしで判断できること

- [ ] **toolchain を固定するか**。`rust-toolchain.toml` は `channel = "stable"` の浮動。clippy の新しい lint や rustfmt の出力変化で CI が突然落ちる（初回 CI がまさにそれ: 手元 1.97.1 / CI 1.98.0）。特に「生成物 diff ゼロ」の検査は rustfmt の出力に依存するので、手元で通って CI で落ちる形で効く。固定すると手動でのバージョン上げが要る
- [ ] **`abi-spec.md` §8 の表に `lcd-rst` の外部プルアップを明記するか。**
      `pin.drop` は §5.2 どおりピンを入力・プル無しに戻すので、**デモが描き
      終わると LCD の `RESET` が浮く**。モジュール側にプルアップが無いと
      パネルがリセットして画面が白に戻る（2026-09-26 に ESP32-S3 実機で観測。
      `docs/verification-report.md` §7「原因 3」）
      - **バグではなく §5.2 の帰結。** 線のアイドルレベルは外部回路が決めるべきもの
      - **出るかどうかはボードで違う**（2026-09-26 に実測）。同じモジュールで
        ESP32-S3 は白に戻り、**Pico 2 W は残った**。仕組みは両ポートで同一
        （ピンが高インピーダンスに戻る）だが、浮いた線がパネルのしきい値を
        割るかはパッドのリーク量と容量で決まる。**ポートのコードの差ではない**
      - 実務上は `RESET` に 10 kΩ のプルアップを入れれば済む。**2026-09-26 に
        ESP32-S3 実機で確認済み**（出荷する構成のまま絵が残った）。**コード変更
        ゼロで、`.wasm` のハッシュも記録済みのトレースも動かない**のでこれを推す。
        Pico 2 W でプルアップ無しでも残ったのは保証ではなく観測なので、
        「両ボードで入れておく」が安全側
      - **ゲスト側でハンドルを解放せずに終わっても回避できない。** §5.2 は
        「`run` から戻ったとき、ホストは残っている全ハンドルを drop する」とも
        定めているので、解放は必ず起きる。外部プルアップか §5.2 の変更しかない
      - §5.2 を変える（drop でピンを入力に戻さない）のは **ABI の変更**に
        あたるので、実装せずここに置く（`docs/handoff.md` §2-3）
      - 「`run()` を抜けたあとに絵が残ることは仕様に含めない」と決めて
        記録だけする、でも筋は通る。現状の文面はどこにも保証していない
- [ ] **`abi-spec.md` §6.6 の「drop → ログ出力」の順序を実装に合わせるか**
      （2026-10-04）。実装は**理由を出してから掃除**にしてある。掃除は
      無制限に待ちうる（`spi_close` の `BSY` 待ちなど。§2.1）ので、先に
      回すとペリフェラルが固まったときに理由が出ないまま無言で止まり、
      `docs/handoff.md` §3 #4「ログを出して停止する」が守れない
      - 観測できる違いは「理由が出るかどうか」だけで、ハンドルの解放は
        どちらの順でも起きる。ABI（import 名・シグネチャ・エラーコード）には
        触らない
      - §6.6 の文面を直すか、実装を文面に戻すかはオーナー判断
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
      - `ports/esp32s3` も rp2350 と同じ判定（`index != 0` → `unsupported`、
        `frequency-hz == 0` → `invalid-argument`）にした。**ただし下限未満の
        要求だけは揃っていない**: rp2350 は最も遅い分周に張り付けて成功を返し、
        esp32s3 は `esp-hal` が範囲外を弾くので `unsupported` を返す
        （APB 80 MHz のとき 78.125 kHz 未満）。どちらのデモも踏まない
      - **I2C も同じ形の食い違いがある**（2026-10-04 に実装して判明）。
        `i2c.bus.open` の `index != 0` は両ポートが `unsupported`、
        7 bit の外のアドレスと長さ 0 の転送は両ポートが `invalid-argument` を
        返すが、**`ports/host` の mock はどれも検査しない**。SHT4x は
        index 0 / 0x44 しか使わないので踏まないが、揃えるなら SPI と同じく
        `ports/common` に検査を置くことになる
        - 長さ 0 は `ports/common` が先に弾くので実際には到達しない。
          それでも両ポートで明示的に弾いているのは、`esp-hal` の
          `transaction_impl` が**空の read を転送から除いてしまう**ため
          （任せると `Ok(0)` が返って rp2350 と食い違う）
        - タイムアウトは **2 ポートで予算を揃えてある**（1 バイト 25 ms）。
          esp32s3 は `SoftwareTimeout::PerByte` を明示的に設定している。
          `Config::default()` は `SoftwareTimeout::None` なので、
          既定のままだと FSM のハードウェアタイムアウト（≒ 0.2 秒）しか
          残らず桁が合わない
- [ ] **SPI の待ちループに上限を設けるか。** `ports/rp2350` の `spi_drain` の
      `BSY` 待ち、RESETS 完了待ち、`spi_write` / `spi_transfer` の `TNF` / `RNE`
      待ちはいずれも無制限に回る。クロックが止まる・ペリフェラルが固まると、
      UART に最後のトレース 1 行を残して無言で停止する。これは
      `ports/rp2350/src/main.rs` と §1.4 が「診断可能にする」と言っている
      失敗の形そのもの
      - `types.error-code` に `timeout` は既にあるので型は足りている。
        ただし**今まで返らなかった状態を返すようになる**ので ABI の変更
- [ ] **ゲストの `ili9341.rs` の重複を解消するか。** `apps/sensor-display-rs` と
      `apps/lcd-demo-rs` に写しがある。**2026-10-05 から `draw_text` が
      違う**（`sensor-display` だけ倍率付き。`lcd-demo-rs` は記録済みトレースを
      守るため据え置き）。境界検査・初期化列・`fill_rect` は今も同一
      意図的に分けたが、**一度は実際に挙動が分岐した**: 境界検査の u16 折り返し
      バグを `321fe3d` で `lcd-demo-rs` 側だけ直し、しばらく振る舞いが違っていた
      （2026-09-28 に `sensor-display` の Rust / AS 両方を直して揃え直した）
      - 共有クレートに切り出すならフォント表をパラメータにする。
        `apps/` の workspace メンバーが 1 つ増える
      - 分けたままにするなら、片方を直したらもう片方も見ることを
        `apps/README.md` に書く（AS 版も含めて 3 箇所になる）
### 2.2 ゲストの境界検査から出た未決（`/code-review` の指摘、2026-09-28）

`apps/sensor-display-*` の u16 折り返しを直したときに出たもの。**ABI には関わらない。**

- [ ] **ゲストの単体テストを CI で回すか**（2026-09-28）。`in_bounds` の単体
      テストを `apps/sensor-display-rs` と `apps/lcd-demo-rs` に入れ、
      2026-09-29 に `sht4x` の換算（CRC / 温度 / 湿度のクランプ）も足したが、
      **どちらも CI では走っていない**。`apps/.cargo/config.toml` が wasm32 を固定して
      いるので、ホストのトリプルを明示しないと実行できない:

      ```
      (cd apps && cargo test --target "$(rustc -vV | sed -n 's/^host: //p')")
      ```

      - **湿度のクランプだけは別経路で CI に入っている**（2026-10-01）。host テスト
        `sensor_display_agrees_at_humidity_clamp_bounds` が境界を踏む合成応答で
        Rust 版と AS 版を突き合わせるので、こちらは CI で走る。**AS 側にある
        唯一のクランプ検査**でもある。ただし**部分的な緩和にすぎず、この項目
        自体は未解決**（`in_bounds` と `sht4x` の換算の大半は今も CI 外）
      - 手元（macOS / aarch64）では debug / release とも通り、`in_bounds` を
        折り返す版に戻すと落ちることも確かめた。**Linux で `extern "C"` の
        未定義シンボルがリンクエラーにならないかは未確認**なので、`guest`
        ジョブ（`ubuntu-latest`）に足すのは CI で 1 回試してからにする。
        ジョブに書くときは上の移植可能な形か `x86_64-unknown-linux-gnu` を使う
        （`aarch64-apple-darwin` を直書きすると CI では std が無くて落ちる）
      - **足す前に `clashing_extern_declarations` を潰す必要がある**（下の項目）。
        `guest` ジョブは `-D warnings` で止まる
- [ ] **`bindings/rust` の生成物が出す `clashing_extern_declarations` を潰すか。**
      `i2c` と `spi` が `[static]bus.open` / `[method]bus.write` を別シグネチャで
      宣言していて、`#[link(wasm_import_module = …)]` で区別している。この lint は
      wasm32 では属性を見るが**ホストターゲットでは見ない**ので、ゲストを
      ホスト向けにビルドしたときだけ警告 2 件が出る（2026-09-28 に単体テストを
      足して初めて見えた）
      - 直す場所は `tools/wasmicon-gen`。生成物を手で編集しない（CLAUDE.md）
      - 直さないと上の CI 項目が `-D warnings` で通らない
- [ ] **境界検査の「呼び出し側」は pin されていない**（2026-09-28）。入れた
      単体テストは `in_bounds` / `inBounds` を検査するだけなので、**呼び出し側を
      折り返す式に戻されても気付けない**（`fill_rect` と `draw_text` の両方を
      戻すと `in_bounds` が dead_code になって clippy が落ちるが、片方だけなら
      通る。AS 側にはその保険も無い）
      - 本当に pin するには `fill_rect` / `draw_text` を敵対的な座標で叩く
        テストが要る。ゲストは `run` しか export していないので、host から
        叩くには export を増やすことになり、**`lcd_demo_rs.wasm` のハッシュが
        変わって記録済みのトレースが取り直しになる**
      - **AS 側には単体テストが無い**（この repo に AS のテスト基盤が無い）。
        `noAssert: true` で境界検査が消えている側なので、危ないほうが
        テストされていないという非対称がある

---

## 3. 分かっている制限（今は困っていない）

直す必要が出たときのために書いておく。

- **`spi.transfer` と `i2c.write-read` は 128 バイトまで**（`ports/common` の `SCRATCH`）。送信元と受信先がどちらもゲストメモリにあり範囲が重なりうるので、送信側を一度写している。超えると `unsupported`。v0.1 の用途（SHT4x の 6 バイト、ILI9341 の ID 読み）には十分
- **`draw_text` は 40 文字まで**（`apps/README.md` §2。`lcd-demo-rs` の写しは 12 文字まで）。超えると描かずに失敗を返す
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

- AoT コンパイル（`wasmicon_compiler`）
- HTTP での動的ロード → **§5 で設計に入った**（`docs/app-workflow.md`）
- Component Model の完全採用（resource type / async / component binary）
- インタプリタの最適化。今は `match` ループのまま。RP2040 で ILI9341 のテキスト描画が
  1 秒以内という目標は未計測（`docs/handoff.md` §5 Phase 2）
    - **速度は 2026-10-05 に初めて測った**（別リポジトリ `wasmicon-doom` の
      `spike/`、C ゲスト、trace なし。数値の全体はその README）。整数ループで
      **約 90 サイクル / Wasm 命令**で、RP2350（150 MHz）と ESP32-S3（240 MHz）で
      ほぼ同じ（host では約 4 ns / 命令）
    - `opt-level = "z"` を `3` にしても ESP32-S3 で 2 割しか速くならない。
      効いていないのはディスパッチの構造の側（毎回の LEB128 デコード、
      分岐ごとの side table の二分探索、u64 のスロット）と見ている。未検証
    - [ ] **ESP32-S3 の CPU クロックを 240 MHz にするか**（オーナー判断）。
      `ports/esp32s3` は `esp_hal::Config::default()` のままで **80 MHz** で
      動いている。`with_cpu_clock(CpuClock::max())` で全計測がちょうど 3 倍に
      なることは手元のビルドで確かめた（コミットはしていない）

---

## 5. アプリ開発フロー（ローダと `wasmicon` CLI）

**設計は `docs/app-workflow.md` が正。ここは残作業と未決だけ。**
2026-10-04 にオーナーが方針を出した: **ボードごとにファームウェアを用意し、
そのファームウェアがアプリを USB / HTTP 経由でロードできるようにする。**

**同日の決定: ファームにアプリは入れない。** `include_bytes!` の内蔵アプリは
外し、空スロットなら理由を出して idle、生存確認はポート層が直接 LED を振る
（`docs/app-workflow.md` §3.3）。**今の `include_bytes!` はファームのビルドを
`apps/` のビルドに依存させていて、ファームを単体のリリース成果物として
作れない**のがもう 1 つの理由。

§1 を押し退けるものではない。**0 段は §1.3 の SHT40 作業を短くする**
（取り込みと突き合わせを CLI に寄せる）。**handoff §5 の Phase 4 / 5 / 6 の
完了条件は変えない。**

### 未決（オーナー判断。本文からは §5-1 … §5-9 で参照する）

- [ ] **§5-1 トラップ後の挙動を「停止したまま次のアプリを受け付ける」に確定するか。**
      今は handoff §3 #4 のとおり「ログを出して停止、再起動しない」で、`main` は
      `loop { wfi() }` に入る。ローダは**同じアプリの自動再実行はしないまま**
      「新しいアプリを待つ」状態を足す。`docs/abi-spec.md` §6.6 がトラップ後の
      挙動をポートに委ねているので **ABI の変更ではない**が、§3 #4 の記録とは
      読みが変わるので確定が要る。§2 の「§10 の未決 2〜5 を確定にするか」の #3 と
      同じ対象
- [x] **§5-2 ボードの品種**（2026-10-04 実測）。`espflash board-info` の出力:
      **`Flash size: 8MB`**、`Features: WiFi, BLE, Embedded Flash`、
      `esp32s3 (revision v0.2)`、水晶 40 MHz
      - **PSRAM は Features に出ていない** → 載っていない（N8R8 ではなく N8）と
        見える。**§5-3 の選択肢が 1 つ消える**（arena を PSRAM に移せないので、
        HTTP をやるなら `max_memory_pages` を 4 → 2 に落とす一択）
      - espflash の出力が根拠なので、**モジュールの刻印（WROOM-1-N8 か
        N8R8）で裏を取れると確実**。§5-3 を判断するときに効く
      - RP2350 側は Pico 2 W（フラッシュ 4 MB）で確定済み（§1.1）
- [ ] **§5-3 ESP32-S3 の RAM 予算をどうするか**（HTTP の前提）。DRAM はほぼ
      使い切っている（`.bss` 315 KB / `ARENA` 300 KB / ネイティブスタック
      17.4 KiB。§1.4）。`esp-wifi` を入れるなら二択:
      - `max_memory_pages` を 4 → 2 に落とす。abi-spec §6.2 は「上限はポートが
        決める」としているので**仕様違反ではない**が、**「同一バイナリがどの
        ボードでも通る」が実質的に崩れる**
      - ~~arena を PSRAM に置く~~ → **この手は無い**。2026-10-04 の実測で
        PSRAM が載っていないと分かった（§5-2）。刻印で裏を取るまでは
        完全には閉じないが、`espflash board-info` の Features に出ていない
- [ ] **§5-4 Pico 2 W / Pico WH の Wi-Fi をやるか。** CYW43439 で、実用的な
      ドライバ `cyw43` は embassy（async）前提。今の blocking 構成から
      **ポートの作り直しになる**。v1 の HTTP は ESP32-S3 だけに絞ることを推す
- [ ] **§5-5 HTTP の向き。** デバイスがサーバ（`POST /app` + mDNS。dev ループが
      楽）か、URL から pull（OTA が楽、NAT 越えが効く）か。両方は後でもよいが、
      先に入れる側を決める
- [ ] **§5-6 技術的な要確認**（実装前に確かめる。推測で進めない）
      - **RP2350 のフラッシュ書き込み**。`rom_data::flash_range_erase` /
        `flash_range_program` を**XIP から実行しているコードから呼べない**。
        書き込みルーチンを RAM に置き（`#[link_section = ".data"]`）割り込みを
        止める形になるはず
      - **UF2 で任意アドレスに書けるか**。RP2350 でパーティションテーブルが
        無いとき absolute family ID (0xe48bff57) が要るかもしれない。通れば
        `deploy` が picotool 無しで済む
      - **ESP32-S3 で任意オフセットが XIP にマップされているとは仮定しない**。
        `esp-storage` で RAM に読み出す前提で設計してある
- [ ] **§5-7 ESP32-S3 のファームをリリース成果物として CI で作るか。** 今は
      Xtensa のため CI で回していない（`.github/workflows/ci.yml` のコメント）。
      アプリ作者にファームをビルドさせない方針（`docs/app-workflow.md` §4.5）を
      取るなら espup を入れるジョブが要る
- [ ] **§5-8 ファーム版の振り方。** 3 ポート共通の 1 本（`0.1.0` を揃える）か、
      ポートごとに独立か。**ABI 版（`wasmicon:hal@0.1.0`）は `wit/` 由来で、
      ファーム版とは別物**。勝手に上げない（handoff §2-5）。互換の判定は
      版 1 本ではなく軸ごとに行う（`docs/app-workflow.md` §3.8）ので、
      ファーム版は由来の記録にしか使わない
- [x] **§5-9 役割マップをデバイス側の設定にするか / いつやるか** → **する。1 段の最小版を
      2026-10-07 に実装した**（オーナー決定）。決めたこと: **役割名はファームの語彙では
      ない**（`ROLE_NAMES` を廃止、名前は自由）／**ファームは既定の表を持たない**
      （空・壊れていれば役割を配らない）／**空スロットの待機でピンを駆動しない**。
      設計は `docs/app-workflow.md` §3.9
      （`docs/app-workflow.md` §3.9 / §4.7）。今は役割 → GPIO がファームの
      `profile::<board>.roles` にあるので、**配線を変えるとファームを焼き直す**ことになり
      「ファームは一度だけ焼く」と衝突する
      - **決定性は壊れない**。abi-spec §9 が役割で配った番号を `role:` に
        正規化するので、対応表を変えてもトレース行は変わらない
      - ただし**ピン番号をハードコードしているアプリのトレースは変わる**
        （`role:` だった行が生の番号になる）。§9 が既に対象外としている
        アプリに限る話だが、症状の説明が要る
      - 最小版は 1 段でも成立する（設定セクタを `espflash write-bin` / UF2 で
        外から書き、ファームは読んで適用するだけ）。プロトコル経由は 2 段

### 実装（段階は `docs/app-workflow.md` §5）

- [x] **`ports/common` に abi-spec §5.2 のハンドル掃除を実装する**（2026-10-04 完了。
      **ローダの有無と関係なく仕様と実装が食い違っていた**）。
      §5.2 は「`run` から戻ったとき、ホストは残っている全ハンドルを drop する」と
      定めるが、**host も rp2040 / rp2350 / esp32s3 もこれをやっていない**。今
      見えていないのは Rust バインディングの `Drop`（`bindings/rust/src/hal.rs`）が
      ゲスト側で解放しているから。ローダでは**次のアプリが `busy` を踏む**
      （前のアプリがトラップしたときは確実に踏む）
      - `Hal::release_all()` を足し、**host と 3 ポートの 4 箇所**で `run` の
        戻りを受けてから呼ぶようにした（トラップで抜けた場合も通る）
      - **掃除はトレース行を出さない**（`board` を直接叩き、トレースを書く
        `Hal::call` を通らない）。host の 10 件のトレース比較テストが通ることで
        記録済みの 14,352 行（`docs/verification-report.md` §6 / §7）が
        変わっていないことを確認した
      - 解放の順序は gpio → i2c → spi で固定（ポート間で揃える）
      - 検査は `ports/common/tests/release_all.rs` の 4 件。`release_all` を
        空にすると 3 件が落ちることを確かめた
- [x] **ボードプロファイルを `ports/common` に集める**（2026-10-04 完了）。
      `Config`（3 つの `main.rs`）、役割割り当て（各 `board.rs` の `ROLES`）、
      **実装済みインターフェース**を `ports/common/src/profile.rs` に集めた。
      各ポートはそこから引くので、CI の rp2040 / rp2350 ジョブの `cargo build` が
      値の一致を見る。値は `ports/common/tests/profiles.rs` に移す前の数値で固定
      - 実装済みインターフェースが無いと `ports/rp2040` 向けの `check` は
        **静的には通ってしまう**（SPI / I2C が `unsupported` を返すのは実行時。§1.2）
      - **`assert_role_names` が `ROLE_NAMES` との包含関係をコンパイル時に検査する。**
        外れると `pin-by-role` は成功するのにトレースが `role:` に正規化されず、
        生の GPIO 番号が出て **2 ボードのトレースが食い違う**（abi-spec §9）。
        **この検査は今まで無かった**
      - `Config::DEFAULT` を `runtime` に足し、`Default` がそれを返すようにした
        （host プロファイルが const 文脈で使うため。二重に書くと食い違う）
- [x] **`tools/wasmicon-cli`（bin 名 `wasmicon`）を作る**（2026-10-04 完了）。
      0 段は `new` / `check` / `run` / `monitor` / `trace diff` / `doctor`。
      1 段の `pack` / `deploy` も同日に入って実機で通した。
      **`size` は入れないことにした**（下の理由。`tools/measure-size.sh` のまま）
      （`/code-review` の指摘を反映済み）
      - **`check` は `Config` だけでなく arena の実寸で見る。** 線形メモリは
        arena の残り全部を取るので、ページ上限に収まっても
        「decode / validate / `Exec` の残りに入らない」ことがある
        （`ports/rp2040` は 160 KiB で 2 ページだと 20 KiB ほどしか余らない）。
        `Profile` が `arena` / `scratch` を持ち、**ポートの `static ARENA` も
        それを使う**
      - 役割名の照合は**終了コードを左右させない**（走査が参考なので）。
        import / export の不備はボードに依存しないので、ボードごとの件数に
        混ぜず「全ボード共通」として 1 回だけ数える
      - 壊れた `.wasm`（import の型インデックスが範囲外）で **panic しない**。
        診断する側が落ちては意味がない
      - **`run` は型まで見る**（`func()` 以外は実行時に `wrong arity` で
        落ちるので、export の有無だけでは素通りする）。memory の定義が
        無いのも落とす（abi-spec §6.2）
      - 落ちた段（decode / validate / instantiate）を区別して出す。
        arena 不足を「validate が落ちた」と言うと `max_memory_pages` を
        縮める方へ誘導してしまう
      - `--i2c-replay` が無ければ **`WASMICON_I2C_REPLAY` を読む**。
        無視すると黙って「センサー無し」に落ちて、記録済みのトレースと
        食い違う
      - **テストのヘルパ（`repo_root` / ゲストのビルド）を共有していない。**
        `ports/host/tests/apps.rs` と逐語で重複し、`Mutex` は
        プロセスをまたげないので、テストバイナリが並走するランナーでは
        書きかけの `.wasm` を読む形が残る。共有のヘルパに切り出すのが筋
        （`repo_root` は今 7 ファイルに散っている）
      - `run` は `wasmicon-host` を lib として呼ぶだけ（`--trace` /
        `--i2c-replay`）。`--i2c-replay` は `load_i2c_replay` を直に呼ぶので、
        環境変数 `WASMICON_I2C_REPLAY` は host の bin 側に残っている
      - `trace diff` は正規化を Rust で持ち、**`verify/diff-traces.sh` と
        同じ判定を出すことを `tests/trace.rs` が突き合わせる**（判定だけでなく
        **行数も**見る。`tools/check-sigs.sh` が `wit2sig.py` と突き合わせて
        いるのと同じ形）。スクリプトは CI の `--self-test` のために残す。
        **NUL 除去を外すと突き合わせが落ちることを確かめた**
      - **行は `Vec<u8>` で持つ。** `String` に落とすと UTF-8 でないバイトが
        どれも `U+FFFD` に潰れて、**違うノイズが混ざった 2 本を「一致」と
        言ってしまう**。あわせて**スクリプトに `LC_ALL=C` を足した**
        （ロケールが UTF-8 のままだと macOS の `tr` が不正なバイトで止まり、
        手元と CI で結果が変わる）
      - `run --trace` は**トラップしてもそこまでのトレースを出す**
        （`run_wasm_capture` を足した。トラップしたときこそ要る）
      - **`monitor` は 2026-10-04 に実装し、実機で通した**
        （`deploy --monitor` で 14,352 行を取り込み、host と完全一致。
        `docs/verification-report.md` §9）。取り込みの罠 3 つ
        （開いたまま `stty` / 流れ始める前に当てる / バイト列として扱う）を
        モジュールに閉じ込めてある。止めどきは無音（既定 3 秒）
      - **`size` は入れない。** コアのコードサイズはアプリ作者の関心ではなく
        リポジトリ保守側の道具なので、`tools/measure-size.sh` のままにする。
        アプリ自身の大きさは `check` が先頭行で出している
      - **`build` も入れない**（2026-10-04 決定）。`cargo build --release` /
        `npm run build` を言語で振り分けるだけの薄いラッパで、ビルドフラグは
        `new` が埋めたファイルが持つ（§4.4）ので足せるものが無い。
        `docs/app-workflow.md` §4.6 の内側のループは `cargo build` で書いた
      - 判定（`facts` / `judge`）と印字を分けてあるので、判定だけをテストから
        呼べる。`tools/wasmicon-cli/tests/check.rs` が `apps/` の実物で固定:
        **`blink-rs` は 4 ボードすべて通り、`sensor-display-rs` は rp2040 だけ
        I2C / SPI の未実装で落ちる**。4 ページ要求の最小 `.wasm` を手で組んで
        「host と rp2350 では通り rp2040 では落ちる」も見ている
      - **役割名の照合の限界を実測した**（どちらも §4.3 に記録）:
        ログ文字列の中の `led` を拾う（`sensor crc failed`）、
        **AssemblyScript のゲストには当たらない**（文字列が UTF-16）
      **ファームの変更ゼロ・実機不要**で、§1.3 の作業に効く
      - `check` は**実ランタイムで** decode / validate / instantiate する。
        `ports/host` は `Config::default()`（`max_memory_pages` = 65536）で走るので
        **host 実行は全ボードより緩い**。`--board` が要る理由
      - `monitor` は `/dev/cu.usb*` を列挙する。`usbserial` を決め打ちしない（§1.1）。
        **0 段の `monitor` は identity より先に出る**（ログ先頭への記録は下の
        「ファームが自分を名乗るようにする」の担当で、これだけでは済まない）
      - **`run` の `--i2c-replay` は既存の環境変数 `WASMICON_I2C_REPLAY` に対応する。**
        指す先の `verify/sht4x-replay.txt` は §1.3 で実機の記録に差し替える予定
      - **「検証だけする入口」が今は無い。** `wasmicon_host::run_wasm_opts` は
        `run` まで呼ぶ。CLI 側で `wasmicon-core` + `wasmicon_port::Hal` +
        `wasmicon_host::hal::HostBoard` を直に組む（推奨）か、`ports/host` に
        `check_wasm(wasm, &Config)` を足すかを決める
      - `wasmicon-host` の crate は残して lib として使う
        （`verify/differential` が依存している）
- [x] **アプリ側ビルドフラグの単一真実**（2026-10-04 完了。
      `docs/app-workflow.md` §4.4）。真実は
      `tools/wasmicon-cli/src/flags.rs` が持つ**ファイルの中身そのもの**で、
      `apps/` の実物との一致は `tools/wasmicon-cli/tests/flags.rs` が
      **バイトで**見る（`wasmicon-gen --check` と同じ形。検査は既に CI に
      ある `cargo test` に乗るので新しいジョブは要らない）
      - **コメント 1 文字の差でも落ちる。** 緩めると「値は合っているが
        どちらが真実か分からない」状態に戻るので、意図的にそうしてある
      - `[profile.release]` は入れない。食い違ってもアプリが**大きくなる
        だけ**で静かには壊れない。§4.4 が名指しする 2 つに絞った
      - **Rust 版と AS 版で初期メモリがページ単位で一致**していること、
        **一番きついボード（rp2040 の 2 ページ）に収まる**ことも見る
        （`profile::PROFILES` から引くので、ボードが増えたら自動で効く）
- [x] **`wasmicon new`**（2026-10-04 完了。§4.6 の 3）。雛形は `flags` から
      書き出すだけにしてある（ここで文字列を持つと 3 重化する）
      - **「`apps/` と一致している」だけでは足りない。** `tests/new.rs` が
        雛形を本当に `cargo build` して **4 ボードすべてで `check` が
        通る**ことと、書いた `wasmicon.toml` を自分で読み直せることを見る
        （`deny_unknown_fields` なので余計なキーを書くと読めない）
      - `Cargo.toml` に `[workspace]` を入れる（無いと既存の workspace の
        中で「workspace に入っていない」で止まる）。その場所では
        `rustflags` が**連結**されて同じ値が 2 回効くので `new` が言う
      - **依存はパスで書く**（`wasmicon-hal` は未配布）。`--hal` か、
        無ければ上に向かって `bindings/` を探す。見つからなければ版指定を
        書いて**その旨を出す**。相対と絶対は**短い方**を選ぶ
        （共通の祖先が無いと `../` が 8 段並ぶのを実際に出した）
- [x] **スロット形式**（2026-10-04 完了。`docs/app-workflow.md` §3.4）。
      `ports/common/src/slot.rs` に置いて**書く側（CLI）と読む側（ファーム）が
      同じ形を使う**。CRC-32 は既にある `crc32` を再利用（トレースと同じ）
      - **空（消去済みの `0xff` / 未使用の `0x00`）は失敗にしない。**
        magic 違いと区別する（ログの意味が変わる）
      - 検査は `ports/common/tests/slot.rs` の 9 件（往復、空、magic 違い、
        版違い、長さ超過、CRC、余りの無視）と
        `tools/wasmicon-cli/tests/pack.rs` の 4 件（**CLI が書いたものを
        ファームの経路で読み直す**往復と、既知の値での CRC 照合）
      - `SlotError` に `Debug` は付けない（ポートに `core::fmt` を
        持ち込まないため）。テストは `reason()` と `matches!` で書く
      - CLI 側は `wasmicon pack`。1 段では**外のフラッシャに渡す素材**
        （`espflash write-bin <offset>` / `picotool load -o <offset>`）
- [ ] **スーパーバイザのループ**（`docs/app-workflow.md` §3.1）。
      **3 ポートがスロットを読むようになった**（2026-10-04。RP2350 と
      ESP32-S3 は実機で確認済み、RP2040 はコードだけ）。残りは
      idle への復帰とサイクルごとの作り直し（arena / `Hal` / **ロール表**。
      持ち越すとトレースが変わりうる。`release_all.rs` が現状を固定）
      - **RP2040 と RP2350 が読む**（XIP で memory-mapped なので RAM に
        写さない）。読み出しは `ports/common` の `slot::read_xip` に 1 つだけ
        置いてある（生スライスを作る `unsafe` を 1 箇所にするため）。
        **host でも試せる**（static をフラッシュに見立てて渡す）
      - **RP2350 は 2026-10-04 に Pico 2 W で確認済み**（空 → 内蔵 /
        スロットのアプリが走る / CRC 破壊 → 内蔵、の 3 点。さらに
        **スロットから走ったアプリのトレースが host と 14,352 行完全一致**。
        `docs/verification-report.md` §9）。**RP2040 は未検証**
        （Pico WH が未入手。コードは同じ `slot::read_xip` を通る）
      - 実機で `picotool` の引数を 2 つ踏んで直した（拡張子は `.bin`、
        `-o` は絶対アドレス）。どちらも host では出ない
      - **ファームの末尾（`__flash_binary_end`）とスロットが重ならないことを
        起動時に検査している。** 重なったら読まずに内蔵へ落ちる（自分の
        コードを wasm として食わせない）。RP2040 にはこのシンボルが
        無かったので `memory.x` に足した（実測で `0x1000de40`、
        スロットは `0x10100000` なので 1 MB 近く空いている）
      - **`ports/esp32s3` も 2026-10-04 に実装し、同日 実機で確認した**
        （`esp-storage` でヘッダを読み、長さの分だけ arena を取って本体を
        読む。64 KiB のスロットに対してアプリは数 KB なので、静的な領域を
        増やさない）。**スロットから走ったアプリのトレースが host と
        14,352 行完全一致**。さらに **Pico 2 W（XIP）と ESP32-S3
        （`esp-storage` で RAM に写す）が同じ `.wasm` で完全一致**
        —— フラッシュの読み方が違う 2 ボードで決定性が保たれている
        （`docs/verification-report.md` §10）
      - **`partitions.csv` は要らなかった**（実測。`espflash flash` は
        アプリのセクタしか消さないので、7 MB 地点の目印が生き残った）。
        置き場所は Pico 系と揃えて `0x100000` から 64 KiB
      - **ESP32-S3 は書き込みとトレースが同じ口**。`monitor` は `stty` しか
        当てないので自分ではリセットできず、順番が詰む。**口ごと
        `espflash monitor` に任せる**ことで解けた（`--before default-reset
        --after hard-reset` が既定なので、開いてから起動させられる）。
        `espflash reset` は居座るので呼ばない
      - **失敗側も同日 実機で踏んだ**（§9 と同じ 3 点）。正しい画像 →
        走る / 本体を 1 バイト反転 → `slot crc mismatch, running built-in` /
        `0xff` 埋め → `slot empty, running built-in`。**ボタン操作が
        要らない**ので Pico より楽に回せる
      - **残り**: `Truncated`（長さだけ嘘をつく画像）は未実施。
        `Overlap` の検査は `read_xip` 側にしか無いので **ESP32-S3 には
        無い**（`esp-storage` 経由で読むため）。重なりを防ぐ責任が
        `espflash` 側に出ているのが妥当か未決
- [x] **内蔵アプリを外す**（2026-10-07 完了。ESP32-S3 の実機で「スロットのアプリが
      走る」「空なら `slot empty, idle`」を確認。`docs/verification-report.md` §13。
      LED の点滅は外付け LED が無いので未確認）。
      空スロットは `wasmicon: slot empty, idle` と出して `ports/common` の
      `idle::heartbeat` に入り、`led` 役を 1 Hz で点滅させる
      （1 段。**3 ポートがスロットを読めるようになってから**
      — 先に外すとファームが何も走らせなくなる。**RP2350 と ESP32-S3 は
      実機で確認済み**。RP2040 は評価対象外になった（2026-10-07）ので、
      **前提は満たした**。RP2040 はコードが同じ `slot::read_xip` を通るので
      一緒に外してよい）。`ports/*/src/main.rs` の `include_bytes!` と
      `guest-lcd-demo` feature を落とし、空スロットは理由を出して idle、
      生存確認はポート層が LED を振る（`Board` 直叩きで Wasm を通らない）
      - **CI も同時に直す**: ポートのジョブから `apps` の先行ビルドが不要になり、
        rp2350 の `cargo clippy --release --features guest-lcd-demo` の行も消える
      - **記録済みトレースは生き続ける**。`lcd_demo_rs.wasm`（`fc470947…`）を
        スロットへ `deploy` すれば**バイト列が同一なので host call 列も同一**
        （XIP の番地や RAM への写しはトレースに出ない）
      - **失うもの**: 焼いた直後に「ランタイムが decode → run まで通る」ことを
        実機で確かめる足場。host テストと最初の `deploy` で代替する
- [x] **`deploy` の 1 段目**（2026-10-04 完了、3 ポート全て）。検査・
      画像の用意・`picotool load -t bin -o <アドレス>`・リセットを畳んだ。
      **Pico 2 W で 1 コマンド通し済み**（`docs/verification-report.md` §9）
      - **走らないものを焼かない**（焼く前に `check --board` を通す）。
        `tests/deploy.rs` が、sensor-display を rp2040 に送ろうとすると
        **画像も書かずに**止まることを固定している
      - トレースの取り込み手順（`cat` で開いたまま `stty`）を最後に出す
      - **ESP32-S3 も 2026-10-04 に通した**（`espflash write-bin
        <オフセット>`。`pack` が出すコマンドもボードごとに分けた）。
        `--monitor` は `espflash monitor` に委譲する
        （`monitor::capture_cmd`）。**1 コマンドで焼いて取り込み、
        host と 14,352 行一致**。Pico と違い**ボタン操作が要らない**
- [ ] **ファームが自分を名乗るようにする**（`info` とバナー。
      `docs/app-workflow.md` §3.8）。今バナーは `wasmicon rp2350` の 1 行だけで
      **版も git も入っていない**（`ports/rp2350/src/main.rs:140`）。
      `docs/verification-report.md` は**ゲストの SHA-256 は記録しているのに
      ファーム側は何も記録していない**ので、どのビルドが 14,352 行を出したかは
      日付と git 履歴から推測するしかない
      - 載せるもの: ボード名 / ファーム版 + `git describe` + dirty / ABI 版 /
        構成（`trace` の有無、内蔵アプリ）/ `Config` / スロット容量と形式版 /
        プロトコル版 / 役割名
      - `git describe` は `build.rs` で埋める。**dirty を落とさない**
      - `monitor` が取り込みログの先頭に identity を記録し、`trace diff` は
        identity 行を比較から外すが**食い違ったら警告する**
      - **`trace` を切って焼いたボードは `monitor` が無言になる**。
        `trace: off` と申告できれば配線から疑わずに済む
- [ ] **ファームの配布**（release manifest と `flash --fw` / `fw list`）。
      ボード × 版の成果物に manifest（ファイル + sha256 + §3.8 の 5 軸）を付ける。
      §5-7 と対になる
- [x] **`wasmicon.toml` を読む**（2026-10-04 完了。`docs/app-workflow.md` §4.7）。
      `[requirements] pin-roles`（このアプリが引く役割名）、既定のボード、
      replay のパス、配線の意図を書く。
      - **カレントから上に探す**（cargo と同じ）。無くても `check` / `run` は動く
      - **`pin-roles` があると役割の照合が保証になる**（走査は出さない）。
        名前は `ROLE_NAMES` と突き合わせるので**デバイスに繋がなくても
        タイポが止まる**
      - `[defaults] board` は `--board` の既定、`i2c-replay` は
        `--i2c-replay` の既定（**toml のある場所基準**で解決する）
      - **未知のキーはエラー**（`deny_unknown_fields`）。`version` が
        CLI より新しければエラー。`"none"` は「この役割は無い」
      - **`[board.<name>.roles]` は読んで検証するだけ**で、まだ使わない。
        デバイスへ押し込むのは `config apply`（§3.9 / §5-9 の決定後）
      - **`wasmicon new` が雛形として出すところまで入った**（2026-10-04）。
        `tests/new.rs` が**自分で書いたものを自分で読み直せる**ことを見る
      **ABI 準拠のビルドフラグは書かせない**（§4.4 の 3 重化を 4 重にする）。
      マシン固有の値は `wasmicon.local.toml`（gitignore）に分ける
      - `[requirements] pin-roles` があると §4.3 の役割名の照合が
        **参考から保証に変わる**。検証は `ports/common` の `ROLE_NAMES` と
        突き合わせるだけなので**デバイスに繋がなくてもタイポが止まる**
      - `deploy` は toml とデバイスの実効マップを毎回照合し、**黙って適用しない**
        （駆動されるピンが変わるので物理的に危ない）
- [x] **`apps/` に `wasmicon.toml` を 5 つ置いてドッグフードする**（2026-10-07。
      ファームが既定の表を持たなくなったので必須になった。`tests/apps.rs` が
      「各アプリに toml があり、宣言した役割が rp2350 / esp32s3 の配線表に揃う」を見る）（§4.7 が
      「この repo の `apps/` をドッグフードするなら 5 つになる」と書いている
      ところ）。**今 1 つも無い**ので、CLI のマニフェスト経路は合成ファイルの
      単体テストでしか踏まれていない
      - **これが一番効くのは AssemblyScript。** 役割名の走査は
        データセグメントの文字列を拾う推測で、**AS は UTF-16 なので一切
        当たらない**（§4.3 に実測を記録済み）。`blink-as` /
        `sensor-display-as` は宣言を書いて初めて照合される
      - `missing_declared_roles` は `fails()` に入るので、宣言すると
        **参考からハードな検査になる**。3 ポートの `ROLES` は同一
        （`led` / `lcd-cs` / `lcd-dc` / `lcd-rst`）なので全ボードで通る
      - `[defaults] board` でアプリのディレクトリからフラグ無しで
        `check` / `run` が通る。`sensor-display-*` は
        `[defaults] i2c-replay` を `verify/sht4x-replay.txt` に向ける
        （§1.3 で実機の記録に差し替えるファイル）
      - **既存のテストは壊れない。** `manifest::find` は上に向かって探すので、
        `apps/<app>/wasmicon.toml` はそのディレクトリから呼んだときだけ効く
        （既存のテストはリポジトリルートから走り、`--board` を明示している）
      - **`tests/flags.rs` と同じ理屈で「0 件を黙って成功にしない」検査を
        付ける**: `apps/` の各アプリに `wasmicon.toml` があることを見る
        （無いアプリが増えたら落ちる）
      - **`apps/` の構造は置き換えない**（検討して却下、2026-10-04）。
        `apps/` は 5 つを 1 workspace で CI ごとビルドするモノレポで、
        `wasmicon new` が出すのは単体で立つ 1 アプリ。独立させると
        `target/` が 5 つに割れて `wasmicon-hal` が 5 回コンパイルされ、
        `include_bytes!`（5 箇所）と `-p` を使うテストヘルパ（4 箇所）と
        CI の 3 ジョブを書き換えることになる。得るのは
        `.cargo/config.toml` の監視対象が 1 → 5 になることだけで、
        **その検証は `tests/flags.rs` と `tests/new.rs` で既に入っている**
- [x] **役割マップの設定スロット**（2026-10-07、1 段の最小版。§3.9）。アプリスロットの
      直後の 4 KiB に `"WMCR"` + テキストの表。`deploy` がアプリと 1 本の画像で書き、
      ファームが起動時に読む（`ports/common` の `roles`）。**残り**: プロトコル経由の
      `config` コマンドと `info` での読み戻し（2 段）。以下は当初の計画: 設定セクタ +
      `config` コマンド + デバイス側の検証（予約ピン・バスのピン・重複を弾く）。
      ~~CRC が合わなければ既定値で起動~~ → **既定の表は持たないので、役割を配らずに
      起動**してバナーに理由を出す（2026-10-07）。identity に実効マップの CRC-32 を
      入れるのは `info`（§3.8）と一緒に
- [ ] **USB 制御チャネル**（2 段）。ESP32-S3 は `esp-hal` の
      `usb::usb_serial_jtag`（追加クレート不要）、RP2350 / RP2040 は
      `rp235x-hal::usb` + `usbd-serial`。**制御は USB、トレースは UART0 に分ける**
      - **`hw-probe` feature を `probe` コマンドにする**（`docs/app-workflow.md` §3.6）。
        焼き直さずに切り分けられ、build 構成が 1 つ減って variant が
        `trace` あり / なしの 2 つに収束する
- [ ] **HTTP**（3 段）。§5-3 / §5-4 / §5-5 の判断が先
- [ ] **bindings の配布**（crates.io / npm）。**`new` は 2026-10-04 に入った**が、
      依存はパスで書いている（上）。出す先と版の付け方はまだ決め打ちしない。
      出したら `new` の既定をパスから版指定に変える
