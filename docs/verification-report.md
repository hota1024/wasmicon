# Wasmicon 検証レポート

作成日: 2026-09-10 / 対象: **本レポートを含むコミット時点**（Phase 6 の道具を入れた地点）

レポート自身のコミットハッシュはここに書けないので、`git log --oneline -- docs/verification-report.md`
で最初にこのファイルが入ったコミットを見ること。

HANDOFF §5 Phase 6 の成果物。**何がどこまで検証されたか**と、**まだ検証されて
いないこと**を分けて書く。

---

## 0. 結論

| 検証項目 | 状態 |
|---|---|
| ランタイムの Wasm 仕様適合（spec testsuite） | **達成** |
| インタプリタの正しさ（wasmtime との差分） | **達成** |
| Rust 版と AS 版が同じ host call 列を出す | **達成**（host 上、成功経路と失敗経路） |
| 同一バイナリが 2 ボードで同じトレースを出す | **未達（実機が必要）** |
| 4 通り（Rust/AS × 2 ボード）で表示が出る | **未達（実機が必要）** |

**Phase 6 の完了条件は満たしていない。** 満たすには実機が要る。
ソフトウェア側で確かめられることは全て確かめた、という段階。

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
HANDOFF §2-10 の「`time` を除く全 host call と結果が一致」の定義**になる。
`spi.write` のトレースは data の CRC-32 なので、一致は「送っているピクセルが
同一」を意味する。

失敗経路も検査しているのは、実機では `ports/rp2040` / `ports/esp32s3` の SPI が
まだ `unsupported` を返すため。成功経路だけ揃えても実機に持って行った瞬間に
比較が意味を失う。

## 4. まだ検証されていないこと

### 4.1 実機での動作

**両ポートともビルドが通るところまでで、一度も焼いていない。**

- GPIO はどちらもレジスタ直叩き（RP2040 は SIO / IO_BANK0 / PADS_BANK0、
  ESP32-S3 は GPIO / IO_MUX）。型は通ったが一つも観測していない。
  実機で最初に起きることとして「blink が光らない」を想定すべき
- `ports/rp2040` / `ports/esp32s3` の I2C / SPI は `unsupported` を返す。
  sensor-display は実機では動かない
- abi-spec §8 の配線（役割名 → ピン番号）はオーナー未確認

### 4.2 ボード間の浮動小数の一致

sensor-display が唯一 f32 を使う温度バーの計算は、**ホスト 1 プラットフォーム
での一致しか確認していない**。RP2040 は `compiler_builtins` のソフトフロート、
ESP32-S3 は f32 のみハード FPU（非正規化数の扱いに設定依存がある）なので、
ここが Phase 6 の本来の実測対象。

### 4.3 記録済み I2C 応答

`verify/sht31-replay.txt` は T=23.44°C / RH=45.66% になる**合成データ**。
実機の SHT31 から記録したものへの差し替えが要る。

### 4.4 CI

**一度も実行されていない**（リモート未設定）。

---

## 5. 実機で検証するときの手順

1. `ports/rp2040` / `ports/esp32s3` の I2C / SPI を実装する
2. abi-spec §8 の配線を確認し、役割名の表を実機に合わせる
3. 焼く
   - RP2040: ELF を `picotool load`、または `elf2uf2-rs` で UF2 にして BOOTSEL
   - ESP32-S3: `ports/esp32s3/build.sh run --release`（espflash）
4. シリアル（どちらも 115200 8N1）を捕まえてファイルに落とす
   - RP2040: UART0 (GP0=TX, GP1=RX)
   - ESP32-S3: UART0 (GPIO43/44、DevKitC-1 では USB シリアルに直結)
5. 突き合わせる: `sh verify/diff-traces.sh pico.log esp32s3.log`
   - バナーとゲストの `[wasm]` 行は自動で落とす
   - `time` はトレースに出ず、役割名で引いた GPIO 番号は `role:led` に
     正規化済みなので、追加の加工は要らない
6. 実機の SHT31 応答を記録して `verify/sht31-replay.txt` を差し替える
