# 残作業

最終更新: 2026-09-29

**全 6 フェーズのソフトウェア側は完了**し、CI も green。残っているものをここに集約する。
散らばると更新漏れで嘘になるので、**残作業はこのファイルだけに書く**。
決定済みの事項と経緯は `docs/handoff.md`、検証の現状は `docs/verification-report.md`。

---

## 1. 実機が要るもの

**Raspberry Pi Pico 2 W と ESP32-S3 DevKitC-1 は手元にある**（どちらも 2026-09-26 に
`lcd-demo-rs` で動作確認済み。`docs/verification-report.md` §6 / §7）。
**温湿度センサーは SHT40 が手元にある**（2026-09-29。`docs/handoff.md` §2 の
決定 9 を SHT31/SHT30 から SHT4x に変更し、ゲストのドライバを直した）。
**未入手は Raspberry Pi Pico WH（RP2040）だけ**で、RP2040 の項目はここで止まって
いる。I2C と sensor-display の**実装と動作確認は Pico 2 W と ESP32-S3 で進められる**
ようになった（ポートの I2C 実装が §1.2 に残っている）。
**ただし Phase 4/5/6 の完了条件は §1.3 のとおり ESP32-S3 と Pico WH の 2 ボードで
定義されており、変えない。**手元の 2 枚で通しても Phase 5/6 は完了にならない。

### 1.1 オーナーに聞くこと

- [ ] **実機の配線**。`docs/abi-spec.md` §8 の表（I2C/SPI のピン、役割名 → GPIO 番号）が実機と合っているか
      - RP2350 の LCD 側（`lcd-cs` / `lcd-dc` / `lcd-rst`、SCK=GP18 / MOSI=GP19）は
        2026-09-26 に実機で確認済み。**残るのは `led`、I2C (SDA=GP4 / SCL=GP5)、
        および RP2040 / ESP32-S3 の全て**
      - ESP32-S3 の LCD 側（CS=GPIO10 / DC=GPIO14 / RST=GPIO15、SCK=GPIO12 /
        MOSI=GPIO11）は 2026-09-26 に実機で確認済み。配線表は
        `apps/lcd-demo-rs/README.md`。**残るのは `led` と I2C
        (SDA=GPIO8 / SCL=GPIO9)**
      - **I2C は SDA / SCL に外部 10 kΩ のプルアップが要る。** 両ポートとも
        内部プルアップを有効にしているが（esp-hal の `connect_pin` が必ず
        `Pull::Up` を掛けるので rp2350 もそれに揃えた）、RP2350 は 50..80 kΩ、
        ESP32-S3 は約 45 kΩ と弱く、`standard` (100 kHz) の短い配線で
        かろうじてという程度。モジュール側に載っていることが多いので、
        **載っているなら足さなくてよい**
- [ ] **シリアルの接続方法**。RP2040 / RP2350 は UART0 (GP0/GP1)、ESP32-S3 は UART0 (GPIO43/44) を前提にしている
      - RP2350 は UART0 + USB シリアル変換 (CP2102N) で 2026-09-26 に確認済み
      - ESP32-S3 も UART0 (GPIO43/44) で 2026-09-26 に確認済み。**手元のボードは
        `USB-UART` 側のブリッジが CH343 (VID 0x1A86 / PID 0x55D3) で、macOS では
        `/dev/cu.usbmodem*` として見える**（CP2102N ではないので `usbserial` を
        探すと見つからない）。書き込みは `USB-OTG` 側（USB-Serial-JTAG）からも通る
- [ ] **モジュールの型番**。ILI9341 は 3.3V ロジックの SPI 版を前提にしている
      - センサーは **SHT40 で確定**（2026-09-29）。ただし **I2C アドレスは
        サフィックスで変わる**（-AD1B が 0x44）。ドライバは `0x44` のままなので、
        **手元の品種の刻印かバススキャンで確認すること**。違っていれば **3 箇所**:
        `apps/sensor-display-rs/src/sht4x.rs` と
        `apps/sensor-display-as/assembly/sht4x.ts` の `ADDRESS`、および
        `ports/host/tests/apps.rs` の `SHT4X_ADDR`（host テストが期待する
        トレースの値。直さないと「計測コマンドが違う」という紛らわしい
        メッセージで落ちる）
- [ ] **役割名**。`led` / `lcd-cs` / `lcd-dc` / `lcd-rst` を既定のまま確定扱いで進めている。変えるなら 3 箇所（`wit/board.wit` のコメント、abi-spec §8 の表、各ポートの `ROLES`）
- [ ] **`led` に外付け LED を充てている**。どのボードもオンボード LED が素の GPIO ではないため（Pico W/WH と Pico 2 W は CYW43439、DevKitC-1 は WS2812）。Pico 2（無線なし）だけは GP25 が素の LED だが、Pico 2 W と揃えて外付けにしている
- [x] **RP2350 ボードの品種** → **Pico 2 W**（RP2350A、GP0..GP29）で確定。`NUM_GPIO` は 30 のままでよい
- [ ] **RP2350 を Arm だけで見るか**。`ports/rp2350` は Cortex-M33（`thumbv8m.main-none-eabihf`）のみ。RISC-V (Hazard3) でも同じトレースが出るかは v0.1 の検証範囲に入れていない

### 1.2 実装

- [ ] **`ports/rp2040` の I2C / SPI**。現在は `unsupported` を返す。Pico WH が
      未入手なので着手していない。足すときは rp2350 の実装をそのまま持って
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
      上限を付けて `timeout` を返す。**実機では未検証**（→ §1.3）
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
- [ ] **SHT40 を実機で読む**（2026-10-04 時点で次の一手）。`ports/rp2350` と
  `ports/esp32s3` の I2C は実装したが**一度も実機で動かしていない**。
  sensor-display を Pico 2 W と ESP32-S3 で走らせ、host のトレースと
  突き合わせる。最初に疑うところは §1.4 に足した
  - **先に I2C アドレスの確認が要る**（§1.1）。ドライバは 0x44 のまま
- [ ] `verify/sht4x-replay.txt` を**実機から記録した応答**に差し替える（現在は合成データ）
- [ ] 結果を `docs/verification-report.md` に反映する

### 1.4 実機で最初に疑うところ

**RP2350 と ESP32-S3 は観測済み**（どちらも 2026-09-26）。`lcd-demo-rs` を Pico 2 W と
ESP32-S3 DevKitC-1 で走らせ、host call のトレースが host ポートと完全一致し
（どちらも 14,352 行、`spi.write` の CRC-32 3,272 件を含む）、画面にも絵が出た
（`docs/verification-report.md` §6 / §7）。以下の懸念のうち **GPIO / SPI のレジスタ
設定に関するもの**は RP2350 / ESP32-S3 では解消済み（`ARENA` とネイティブスタックの
項目は解消ではなく、今も有効な注意書き）。ただし **ESP32-S3 は「トレースが完全一致
するのにピンが動かない」を実際に踏んでいる**（`out_sel`。下の項目）ので、
**トレースの一致だけでは GPIO が動いた証拠にならない**。**RP2040 は未観測のまま**で、
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
- **I2C は 2 ポートとも一度も実機で動かしていない**（2026-10-04 に実装）。
  コンパイルが通ることしか確かめていないので、ここが今いちばん疑わしい。
  最初の期待値は「`i2c.bus.open` は成功するのに `write` が `nack` を返す」。
  見る順番:
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
      - **I2C も同じ形の食い違いがある**（2026-10-04 に実装して判明）。
        `i2c.bus.open` の `index != 0` は両ポートが `unsupported`、
        7 bit の外のアドレスは両ポートが `invalid-argument` を返すが、
        **`ports/host` の mock はどちらも検査しない**。SHT4x は index 0 /
        0x44 しか使わないので踏まないが、揃えるなら SPI と同じく
        `ports/common` に検査を置くことになる
- [ ] **SPI の待ちループに上限を設けるか。** `ports/rp2350` の `spi_drain` の
      `BSY` 待ち、RESETS 完了待ち、`spi_write` / `spi_transfer` の `TNF` / `RNE`
      待ちはいずれも無制限に回る。クロックが止まる・ペリフェラルが固まると、
      UART に最後のトレース 1 行を残して無言で停止する。これは
      `ports/rp2350/src/main.rs` と §1.4 が「診断可能にする」と言っている
      失敗の形そのもの
      - `types.error-code` に `timeout` は既にあるので型は足りている。
        ただし**今まで返らなかった状態を返すようになる**ので ABI の変更
- [ ] **ゲストの `ili9341.rs` の重複を解消するか。** `apps/sensor-display-rs` と
      `apps/lcd-demo-rs` に 228 行 / 233 行の写しがある（**差分は module
      コメントだけ**。`diff -u` で 1 hunk に保ってある）。
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
