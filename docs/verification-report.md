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
docs/handoff.md §2-10 の「`time` を除く全 host call と結果が一致」の定義**になる。
`spi.write` のトレースは data の CRC-32 なので、一致は「送っているピクセルが
同一」を意味する。

失敗経路も検査しているのは、実機では `ports/rp2040` / `ports/esp32s3` の SPI が
まだ `unsupported` を返すため。成功経路だけ揃えても実機に持って行った瞬間に
比較が意味を失う。

## 4. まだ検証されていないこと

### 4.1 実機での動作

**2026-09-21 に ESP32-P4 / M5Stack Tab5 でランタイムが初めて実機で動いた。**
`decode` / `validate` / `Exec::new` までは実機で正しく動くことを確認済み。
`instantiate` を通ると `Module` が壊れる問題が残っており、ゲストの実行までは
到達していない（docs/TODO.md §1.1.5）。RP2040 / ESP32-S3 は未着手。

トレースは UART0 (G37) から 3.3V USB シリアル変換（CP2102N）経由で取り込む。
そこに至るまでに **esp-hal 1.2 が P4 v3.x / ECO5 を前提にしている**ことに
起因する不整合を 4 つ潰した（シリコンリビジョン / esp-sync の Zcmp 回避 /
ROM 関数アドレス表 / memcpy 系）。詳細は docs/TODO.md §1.1.5。

- GPIO はいずれもレジスタ直叩き（RP2040 は SIO / IO_BANK0 / PADS_BANK0、
  ESP32-S3 / ESP32-P4 は GPIO / IO_MUX）。型は通ったが一つも観測していない。
  実機で最初に起きることとして「blink が光らない」を想定すべき
- `ports/rp2040` / `ports/esp32s3` / `ports/esp32p4` の I2C / SPI は
  `unsupported` を返す。sensor-display は実機では動かない
- abi-spec §8 の配線（役割名 → ピン番号）はオーナー未確認

#### ESP32-P4 / M5Stack Tab5 への書き込み（2026-09-11）

**到達点**: アプリはロードされ実行に入るところまで来たが、**トレースを
取り込めていないので、ランタイムの動作は何も検証できていない**。
経緯と残りの選択肢は `docs/TODO.md` §1.1.5。

最初の書き込みは 2nd stage bootloader がシリコンリビジョンで拒否した:

```
I (27) boot: chip revision: v1.0
I (28) boot: efuse block revision: v0.3
E (79) boot_comm: chip revision check failed. Required >= v3.0, found v1.0.
E (85) boot: Factory app partition is not bootable
```

- 手元の Tab5 は **ESP32-P4 v1.0**（ROM `esp32p4-eco2-20240710`）
- **ポートの不具合ではなく esp-hal の既定値だった。** esp-config の
  `min-chip-revision` が P4 で 300 (v3.0) 既定になっており、espflash はその値を
  ELF のメタデータから読んでヘッダに書く（`--min-chip-rev` は下から clamp される）
- `ESP_HAL_CONFIG_MIN_CHIP_REVISION = "100"` を `.cargo/config.toml` の `[env]` に
  置いて解決した。ESP-IDF の `CONFIG_ESP32P4_REV_MIN_100` に対応する正規の設定で、
  ヘッダは min=100 / max=199 になる（max は ESP-IDF の `REV_LESS_V3` の範囲と一致）
- 解決後は `--force` なしで書き込め、`boot: Loaded app from partition at
  offset 0x10000` まで到達する

**ただしトレースが取り込めていない。** USB-Serial-JTAG と USB-OTG の両方を
試したがアプリの出力は 1 バイトも取れず、**失敗の原因を切り分ける手段が無い**まま
終わった。2026-09-11 に **UART0 に戻す**判断をしたので、3.3V の USB シリアル変換
（未入手）を M5-Bus 13/14 に繋げば読めるようになる。
現時点では **GPIO もトレースも何一つ検証できていない。**

この試行で**ビルドでは出ない不具合が 3 件**見つかり、いずれも修正済み:

1. espflash は ESP-IDF のアプリ記述子が無いイメージを焼かない
   → `esp-bootloader-esp-idf` の `esp_app_desc!()` を追加
2. ポートの `rust-version` 宣言 1.85 が誤り（esp-hal 1.2.1 自身が 1.95 を要求）
   → 1.95 に修正
3. トレースの取り込みに USB シリアル変換と M5-Bus への配線が要る問題
   → 一度 USB-Serial-JTAG に変更したが、**この個体で出力が取れず UART0 に戻した**
   （2026-09-11）。USB-OTG も試したが列挙されず、いずれも原因を切り分ける手段が
   無いまま終わった。経緯と教訓は docs/TODO.md §1.1.5

書き込み前に工場出荷ファーム 16MB を全て退避し、試行後に書き戻して
**先頭 1MB のバイト一致を確認**した。実機は試行前の状態に戻してある。

### 4.2 ボード間の浮動小数の一致

sensor-display が唯一 f32 を使う温度バーの計算は、**ホスト 1 プラットフォーム
での一致しか確認していない**。RP2040 は `compiler_builtins` のソフトフロート、
ESP32-S3 は f32 のみハード FPU（非正規化数の扱いに設定依存がある）、
ESP32-P4 は RV32IMAFC の単精度ハード FPU と、実装が 3 通りある。
ここが Phase 6 の本来の実測対象。

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

**wasmtime との一致は x86_64 Linux でも確認できた**（手元は AArch64 macOS）。
インタプリタの一致がホストのアーキテクチャに依存しないことの傍証にはなるが、
RP2040 / Xtensa のソフトフロートを跨いだ一致は依然として未検証（§4.2）。

`rust-toolchain.toml` は `channel = "stable"` の浮動。clippy の新しい lint や
rustfmt の出力変化で CI が突然落ちうる（今回まさにそれ）。**固定するかは未決**。

---

## 5. 実機で検証するときの手順

1. `ports/rp2040` / `ports/esp32s3` / `ports/esp32p4` の I2C / SPI を実装する
2. abi-spec §8 の配線を確認し、役割名の表を実機に合わせる
3. 焼く
   - RP2040: ELF を `picotool load`、または `elf2uf2-rs` で UF2 にして BOOTSEL
   - ESP32-S3: `ports/esp32s3/build.sh run --release`（espflash）
   - ESP32-P4 (Tab5): `cd ports/esp32p4 && cargo run --release`（espflash。espup は要らない）
4. シリアル（いずれも 115200 8N1）を捕まえてファイルに落とす
   - RP2040: UART0 (GP0=TX, GP1=RX)
   - ESP32-S3: UART0 (GPIO43/44、DevKitC-1 では USB シリアルに直結)
   - ESP32-P4 (Tab5): UART0 (G37/G38)。**M5-Bus の 13/14 番ピンに出ているだけで
     USB には繋がっていない**ので 3.3V の USB シリアル変換が要る。
     **これが無いと実機のトレースは読めない**（2026-09-11 時点で未入手）
5. 突き合わせる: `sh verify/diff-traces.sh pico.log esp32s3.log`
   （3 ボード目を足すなら `sh verify/diff-traces.sh esp32s3.log esp32p4.log` も）
   - バナーとゲストの `[wasm]` 行は自動で落とす
   - `time` はトレースに出ず、役割名で引いた GPIO 番号は `role:led` に
     正規化済みなので、追加の加工は要らない
6. 実機の SHT31 応答を記録して `verify/sht31-replay.txt` を差し替える
