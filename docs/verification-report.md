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
| 同一バイナリが 2 ボードで同じトレースを出す | **未達**（RP2350 と host は一致。2 ボード目が無い） |
| 4 通り（Rust/AS × 2 ボード）で表示が出る | **未達**（I2C が未実装で sensor-display が動かない） |

**Phase 6 の完了条件は満たしていない。** 満たすには 2 ボード目（ESP32-S3 か
Pico WH）と I2C の実装が要る。RP2350 については、実機で動くところまで来た（§6）。

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

失敗経路も検査しているのは、実機では `ports/rp2040` / `ports/rp2350` / `ports/esp32s3` の SPI が
まだ `unsupported` を返すため。成功経路だけ揃えても実機に持って行った瞬間に
比較が意味を失う。

## 4. まだ検証されていないこと

### 4.1 実機での動作

**RP2350 は観測済み（§6）。RP2040 と ESP32-S3 はビルドが通るところまでで、
一度も焼いていない。**

- RP2040 / ESP32-S3 の GPIO はレジスタ直叩き（RP2040 は SIO / IO_BANK0 /
  PADS_BANK0、ESP32-S3 は GPIO / IO_MUX）。型は通ったが一つも観測していない。
  実機で最初に起きることとして「blink が光らない」を想定すべき
- `ports/rp2040` / `ports/esp32s3` の I2C / SPI と、`ports/rp2350` の I2C は
  `unsupported` を返す。**sensor-display はどのボードでも動かない**
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

`verify/sht31-replay.txt` は T=23.44°C / RH=45.66% になる**合成データ**。
実機の SHT31 から記録したものへの差し替えが要る。

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
2. abi-spec §8 の配線を確認し、役割名の表を実機に合わせる
3. 焼く
   - RP2040: ELF を `picotool load`、または `elf2uf2-rs` で UF2 にして BOOTSEL
   - RP2350: ELF を `picotool load -u -v -x -t elf`（RP2350 は picotool 2.0 以降が要る。
     `elf2uf2-rs` は RP2040 用で使えない）
   - ESP32-S3: `ports/esp32s3/build.sh run --release`（espflash）
4. シリアル（いずれも 115200 8N1）を捕まえてファイルに落とす
   - RP2040 / RP2350: UART0 (GP0=TX, GP1=RX)
   - ESP32-S3: UART0 (GPIO43/44、DevKitC-1 では USB シリアルに直結)
5. 突き合わせる: `sh verify/diff-traces.sh pico.log esp32s3.log`
   - バナーとゲストの `[wasm]` 行は自動で落とす
   - `time` はトレースに出ず、役割名で引いた GPIO 番号は `role:led` に
     正規化済みなので、追加の加工は要らない
6. 実機の SHT31 応答を記録して `verify/sht31-replay.txt` を差し替える

---

## 6. RP2350 実機の実測（2026-09-26）

`ports/rp2350` を初めて実機で動かした記録。**Phase 6 の完了条件そのものではない**
（あれは 2 ボードで定義されている。handoff §5）。Phase 5 の「表示が出る」のうち、
ディスプレイ側だけを SHT31 を待たずに切り分けたもの。

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
