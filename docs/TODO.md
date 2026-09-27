# 残作業

最終更新: 2026-09-26

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
      - ESP32-S3 の LCD 側（CS=GPIO10 / DC=GPIO14 / RST=GPIO15、SCK=GPIO12 /
        MOSI=GPIO11）は 2026-09-26 に実機で確認済み。配線表は
        `apps/lcd-demo-rs/README.md`。**残るのは `led` と I2C
        (SDA=GPIO8 / SCL=GPIO9)**
- [ ] **シリアルの接続方法**。RP2040 / RP2350 は UART0 (GP0/GP1)、ESP32-S3 は UART0 (GPIO43/44) を前提にしている
      - RP2350 は UART0 + USB シリアル変換 (CP2102N) で 2026-09-26 に確認済み
      - ESP32-S3 も UART0 (GPIO43/44) で 2026-09-26 に確認済み。**手元のボードは
        `USB-UART` 側のブリッジが CH343 (VID 0x1A86 / PID 0x55D3) で、macOS では
        `/dev/cu.usbmodem*` として見える**（CP2102N ではないので `usbserial` を
        探すと見つからない）。書き込みは `USB-OTG` 側（USB-Serial-JTAG）からも通る
- [ ] **モジュールの型番**。ILI9341 は 3.3V ロジックの SPI 版、SHT31 は I2C アドレス 0x44 を前提にしている
- [ ] **役割名**。`led` / `lcd-cs` / `lcd-dc` / `lcd-rst` を既定のまま確定扱いで進めている。変えるなら 3 箇所（`wit/board.wit` のコメント、abi-spec §8 の表、各ポートの `ROLES`）
- [ ] **`led` に外付け LED を充てている**。どのボードもオンボード LED が素の GPIO ではないため（Pico W/WH と Pico 2 W は CYW43439、DevKitC-1 は WS2812）。Pico 2（無線なし）だけは GP25 が素の LED だが、Pico 2 W と揃えて外付けにしている
- [x] **RP2350 ボードの品種** → **Pico 2 W**（RP2350A、GP0..GP29）で確定。`NUM_GPIO` は 30 のままでよい
- [ ] **RP2350 を Arm だけで見るか**。`ports/rp2350` は Cortex-M33（`thumbv8m.main-none-eabihf`）のみ。RISC-V (Hazard3) でも同じトレースが出るかは v0.1 の検証範囲に入れていない

### 1.2 実装

- [ ] **`ports/rp2040` の I2C / SPI**。現在は `unsupported` を返す。これが無いと sensor-display は実機で動かない
- [x] **`ports/rp2350` の SPI**。SPI0 (PL022) をレジスタ直叩きで実装した。
      **2026-09-26 に実機で確認済み**（`docs/verification-report.md` §6）。
      rp2040 に足すときは、周波数の丸め（要求値を超えない最大）と
      「送信後 `BSY` が落ちるまで戻らない」を揃えること
      （esp32s3 はこの 2 点を `esp-hal` のドライバが満たしている）
- [ ] **`ports/rp2350` の I2C**。まだ `unsupported`
- [x] **`ports/esp32s3` の SPI**。SPI2 (FSPI) を `esp-hal` の `spi::master`
      ドライバで実装した（GPIO と違いレジスタ直叩きにしていない）。rp2350 と
      揃えた点は `ports/esp32s3/src/board.rs` の module コメント。
      **2026-09-26 に実機で確認済み**（トレース 14,352 行完全一致 + ILI9341 に
      絵が出た。`docs/verification-report.md` §7）
- [ ] **`ports/esp32s3` の I2C**。まだ `unsupported`

### 1.3 検証（Phase 4 / 5 / 6 の完了条件）

- [x] **`lcd-demo-rs` が Pico 2 W で表示される**（2026-09-26 達成。詳細は
      `docs/verification-report.md` §6）。トレースが host ポートと 14,352 行完全一致し、
      画面にも絵が出た。**残るのは I2C 側**
- [ ] 両ボードで `blink-rs` / `blink-as` が動き、シリアルのトレースが host 版と一致（Phase 4）
- [x] **`lcd-demo-rs` が ESP32-S3 で表示される**（2026-09-26 達成。
      `docs/verification-report.md` §7）。トレースが host と 14,352 行完全一致し、
      画面にも絵が出た。**残るのは I2C 側**
- [ ] 4 通り（Rust/AS × 2 ボード）で表示が出る（Phase 5）
- [x] 同一 `.wasm` を両ボードで走らせ、`time` を除くトレースと SPI ピクセル CRC が完全一致（Phase 6）
      → **2026-09-26 に RP2350 と ESP32-S3 で達成**（どちらも host リファレンスと
      14,352 行完全一致、`spi.write` の CRC-32 3,272 件を含む。
      `docs/verification-report.md` §6 / §7）。**表示の一致は別途**（上の項目）
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
- ESP32-S3: GPIO / IO_MUX（MCU_SEL=1、GPIO マトリクスの **out_sel=256**）。
  **ここは 2026-09-26 に実機で踏んだ。** `out_sel` を 128（C3 / C6 など
  RISC-V 勢の値。ESP32 / S2 / S3 は 256）にしていたため、
  `GPIO_OUT` / `GPIO_ENABLE` は正しく読めるのにピンが
  一切動かなかった。症状は「host call のトレースは host と 14,352 行完全一致
  するのに ILI9341 が真白」。`docs/verification-report.md` §7
- ESP32-S3: SPI2 は `esp-hal` のドライバ任せなので信号番号を自前で持たない。
  上のような取り違えは起きない
- ESP32-S3: **ネイティブスタックは `ARENA` の残り**。`esp-hal` の
  リンカスクリプトは `.stack` を dram_seg の余りに置くので、`ARENA` を
  300 KB にしている今は 17.4 KiB しかない（`.bss` 315,496 B の直後、
  dram_seg の端 0x3FCDB700 まで）。インタプリタは呼び出しフレームを arena の
  配列に積むのでネイティブ再帰はしないが、`ARENA` を増やすとここが削れる。
  溢れると panic handler（理由を出せない）に入って無言で止まるので、
  「トレースが途中で切れて何も出ない」はこれを疑う

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
      （`594a63b` で `lcd-demo-rs` 側だけ修正）は両方にあり、今は振る舞いが違う
      - `sensor-display` 側も直せる。画面内の座標では送るバイト列が変わらない
        ので CRC は動かない。ただし `apps/README.md` §4 の規則どおり
        **AssemblyScript 版も同時に直す**必要がある（片方だけだと
        `sensor_display_rs_and_as_agree` が落ちる）
      - 共有クレートに切り出すならフォント表をパラメータにする。
        `apps/` の workspace メンバーが 1 つ増える
      - 分けたままにするなら、片方を直したらもう片方も見ることを
        `apps/README.md` に書く（AS 版も含めて 3 箇所になる）

---

## 3. 分かっている制限（今は困っていない）

直す必要が出たときのために書いておく。

- **`spi.transfer` と `i2c.write-read` は 128 バイトまで**（`ports/common` の `SCRATCH`）。送信元と受信先がどちらもゲストメモリにあり範囲が重なりうるので、送信側を一度写している。超えると `unsupported`。v0.1 の用途（SHT31 の 6 バイト、ILI9341 の ID 読み）には十分
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

- AoT コンパイル（`wasmicon_compiler`）
- HTTP での動的ロード
- Component Model の完全採用（resource type / async / component binary）
- インタプリタの最適化。今は `match` ループのまま。RP2040 で ILI9341 のテキスト描画が
  1 秒以内という目標は未計測（`docs/handoff.md` §5 Phase 2）
