# Wasmicon 検証レポート

作成日: 2026-09-10 / 対象: **本レポートを含むコミット時点**（Phase 6 の道具を入れた地点）

レポート自身のコミットハッシュはここに書けないので、`git log --oneline -- docs/verification-report.md`
で最初にこのファイルが入ったコミットを見ること。

docs/handoff.md §5 Phase 6 の成果物。**何がどこまで検証されたか**と、**まだ検証されて
いないこと**を分けて書く。

---

## 0. 結論

| 検証項目 | 状態 |
|---|---|
| ランタイムの Wasm 仕様適合（spec testsuite） | **達成** |
| インタプリタの正しさ（wasmtime との差分） | **達成** |
| Rust 版と AS 版が同じ host call 列を出す | **達成**（host 上、成功経路と失敗経路） |
| RP2350 実機で GPIO / SPI / ILI9341 の描画が動く | **達成**（2026-09-26、`lcd-demo-rs`。§6） |
| ESP32-S3 実機で host call のトレースが一致する | **達成**（2026-09-26、14,352 行完全一致。§7） |
| ESP32-S3 実機で ILI9341 に絵が出る | **達成**（2026-09-26。§7） |
| 同一バイナリが 2 ボードで同じトレースを出す | **達成**（2026-09-26、RP2350 と ESP32-S3。§6 / §7） |
| 4 通り（Rust/AS × 2 ボード）で表示が出る | **未達**（I2C は実装したが未検証。sensor-display は未観測） |

**Phase 6 の「同一バイナリが 2 ボードで同じトレースを出す」は満たした。**
残っているのは 4 通りの表示。I2C は `ports/rp2350` / `ports/esp32s3` で
実装したが**実機で一度も動かしていない**（`ports/rp2040` は未実装）。
→ `docs/TODO.md` §1.2 / §1.3。

2026-09-26 に `ports/esp32s3` の SPI2 を実装し、RP2350 と**同じ
`lcd_demo_rs.wasm`**（SHA-256 `fc470947…`）を実機で走らせた。**host call の
トレースは 14,352 行完全一致**で、これは RP2350 の実測と同じ数・同じ内容。
**画面にも絵が出た。** つまり **Phase 6 の「同一バイナリが 2 ボードで同じ
host call 列を出す」は実機で達成**した（§6 / §7）。

途中で ESP32-S3 のポートに実バグが 1 件あった（`GPIO_FUNCn_OUT_SEL` の値）。
**トレースが完全一致していても検出できない種類の失敗**だったので、§7 に
経緯ごと残してある。

---

## 1. ランタイムの仕様適合

WebAssembly spec testsuite（固定 SHA `34f1f40`）のコア 74 ファイルで
**22507 コマンドが通る**。

- 対象: `runtime/tests/spec.rs` の `FILES`
- 除外: 同ファイルの `EXCLUDED` に 11 分類（SIMD / GC / EH / tail-call /
  multi-memory / memory64 / linking など）を理由つきで列挙
- スキップ 548 件: 大半は `(module quote ...)` のテキスト形式モジュール。
  WAT パーサを持たないので扱えない。バイナリ形式の同等ケースは実行している

浮動小数の自前実装（`no_std` に無い `sqrt` / `floor` / `nearest` など）は
40 万件の乱数で std とビット単位一致することを別途確認した（`runtime/tests/float.rs`）。
**これはホスト（AArch64）での一致**であり、ソフトフロートに落ちる RP2040 や
f32 のみ FPU を持つ Xtensa での一致は未確認。

## 2. インタプリタの正しさ（wasmtime との差分）

`apps/` の 4 つのゲスト全部について、**同じ `.wasm` を自作インタプリタと
wasmtime 45 で走らせ、トレースが完全一致する**ことを確認した
（`verify/differential/tests/agree.rs`）。

肝は **HAL を共有している**こと。`wasmicon-port` の `Hal` は
`wasmicon_core::Resolver` を実装しているが、その `call` はエンジンに依存しない
（引数のスロット列とゲストメモリのスライスしか触らない）。同じ `Hal` を wasmtime
からも駆動しているので、トレースに差が出たらそれは**エンジンの差**、つまり
自作インタプリタのバグということになる。

これは実機と無関係に効く検証で、固定小数の計算・唯一の f32 演算・
1200 回を超える host call の順序すべてを含む。

## 3. Rust 版と AS 版の一致

`ports/host` で両ゲストを走らせ、**トレース全文が完全一致**する。

| ゲスト | 行数 | 経路 |
|---|---|---|
| blink | 22 | 成功 |
| sensor-display | 1215 | 成功 |
| sensor-display | 短 | SPI が `unsupported`（実機ポートの現状を模す） |
| sensor-display | 短 | センサー無応答 |

abi-spec §9 により `time` はトレースに出ないので、**全文一致がそのまま
docs/handoff.md §2-10 の「`time` を除く全 host call と結果が一致」の定義**になる。
`spi.write` のトレースは data の CRC-32 なので、一致は「送っているピクセルが
同一」を意味する。

失敗経路も検査しているのは、`ports/rp2040` の SPI と 3 ポートの I2C が
まだ `unsupported` を返すため（SPI は rp2350 / esp32s3 とも実装済み）。
成功経路だけ揃えても実機に持って行った瞬間に比較が意味を失う。

## 4. まだ検証されていないこと

### 4.1 実機での動作

**RP2350（§6）と ESP32-S3（§7）は観測済み。RP2040 はビルドが通るところまでで、
一度も焼いていない。**

- RP2040 の GPIO はレジスタ直叩き（SIO / IO_BANK0 / PADS_BANK0）で、
  型は通ったが一つも観測していない。実機で最初に起きることとして
  「blink が光らない」を想定すべき
- `ports/rp2040` の I2C / SPI は `unsupported` を返す（Pico WH 未入手）。
  `ports/rp2350` / `ports/esp32s3` の I2C は **2026-10-04 に実装したが、
  一度も実機で動かしていない**（コンパイルが通ることしか確かめていない）。
  **sensor-display はまだどのボードでも観測できていない**
- `ports/esp32s3` の SPI2 と GPIO は実機で動いた（§7）。ただし
  **host call の一致は「ゲストが正しいバイト列を HAL に渡した」ことしか言わない。**
  レジスタへの書き込みがパッドまで届いているかはトレースに現れない。
  §7 のバグはまさにそこを突いていた
- abi-spec §8 の配線は RP2350 の SPI / LCD 側（`lcd-cs` / `lcd-dc` / `lcd-rst`、
  SCK=GP18 / MOSI=GP19）だけ実機で確認できた（§6）。**`led` と I2C の配線、
  および他の 2 ボードは依然オーナー未確認**

### 4.2 ボード間の浮動小数の一致

sensor-display が唯一 f32 を使う温度バーの計算は、**ホスト 1 プラットフォーム
での一致しか確認していない**。RP2040 は `compiler_builtins` のソフトフロート、
ESP32-S3 と RP2350 は f32 のみハード FPU（非正規化数の扱いに設定依存がある）なので、
ここが Phase 6 の本来の実測対象。

RP2350 は hard-float ABI（`thumbv8m.main-none-eabihf`）でビルドしている。
f64 を速くする DCP は使っていない（`rp235x-hal` の `dcp-fast-f64` を入れると
`__aeabi_dadd` / `__aeabi_dmul` が差し替わるが、結果の一致を確かめていない）。

### 4.3 記録済み I2C 応答

`verify/sht4x-replay.txt` は T=23.44°C / RH=51.08% になる**合成データ**。
実機の SHT4x から記録したものへの差し替えが要る。
（バイト列は SHT31 前提だったころのまま。応答形式と CRC-8 は SHT3x / SHT4x で
同一で、**湿度の換算式だけが違う**ので表示値が 45.66% → 51.08% に変わった。）

### 4.4 CI

**2026-09-10 に初めて実行し、4 ジョブすべて green**（`v2` ブランチ、ubuntu-latest）。

初回は `root` ジョブの clippy で落ちた。CI の stable が 1.98.0 で手元が 1.97.1 で、
1.98 で入った `map_or_identity` に引っかかった。直して 2 回目で green。

- `root`: fmt / clippy / spec テスト（testsuite を固定 SHA で取得して実行）/
  生成物 diff ゼロ / check-sigs / diff-traces の self-test
- `guest`: apps workspace の fmt / clippy / wasm32 ビルド
- `rp2040`: thumbv6m のビルド
- `differential`: wasmtime との差分テスト 4 件

2026-09-22 に `rp2350` ジョブ（thumbv8m.main-none-eabihf のビルド）を足した。
手元では fmt / clippy / build とも通っているが、**CI で回したのはまだ見ていない**。

**wasmtime との一致は x86_64 Linux でも確認できた**（手元は AArch64 macOS）。
インタプリタの一致がホストのアーキテクチャに依存しないことの傍証にはなるが、
RP2040 / Xtensa のソフトフロートを跨いだ一致は依然として未検証（§4.2）。

`rust-toolchain.toml` は `channel = "stable"` の浮動。clippy の新しい lint や
rustfmt の出力変化で CI が突然落ちうる（今回まさにそれ）。**固定するかは未決**。

---

## 5. 実機で検証するときの手順

1. `ports/rp2040` / `ports/rp2350` / `ports/esp32s3` の I2C / SPI を実装する
   （**SPI は rp2350 / esp32s3 とも済み。残りは I2C と rp2040**）
2. abi-spec §8 の配線を確認し、役割名の表を実機に合わせる
3. 焼く
   - RP2040: ELF を `picotool load`、または `elf2uf2-rs` で UF2 にして BOOTSEL
   - RP2350: ELF を `picotool load -u -v -x -t elf`（RP2350 は picotool 2.0 以降が要る。
     `elf2uf2-rs` は RP2040 用で使えない）
   - ESP32-S3: `sh ports/esp32s3/build.sh run --release`（espflash）
4. シリアル（いずれも 115200 8N1）を捕まえてファイルに落とす
   - RP2040 / RP2350: UART0 (GP0=TX, GP1=RX)
   - ESP32-S3: UART0 (GPIO43/44、DevKitC-1 の `USB-UART` ポートが USB-シリアル
     ブリッジ経由で直結。`espflash flash --monitor` の出力をそのまま落とせる)。
     **ブリッジの型番は個体差がある**（公式の回路図は CP2102N だが、手元の
     ボードは CH343 で macOS では `/dev/cu.usbmodem*` に見える。§7 の条件表）。
     デバイス名を決め打ちせず `ls /dev/cu.usb*` で確かめること
5. 突き合わせる: `sh verify/diff-traces.sh pico.log esp32s3.log`
   - バナーとゲストの `[wasm]` 行は自動で落とす
   - `time` はトレースに出ず、役割名で引いた GPIO 番号は `role:led` に
     正規化済みなので、追加の加工は要らない
6. 実機の SHT4x 応答を記録して `verify/sht4x-replay.txt` を差し替える

### 5.1 LCD だけで 2 ボードの一致を測る（I2C を待たない経路）

`lcd-demo-rs` は SPI と GPIO だけを使う。SPI は rp2350 と esp32s3 の両方で
実装済みなので、**I2C を待たずに Phase 6 の「同一バイナリで同一トレース」を
LCD 側だけ先に測れる**。ゲストを 1 回だけ作り、2 ボードに同じものを焼く:

```bash
(cd apps && cargo build --release)
shasum -a 256 apps/target/wasm32-unknown-unknown/release/lcd_demo_rs.wasm
# 期待値（この記録を取った時点）:
# fc470947ac08b230ccdc59e04e09aa11105a5c8754efd00533858470f11d2453

(cd ports/rp2350 && cargo build --release --features guest-lcd-demo)
picotool load -u -v -x -t elf \
  ports/rp2350/target/thumbv8m.main-none-eabihf/release/wasmicon-rp2350
# → pico.log を取る

sh ports/esp32s3/build.sh run --release --features guest-lcd-demo | tee esp32s3.log
# monitor が色や接頭辞を付けて diff-traces.sh が 1 行も取れない場合の取り方は
# apps/lcd-demo-rs/README.md「トレースをファイルに落とす」

sh verify/diff-traces.sh pico.log esp32s3.log
```

`lcd-demo-rs` は f32 を使わないので、**これが一致しても §4.2 の浮動小数は
1 ミリも進まない**。SPI のバイト列（`spi.write` の CRC-32）と GPIO の順序が
2 ボードで同じであることだけが言える。

---

## 6. RP2350 実機の実測（2026-09-26）

`ports/rp2350` を初めて実機で動かした記録。**Phase 6 の完了条件そのものではない**
（あれは 2 ボードで定義されている。handoff §5）。Phase 5 の「表示が出る」のうち、
ディスプレイ側だけを SHT31 を待たずに切り分けたもの。
（この実測の時点では決定 9 は SHT31/SHT30 だった。センサーを SHT4x に
変えたのは 2026-09-29 で、**この記録より後**。記録の本文は当時のまま残す。）

### 条件

| | |
|---|---|
| ボード | Raspberry Pi Pico 2 W |
| ゲスト | `apps/lcd-demo-rs`（Rust）。SPI と GPIO のみ、I2C を使わない |
| 配線 | abi-spec §8 の既定（CS=GP17 / DC=GP20 / RST=GP21 / SCK=GP18 / MOSI=GP19、MISO 未接続） |
| SPI | 要求 16 MHz → 実際 15 MHz（`clk_peri` 150 MHz、切り下げ規則は `spi_divisors`） |
| シリアル | UART0 (GP0/GP1) 115200 8N1、CP2102N 経由 |
| 書き込み | `picotool` 2.3.1、`picotool load -u -v -t elf` → `picotool reboot -f` |
| ビルド | `cargo build --release --features guest-lcd-demo`（`trace` 有効） |

### 再現手順

比較相手の `host.log` は host ポートで作る（`pico.log` は UART をそのまま落としたもの。
バナーとログ行が混ざっていてよい）。

```bash
(cd apps && cargo build --release)
cargo run -q -p wasmicon-host -- --trace \
  apps/target/wasm32-unknown-unknown/release/lcd_demo_rs.wasm > host.log
sh verify/diff-traces.sh pico.log host.log
```

host 側だけなら CI が毎回見ている（`cargo test -p wasmicon-host --test apps`
の `lcd_demo_rs_runs_on_host`）。実機側のログは手元にしかない。

### 結果

- **host call のトレースが host ポートと完全一致**。`sh verify/diff-traces.sh pico.log host.log`
  → `一致: 14352 行`。`spi.write` の CRC-32 **3,272 件を含めて全て同じ**なので、
  ILI9341 に出たバイト列は host モデルの予測とビット単位で一致している
- 失敗ステータスは 1 件も無い（`pin.open` × 3、`spi.bus.open`、以降の
  `pin.write` / `spi.write` すべて `< 0`）
- ハンドルは `spi` → `rst` → `dc` → `cs` の順で解放された（`apps/README.md` §4）
- **画面に絵が出た。** カラーバーの左端が赤（MADCTL の BGR ビットが正しい）、
  文字が読める（`draw_text` の経路と DC の配線が正しい）

これで、懸念していた RP2350 固有の 2 点が実機で潰れた:

- **PADS_BANK0 の `ISO`**（リセット値 1）。落とし損ねていれば `pin.write` は
  成功を返すのに GPIO が無反応になっていた
- **SPI0 の RESETS 解除**。忘れていれば `spi.bus.open` 以降のレジスタ書き込みが
  素通りしていた

### この実測が言っていないこと

- **浮動小数の一致（§4.2）は 1 ミリも進んでいない。** `lcd-demo-rs` は f32 を
  使わない。あれは sensor-display の温度バーの話で、I2C が要る
- **グレーのランプの見え方は未確認。** RGB565 のビット位置は「左端が赤」で
  R と B の入れ替わりが無いことしか見ていない
- **2 ボード間の一致（Phase 6）は未達。** 突き合わせた相手は host の mock HAL で、
  2 枚目の実機ではない
- **`led` の役割名（外付け LED）は未検証。** `lcd-demo-rs` は LED を触らない
- **ESP32-S3 については何も言っていない**（§7 で別に測った）

---

## 7. ESP32-S3 実機の実測（2026-09-26）

`ports/esp32s3` を初めて実機で動かした記録。**トレースの一致と画面が出ることは
別の話**なので分けて書く。結論はどちらも達成だが、**トレースは一発で完全一致した
のに画面は真白**で、そこから 3 つの原因（うち 1 つは実バグ）を潰した。
その経緯も残してある — 同じ形の失敗は他のポートでも起きる。

### 条件

| | |
|---|---|
| ボード | ESP32-S3-WROOM-1 搭載ボード（`USB-UART` / `USB-OTG` の 2 ポート）。chip rev v0.2、flash 8 MB、PSRAM なし |
| ゲスト | `apps/lcd-demo-rs`。**RP2350 に焼いたものと同一ファイル**（SHA-256 `fc470947ac08b230ccdc59e04e09aa11105a5c8754efd00533858470f11d2453`） |
| 配線 | abi-spec §8 の既定（CS=GPIO10 / DC=GPIO14 / RST=GPIO15 / SCK=GPIO12 / MOSI=GPIO11、MISO 未接続） |
| SPI | 要求 16 MHz。APB 80 MHz なので 80/5 でちょうど 16 MHz |
| シリアル | UART0 (GPIO43/44) 115200 8N1 |
| 書き込み | `espflash` 4.5。`USB-OTG` 側（USB-Serial-JTAG）と `USB-UART` 側の CH343 のどちらからでも通る |
| ビルド | `sh ports/esp32s3/build.sh build --release --features guest-lcd-demo`（`trace` 有効） |

### 達成: トレースが host と完全一致

```
sh verify/diff-traces.sh esp32s3.log host.log
→ 一致: 14352 行
```

**RP2350 の実測（§6）と同じ 14,352 行**で、`spi.write` の CRC-32 3,272 件を含めて
全て同じ。失敗ステータスは 1 件も無く、`[wasm] lcd-demo done` まで到達した。

これで **「同一バイナリが 2 ボードで同じ host call 列を出す」は実機で確認できた**
（両ボードがそれぞれ同じ host リファレンスと完全一致したので、2 ボード間でも一致）。

`spi.bus.open(0, 16000000, 0)` も `pin.open` 3 件もすべて成功している。
つまり **GPIO / SPI2 のレジスタ操作は成功を返している**。

### 達成: 画面に絵が出た

`lcd-demo-rs` の描画がすべて出た。**カラーバーの左端が赤**（MADCTL の BGR ビットが
正しい）、**文字が読める**（`draw_text` の経路と DC の配線が正しい）。
`RESET` に 10 kΩ のプルアップを入れると**描き終わったあとも絵が残る**
（原因 3）。ここは RP2350 の §6 と同じ粒度で観測した。

ただしここに至るまでに 3 つ潰している（次節以降）。
**最初の状態は「トレースは完全一致するのに画面は真白」**だった。
全画面を黒で塗るのが最初の描画なので、SPI が少しでもパネルに届いていれば
まず黒くなる。真白は「バックライトは点いているが有効なデータが 1 バイトも
届いていない」形。

**トレースの一致はここを保証しない。** 一致が言うのは「ゲストが正しいバイト列を
HAL に渡した」ことだけで、レジスタへの書き込みが**パッドまで届いているか**は
トレースに現れない（成功を返しつつ無反応、という形になりうる。これは
`docs/TODO.md` §1.4 が ESP32-S3 の最初の懸念として挙げていたもの）。

試したこと:

- **SPI クロックを 16 MHz → 1 MHz に落とした**。トレースは同じく 14,352 行一致、
  画面は変わらず真白。**信号品質は原因ではない**
- **ポート層を直接叩く切り分けを入れた**（`ports/esp32s3/src/probe.rs`、
  `--features hw-probe`）。ゲストもインタプリタも通さず `Board` を呼ぶ。結果:

  ```
  probe: configure cs: ok / dc: ok / rst: ok
  probe: spi_open(0, 1MHz, mode0): ok
  probe: init sequence: ok
  probe: 0xD3 (want 00 00 93 41) -> 00 00 00 00 00
  ```

  ID 読み出しは全部ゼロだったが、**`MISO` を繋いでいないので判定材料にならない**。

### 原因 1（ソフト）: `GPIO_FUNCn_OUT_SEL` の値が S3 では違う

USB シリアル変換の `RXD` を `GPIO43` から `GPIO14` に移し、`probe` が
GPIO14 を 1 Hz で振っている間に受信バイト数を数えた（UART 受信機から見ると
500 ms の Low は BREAK に見えるので、振れていれば Low 期間ごとに `00` が 1 つ出る）。

| | 20 秒間の受信バイト数 |
|---|---|
| `out_sel = 128`（修正前） | **1** |
| `out_sel = 256`（修正後） | **28** |

同じアダプタで `GPIO43` からは 442,982 バイト取れていたので、アダプタ側は生きている。
つまり修正前は**ピンがパッドで動いていなかった**。

`ports/esp32s3/src/board.rs` の `SIG_GPIO_OUT`（`GPIO_FUNCn_OUT_SEL` に入れる
「GPIO 出力」信号の番号）を **128 → 256** に直した。**この番号はチップごとに
違う**（`esp-metadata-generated` の `OutputSignal::GPIO`）: ESP32 / S2 / S3 は
256、C3 / C6 など RISC-V 勢は信号マップが 128 本ぶん短いので 128。S3 で 128 を
書くと `I2S0O_SD1` の出力がパッドに繋がるので、`GPIO_OUT` / `GPIO_ENABLE` を
読み返すと正しく見えるのにピンは動かない。

**この失敗の形が厄介な点:** `pin.open` / `pin.write` はすべて成功を返し、
host call のトレースは host と完全一致する。`gpio_write` は `GPIO_ENABLE` を
読んで出力かどうかを検査しているが、それも通る。**トレースだけを見ていると
「動いている」と読めてしまう。** `docs/TODO.md` §1.4 が ESP32-S3 の最初の
懸念として「out_sel」を挙げていたのは当たっていた。

SPI2 は `esp-hal` のドライバが信号番号を持っているので影響を受けていない。
壊れていたのは `lcd-cs` / `lcd-dc` / `lcd-rst` の 3 本で、CS が Low に
ならなければパネルには 1 バイトも入らない。真白はその形。

### 原因 2（配線）: 接触不良

`out_sel` を直しても画面は真白のままだった。SCK (GPIO12) には 15,770 バイトぶんの
クロックが出ていて、CS / DC / RST も振れていることを実測できていたので、
ソフト側は出し切った状態だった。**その後ブレッドボードの配線を挿し直したら
絵が出た。** 途中で読み取り用アダプタの線が 1 本外れていたことも分かっており、
他の線も接触が怪しかったと見られる。

ここから得た教訓: **「レジスタは正しい」と「信号が相手に届いている」は別**で、
後者はトレースからは一切見えない。ブレッドボードで組む場合は最初に配線を
疑う余地を残しておくこと。

### 原因 3（仕様の帰結）: 描き終わると `RESET` が浮いて絵が消える

絵が出るようになったあと、**デモが描き終わると画面が白に戻る**という症状が残った。
これはバグではなく `docs/abi-spec.md` §5.2 の帰結で、**ポート固有でもない**。

トレースの最後 4 件がそのまま答えになっている:

```
[wasm] lcd-demo done
> spi@0.1.0/[resource-drop]bus(1)     ← SCK / MOSI / MISO が入力に戻る
> gpio@0.1.0/[resource-drop]pin(3)    ← lcd-rst が入力・プル無しに戻る = RESET が浮く
> gpio@0.1.0/[resource-drop]pin(2)    ← lcd-dc
> gpio@0.1.0/[resource-drop]pin(1)    ← lcd-cs
```

`pin.drop` は §5.2 どおりピンを入力・プル無しに戻す。手元の ILI9341 モジュールは
`RESET` にプルアップを持っていないので、線が浮いてパネルがリセットし、画面が
白に戻る。

**確かめ方（2 段階、どちらも実測）:**

1. `gpio_release` で入力に戻すのをやめた切り分けビルドを焼くと、**絵はそのまま
   残った**。チップの再起動ではないことも別に確認した（無干渉で 150 秒観測して、
   起動は 1 回・トレース 14,352 行・`lcd-demo done` のあと出力なし）
2. **`RESET` ↔ `3V3` に 10 kΩ のプルアップを入れると、出荷する構成
   （切り分け feature なし）のままで絵が残った。** これが対処の実測。
   `gpio_release` は §5.2 どおり入力に戻すので、`RESET` を High に保っているのは
   プルアップだけ

**対処は外部回路側しかなく、実機で効くことを確認した。** `RESET` に 10 kΩ 程度の
プルアップを 3V3 から入れる（既存の `GPIO15` → `RESET` は外さない。ESP32 が
`RESET` を Low に駆動するときプルアップ経由で 330 µA 流れるだけで、初期化の
リセットパルスは問題なく出る）。ゲスト側でハンドルを解放せずに終わっても
回避できない — §5.2 は「`run` から戻ったとき、ホストは残っている全ハンドルを
drop する」とも定めているので、解放は必ず起きる。理由:

- コード変更ゼロ。`.wasm` は `fc470947…` のままで、記録したトレースが生き続ける
- **リセット入力を浮かせないのは回路としてまっとう。** §5.2 が「入力・プル無しに
  戻す」と定めている以上、**線のアイドルレベルは外部回路が決めるべき**もので、
  ホストに保証させる話ではない
- ILI9341 モジュールの多くは `RESET` にプルアップを持っている。手元の個体は
  持っていなかった、という整理になる

#### Pico 2 W では起きなかった（2026-09-26 に実測）

**この症状が出るかどうかはボードで違う。** 同じモジュールを Pico 2 W に繋ぎ替えて
プルアップ無しで走らせたところ、**描き終わっても絵は残った**。当初「`gpio_release`
の意味は両ポートで同じなので Pico 2 W でも起きるはず」と書いていたが、**実測では
起きなかった**ので訂正する。

| ボード | プルアップ無し | プルアップ有り |
|---|---|---|
| ESP32-S3 | **白に戻る** | 残る |
| Raspberry Pi Pico 2 W | **残る** | （試していない） |

分けて考えるべきものが 2 つある:

- **仕組みは両ポートで同一。** §5.2 によりピンは高インピーダンスに戻るので、
  **線のアイドルレベルをソフトウェアは決めていない**。これはポート固有の話ではない
- **電気的な結果はボードで違う。** 浮いた線がパネルのしきい値を割るかは、パッドの
  リーク量・容量とモジュール側の入力仕様で決まる。ESP32-S3 では割り、Pico 2 W では
  割らなかった。**ポートのコードの差ではない**

したがって **`RESET` のプルアップは「バグの回避」ではなく「浮いた線に依存しない」
ための対策**として要る。Pico 2 W で動いているのは保証ではなく、たまたま割らなかった
という観測にすぎない（温度・個体・配線長で変わりうる）。

`docs/TODO.md` §2 に「abi-spec §8 の表に `lcd-rst` の外部プルアップを明記するか」を
オーナー確認事項として挙げてある。

### 切り分けに使った道具

`ports/esp32s3/src/probe.rs`（`--features hw-probe`）。ゲストもインタプリタも
通さず `Board` を直接叩き、次を行う:

- ILI9341 の ID 読み出し（`0xD3` / `0x04`）
- **全画面を 1 色で塗る**（ポート層だけで SPI が生きているかを目で見る）
- **DC を 1 Hz で振る**（パッドまで届いているかを外から当てて見る）

**トレースが一致しているのに絵が出ない**という状況では、ここが
「レジスタがパッドまで届いているか」を見られる唯一の場所だった。
`rp2040` のブリングアップでも同じものが要るはず。

なお **MOSI をソフトだけで読み返す検査は成立しない。** MISO を MOSI と同じ
ピンに向ければ読み返せるはずだが、ESP32-S3 では GPIO11 が FSPID そのもので
`with_mosi` が IO_MUX の直結機能を選ぶため、`with_miso` を同じピンに向けると
`mcu_sel` が GPIO 機能に書き換わって**直結していた出力が切り離される**
（実際に試すと `ff ff ff ff` が返る）。`probe.rs` にコメントで残してある。

---

## 8. 2 ボードの直接突き合わせ（2026-09-26）

§6 と §7 はそれぞれのボードを host リファレンスと比べたもの。**§5 手順 5 が言って
いる board-to-board の突き合わせはこれ。**

```bash
(cd apps && cargo build --release)      # fc470947... を 1 回だけ作る
(cd ports/rp2350 && cargo build --release --features guest-lcd-demo)
sh ports/esp32s3/build.sh build --release --features guest-lcd-demo
# 同じ .wasm が両方の ELF に入っていることをバイト列で照合してから焼く

sh verify/diff-traces.sh pico.log esp32s3.log
→ 一致: 14352 行
```

| | |
|---|---|
| ゲスト | `apps/lcd-demo-rs`。SHA-256 `fc470947ac08b230ccdc59e04e09aa11105a5c8754efd00533858470f11d2453` |
| ボード 1 | Raspberry Pi Pico 2 W（RP2350 A2 / QFN60）。UART0 (GP0) 115200 8N1 |
| ボード 2 | ESP32-S3-WROOM-1 搭載ボード（chip rev v0.2、flash 8 MB）。UART0 (GPIO43) 115200 8N1 |
| 結果 | **14,352 行完全一致**（`spi.write` の CRC-32 3,272 件を含む） |

3 通りの突き合わせがすべて一致している（Pico ↔ host、ESP32-S3 ↔ host、
Pico ↔ ESP32-S3）。**`docs/handoff.md` §5 Phase 6 の (2)「同一バイナリを 2 ボードで
走らせ、`time` を除くトレースと SPI ピクセル CRC が完全一致」は達成。**

同じ `.wasm` が両方の ELF に入っていることは、焼く前に ELF の中からゲストの
バイト列を検索して確認した（`include_bytes!` なのでそのまま埋まっている）。

### ここで `verify/diff-traces.sh` のバグを踏んだ

最初の突き合わせは「`pico.log` は 1 行、`esp32s3.log` は 14352 行」という不一致に
なった。**トレースの中身ではなくスクリプト側の問題だった。**

実機のシリアルはリセットや電源投入の瞬間にライン・ノイズで NUL を吐く。
`pico.log` には **441,254 バイト中 1 個**だけ NUL が混ざっていて、それだけで
`grep` がファイルをバイナリと判断し、`Binary file matches` の 1 行を返していた。
実際には 14,352 行ある。

`normalize()` で NUL も落とし、`grep -a` を付けて直した。**完了条件の判定に使う
スクリプトなので、この壊れ方は原因の誤診に直結する**（今回は偽陰性だったが、
「2 ボードが一致しない」と読んで実機を疑い始めるところだった）。

**この一致が言っていないこと:**

- **浮動小数の一致（§4.2）は進んでいない。** `lcd-demo-rs` は f32 を使わない
- **表示が同じであることは、この一致からは出ない。** 一致しているのは host call の
  列で、パッドまで届いているかは別（§7 の `out_sel` のバグがまさにその例）。
  表示は両ボードで目視した（§6 / §7）
- **描き終わったあとの挙動は 2 ボードで違った。** プルアップ無しで ESP32-S3 は
  白に戻り、Pico 2 W は絵が残った（§7「原因 3」）。トレースは同一なので、
  **host call の一致は表示の保持を保証しない**という例がもう 1 つ増えた形

この突き合わせをやる過程で `verify/diff-traces.sh` のバグを 1 件踏んで直した
（上記）。**実機のログで初めて出た壊れ方**で、host 同士の比較や self-test では
出ない（NUL が混ざらないため）。

---

## 9. アプリスロットの実測（2026-10-04、Pico 2 W）

**ファームを焼き直さずにアプリを差し替えられることを実機で確認した。**
`docs/app-workflow.md` §3.1 / §3.4 の 1 段目。

ファームは `ports/rp2350`（既定 feature = trace 有効、内蔵アプリ = `blink_rs`）。
スロットには `lcd_demo_rs` を入れた。**内蔵とスロットで別のアプリにしてある**
ので、どちらが走ったかがトレースの先頭で分かる（LCD の配線は要らない）。

| 見たこと | 期待 | 実機の出力 |
|---|---|---|
| スロットが空 | 内蔵へ落ちる | `wasmicon: slot empty, running built-in` → blink のトレース |
| スロットにアプリ | そちらが走る | `wasmicon: slot 4088 B crc32=0f19d973` → lcd-demo のトレース（`spi` 657 件） |
| CRC を 1 bit 壊す | 内蔵へ落ちる | `wasmicon: slot crc mismatch, running built-in` → blink のトレース |

CRC は `wasmicon pack` が報告した値（`0f19d973`）と一致した。

### スロットから走ったアプリのトレースが host と完全一致した

**14,352 行**。§6 で `include_bytes!` で焼き込んで記録した行数と同じで、
1 行も違わない。**スロット経由で配ってもゲストの振る舞いは変わらない**ことの
実測。突き合わせは `wasmicon trace diff` と `verify/diff-traces.sh` の両方で
行い、**実機のログで両者が同じ判定を出した**（`tests/trace.rs` の
突き合わせが合成データでしか見ていなかった部分）。

```
$ wasmicon run --trace lcd_demo_rs.wasm > host.log     # 14,354 行
$ wasmicon trace diff host.log pico.log
一致: 14352 行
```

取り込んだログの先頭には NUL が 1 つ入っていた（リセットの瞬間のライン
ノイズ）。正規化が落とすので判定には出ない —— §8 で踏んだ壊れ方と同じ形。

### 副産物: ハンドル掃除がトレースを汚していない

空スロットのときのトレースは、**ゲスト自身の `[resource-drop]pin(1)` で終わって
いる**。`Hal::release_all`（abi-spec §5.2）はその後に走るが、**行が 1 つも
増えていない。** 記録済みの 14,352 行（§6 / §7）が変わらないことの実機での裏付け。

### `picotool` の引数で 2 つ踏んだ

どちらも host では出ない。`wasmicon pack` が出していたコマンドが通らなかった。

- **拡張子は `.bin` でないといけない。** `picotool load` は拡張子でファイル
  種別を判定するので、`.slot` だと
  `does not have a recognized file type (extension)` で止まる
- **`-o` は絶対アドレス。** フラッシュのオフセット（`0x100000`）を渡すと
  `invalid memory range 0x00100000-0x00101008` で弾かれる。
  正しくは XIP base を足した `0x10100000`

### 取り込みの手順（`monitor` を作るときに要る）

**macOS では `stty -f` だけでは baud が保持されない。** 開いて設定して
閉じるので、次に開くと 9600 に戻る。実際に 1 回、全部ノイズのログを
取ってしまった（11,799 バイトすべて化けた）。

**正しい順序は「`cat` で開いたまま `stty` を当てる」**:

```
cat /dev/cu.usbserial-21420 > pico.log &
stty -f /dev/cu.usbserial-21420 115200 raw -echo   # 開いている間に当てる
# ここでボードをリセット（USB を抜き差し）
```

**データが流れている最中に `stty` を当てると行が混ざる**（`<")` のような形で
バイトが落ちる）。当てるのはリセットの前。この順で取ったログは 14,352 行が
完全一致した（上記）。崩れたログでも先頭の `slot ...` 行は読めたので
判定自体は付いたが、トレースの突き合わせには使えない。

### `pack` が出すコマンドが通ることの確認

2 つのバグを直したあと、**`wasmicon pack --board rp2350` が出力した
コマンドを一字も変えずに実行して通った**（`picotool load -o 0x10100000
<file>.bin`）。その状態で走らせたのが上のトレース一致。

### まだ見ていないこと

- **ファームの末尾とスロットの重なり検査**（`__flash_binary_end`）は発動して
  いない。実測でファームは `0x10011100` までで、スロットは `0x10100000` なので
  1 MB 近く空いている。重なる状況を作らないと通らない経路
- **RP2040 は未検証**（Pico WH が未入手）。コードは RP2350 と同じ
  `slot::read_xip` を通る
