# lcd-demo-rs

ILI9341 に絵を出すだけのデモ。**センサーは要らない。LCD を 1 枚繋ぐだけで動く。**

`sensor-display-rs` は SHT4x を I2C で読むので、センサーが無いと
（I2C が開けない時点で戻って）画面に何も出ない。こちらは SPI と GPIO しか
使わないので、ディスプレイだけで完結する。

Rust 版のみ。AssemblyScript の対になるものは用意していない
（`sensor-display-*` と違い、2 実装の一致を見るためのアプリではないため）。

**Raspberry Pi Pico 2 (W) と ESP32-S3 DevKitC-1 で同じ `.wasm` が動く。**
ピン番号は `board.pin-by-role` でホストに問い合わせるので、ゲスト側に
ボード固有のものは入っていない（abi-spec §8）。可搬性の確認は
「同じバイナリを 2 ボードで焼いてトレースを突き合わせる」で行う
（`docs/verification-report.md` §5）。

## 出るもの

320×240 の横向き。上から順に:

| 要素 | 何を見ているか |
|---|---|
| 青い帯に `WASMICON` / `RP2350` | 文字の描画 |
| カラーバー 8 本（赤・橙・黄・緑・シアン・青・マゼンタ・白） | 色順。**赤と青が入れ替わっていたら** MADCTL (`0x36`) の BGR ビット |
| グレーのランプ 16 段 | RGB565 の詰め方。段が色付いて見えたらビット位置がずれている |
| 緑の四角が左右に 1 往復 | `time.sleep-ms` と繰り返しの描画 |
| `HELLO PICO 2` / `ILI9341 SPI` | 文字の描画 |
| 外周 1 px の白い枠 | アドレス指定の端。`x + w == 320` / `y + h == 240` ちょうどの矩形 |

**ESP32-S3 でも画面には `RP2350` と `HELLO PICO 2` と出る。これは意図的。**
このデモの目的は「同一バイナリが 2 ボードで同じ host call 列を出す」ことの
確認なので、ボードごとに文字を変えると比べるものが無くなる。文字を変えると
`.wasm` が変わり、`docs/verification-report.md` §6 に記録した 14,352 行という
トレースも取り直しになる。

## 配線

### Raspberry Pi Pico 2 W

**この配線は 2026-09-26 に Pico 2 W 実機で確認済み**（`docs/verification-report.md` §6）。
違うピンに繋ぎたいときは、役割（`lcd-cs` / `lcd-dc` / `lcd-rst`）は
**`wasmicon.toml` の `[board.rp2350.roles]`**（`deploy` が設定スロットに書く。
ファームは焼き直さなくてよい）、SPI のピンは `ports/rp2350/src/board.rs` の
`SPI0_SCK` / `SPI0_MOSI` / `SPI0_MISO` を直す。

| ILI9341 モジュール | Pico 2 W | ピン番号（物理） |
|---|---|---|
| `VCC` | 3V3 OUT | 36 |
| `GND` | GND | 23 / 28 など |
| `CS` | GP17 | 22 |
| `RESET` | GP21 | 27 |
| `DC` / `D/C` / `RS` | GP20 | 26 |
| `SDI` / `MOSI` | GP19 | 25 |
| `SCK` / `SCL` | GP18 | 24 |
| `LED` / `BL` | 3V3 OUT | 36（VCC と同じピン） |
| `SDO` / `MISO` | GP16（繋がなくてよい） | 21 |

**LCD 側は全て基板の右列に集まっている。** USB を上にして持つと、右列は
下から上へ 21, 22, 23, ... 40 と並ぶ（左列は上から下へ 1..20）。21 番が
右下の角。シリアルの GP0/GP1 だけが左上（1 番・2 番）。

**`3V3(OUT)` は 36 番の 1 本しかない。** `VCC` と `LED` の 2 つに供給するので、
ブレッドボードの 3.3V レールに一度渡してから分岐する（または LCD 側で
2 つを繋ぐ）。隣の 37 番 `3V3_EN` はレギュレータの制御入力で、電源ではない。
GND は 8 本ある（3 / 8 / 13 / 18 / 23 / 28 / 33 / 38）。

- **`RESET` に 10 kΩ のプルアップ（`RESET` ↔ `3V3`）を入れておくと確実。**
  手元の Pico 2 W では無くても絵は残ったが、それは保証ではなく観測にすぎない
  （下の「描き終わると絵が消えて白くなる」。ESP32-S3 では無いと白に戻った）
- **3.3 V ロジックの SPI モジュールであること。** 5 V 専用の基板や、
  8080 パラレル設定の基板は繋がらない
- `LED`（バックライト）を繋がないと、描けていても真っ暗にしか見えない
- このデモは MISO を読まないので `SDO` は未接続でよい
- タッチパネル付きのモジュールの `T_*` 系は全て未接続でよい

シリアル（トレースとログ）は **UART0: GP0=TX(1 番ピン) / GP1=RX(2 番ピン)、
115200 8N1**。USB ではないので、USB-シリアル変換が要る（GND も共通にする）。

### ESP32-S3 DevKitC-1

**この配線は 2026-09-26 に DevKitC-1 実機で確認済み**（`docs/verification-report.md` §7）。
違うピンに繋ぎたいときは、役割は **`wasmicon.toml` の `[board.esp32s3.roles]`**、
SPI のピンは `ports/esp32s3/src/board.rs` の `SPI2_SCK` / `SPI2_MOSI` / `SPI2_MISO` を直す。

| ILI9341 モジュール | ESP32-S3 DevKitC-1 |
|---|---|
| `VCC` | `3V3` |
| `GND` | `G`（`GND`） |
| `CS` | `GPIO10` |
| `RESET` | `GPIO15` |
| `DC` / `D/C` / `RS` | `GPIO14` |
| `SDI` / `MOSI` | `GPIO11` |
| `SCK` / `SCL` | `GPIO12` |
| `LED` / `BL` | `3V3`（`VCC` と同じ） |
| `SDO` / `MISO` | `GPIO13`（繋がなくてよい） |

**DevKitC-1 はシルクに GPIO 番号がそのまま書いてある**ので、Pico のように
物理ピン番号から引く必要はない。上の 6 本はすべて `IO10`..`IO15` の並びで探せる。

**`RESET` に 10 kΩ のプルアップ（`RESET` ↔ `3V3`）を足すこと。** 無くても絵は
出るが、**手元の ESP32-S3 では描き終わった瞬間に消えて白くなった**
（理由は下の「描き終わると絵が消えて白くなる」）。

`GPIO11` / `GPIO12` / `GPIO13` は SPI2 (FSPI) の IO_MUX 既定ピンでもある。
ドライバは GPIO マトリクス経由で繋ぐので必須ではないが、既定に合わせてある。

`3V3` は両列にある（左列の先頭 2 本と右列に 1 本）ので、`VCC` と `LED` を
別のピンから取れる。Pico と違って 1 本しかないという制約は無い。

シリアル（トレースとログ）は **UART0: GPIO43=TX / GPIO44=RX、115200 8N1**。
**DevKitC-1 では `USB-UART` と書かれた USB-C ポートが USB-シリアルブリッジ経由で
ここに直結している**ので、そのケーブル 1 本で書き込みとトレースの取り込みが
両方できる（USB-シリアル変換を別に用意しなくてよい）。もう一方の `USB-OTG`
ポートは native USB で、このファームウェアは何も出さない（書き込みだけなら
USB-Serial-JTAG 経由で通る）。

**ブリッジの型番は個体差がある。** 公式の回路図は CP2102N（macOS では
`/dev/cu.usbserial-*`）だが、手元のボードは **CH343**（VID 0x1A86 / PID 0x55D3）で
**`/dev/cu.usbmodem*`** に見えた。`usbserial` を決め打ちで探すと見つからないので、
`ls /dev/cu.usb*` で確かめること。

- **どちらのボードでも `LED`（バックライト）は 3.3 V に繋ぐ。** 繋がないと
  描けていても真っ暗にしか見えない
- ILI9341 モジュールは 2 枚要らない。1 枚を差し替えて順に焼けばよい

## ビルドと書き込み

**ファームにアプリは入っていない**（2026-10-07 に内蔵アプリを外した。
`docs/app-workflow.md` §3.3）。ファームを一度焼き、このデモはスロットに焼く。
**2 ボードに焼くのは同じ `lcd_demo_rs.wasm`**（ゲストは作り直さない）。

```bash
# ゲストと CLI を作る
(cd apps && cargo build --release -p lcd-demo-rs)
cargo build --release -p wasmicon-cli
w=./target/release/wasmicon
app=apps/target/wasm32-unknown-unknown/release/lcd_demo_rs.wasm
```

### Raspberry Pi Pico 2 (W)

```bash
# 1. ファームを作る（apps/ に依存しない）
(cd ports/rp2350 && cargo build --release)

# 2. BOOTSEL を押しながら USB を挿して、ファームを焼く。
#    -x を付けないと BOOTSEL のまま残るので、続けて 3 が焼ける
picotool load -u -v -t elf ports/rp2350/target/thumbv8m.main-none-eabihf/release/wasmicon-rp2350

# 3. デモをスロットに焼く（ここで起動する）。トレースは UART0 から取る
$w monitor --port /dev/cu.usbserial-XXXX --idle 0 -o pico.log &   # 先に開く
$w deploy "$app" --board rp2350
```

`picotool` は 2.0 以降が要る（RP2350 対応）。`elf2uf2-rs` は RP2040 用で使えない。
2 回目以降、ファームを変えないならスロットだけ焼き直せばよい（3 だけ）。

### ESP32-S3 DevKitC-1

```bash
# 1. ファームを作って焼く（build.sh が ~/export-esp.sh を読む）
sh ports/esp32s3/build.sh build --release
espflash flash --non-interactive \
  ports/esp32s3/target/xtensa-esp32s3-none-elf/release/wasmicon-esp32s3

# 2. デモをスロットに焼いて、そのままトレースを取り込む
$w deploy "$app" --board esp32s3 --monitor -o esp32s3.log
```

`espflash` が繋ぐ先は DevKitC-1 の **`USB-UART` ポート**。複数の USB シリアルが
見えているときは `--port` で指定する（**ブリッジの型番で名前が変わる** — 上の
「配線」参照）。

**USB からのリセットが効かずに走らないことがある**（2026-10-07 に踏んだ。
`docs/verification-report.md` §12「ESP32-S3 で詰まったこと」）。そのときは
書き込みを `USB-OTG` 側から、トレースを `USB-UART` 側で取り、基板の **EN
（`RST`）ボタン**で起動する。

**取り込んだログに NUL が混ざっていても `wasmicon trace diff` /
`verify/diff-traces.sh` は落とす。** リセットや電源投入の瞬間にライン・ノイズで
出るもので、放っておくと `grep` がファイルをバイナリと判断して 1 行しか返さない。

### トレースを切ると速い

既定の `trace` feature は host call を 1 件ずつ UART に出す。このデモは
host call が約 7,200 件あるので、115200 baud では描き切るまで 1 分ほどかかる
（画面が上から順に埋まっていく様子は見える）。

絵だけ見たいならファームから切る（アプリは焼き直さなくてよい）:

```bash
(cd ports/rp2350 && cargo build --release --no-default-features)
sh ports/esp32s3/build.sh build --release --no-default-features
```

逆に**最初のブリングアップではトレースを付けたまま**にする。
SPI が動いていないときに、どこまで進んだかが分かる唯一の手掛かりになる。

## 描き終わると絵が消えて白くなる（両ボード共通）

**バグではない。** `run()` を抜けるときゲストがハンドルを解放し、
`docs/abi-spec.md` §5.2 の規定でピンが**入力・プル無し**に戻る。その結果
**LCD の `RESET` が浮き**、モジュール側にプルアップが無いとパネルがリセットして
画面が白に戻る（2026-09-26 に ESP32-S3 実機で観測。
`docs/verification-report.md` §7「原因 3」）。

**`RESET` ↔ `3V3` に 10 kΩ のプルアップを入れれば直る**（2026-09-26 に ESP32-S3
実機で確認。出荷する構成のまま絵が残る）。既存の `lcd-rst` の線は外さない。
ESP32 が `RESET` を Low に駆動するときプルアップ経由で 330 µA 流れるだけで、
初期化のリセットパルスは問題なく出る。抵抗値は 4.7 kΩ〜100 kΩ なら何でもよい。

リセット入力を浮かせないのは回路としてまっとうで、コードも `.wasm` も触らずに
済む。**ゲスト側でハンドルを解放せずに終わっても回避できない** —
`docs/abi-spec.md` §5.2 は「`run` から戻ったとき、ホストは残っている全ハンドルを
drop する」とも定めているので、解放は必ず起きる。

**症状が出るかどうかはボードで違う。** 手元の実測（2026-09-26、同じモジュール）:

| ボード | プルアップ無し |
|---|---|
| ESP32-S3 | **白に戻る** |
| Raspberry Pi Pico 2 W | **残る** |

仕組みは両ポートで同一（§5.2 でピンは高インピーダンスに戻り、線のアイドル
レベルをソフトウェアが決めていない）が、**浮いた線がパネルのしきい値を割るかは
パッドのリーク量と容量で決まる**ので結果が違う。Pico 2 W で残るのは保証ではなく
観測にすぎない。**どちらのボードでもプルアップを入れておくのが確実。**

## 動かないとき

この構成は Pico 2 W と ESP32-S3 DevKitC-1 の両方の実機で動くことを確認してある
（`docs/verification-report.md` §6 / §7）。それでも出ないときに疑う順:

1. **UART に何も出ない** — 配線（TX/RX の向き、GND）と 115200 8N1、
   焼けているか。バナー `wasmicon rp2350` が最初に出る
2. **`role not found` が出る** — 配線表が設定スロットに無い。バナーの
   `wasmicon: roles …` の行を見る。`roles empty, no roles` なら、
   `wasmicon.toml` のあるディレクトリから `deploy` し直す（表も一緒に書かれる）
3. **`spi open failed` / `gpio open failed` が出る** — ポート側
4. **トレースは最後まで流れるのに画面が真っ暗** — バックライト（`LED` ピン）、
   `RESET` の配線、電源。ILI9341 は 3.3 V
5. **表示が出るが化けている** — DC の配線を最初に疑う。次に SPI のクロック。
   `SPI_HZ`（`src/lib.rs`）を 4 MHz あたりまで落として切り分ける。
   配線が長いブレッドボードだと 15 MHz は通らないことがある
6. **赤と青が逆、または色が反転** — MADCTL (`0x36`) の値。
   `src/ili9341.rs` の `init` が `0x28`（横向き・BGR）を送っている

### ESP32-S3 でだけ出ないとき

`ports/esp32s3` も実機で動くところまで来ている（`docs/verification-report.md` §7）。
**§7 で実際に踏んだ順**に並べてある:

1. **バナー `wasmicon esp32s3` も出ない** — UART の口を間違えている。
   DevKitC-1 の USB-C は 2 つあり、トレースが出るのは **`USB-UART` 側**
   （USB-シリアルブリッジ → GPIO43/44）。`USB-OTG` 側は native USB で、
   このファームウェアは何も出さない。デバイス名はブリッジの型番で変わるので
   `ls /dev/cu.usb*` で確かめる
2. **`spi open failed`** — `spi_open` が `unsupported` を返している。
   要求周波数が出せる範囲の外（APB 80 MHz のとき 78.125 kHz 未満、
   または 80 MHz 超）のときだけそうなる。`SPI_HZ` は 16 MHz なので通常は起きない
3. **トレースが host と完全一致するのに画面が真白** — §7 で実際に 2 段踏んだ形。
   **トレースはここを一切教えてくれない**（host call は全部成功を返す）。
   切り分けはゲストを通さずポート層を直接叩く
   `sh ports/esp32s3/build.sh run --release --features hw-probe`
   （`ports/esp32s3/src/probe.rs`）で行う
   - まず `ports/esp32s3/src/board.rs` の `SIG_GPIO_OUT`。**S3 は 256**
     （128 は C3 / C6 の値）。間違っていると `GPIO_OUT` / `GPIO_ENABLE` は
     正しく読めるのにピンがパッドで一切動かない
   - 次に**ブレッドボードの配線の接触**。§7 では挿し直したら出た
   - `SPI2_SCK` / `SPI2_MOSI` を GPIO12 / GPIO11 から変えたなら、
     定数と abi-spec §8 の表の両方を直したか
4. **途中で止まって最初からやり直す** — 電源。ブレッドボードの 3V3 で
   バックライトまで賄うと足りないことがある（DevKitC-1 の 3V3 は
   オンボードレギュレータ出力）
5. **絵は出るが、描き終わると白に戻る** — 上の「描き終わると絵が消えて
   白くなる」。`RESET` のプルアップで直る
6. **トレースが途中で切れて、以降何も出ない** — ネイティブスタック。
   `esp-hal` のリンカスクリプトは `.stack` を DRAM の余りに置くので、
   `ports/esp32s3/src/main.rs` の `ARENA`（300 KB）を増やすとスタックが削れる
   （今は 17.4 KiB）。溢れると panic handler に入るが、**理由は出せない**
   （シリアルがボード側にあって panic handler から届かない）ので、
   症状は「無言で止まる」だけになる
