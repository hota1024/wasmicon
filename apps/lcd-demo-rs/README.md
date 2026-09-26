# lcd-demo-rs

ILI9341 に絵を出すだけのデモ。**センサーは要らない。LCD を 1 枚繋ぐだけで動く。**

`sensor-display-rs` は SHT40 を I2C で読むので、センサーが無いと
（I2C が開けない時点で戻って）画面に何も出ない。こちらは SPI と GPIO しか
使わないので、ディスプレイだけで完結する。

Rust 版のみ。AssemblyScript の対になるものは用意していない
（`sensor-display-*` と違い、2 実装の一致を見るためのアプリではないため）。

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

## 配線（Raspberry Pi Pico 2 W）

`docs/abi-spec.md` §8 の既定のまま。**この配線は 2026-09-26 に Pico 2 W 実機で
確認済み**（`docs/verification-report.md` §6）。
違うピンに繋ぎたいときは `ports/rp2350/src/board.rs` の `ROLES` と
`SPI0_SCK` / `SPI0_MOSI` / `SPI0_MISO` を直す（abi-spec §8 の表も合わせる）。

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

- **3.3 V ロジックの SPI モジュールであること。** 5 V 専用の基板や、
  8080 パラレル設定の基板は繋がらない
- `LED`（バックライト）を繋がないと、描けていても真っ暗にしか見えない
- このデモは MISO を読まないので `SDO` は未接続でよい
- タッチパネル付きのモジュールの `T_*` 系は全て未接続でよい

シリアル（トレースとログ）は **UART0: GP0=TX(1 番ピン) / GP1=RX(2 番ピン)、
115200 8N1**。USB ではないので、USB-シリアル変換が要る（GND も共通にする）。

## ビルドと書き込み

```bash
# 1. ゲストを作る（ports 側が include_bytes! で取り込む）
cd apps && cargo build --release && cd ..

# 2. ポートをデモ入りで作る
cd ports/rp2350 && cargo build --release --features guest-lcd-demo

# 3. BOOTSEL を押しながら USB を挿して、焼く
picotool load -u -v -x -t elf target/thumbv8m.main-none-eabihf/release/wasmicon-rp2350
```

`picotool` は 2.0 以降が要る（RP2350 対応）。`elf2uf2-rs` は RP2040 用で使えない。

`--features guest-lcd-demo` を外すと既定の blink に戻る。

### トレースを切ると速い

既定の `trace` feature は host call を 1 件ずつ UART に出す。このデモは
host call が約 7,200 件あるので、115200 baud では描き切るまで 1 分ほどかかる
（画面が上から順に埋まっていく様子は見える）。

絵だけ見たいなら切る:

```bash
cargo build --release --no-default-features --features guest-lcd-demo
```

逆に**最初のブリングアップではトレースを付けたまま**にする。
SPI が動いていないときに、どこまで進んだかが分かる唯一の手掛かりになる。

## 動かないとき

この構成は Pico 2 W 実機で動くことを確認してある
（`docs/verification-report.md` §6）。それでも出ないときに疑う順:

1. **UART に何も出ない** — 配線（TX/RX の向き、GND）と 115200 8N1、
   焼けているか。バナー `wasmicon rp2350` が最初に出る
2. **`spi open failed` / `gpio open failed` が出る** — ポート側。
   役割名が `ROLES` に無いか、GPIO 番号が `RESERVED` に入っている
3. **トレースは最後まで流れるのに画面が真っ暗** — バックライト（`LED` ピン）、
   `RESET` の配線、電源。ILI9341 は 3.3 V
4. **表示が出るが化けている** — DC の配線を最初に疑う。次に SPI のクロック。
   `SPI_HZ`（`src/lib.rs`）を 4 MHz あたりまで落として切り分ける。
   配線が長いブレッドボードだと 15 MHz は通らないことがある
5. **赤と青が逆、または色が反転** — MADCTL (`0x36`) の値。
   `src/ili9341.rs` の `init` が `0x28`（横向き・BGR）を送っている
