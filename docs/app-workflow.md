# アプリ開発フローの設計（ローダと `wasmicon` CLI）

最終更新: 2026-10-04

この文書は**設計**。**残作業と未決は `docs/TODO.md` §5 が正**で、ここには書かない
（散らすと必ず古くなる。CLAUDE.md）。本文からは §5-1 のように番号で参照する。

変えないものは §6 にまとめた。要するに **ABI（`docs/handoff.md` §2）、`wit/`、
`docs/abi-spec.md` の規則、記録済みのトレースは動かさない。**

---

## 1. 何を変えるか

### 1.1 今のフロー（実体）

アプリの `.wasm` は**ポートが `include_bytes!` でハードコードしたパスから焼き込む**
（`ports/rp2350/src/main.rs:48`）。差し替えは cargo feature `guest-lcd-demo` の
二択しかない。したがって:

| やりたいこと | 今やること |
|---|---|
| アプリを 1 行直して試す | `cd apps && cargo build --release` → ポートを再ビルド → 再書き込み（Pico は BOOTSEL 押下、ESP32-S3 は espflash） |
| 別のアプリを動かす | ポートの `main.rs` を編集するか feature を足す |
| リポジトリ外でアプリを書く | **できない**。ABI 準拠のビルドフラグが `apps/.cargo/config.toml` にしか無い |
| ファームを焼く | ESP32-S3 は espup + Xtensa ツールチェーンが必須 |

つまり**アプリとファームが同じ成果物に固着している**。Wasm を使う動機
（アプリだけを差し替える）がフローに現れていない。

### 1.2 目標

- **ファームはボードごとに一度焼く。** 以降アプリ作者はファームをビルドしない
- **アプリは独立した成果物**（`.wasm` 1 個）。USB か HTTP でファームに送る
- アプリ作者が触る面は `wasmicon` CLI 1 つ。ABI 準拠のビルドフラグは CLI 側の責務
- 1 サイクル: `wasmicon run`（host、秒）→ `wasmicon deploy`（実機、数秒）

### 1.3 v0.1 の検証（TODO §1）との関係

**この作業は TODO §1 を押し退けるのではなく、短くする。** §1.3 の次の一手は
「SHT40 を実機で読む」で、手順は「ゲストをビルド → ファームを再ビルド → 焼く →
シリアルを取り込む → host のトレースと突き合わせる」。このうち
**ファーム再ビルドと取り込み・突き合わせは §5 の 0 段と 1 段がそのまま置き換える**
（`monitor` は `/dev/cu.usb*` を列挙する。TODO §1.1 の CH343 の件がここに効く）。

**handoff §5 の Phase 4 / 5 / 6 の完了条件（ESP32-S3 と Pico WH の 2 ボードで定義）は
動かさない。**

---

## 2. 3 層の役割分担

| 層 | 成果物 | 誰が作るか | 変更の頻度 |
|---|---|---|---|
| ファームウェア | `wasmicon-rp2350.uf2` / `wasmicon-esp32s3.bin` | このリポジトリ（リリース成果物） | 稀 |
| アプリ | `app.wasm` | アプリ作者 | 常時 |
| CLI | `wasmicon` | このリポジトリ | — |

ファームウェアの中身は **ランタイム + HAL + ローダ/スーパーバイザ**。
アプリの契約は `docs/abi-spec.md` §6（機能セット、`run` と `memory` の export、
import の名前とシグネチャ）だけで、ボード固有のものは入らない
（ピン番号は `board.pin-by-role` で引く。abi-spec §8）。

---

## 3. ファームウェア側: ローダとスーパーバイザ

### 3.1 スーパーバイザのループ

```
起動
 ├ バナー（例 "wasmicon rp2350"）を UART0 に出す
 ├ スロットを読む
 │   ├ 有効 → decode → validate → Exec → instantiate → start → run → 後始末 → idle
 │   └ 空 / 壊れている → 理由を出して idle（§3.3）
 └ idle: 制御チャネルのフレームを待つ（§3.6）
        ポート層が LED を振って生存を示す（Wasm を通らない）
```

今の `main` は `run` のあと止まったままになる
（`ports/rp2040/src/main.rs:146` と `ports/rp2350/src/main.rs:173` は
`loop { wfi() }`、`ports/esp32s3/src/main.rs:117` は `loop { spin_loop() }`、
host はプロセスが終わる）。handoff §3 #4「ログを出して停止、再起動しない」の
とおりで、**この方針は変えない**（確定は `docs/abi-spec.md` §10 の未決 #3 が
持っていて、TODO §2 に残っている）。
「停止」の意味を「**同じアプリを勝手に再実行しない**」に保ったまま、
「新しいアプリを受け付ける」状態を足す。`docs/abi-spec.md` §6.6 は
トラップ後の挙動を「ポートが決める」としているので、**ABI の変更ではない**
（確定は §5-1）。

サイクルごとに作り直すもの:

- **arena**。`Arena` は bump なので解放は作り直しのみ（`runtime/src/arena.rs`）
- **`Hal`**。`Hal::new(board, trace_on)` が board を所有する
  （`ports/common/src/lib.rs:114`）ので、`into_board()` かハンドル表の
  `reset()` が要る
- **ロール表 `roles` / `nroles`**。`pin-by-role` で配った番号を覚えていて、
  abi-spec §9 の正規化（番号 → `role:led`）に使う。**持ち越すと前のアプリで
  配った番号が `role:` 表記になり、トレースが変わりうる**

アプリのバイト列は **arena の外**に置く。`decode` は
`Module<'m, 'a>` を返し、**wasm バイト列 `'m` と arena `'a` を別に借りる**
（`runtime/src/decode.rs:40`、`runtime/src/module.rs:92`）。XIP のスライスを
そのまま渡す今の設計（design-notes §4）がこれに乗っている。

### 3.2 先に塞いだ穴: abi-spec §5.2 のハンドル掃除（2026-10-04 実装）

`docs/abi-spec.md` §5.2 は「**`run` から戻ったとき、ホストは残っている全ハンドルを
drop する**」と定めているが、**どのポートもこれを実装していなかった**。
`ports/host/src/lib.rs` の `run_wasm_opts` も各ポートの `main.rs` も `run` の
あとに掃除をしておらず、見えていなかったのは Rust バインディングの `Drop`
（`bindings/rust/src/hal.rs:90` / `:176` / `:236`）がゲスト側で解放していたから。

単発ファームでは無害（走り終わって止まるだけ）。**ローダでは次のアプリが
`busy` を踏む**。ゲストがトラップするとハンドルが残るので確実に踏む。

`ports/common` に `Hal::release_all()` を足し、**4 箇所（host と 3 ポート）で
`run` の戻りを受けてから呼ぶ**ようにした。トラップで抜けた場合も通る。
**仕様に既に書いてあることの実装で、ABI の変更ではない。**

- **掃除はトレース行を出さない。** ゲストの host call ではないし、出すと
  記録済みの 14,352 行（verification-report §6 / §7）が変わる。
  `board` を直接叩くので、トレースを書く `Hal::call` を通らない
- **解放の順序はハンドルの小さい順に gpio → i2c → spi で固定**した。
  ポートごとに違うと、トレースに出ない差がボード間に生まれる
- 検査は `ports/common/tests/release_all.rs`（解放の順序、冪等、
  **トレース行を出さないこと**、掃除後に再 `open` できること）。
  `release_all` を空にすると 3 件が落ちることを確かめてある

### 3.3 アプリの置き場所

2 通りあり、**両方要る**。

**(S1) RAM スロット** — 受信して即実行。再起動で消える。dev ループ用。

**(S2) フラッシュスロット** — 電源投入で走る。永続。

| | RP2040 | RP2350 | ESP32-S3 |
|---|---|---|---|
| SRAM | 264 KB（`ARENA` 160 KB） | 520 KB（`ARENA` 320 KB） | 512 KB（`ARENA` 300 KB、残り DRAM はほぼ無い。TODO §1.4） |
| RAM スロットの取り方 | `ARENA` の先頭を `split_at_mut` で切る | 同じ | 同じ |
| フラッシュ書き込み | `rom_data`（RP2350 と同系） | `rom_data::connect_internal_flash` / `flash_exit_xip` / `flash_range_erase` / `flash_range_program` / `flash_flush_cache`（rp235x-hal 0.4.0） | `esp-storage` 0.10 の `FlashStorage`（`SECTOR_SIZE` = 4096） |
| フラッシュからの実行 | XIP スライスをそのまま渡す（RAM コピー不要） | 同じ | **RAM にコピーする。** 任意オフセットが XIP にマップされている前提を置かない（MMU / DROM） |

- アプリは現状 775 B〜5.3 KB（`blink_rs` 775 B / `lcd_demo_rs` 4.0 KB /
  `sensor_display_as` 5.3 KB）なので、64 KiB のスロットでも RP2350 / ESP32-S3 では
  `ARENA` の 1/5 未満。**RP2040 は SRAM が厳しいので小さく取る**（16 KiB など）
- RP2350 で消去・書き込みをするとき **XIP から実行しているコードは走れない**。
  書き込みルーチンを RAM に置き（`#[link_section = ".data"]`）、割り込みを止めて
  呼ぶ必要がある（**要確認** → §5-6）
#### ファームにアプリは入らない（オーナー決定 2026-10-04）

**`include_bytes!` の内蔵アプリは外す。** 空スロットのときは理由を UART に出して
idle に入り、生存確認は**ポート層が直接 LED を振る**（`Board` を直叩きするので
Wasm を通らない。`ports/esp32s3/src/probe.rs` と同じ層）。

理由は「アプリはファームに入らない」を例外なしにすることだけではない。
**今の `include_bytes!` はファームのビルドを `apps/` のビルドに依存させている** —
CI の rp2040 / rp2350 ジョブが `apps` を先にビルドしているのはこのためで、
**ファームを単体のリリース成果物として作れない**（§4.5 の配布と正面からぶつかる）。

- **外すのはスロットが入るのと同時**（§5 の 1 段）。先に外すとファームが
  何も走らせなくなる
- **CI も同時に直す**: ポートのジョブから `apps` の先行ビルドが要らなくなり、
  `cargo clippy --features guest-lcd-demo` の行も消える（feature 自体が無くなる）
- **失うもの**: 焼いた直後に「ランタイムが decode → run まで通る」ことを実機で
  確かめる足場。host テストと最初の `deploy` で代替する。LED のハートビートが
  示すのは「ファームが生きている」までで、ランタイムが動くことは示さない
- **記録済みトレースは生き続ける。** `lcd_demo_rs.wasm`（`fc470947…`）を
  スロットに `deploy` して走らせれば、**バイト列が同一なので host call 列も同一**
  （ゲストは自分がどう届いたか知らない）。XIP のどの番地に載るか、RAM に
  写されるかはトレースに出ない

スロットをフラッシュのどこに置くかは**ボードごとに別の問題**:

- **RP2350 / RP2040**: フラッシュ 4 MB。ファームの末尾は `__flash_binary_end`
  （`ports/rp2350/memory.x`）。スロットは固定オフセット（例 先頭から 1 MB =
  `0x1010_0000`）に置き、**ファームの末尾と重ならないことを起動時に検査して
  UART に出す**。重なれば自分を壊す
- **ESP32-S3**: **固定オフセットを決め打ちしてはいけない。** espflash の既定の
  パーティションテーブルは `factory` をフラッシュ末尾まで広げるので、
  「3 MB の位置」のような決め方では factory の中に入る。`partitions.csv` に
  `wasmicon, data, …` の専用エントリを足し、`espflash --partition-table` で焼いて、
  そのオフセットへ `write-bin` する。今は `esp-bootloader-esp-idf` を
  `default-features = false` にしてパーティションテーブルの検査を切っている
  （`ports/esp32s3/Cargo.toml`）ので、**整合は自分で取る**
- フラッシュ容量と PSRAM の有無は手元のボードの品種で決まる → §5-2

### 3.4 スロットの形式

```
offset  size  内容
0       4     magic "WMCA"（Wasmicon App）
4       2     フォーマット版（= 1）
6       2     予約（0）
8       4     wasm の長さ（u32 LE）
12      4     wasm の CRC-32（u32 LE）
16      n     wasm 本体
```

- wasm が 16 バイト境界から始まるので、スライスをそのまま `decode` に渡せる
- CRC は「転送の事故」と「アプリのバグ」を切り分けるために入れる。TODO §1.4 の
  「無言で止まる」を 1 つ減らせる
- **ABI バージョンはヘッダに入れない。** import のモジュール名
  （`wasmicon:hal/gpio@0.1.0`）が既にバージョンを持ち、完全一致が要求される
  （abi-spec §3.1 / §6.4）ので、不一致は `instantiate` が `unknown import` で弾く。
  CLI は `info`（§3.6）が返す ABI バージョンで**送る前に**弾く

### 3.5 転送路

**制御は USB、トレースは UART0 に分ける。** 同じ UART に混ぜるとフレーム同期と
ログ行がぶつかる。しかも**両ボードとも USB の口が空いている**
（ESP32-S3 の `USB-OTG` 側は今のファームが何も出していない。
`apps/lcd-demo-rs/README.md`。Pico は USB 未使用）。

| ボード | 制御チャネル | 根拠 |
|---|---|---|
| ESP32-S3 | USB-Serial-JTAG（`USB-OTG` ポート） | `esp-hal` の `usb::usb_serial_jtag`。**追加クレート不要** |
| RP2350 / RP2040 | USB CDC | `rp235x-hal::usb`（`usb-device` の `UsbBus` 実装）+ `usbd-serial` |

**USB スタックを入れる前の 0 段目**（既存のフラッシャでスロットだけ書く）:

- ESP32-S3: `espflash write-bin <offset> app.bin`。DTR/RTS で ROM ブートローダに
  落ちるので**ボタン操作が要らない**
- RP2350 / RP2040: BOOTSEL + `picotool load -o <offset>`、または
  **スロットのアドレスを指す UF2 を CLI が作って `RPI-RP2` ドライブにコピー**
  （picotool 不要）。RP2350 で任意アドレスに absolute family ID
  (0xe48bff57) が要るかは **要確認**（→ §5-6）
- ファームが USB 制御チャネルを持てば
  `reboot::RebootKind::BootSel`（rp235x-hal 0.4.0 `src/reboot.rs`）で自分を
  BOOTSEL に落とせるので、Pico のボタン操作も消せる

**HTTP:**

- **ESP32-S3 + `esp-wifi` から。RAM 予算が本題。** 今 DRAM はほぼ使い切っている
  （`.bss` 315 KB / `ARENA` 300 KB / ネイティブスタック 17.4 KiB。TODO §1.4）。
  esp-wifi はヒープを数十 KB 要求するので、**`max_memory_pages` を 4 → 2 に
  落とすか、arena を PSRAM に移すかの二択**になる。前者は abi-spec §6.2 が
  許す範囲（上限はポートが決める）だが、**「同一バイナリがどのボードでも通る」が
  実質的に崩れる**。後者は DevKitC-1 の品種次第（N8R8 なら PSRAM 8 MB、
  `esp-hal` に `psram` がある）で、**線形メモリを PSRAM に置くと遅くなる**
  （未計測）→ §5-3
- **Pico 2 W / Pico WH は CYW43439。** 実用的なドライバ `cyw43` は embassy
  （async）前提で、今の blocking 構成からポートの作り直しになる。
  **v1 の HTTP は ESP32-S3 だけ**を推す → §5-4
- 向き（デバイスがサーバか、URL から pull するか）は §5-5。dev ループは
  `POST /app` + mDNS が楽で、OTA は pull が楽

### 3.6 プロトコル

**CLI とファームが USB / HTTP 越しに話すための取り決め。** ゲストとランタイムの
間の ABI（`wasmicon:hal@0.1.0`）とは相手も目的も別物で、**アプリは一切関与しない。**

```
フレーム: "WMCN" | ver u8 | cmd u8 | len u32 LE | payload | crc32 LE
```

magic で resync する。コマンド:

| cmd | 名前 | payload | 意味 |
|---|---|---|---|
| 0x01 | `info` | なし | ボードの申告を返す |
| 0x02 | `load` | wasm | RAM スロットへ（S1） |
| 0x03 | `store` | wasm | フラッシュスロットへ（S2） |
| 0x04 | `run` | なし | スロットのアプリを走らせる |
| 0x05 | `stop` | なし | 走行を終わらせて idle に戻す |
| 0x06 | `reset` | なし | リセット |
| 0x07 | `probe` | なし | ポート層を直接叩いて切り分ける（下記） |
| 0x08 | `config` | 設定テキスト（空なら消去） | 役割マップを書く（§3.9）。実効値は `info` が返す |

`info` が返すもの → **§3.8**（ボード名、ファーム版、ABI 版、`Config`、スロット容量と
形式版、プロトコル版、役割名、構成）。**これで CLI がボードの上限をハードコード
しなくなる**（§4.3）。

`probe` は今の `hw-probe` feature（`ports/esp32s3/src/probe.rs`）をコマンドにした
もの。**ゲストを走らせず `Board` を直接叩く**ので、「トレースが host と完全一致
するのに画面が真白」— レジスタ設定がパッドまで届いているか — を見られる唯一の
手段で、ESP32-S3 の `out_sel` で実際に効いた（verification-report §7）。
**コマンドにすると焼き直さずに切り分けられ、build 構成が 1 つ減る**（§3.8）。

**走行中のアプリへの割り込みは素直にリセットで行う。** ゲストがいつ yield するか
分からず、host call の中で USB をポーリングすると host call の所要時間が変わる
（トレースの内容は変わらないが `time` の値は変わる）。dev ループは
「リセット → idle → 送る」で足りる。

HTTP は同じコマンドを別の殻に入れるだけ（`GET /info`、`POST /app`）。
**プロトコル版はどちらの経路にも同じく効く**（版が縛るのはコマンドの集合と
payload の形で、殻ではない）。

#### 版を持つ理由と、凍結する部分

**CLI とファームは別々に更新される。** ファームは一度焼いたら放置、CLI は
`cargo install` で新しくなるので、「**新しい CLI が古いファームに話しかける**」が
普通に起きる。`info` が `protocol 1` と返せば、CLI は「このファームは v1 しか
喋れない → v2 の機能は使わない / 焼き直してくれと言う」を判断できる。

そのためには **フレームの封筒（magic + `ver` + `cmd` + `len` + `crc`）を一度決めたら
凍結する**必要がある。でないと版を問い合わせるフレーム自体が読めなくなる。
版が付くのは封筒ではなく中身（コマンドの集合と payload の形）で、**未知の `ver` を
受けたファームは封筒だけ読んで「その版は喋れない」と返す。**

#### 設計に出てくる「版」4 つ

混ざりやすいので並べておく。**上の 3 つは独立に動くので別々に持つ。**

| 名前 | 誰と誰の間 | どこに実体があるか | 食い違うと |
|---|---|---|---|
| **ABI 版** `wasmicon:hal@0.1.0` | ゲスト（`.wasm`）↔ ランタイム | import のモジュール名（完全一致。abi-spec §3.1）が強制。`info` も申告する（§3.8。`deploy` 前に弾くため） | 全アプリが `unknown import` で落ちる |
| **プロトコル版** | CLI ↔ ファーム | フレームの `ver u8`（本節） | CLI が喋れない |
| **スロット形式版** | 書き手 ↔ フラッシュ上のバイト列 | スロットヘッダ（§3.4） | 古いファームが新しいスロットを読む |
| **ファーム版** | —（互換に関与しない） | `info` とバナー（§3.8） | 何も壊れない。由来の記録専用 |

ABI を触らずにコマンドを 1 つ足せばプロトコル版だけが上がり、**アプリは何の
影響も受けない**。この切り分けが §3.8 の「版 1 本で範囲比較させると嘘になる」の
中身で、ファーム版の振り方は §5-8。

### 3.7 決定性との関係

- **ローダは host call を増やさない**ので abi-spec §9 のトレース行は変わらない。
  記録済みの 14,352 行（verification-report §6 / §7）はそのまま有効
- `verify/diff-traces.sh` は `>` / `<` で始まらない行を捨てるので、ローダのログ
  （バナー、`loaded 4096 bytes crc=…` など）は混ざってよい
- **壊れる経路は 2 つだけ**で、どちらも §3.1 / §3.2 で回避している:
  ハンドル掃除がトレース行を出す / ロール表を持ち越す
- **ただし「同じ `.wasm` なのにトレースが違う」の原因がファーム差になる**ので、
  どのファームが出したトレースなのかを記録できる必要がある → §3.8

---

### 3.8 ファームウェアの identity と互換の軸

ファームは**ボードごとに別物**で、**版ごとにも違う**。そこで要るのは
「ファームのパッケージマネージャ」ではなく、**ファームが自分を名乗ること**と、
**送る前に突き合わせること**。

#### 互換は版 1 本では表せない

アプリから見た互換の軸は独立に 5 本ある。

| 軸 | 今どこにあるか | 食い違うと何が起きるか |
|---|---|---|
| ABI 版 `wasmicon:hal@0.1.0` | import のモジュール名（完全一致。abi-spec §3.1） | 全アプリが `unknown import` で落ちる |
| ボードプロファイル（`Config` + **実装済みインターフェース**） | 各 `main.rs`（RP2040 = 2 ページ、他 = 4）と各 `board.rs`（RP2040 は SPI / I2C が未実装） | 大きいアプリが validate で落ちる / 実機で `unsupported` が返る |
| 役割名（`led` / `lcd-cs` …） | `ports/common` の `profile::<board>.roles`（abi-spec §8） | `pin-by-role` が `unsupported` を返す |
| プロトコル版 | フレームの `ver u8`（§3.6） | CLI が喋れない |
| スロット形式版 | スロットヘッダ（§3.4、= 1） | 古いファームが新しいスロットを読む |

**ファーム版を 1 本持って範囲比較させる設計にすると必ず嘘になる。** `info` は
**軸ごとに申告**し、CLI は軸ごとに判定する。ファーム版は**由来（provenance）専用**。

#### 今、ファームは何も名乗っていない

- バナーは `wasmicon rp2350` の 1 行だけで、**版も git も入っていない**
  （`ports/rp2350/src/main.rs:140`、`ports/esp32s3/src/main.rs:87`、
  `ports/rp2040/src/main.rs:124`）
- `docs/verification-report.md` は**ゲストの SHA-256（`fc470947…`）は記録している
  のに、ファーム側は何も記録していない**。どのビルドが 14,352 行を出したかは、
  レポートの日付と git 履歴から推測するしかない
- 単発ファームなら「焼いた本人が覚えている」で済むが、**ローダが入ると
  「同じ `.wasm` なのにトレースが違う」の原因がファーム差になる**（§3.7）

#### 載せるもの

`info` と**バナーの 1 行**（シリアルしか無い場面で効く）に、同じものを出す:

ボード名 / ファーム版 + `git describe` + dirty / ABI 版 / 構成（`trace` の有無）/
`Config` / **実装済みインターフェース** / スロット容量と形式版 / プロトコル版 /
役割名の一覧。

- `git describe` はビルド時に埋める（`build.rs`）。**dirty を落とさない。**
  手元ビルドが名乗れないと検証の記録が曖昧になる
- **変種（variant）は版ではなく構成**。`trace` を切って焼いたボードに `monitor` を
  当てると無言で何も出ない（今なら配線から疑い始める）。`trace: off` と言えば一発
- **variant は `trace` あり / なしの 2 つだけになる。** 今ある `guest-lcd-demo` は
  内蔵アプリを外すので消え（§3.3）、`hw-probe` は build 構成ではなく
  `probe` コマンドになる（§3.6）。manifest とキャッシュの鍵は
  **board × version × variant**（版だけでは一意にならない）

#### 使い道

- **`deploy` 前の照合**: ABI 版、初期ページ数 ≤ `max_memory_pages`、
  **アプリが import するインターフェースがそのポートで実装済みか**、
  サイズ ≤ スロット容量、スロット形式版。`check --board`（§4.3）の表を
  デバイスの申告に差し替えるだけ
  - **役割名は静的に列挙できない**（§4.3）。`info` が持つ役割名の一覧と
    突き合わせられるのは、データセグメントから見つかった名前だけ
  - 実装状況を持たせないと、`ports/rp2040` 向けの `check` は**静的には通って
    しまう**（SPI / I2C が `unsupported` を返すのは実行時）。§4.6 の例がこれ
- **トレースへのスタンプ**: `monitor` が取り込みログの先頭に identity を記録する。
  `trace diff` は identity 行を比較から外すが、**食い違ったら警告する**
  （`time` を除外しているのと同じ扱い。handoff §2-10）
- **配布**: ボード × 版の成果物に manifest（ファイル + sha256 + 上の 5 軸）。
  `wasmicon flash --board rp2350 [--fw 0.2.0]` とローカルキャッシュ、
  `wasmicon fw list`（手元にある版と、デバイスに載っている版）。§4.5 と対になる

#### 作らないもの（今は）

A/B スロットでのロールバック、署名検証、HTTP 経由のファーム自己更新、
デバイス台帳。1 人・3 枚・ケーブルが手元にある状況では過剰で、特に A/B は
**フラッシュ予算とアプリスロットの配置を同時に複雑にする**（§3.3）。

**ただしスロット形式版とプロトコル版は最初から持つ。** 払うのは数バイトで、
後から足せない。版番号の振り方（3 ポート共通の 1 本か、ポートごとか）は §5-8。

---

### 3.9 役割マップをデバイス側の設定にする

役割 → GPIO 番号の対応は今**ファームの中**にある（`ports/common` の
`profile::<board>.roles`。各ポートの `board.rs` がこれを引く）。
つまり **配線を変えるとファームを焼き直す**ことになり、「ファームは一度だけ焼く」
（§1.2）と衝突する。LCD の CS を隣のピンに移したいだけでファームのリリースを
待つのは筋が悪い。

#### 何を設定可能にするか（最初は役割だけ）

| 対象 | 設定可能にするか | 理由 |
|---|---|---|
| 役割 → GPIO（`led` / `lcd-cs` / `lcd-dc` / `lcd-rst`） | **する** | ゲストが直接振るピン。配線を変えたいのはここ |
| バスのピン（SPI の SCK/MOSI/MISO、I2C の SDA/SCL） | **しない**（当面） | ハードの制約がチップごとに違う。RP2350 は PL022 の SPI0 が FUNCSEL で特定ピンに固定、ESP32-S3 は GPIO マトリクスでほぼ自由。**ポートごとに許可ピンの表が要る**ので別の話 |
| バスの `index` | しない | ゲストに見えるのは `index` だけで、裏は元々ポートの責務（abi-spec §8） |
| 語彙そのもの（役割を増やす） | しない | `wit/` 側の話（TODO §1.1）。設定ではない |

#### 置き場所

**アプリスロットと兄弟**にする。寿命が違う（アプリを差し替えても配線は変わらない）
ので同じスロットには入れない。フラッシュの消去単位はどちらのボードも 4 KiB なので、
**1 つの領域を取ってセクタ 0 = 設定、セクタ 1 以降 = アプリ**にすると、合意すべき
オフセットが 1 つで済む（§3.3 の置き場所の話がそのまま使える）。

```
wasmicon 領域
  +0x0000  設定スロット（4 KiB。実際に使うのは数十バイト）
  +0x1000  アプリスロット（§3.4 のヘッダ + wasm）
```

#### 形式はテキスト

ヘッダはアプリスロットと同じ作り（magic `"WMCC"` + 形式版 + 長さ + CRC-32）で、
中身は 1 行 1 項目:

```
lcd-cs=22
lcd-dc=20
led=none
```

バイナリ（`[role_id u8, gpio u8]`）の方が小さいが、テキストを推す。

- **役割 ID という新しい契約を作らずに済む。** ID を振ると「版ごとに ID がずれる」
  という第 5 の互換軸（§3.6）が生まれる
- `info` がそのまま読み上げられる。フラッシュを吸い出しても人間が読める
- パースは `no_std` で済む（`=` で割って 10 進を読むだけ）。4 項目で 50 バイト程度
- `=none` は「このボードにこの役割は無い」。`pin-by-role` が `unsupported` を返す

#### 検証はデバイス側でやる

CLI はボードの制約を知らないので、**書き込み時にファームが弾いて理由を返す。**

| 検査 | 弾く理由 |
|---|---|
| 未知の役割名 | 名前をそのまま返す。タイポがここで止まる |
| `番号 >= gpio_count` | 範囲外 |
| **`RESERVED` の番号** | GP0/GP1 は**トレースの UART0**。割り当てるとトレースが死ぬ。CYW43439 側（GP23/24/25/29）も同様 |
| 同じ番号に 2 つの役割 | 実行時は `busy` になるだけだが原因が分かりにくい |
| **ポートがバスに使う番号**（rp2350 の GP16/18/19、GP4/5 など） | 割り当てると SPI / I2C が黙って壊れる |

最後の行は注意が要る。abi-spec §8 の経緯で**バスのピンは意図的に `reserved` に
入れていない**（入れると GPIO の開放可否が host の mock と食い違い、トレースの
突き合わせがボードごとに別物になる）。**設定の書き込み時に弾くのは実行時の
振る舞いを変えないので、この決定と衝突しない。**

**壊れた設定で起動不能にしない。** CRC が合わなければ既定値で起動してログに出す。
手が届くのが同じ UART しかないので、ここは fail-safe にする。

**適用は再起動後。** 走行中に差し替えると、開いているピンと設定が食い違う。

#### なぜ決定性を壊さないか

abi-spec §9 が**役割で配った番号を `role:lcd-cs` に正規化して出す**ので、
**対応表を変えてもトレース行は変わらない。** 役割という間接層が最初から
この性質を持っていたということで、配線の自由度をファームから剥がしても
Phase 6 は保てる。

**ただし罠が 1 つ。** §9 はホストが引数の**数値からしか判断できない**とも書いて
いて、ゲストがピン番号をハードコードしていてそれが役割の割り当てと一致すると、
それも `role:` に置き換わる。したがって **役割マップを変えると、ハードコードして
いるアプリのトレースが変わる**（`role:lcd-cs` だった行が生の番号になる）。
そういうアプリは §9 が既に決定性検証の対象外だと言っているが、症状が
「設定を変えたらトレースが変わった」になるので CLI が説明できるようにしておく。

#### identity に反映する（§3.8 の拡張）

**同じファーム版でも配線が違えば別のセットアップ。** だから:

- `info` は**実効値**（既定 + 上書き）と、上書きされているかを申告する
- `monitor` がログ先頭に刻む identity に、**実効マップの CRC-32 を入れる**

入れないと、§3.8 で解こうとしていた「同じ `.wasm` なのにトレースが違う」の
原因候補が 1 つ増えるだけになる。

#### これでも解けないもの

- **バスのピン** → ポートごとの許可ピン表が要る。別件
- **役割を増やす**（ボタンなど） → 語彙の話（TODO §1.1）
- **電気的な事実** — 外部プルアップ、`lcd-rst` が浮いて画面が白に戻る件
  （TODO §2） → 設定の対象ではない

やるか / いつやるかは §5-9。フラッシュ書き込み（1 段）とプロトコル（2 段）に
乗るが、**設定セクタを外から `espflash write-bin` / UF2 で書くだけなら 1 段でも
最小版が成立する**（ファームは読んで適用するだけ）。

---

## 4. `wasmicon` CLI

### 4.1 置き場所と流儀

`tools/wasmicon-cli`（root workspace、bin 名 `wasmicon`）。依存クレートは入れてよい
（前例: `wasmicon-gen` の anyhow / wit-parser）。**「依存ゼロ・`alloc` 不使用」は
`runtime/` だけの制約**で、CLI には及ばない。引数パースは `wasmicon-gen` と同じ
手書き + `USAGE` 定数（日本語）に揃える。フラッシャ（picotool / espflash /
probe-rs）は**呼ぶだけで、自前実装しない**。

### 4.2 コマンド

**実装済みは `check` / `run` / `trace diff` / `doctor`**（2026-10-04）。
`monitor` と `size` は未着手（`docs/TODO.md` §5）。

| コマンド | 中身 | 今あるもの |
|---|---|---|
| `new <name> --lang rust\|as` | 雛形。ABI 準拠のビルドフラグを埋める（§4.4） | 無し |
| `build` | `cargo` / `asc` を呼ぶ。フラグは雛形側が持つ | `apps/.cargo/config.toml` + 手順書 |
| `check <app.wasm> --board X` | **実ランタイムで** decode / validate / instantiate（§4.3） | 無し（`wasm-tools validate` は弱い） |
| `run <app.wasm> [--trace] [--i2c-replay f]` | host ポートで実行。`--i2c-replay` は既存の環境変数 `WASMICON_I2C_REPLAY` に対応する | `cargo run -p wasmicon-host`（crate は残して lib として使う） |
| `deploy <app.wasm>` | USB / HTTP でアプリを送る。`--persist` でフラッシュスロット | 無し |
| `flash --board X [--fw v]` | **ファームを**焼く（一度だけ）。版は manifest から引く（§3.8） | 手順書（`apps/lcd-demo-rs/README.md`） |
| `fw list` | 手元にある版と、デバイスに載っている版を並べる（§3.8） | 無し |
| `config show` / `set` / `apply` / `reset` | 役割マップを見る・書く・`wasmicon.toml` から押し込む・消す（§3.9 / §4.7） | 無し（今はファームを焼き直す） |
| `monitor` | シリアルを開いてトレースを取る。`/dev/cu.usb*` を列挙し、**先頭に identity を記録する**（§3.8） | `cat /dev/cu.usbmodemXXXX \| tee` |
| `trace diff a.log b.log` | 正規化して突き合わせ。**最初に食い違う行を出す**（トレースは追記しかされないので、そこが原因に最も近い） | `verify/diff-traces.sh`（**残す**。CLI と同じ判定を出すことをテストが突き合わせる） |
| `size [target]` | コアのコードサイズ | `tools/measure-size.sh` |
| `doctor` | ツールチェーンの検査 | 無し |

`monitor` は **`usbserial` を決め打ちしない**。手元の ESP32-S3 のブリッジは CH343 で
`/dev/cu.usbmodem*` に見える（TODO §1.1）。

### 4.3 `check` が見るもの

**実ランタイムで検査するのが肝。** `wasm-tools validate --features=…` より強い:

- import の**名前とシグネチャの完全一致**（abi-spec §6.4 / §7）。表は
  `runtime/src/generated.rs`（= `wit/` 由来）
- `run` と `memory` の export（abi-spec §3.3 / §6.2）
- **そのボードの `Config` での validate**（`max_memory_pages` など）
- **アプリが import するインターフェースがそのポートで実装済みか**（§3.8）。
  `ports/rp2040` は SPI / I2C が `unsupported` を返すので、これが無いと
  静的検査は通って実機で初めて落ちる

**できないこと: 役割名の列挙。** `pin-by-role` の引数は実行時に `(ptr, len)` で
渡る `string` で（`wit/board.wit`）、語彙は WIT の型に入っていない。だから
「このアプリがどの役割を引くか」を `.wasm` から**確実に列挙することはできない**。
`check` は既知の名前がバイト列に現れるかを見るだけで、**参考**であって保証ではない。
実測した限界は 2 つ:

- **ログ文字列の中の名前も拾う。** `sensor-display` は `led` を使っていないのに
  `roles` に出る（`sensor crc failed` の中に `led` がある）
- **AssemblyScript のゲストには当たらない。** AS の文字列リテラルは UTF-16 で
  置かれ、UTF-8 への変換は呼び出し時に起きるので、UTF-8 の役割名がバイナリに
  現れない（`sensor_display_as.wasm` は「見つからない」になる）確実に分かるのは実行時で、`pin-by-role` が
`unsupported` を返した行がトレースに残る（§3.1）。
**`wasmicon.toml` の `requirements.pin-roles`（§4.7）があれば、列挙が宣言に
なるので保証に
変わる。** プロジェクトの外から `.wasm` 単体を受け取ったときだけ参考に落ちる。

**今は「検証だけする入口」が無い。** `wasmicon_host::run_wasm_opts` は
decode / validate / instantiate のあと `run` まで呼ぶ 1 本の関数なので、
`check` は (a) CLI 側で `wasmicon-core` + `wasmicon_port::Hal` +
`wasmicon_host::hal::HostBoard`（どちらも `pub`）を直に組んで 4 呼び出しを
並べるか、(b) `ports/host` に `check_wasm(wasm, &Config)` を足すかのどちらか。
**(a) を推す**（host ポートを CLI のために太らせない）。

**host 実行は全ボードより緩い。** `ports/host` は `Config::default()`
（`max_memory_pages` = 65536）で走るが、実機は RP2040 = 2 / RP2350 = 4 /
ESP32-S3 = 4。**`wasmicon run` が通っても実機の validate で落ちるアプリが書ける。**
これが `check --board` の存在理由。

**ボードプロファイルは `ports/common` の `profile` に集めた**（2026-10-04）。
散っていた `Config`（3 つの `main.rs`）と役割割り当て（各 `board.rs` の `ROLES`）、
それに実装済みインターフェースを 1 箇所にしてある。各ポートはそこから引くので
**CI の rp2040 / rp2350 ジョブの `cargo build` が値の一致を見てくれる**。
値は `ports/common/tests/profiles.rs` に移す前の数値で固定した。

```
profile::RP2040   2 ページ / I2C と SPI は未実装 / Pico の役割割り当て
profile::RP2350   4 ページ / 全実装       / 同じ割り当て（ヘッダが同じ）
profile::ESP32S3  4 ページ / 全実装       / DevKitC-1 の割り当て
profile::HOST     Config::DEFAULT（緩い） / mock の割り当て
```

`Config::DEFAULT` はこのために `runtime` に足した const で、`Default` が
それを返す（二重に書くと必ず食い違う）。

**役割名の語彙は `assert_role_names` がコンパイル時に検査する。** `ROLE_NAMES`
に無い名前を割り当てると、`pin-by-role` は成功するのにトレースが `role:` に
正規化されず、生の GPIO 番号が出る（番号はボードごとに違うので**2 ボードの
トレースが食い違う**。abi-spec §9）。**これを検査しているものは今まで無かった。**

`info`（§3.6）が入ったあとは、実機に繋がっているなら**デバイスの申告を使う**。
番号そのものの検査（範囲外・予約ピン）はデバイス側の責務で、プロファイルには
置かない（§3.9）。

### 4.4 ビルドフラグの単一真実（腐ると静かに壊れる）

アプリの契約の実体はこの 2 つ:

- `apps/.cargo/config.toml` — `-Ctarget-feature=-reference-types` /
  `-Clink-arg=--initial-memory=65536` / `-Clink-arg=-zstack-size=8192`
- `apps/*/asconfig.json` — `runtime: stub` / `disable: [..., reference-types]` /
  `enable: [sign-extension, nontrapping-f2i, bulk-memory, mutable-globals]`

`wasmicon new` の雛形がこれを写すと **3 重化**する。食い違うと次の形で静かに壊れる:
`reference-types` が混ざれば `call_indirect` のテーブル索引で弾かれ
（abi-spec §6.1 の注）、`--initial-memory` が増えれば RP2040 の 2 ページを超える。

**`wasmicon-gen --check` と同じ形にする**: 雛形が吐く設定が `apps/` のものと
一致することを CI で検査する（または雛形を `apps/` から生成する）。

### 4.5 ファームウェアの配布

**アプリ作者にファームをビルドさせない。** でないと ESP32-S3 を焼くだけで
espup と Xtensa ツールチェーンが要る（`ports/esp32s3/build.sh`）。

- ファームはリリース成果物（`wasmicon-rp2350.uf2` / `wasmicon-esp32s3.bin`）。
  `wasmicon flash --board X` はそれを焼くだけ
- `doctor` が要求するのは rustc + wasm32 ターゲット（または `asc`）とフラッシャだけ
- 成果物には **manifest**（board → ファイル + sha256 + §3.8 の 5 軸）を付ける。
  `flash --fw` と `fw list` がこれを読む
- **CI は今 ESP32-S3 をビルドしていない**（Xtensa のため。`ci.yml` のコメント）。
  リリース成果物を出すなら espup を入れるジョブが要る → §5-7

---

### 4.6 一通り（CLI の使用例）

**これは設計が全部入ったあとの姿。** どのコマンドがどの段で現れるかは末尾の表。
出力は書式の例で、版や sha は実物ではない。**ただしアプリのサイズ・import 構成・
トレース行・行数（1,214 行 / `spi.write` 277 件）は実物から取ってある**
（`apps/sensor-display-rs` を host ポートで走らせた実測）。

題は「SHT4x を読んで ILI9341 に出すアプリを、Pico 2 W と ESP32-S3 の両方で
動かし、トレースを突き合わせる」。TODO §1.3 の次の一手そのもの。

#### 0. 道具を確かめる

```
$ wasmicon doctor
rustc 1.98.0 + wasm32-unknown-unknown   ok
assemblyscript 0.28.8                   ok
picotool 2.1.1                          ok
espflash 4.5.0                          ok
→ ファームを焼ける: rp2040 / rp2350 / esp32s3
```

ファームはリリース成果物なので、**espup と Xtensa ツールチェーンは要らない**（§4.5）。

#### 1. ファームを一度だけ焼く

```
$ wasmicon fw list
手元     rp2350  0.1.0  (g6b1a2c3)  sha256 9c1f…
         esp32s3 0.1.0  (g6b1a2c3)  sha256 4ab7…
デバイス  繋がっていない

$ wasmicon flash --board rp2350
BOOTSEL を押しながら USB を挿して、Enter を押してください… 
RPI-RP2 に wasmicon-rp2350 0.1.0 (g6b1a2c3) を書き込んだ
（焼いた直後はスロットが空。UART に "slot empty" が出て LED が点滅する）

$ wasmicon flash --board esp32s3
/dev/cu.usbmodem1101 (CH343) に wasmicon-esp32s3 0.1.0 (g6b1a2c3) を書き込んだ
```

**ここから先、ファームには触らない。**

#### 2. ボードに名乗らせる（§3.8）

```
$ wasmicon info
board      rp2350 (Pico 2 W)
firmware   0.1.0 (g6b1a2c3, clean)
abi        wasmicon:hal@0.1.0
protocol   1
slot       65536 B / format 1 / 空
config     memory 4 pages, call depth 32, table 256 elems
interfaces gpio, i2c, spi, time, log, board
features   trace on
roles      led, lcd-cs, lcd-dc, lcd-rst
```

#### 3. アプリを作る

```
$ wasmicon new room-monitor --lang rust
room-monitor/
  wasmicon.toml        [requirements] pin-roles は空（使う役割を書き足す）
  Cargo.toml           wasmicon-hal 0.1
  .cargo/config.toml   -reference-types / --initial-memory=65536 / -zstack-size=8192
  rust-toolchain.toml
  src/lib.rs           run() の雛形
```

`.cargo/config.toml` の中身は `apps/.cargo/config.toml` と**同一**。一致は CI が
見る（§4.4）。`--lang as` なら `asconfig.json` と `assembly/index.ts` が出る。

#### 4. 内側のループ — 実機を触らない（host、1〜2 秒）

```
$ wasmicon build
room_monitor.wasm  4,549 B

$ wasmicon check --board rp2350
abi      wasmicon:hal@0.1.0                          import 名で強制される
imports  13 件すべて一致                             (board 1 / gpio 3 / i2c 4 / log 1 / spi 3 / time 1)
exports  run, memory                                 あり
memory   初期 1 ページ
table    なし

[rp2350]
validate 初期 1 ページ ≤ 4                           ok
roles    led, lcd-cs, lcd-dc, lcd-rst                rp2350 にある（参考）
→ 通る

$ wasmicon run --trace --i2c-replay sht4x-replay.txt > host.log
$ head -6 host.log
> wasmicon:hal/log@0.1.0/log(2, "sensor-display start")
<
> wasmicon:hal/board@0.1.0/pin-by-role("lcd-cs")
< 0 [role:lcd-cs]
> wasmicon:hal/gpio@0.1.0/[static]pin.open(role:lcd-cs, 3)
< 0 [1]
```

`check` はボードを変えると結果が変わる。**ここが host 実行では分からない**（§4.3）。
`ports/rp2040` は SPI / I2C が未実装（`spi_open` / `i2c_open` が `unsupported` を
返す。TODO §1.2）なので、**静的検査だけでは通ってしまう**:

```
$ wasmicon check --board rp2040
[rp2040]
validate 初期 1 ページ ≤ 2                           ok
i2c      このポートは未実装（実機では unsupported）  ← 落ちる
spi      このポートは未実装（実機では unsupported）  ← 落ちる
roles    led, lcd-cs, lcd-dc, lcd-rst                rp2040 にある（参考）
→ 落ちる（2 件）
```

`roles` の行に **（参考）**と付いているのは、役割名を `.wasm` から確実に列挙
できないため（§4.3）。データセグメントで見つかった名前だけを照合している。
**`wasmicon.toml` に `requirements.pin-roles` を書けば保証に変わる**（§4.7）。

**これを言えるのはボードプロファイルが「実装済みインターフェース」を持つから**
（§3.8）。持たせなければ `check` は通り、実機で `i2c.bus.open` が
`unsupported` を返して初めて分かる。初期ページ数が収まり、役割名も 3 ポートで
同じ（`led` / `lcd-cs` / `lcd-dc` / `lcd-rst`）なので、**他に落ちる要素が無い。**

#### 5. 外側のループ — 実機に送る（数秒）

```
$ wasmicon deploy room_monitor.wasm --monitor > pico.log
rp2350 0.1.0 (g6b1a2c3) と照合
  abi ok / 1 ページ ≤ 4 / roles ok / 4,549 B ≤ 65536 B / slot format 1
RAM スロットへ 4,549 B 送った (crc32 1f3a9c21)
run → UART0 115200 からトレースを取り込み中（Ctrl-C で止める）
```

**制御は USB、トレースは UART0**（§3.5）なので CLI が両方を握る。`--monitor` を
付けなければ送って走らせるだけ。ログの先頭には identity が入る:

```
$ head -2 pico.log
# wasmicon rp2350 0.1.0 (g6b1a2c3) abi=wasmicon:hal@0.1.0 trace=on app=1f3a9c21
> wasmicon:hal/log@0.1.0/log(2, "sensor-display start")
```

#### 6. 2 ボードで突き合わせる（このプロジェクトの本題）

ESP32-S3 に差し替えて、**同じ `.wasm` を**送る:

```
$ wasmicon deploy room_monitor.wasm --board esp32s3 --monitor > esp32s3.log
$ wasmicon trace diff pico.log esp32s3.log
app       1f3a9c21 / 1f3a9c21                     同一バイナリ
identity  rp2350 0.1.0 (g6b1a2c3) / esp32s3 0.1.0 (g6b1a2c3)
time を除く 1,214 行                               一致
spi.write の CRC-32 277 件                        一致
→ 一致
```

ファームの版が食い違っていたら、落とさずに警告する（§3.8）:

```
identity  rp2350 0.1.0 (g6b1a2c3) / esp32s3 0.1.0 (g7f09de1)   版が違う
```

#### 7. 焼き付ける（電源投入で走る）

```
$ wasmicon deploy room_monitor.wasm --persist
フラッシュスロット 0x1010_0000 に 4,549 B 書いた (crc32 1f3a9c21, format 1)
次の電源投入から走る
```

#### 2 つのループ

| ループ | 何を回すか | 頻度 |
|---|---|---|
| 内側 | `build` → `check` → `run`（host、mock HAL とリプレイ） | 常時 |
| 外側 | `deploy --monitor` → `trace diff` | 内側が通ったら |
| ファーム | `flash` | 稀（版を上げるときだけ） |

#### どのコマンドがどの段か（§5）

| 段 | 使えるようになるもの |
|---|---|
| 0 | `doctor` / `build` / `check` / `run` / `trace diff` / `size` / `monitor`（identity 無し） |
| 1 | `flash`（ファーム）/ `deploy --persist`（既存フラッシャ経由。**Pico は BOOTSEL 押下が残る**） |
| 2 | `info` / `deploy`（USB、ボタン不要）/ `deploy --monitor` / `monitor` の identity / `fw list` |
| 3 | `deploy --via http`（ESP32-S3） |

---

### 4.7 `wasmicon.toml`

プロジェクトに 1 つ置く。**ここに書くのは「アプリの性質」と「意図」で、
真実がデバイス側にあるものは宣言にとどめる**（§3.9）。

```toml
# wasmicon.toml — プロジェクトに 1 つ
version = 1

[requirements]
# このアプリが `board.pin-by-role` で引く役割名。**番号は書かない。**
# これが §4.3 の照合を「参考」から「保証」に変える
pin-roles = ["lcd-cs", "lcd-dc", "lcd-rst"]

[defaults]
board = "rp2350"
i2c-replay = "fixtures/sht4x.txt"

# 机の上のボードの配線（= 意図）。押し込むのは `config apply` のときだけ
[board.rp2350.roles]
lcd-cs = 22

[board.esp32s3.roles]
led = 4
```

#### 何がどこの真実か

| 情報 | 真実はどこか | toml の役割 |
|---|---|---|
| 必要な役割名（`requirements.pin-roles`） | **toml**。アプリの性質 | 真実そのもの |
| 役割 → GPIO | **デバイス**（`info` が実効値を申告。§3.9） | 「こうであってほしい」の宣言。差分を見るために使う |
| ABI 準拠のビルドフラグ | **`.cargo/config.toml` / `asconfig.json`** | **書かせない**（下記） |

#### `requirements.pin-roles` が照合を保証に変える

§4.3 のとおり、役割名は実行時に文字列で渡るので `.wasm` から確実に列挙できない。
宣言があれば列挙が確定するので、**送る前に**「このボードは `lcd-rst=none` なので
このアプリは動かない」と言える。

**節に分けてあるのは、同じ性質の未宣言項目がもう 1 つあるから。**
`i2c.bus.open` / `spi.bus.open` の `index` も実行時の整数引数で、`.wasm` から
静的に列挙できない（バスが 1 本のボードに index 1 を開くアプリを送ると実機で
`unsupported`）。v0.1 では書かないが、必要になれば
`[requirements]` に `i2c-buses = [0]` を足すだけで済む。
**インターフェースの実装状況は import から分かる**ので宣言は要らない（§4.3）。

名前は `requirements.pin-roles`。`requires` だと何が必要なのか読めず、
`required-pin-roles` のような平坦なキーは項目が増えるたびに長くなる。

**書かれた名前の検証はデバイスに繋がなくてもできる。** 役割名の語彙は
`ports/common` の `ROLE_NAMES` にあるので、CLI はそれと突き合わせるだけで
タイポを止められる（§4.3 の「ボードにあるか」の検査はデバイスの申告が要るが、
「そんな役割名は存在しない」はオフラインで分かる）。

#### スキーマの規則

| 決め | 内容 |
|---|---|
| **導出できるものは書かない** | アプリ名と言語は `Cargo.toml` の `package.name` / `asconfig.json` の有無から取る。toml に書くのは上書きとしてだけ。§4.4 と同じ理由で、二重に持つと必ず drift する |
| **1 アプリに 1 つ** | workspace でも**アプリごとに置く**。この repo の `apps/` をドッグフードするなら 5 つになる。`wasmicon new` が 1 つ出す形と揃う |
| **必須のみ** | `pin-roles` は「無ければ動かない」ものだけを並べる。`led` が無くても動く degradation は v0.1 では表現しない（`optional-pin-roles` は実例が出てから） |
| **「この役割は無い」は `"none"`** | `lcd-rst = "none"`。キーを省略すればファームの既定どおり。`false` や `0` より誤読しにくい |
| **パスは toml のあるディレクトリ基準** | `i2c-replay = "fixtures/sht4x.txt"`。CLI の cwd 基準にすると、どこから呼んだかで壊れる |
| **未知のキーはエラー** | 黙って無視すると「設定したのに効いていない」に気付けない。タイポはここで止める |
| **`version`** | CLI の対応より新しければエラー、古ければ受ける |
| **toml は任意** | `build` / `check` / `run` は **toml が無くても動く**（プロジェクトの形だけで足りる）。toml が増やすのは `pin-roles` の保証と既定値だけ |

- **プロジェクトの中で `deploy` → 保証**（toml がある）
- **`.wasm` 単体を受け取って `deploy` → 参考**（toml が無い。今と同じ）

バイナリと一緒に運びたくなったら、そこで初めてカスタムセクション
（`wasmicon.roles`）か `wasmicon pack` で束ねる話になる。ランタイムは custom
セクションを読み飛ばすので実行には影響しないが（`runtime/src/decode.rs:79`）、
**`.wasm` のバイト列が変わる**ので記録済みハッシュ（`fc470947…`）を取り直すことに
なる。それが要るまで入れない。

#### `deploy` は毎回照合し、黙って適用しない

```
$ wasmicon deploy
rp2350 0.1.0 と照合
  pin-roles lcd-cs, lcd-dc, lcd-rst            ボードにある
  roles     lcd-cs: toml=GP22 / device=GP17    食い違い
→ 中止。`wasmicon config apply` で押し込むか、toml を直す
```

**役割マップを書き換えると実際に駆動されるピンが変わる**ので、繋いでいる物に
よっては物理的に危ない。押し込むのは明示的なコマンド（`config apply` か
`--apply-config`）だけにする。

#### 書かせないもの

- **ABI 準拠のビルドフラグ。** §4.4 で「`apps/.cargo/config.toml` と
  `asconfig.json` と雛形で 3 重化する、腐ると静かに壊れる」と書いたものを
  **4 重にしてしまう**。toml に `initial-memory` を書けるようにすると
  「RP2040 で validate に落ちるアプリ」が作れて、しかも原因が分散する
- **マシン固有の値**（`/dev/cu.usbmodem1101` など）。ブリッジの型番で名前が
  変わるので（TODO §1.1 の CH343）、コミットすると他人のマシンで壊れる

```
wasmicon.toml        git にコミットする
wasmicon.local.toml  gitignore。シリアルポートなどマシン固有の上書き
```

優先順位は **CLI フラグ > `local.toml` > `wasmicon.toml` > デバイスの既定値**。
`info` が返すのは常にデバイスの実効値で、toml はそれを上書きしない（§3.9）。

`wasmicon new` がこのファイルも雛形として出す。CLI に toml パーサの依存が 1 つ
増える（`wasmicon-gen` の `anyhow` / `wit-parser` と同じ範囲。§4.1）。

---

## 5. 段階

**0 段と 1 段で「ファーム再ビルド不要」まで届く。** USB スタックと Wi-Fi は
そのあと。

| 段 | 中身 | ファームの変更 | 実機 |
|---|---|---|---|
| 0 | CLI の骨 + `check` / `run` / `monitor` / `trace diff` / `size` / `doctor` | **ゼロ** | 不要 |
| 1 | §3.2 の掃除 → スロット形式 → 「スロットを読んで走る」ファーム → **内蔵アプリの撤去**（§3.3）→ 既存フラッシャで `deploy` | ローダの芯 | 要 |
| 2 | USB 制御チャネル + §3.6 のプロトコル。`info` と `probe`、役割マップの設定（§3.9）が入る | USB スタック | 要 |
| 3 | HTTP（ESP32-S3） | Wi-Fi + RAM 予算の判断 | 要 |

- **0 段は TODO §1.3 の SHT40 作業にそのまま効く**（取り込みと突き合わせ）
- **1 段の時点で ESP32-S3 はボタン操作不要**（espflash が DTR/RTS でリセットする）。
  Pico は BOOTSEL 押下が残り、2 段で消える
- `new` と bindings の配布（crates.io / npm）は 0 段と独立。ここで決め打ちしない

---

## 6. 変えないもの / 触ると壊れるもの

- **ABI**: import 名、シグネチャ、エラーコード（discriminant + 1）、ハンドル 0 = 無効
  （handoff §2）。必要になったら実装せずオーナーに確認する
- **`wit/` が唯一の真実**、生成物はジェネレータ出力のみ（CLAUDE.md）
- **記録済みの `.wasm` ハッシュとトレース**（verification-report §6 / §7、TODO §3）。
  `build` は `apps/` と同じフラグで同じバイト列を出す。ローダはトレース行を増やさない
  （§3.7）
- **内蔵アプリと `guest-lcd-demo` feature は 1 段まで触らない。** 先に外すと
  ファームが何も走らせなくなる。**外すときは CI の rp2040 / rp2350 ジョブも
  同時に直す**（§3.3。残すのではなく、順番を守るという話）
- **handoff §5 の Phase 4 / 5 / 6 の完了条件**（ESP32-S3 と Pico WH の 2 ボード定義）
- **トラップ後に同じアプリを自動再実行しない**（handoff §3 #4）。ローダは
  「新しいアプリを受け付ける」だけを足す（§3.1）

---

## 7. 未決

**`docs/TODO.md` §5 に集約した。** 判断が要るのは、トラップ後の idle 復帰の確定、
ESP32-S3 の RAM 予算（`max_memory_pages` を落とすか PSRAM か）、HTTP の向き、
Pico W の Wi-Fi（embassy 移行）、ボードの品種（フラッシュ容量と PSRAM の有無）、
ファーム版の振り方。
