# アプリ開発フローの設計（ローダと `wasmicon` CLI）

最終更新: 2026-10-04

この文書は**設計**。**残作業と未決は `docs/TODO.md` §5 が正**で、ここには書かない
（散らすと必ず古くなる。CLAUDE.md）。本文からは §5-1 のように番号で参照する。

変えないものは §6 にまとめた。要するに **ABI（`docs/handoff.md` §2）、`wit/`、
`docs/abi-spec.md` の規則、記録済みのトレースは動かさない。**

---

## 1. 何を変えるか

### 1.1 今のフロー（実体）

> **2026-10-07 追記**: 下は着手前（2026-10-04）の実体。今はアプリをスロットに
> `deploy` し、ファームにアプリは入っていない（§3.3。内蔵アプリは 2026-10-07 に外した）。

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
  **トレース行を出さないこと**、掃除後に再 `open` できること、
  **ロール表は消えないこと**）。`release_all` を空にすると 3 件が落ちる
- **ロール表は消さない。** あれはハンドルではなく §9 の正規化に使う状態。
  ローダが 1 つの `Hal` を使い回すなら別に消す必要がある（§3.1）
- **代償: トラップ後の画面が残らなくなった。** 掃除はピンを入力に戻すので
  （§5.2）、`lcd-rst` が浮いてパネルが白に戻る（§3.3 の「描き終わると
  白くなる」と同じ現象）。トラップした瞬間の表示を見て切り分けることは
  できなくなる。それでも掃除を入れたのは **abi-spec §6.6 が
  「ホストはインスタンスを破棄し、全ハンドルを drop し、ログに理由を出力する」
  とトラップ時の drop を明示的に要求している**ため。**画面を見て切り分けたい
  ときは `probe` コマンド（§3.6）側で行う**
- **ただし順序は「理由を出してから掃除」にした。** §6.6 の文面は
  drop → ログだが、掃除は**無制限に待ちうる**（`spi_close` の `BSY` 待ちなど。
  TODO §2.1）。先に回すとペリフェラルが固まったときに理由が出ないまま
  無言で止まり、handoff §3 #4「ログを出して停止する」が守れない。
  §6.6 の順序をどう扱うかは TODO §2 に置いた

### 3.3 アプリの置き場所

2 通りあり、**両方要る**。

**(S1) RAM スロット** — 受信して即実行。再起動で消える。dev ループ用。

**(S2) フラッシュスロット** — 電源投入で走る。永続。

| | RP2040 | RP2350 | ESP32-S3 |
|---|---|---|---|
| SRAM | 264 KB（`ARENA` 160 KB） | 520 KB（`ARENA` 320 KB） | 512 KB（`ARENA` 300 KB、残り DRAM はほぼ無い。TODO §1.4） |
| RAM スロットの取り方 | `ARENA` の先頭を `split_at_mut` で切る | 同じ | 同じ |
| フラッシュ書き込み | `rom_data`（RP2350 と同系） | `rom_data::connect_internal_flash` / `flash_exit_xip` / `flash_range_erase` / `flash_range_program` / `flash_flush_cache`（rp235x-hal 0.4.0） | `esp-storage` 0.10 の `FlashStorage`（`SECTOR_SIZE` = 4096） |
| フラッシュからの実行 | **2026-10-04 に実装**（XIP スライスをそのまま渡す。RAM コピー不要） | 同じ（`0x10100000` から 64 KiB。どちらも実機未検証） | **RAM にコピーする。** 任意オフセットが XIP にマップされている前提を置かない（MMU / DROM） |

- アプリは現状 775 B〜5.3 KB（`blink_rs` 775 B / `lcd_demo_rs` 4.0 KB /
  `sensor_display_as` 5.3 KB）なので、64 KiB のスロットでも RP2350 / ESP32-S3 では
  `ARENA` の 1/5 未満。**RP2040 は SRAM が厳しいので小さく取る**（16 KiB など）
- RP2350 で消去・書き込みをするとき **XIP から実行しているコードは走れない**。
  書き込みルーチンを RAM に置き（`#[link_section = ".data"]`）、割り込みを止めて
  呼ぶ必要がある（**要確認** → §5-6）
#### ファームにアプリは入らない（オーナー決定 2026-10-04）

> **2026-10-07 に実施した。** 3 ポートの `include_bytes!` と `guest-lcd-demo` feature を
> 落とし、CI の rp2040 / rp2350 ジョブから `apps` の先行ビルドを外した。空スロットは
> `wasmicon: slot empty, idle` と出して `ports/common` の `idle::heartbeat` に入り、
> `led` 役のピンを 1 Hz で点滅させる（`Board` 直叩き。トレースは出ない）。

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

形の正は `ports/common` の `slot`（**書く側と読む側で 2 回書かない**）。
CLI 側は `wasmicon pack` が画像を作る。

- wasm が 16 バイト境界から始まるので、スライスをそのまま `decode` に渡せる
- CRC は「転送の事故」と「アプリのバグ」を切り分けるために入れる。TODO §1.4 の
  「無言で止まる」を 1 つ減らせる
- **空（消去済みの `0xff` / 未使用の `0x00`）は失敗ではない。** ファームは
  これを見て idle に入る（§3.1）。magic 違い（別のものが書かれている）とは
  区別する — ログの意味が変わる
- スロット領域**全体**を渡してよい（長さはヘッダが持つので余りは無視する）。
  4 KiB のセクタをまるごと読んで渡す使い方を想定している
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
- **variant は `trace` あり / なしの 2 つだけになる。** `guest-lcd-demo` は
  内蔵アプリを外して消えた（§3.3、2026-10-07）。`hw-probe` は build 構成ではなく
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

> **2026-10-07 にオーナーが確定し、1 段の最小版を実装した**（TODO §5-9）。
> 決めたこと: **役割名はファームの語彙ではない**（固定の語彙 `ROLE_NAMES` を廃止）／
> **ファームは既定の表を持たない**／**空スロットの待機でピンを駆動しない**（LED の
> 点滅をやめた）。

もともと役割 → GPIO 番号の対応は**ファームの中**にあった（`profile::<board>.roles`）。
**配線を変えるとファームを焼き直す**ことになり、「ファームは一度だけ焼く」（§1.2）と
衝突していた。LCD の CS を隣のピンに移したいだけでファームのリリースを待つのは筋が悪い。

#### 役割名は「アプリと配線の間の約束」

固定の語彙は、`alloc` が無い中でトレースの正規化（abi-spec §9）に `'static` な名前が
欲しかったから入っていた近道で、設計上の要請ではなかった。役割名は:

- アプリが `wasmicon.toml` の `[requirements] pin-roles` で**宣言**し
- 配線を `[board.<name>.roles]` に**番号で**書き
- `deploy` がその表を設定スロットに書き、ファームは**読んだ表のとおりに**答える

名前は英小文字で始まる 16 文字までの `a-z` / `0-9` / `-`、8 個まで
（`ports/common` の `roles::valid_name` / `MAX_ROLES`。`alloc` が無いので上限がある）。
`led` もファームにとって特別な名前ではなくなった（blink が引く名前の 1 つ）。

**タイポの検出**は語彙ではなく「宣言と配線表の食い違い」で行う。`check` / `deploy` は
宣言した役割がそのボードの配線表に無ければ**焼く前に**落とす（デバイスに繋がなくてよい）。

#### 何を設定可能にするか（最初は役割だけ）

| 対象 | 設定可能にするか | 理由 |
|---|---|---|
| 役割 → GPIO | **する** | ゲストが直接振るピン。配線を変えたいのはここ |
| バスのピン（SPI の SCK/MOSI/MISO、I2C の SDA/SCL） | **しない**（当面） | ハードの制約がチップごとに違う。RP2350 は PL022 の SPI0 が FUNCSEL で特定ピンに固定、ESP32-S3 は GPIO マトリクスでほぼ自由。**ポートごとに許可ピンの表が要る**ので別の話 |
| バスの `index` | しない | ゲストに見えるのは `index` だけで、裏は元々ポートの責務（abi-spec §8） |

#### 置き場所

**アプリスロットの直後に 4 KiB**（`profile::<board>.role_slot`。今は 3 ボードとも
`0x110000`）。寿命は違うが、**`deploy` はアプリスロット（64 KiB）と設定スロットを
1 本の画像にして 1 回で書く**（`pack::with_roles`）ので、書き込みの手間は増えない。
アプリの後ろはスロットの終わりまで `0xff` で埋める。

**配線表は毎回書く。** 表が空でも書くので、`deploy` の結果はその時の
`wasmicon.toml` だけで決まる（前に焼いた表が残らない）。

#### 形式はテキスト

ヘッダはアプリスロットと同じ作り（magic `"WMCR"` + 形式版 + 長さ + CRC-32）で、
中身は 1 行 1 項目:

```
lcd-cs=17
lcd-dc=20
lcd-rst=21
```

バイナリ（`[role_id u8, gpio u8]`）の方が小さいが、テキストにした。

- **役割 ID という新しい契約を作らずに済む。** ID を振ると「版ごとに ID がずれる」
  という第 5 の互換軸（§3.6）が生まれる
- `info` がそのまま読み上げられる。フラッシュを吸い出しても人間が読める
- パースは `no_std` で済む（`=` で割って 10 進を読むだけ）。本文は 256 バイトまで
- `wasmicon.toml` の `"none"` は「配らない」で、表には書かない

#### 検証は 2 回（CLI とファーム）

同じ検査（`roles::RoleMap::insert`）を、**CLI が焼く前に**（`wasmicon.toml` を読んだ
時点）、**ファームが起動時に**もう一度かける。制約はプロファイルの `gpio_count` /
`reserved` / `bus_pins` にあり、各ポートの `board.rs` が自分の値と一致することを
コンパイル時に確かめている。

| 検査 | 弾く理由 |
|---|---|
| 名前の書式 | 本文の `=` や改行、トレースの `role:` と衝突させない |
| `番号 >= gpio_count` | 範囲外 |
| **予約ピン** | GP0/GP1 は**トレースの UART0**。割り当てるとトレースが死ぬ。CYW43439 側（GP23/24/25/29）も同様 |
| 同じ番号に 2 つの役割 | 実行時は `busy` になるだけだが原因が分かりにくい |
| **ポートがバスに使う番号**（rp2350 の GP4/5・16/18/19、esp32s3 の GPIO8/9・11/12/13） | 割り当てると SPI / I2C が黙って壊れる |

最後の行は注意が要る。abi-spec §8 の経緯で**バスのピンは意図的に `reserved` に
入れていない**（入れると GPIO の開放可否が host の mock と食い違い、トレースの
突き合わせがボードごとに別物になる）。**配線表の検査で弾くのは実行時の
振る舞いを変えないので、この決定と衝突しない。**

**壊れた表で勝手にピンを動かさない。** ファームは既定の表を持たないので、
空・CRC 違い・制約違反のどれでも**役割を 1 つも配らずに起動**し、理由をバナーに
出す（`wasmicon: roles crc mismatch, no roles`）。1 項目でもおかしければ表全体を
捨てる。読めたときは `wasmicon: roles lcd-cs=17 lcd-dc=20 lcd-rst=21` と出す。
どちらもトレース行ではないので `trace diff` は比べない。

**適用は再起動後**（起動時に 1 回だけ読む）。`deploy` は書いたあとリセットするので、
焼いた表がそのまま効く。

#### host は表をプロファイルに持つ

host（mock）はフラッシュを持たないので、表を `profile::HOST.roles` に置き、
`wasmicon run` とテストが使う（`led` / `lcd-cs` / `lcd-dc` / `lcd-rst`）。
実機ボードの `roles` は空。

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
- **電気的な事実** — 外部プルアップ、`lcd-rst` が浮いて画面が白に戻る件
  （TODO §2） → 設定の対象ではない
- **プロトコル経由の書き換え**（`config apply`）は 2 段（USB 制御チャネル）。
  今は `deploy` がアプリと一緒に外から書く（1 段）

---

## 4. `wasmicon` CLI

### 4.1 置き場所と流儀

`tools/wasmicon-cli`（root workspace、bin 名 `wasmicon`）。依存クレートは入れてよい
（前例: `wasmicon-gen` の anyhow / wit-parser）。**「依存ゼロ・`alloc` 不使用」は
`runtime/` だけの制約**で、CLI には及ばない。引数パースは `wasmicon-gen` と同じ
手書き + `USAGE` 定数（日本語）に揃える。フラッシャ（picotool / espflash /
probe-rs）は**呼ぶだけで、自前実装しない**。

### 4.2 コマンド

**どこまで実装されているかは `docs/TODO.md` §5 が正**（ここには書かない。
CLAUDE.md「残作業を他の場所に書き足さない」）。「置き換える元」は、
今その仕事をしている既存のものを指す。

| コマンド | 中身 | 置き換える元 |
|---|---|---|
| `new <name> --lang rust\|as` | 雛形。ABI 準拠のビルドフラグを埋める（§4.4） | 無し |
| ~~`build`~~ | **作らない。** `cargo build --release` / `npm run build` を言語で振り分けるだけの薄いラッパで、フラグは `new` が埋めたファイルが持つ（§4.4）ので足せるものが無い | —— |
| `check <app.wasm> --board X` | **実ランタイムで** decode / validate / `Exec` / instantiate を**そのボードの arena の実寸で**通す（§4.3） | 無し（`wasm-tools validate` では import 表もボードの上限も見られない） |
| `run <app.wasm> [--trace] [--i2c-replay f]` | host ポートで実行。`--i2c-replay` は既存の環境変数 `WASMICON_I2C_REPLAY` に対応する | `cargo run -p wasmicon-host`（crate は残して lib として使う） |
| `deploy <app.wasm>` | USB / HTTP でアプリを送る。`--persist` でフラッシュスロット | 無し |
| `flash --board X [--fw v]` | **ファームを**焼く（一度だけ）。版は manifest から引く（§3.8） | 手順書（`apps/lcd-demo-rs/README.md`） |
| `fw list` | 手元にある版と、デバイスに載っている版を並べる（§3.8） | 無し |
| `pack <app.wasm>` | スロット画像にする（§3.4）。1 段では外のフラッシャに渡す素材 | 無し |
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
- `run` と `memory` の export（abi-spec §3.3 / §6.2）。**`run` は型まで見る**
  （`func()` 以外だと実行時に `wrong arity` で落ちるので、export の有無だけでは
  素通りする）。memory の**定義**が無いのも落とす（§6.2 は 1 つ要求している）
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

**検査は CLI 側で組む。** `wasmicon_host::run_wasm_opts` は `run` まで
呼ぶので検査には使えない。CLI が `wasmicon-core` + `wasmicon_port::Hal` +
`wasmicon_host::hal::HostBoard` を直に組んで decode → validate → `Exec` →
instantiate を並べる（host ポートを CLI のために太らせない）。
`instantiate` はゲストを実行しないので、ボード実装は何でも結果が同じ。

**`Config` だけでは足りない。** 線形メモリは arena の残り全部を取るので
（`Arena::alloc_rest`）、`max_memory_pages` に収まっていても
「decode / validate / `Exec` が先に取った残りに `min_pages * 64 KiB` が
入らない」ことがある。実機はそこで `instantiate` が落ちる。だから
`Profile` は `arena` / `scratch` の実寸を持ち、`check` はそれで通す
（`ports/rp2040` は 160 KiB しかなく、2 ページだと 20 KiB ほどしか余らない）。
**ポートの `static ARENA` もこの値を使う**ので、2 箇所に分かれない。

落ちた段（decode / validate / instantiate）は**区別して出す**。arena 不足は
instantiate で出るので、「validate が落ちた」と言うと `max_memory_pages` を
縮める方へ誘導してしまう（正しくは arena を増やす）。

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
profile::RP2040   2 ページ / I2C と SPI は未実装 / GPIO の制約（予約・バスのピン）
profile::RP2350   4 ページ / 全実装       / 同じ制約（ヘッダが同じ）
profile::ESP32S3  4 ページ / 全実装       / DevKitC-1 の制約
profile::HOST     Config::DEFAULT（緩い） / mock の役割の表（host だけが持つ）
```

`Config::DEFAULT` はこのために `runtime` に足した const で、`Default` が
それを返す（二重に書くと必ず食い違う）。

**実機ボードは役割の表を持たない**（2026-10-07。§3.9）。プロファイルが持つのは
配線表の検査に使う GPIO の制約（`gpio_count` / `reserved` / `bus_pins`）で、CLI は
`wasmicon.toml` の配線表をこれで検査し、ファームは起動時に同じ検査をもう一度かける。
（固定の語彙 `ROLE_NAMES` と、それをコンパイル時に見る `assert_role_names` は廃止した。）

### 4.4 ビルドフラグの単一真実（腐ると静かに壊れる）

アプリの契約の実体はこの 2 つ:

- `apps/.cargo/config.toml` — `-Ctarget-feature=-reference-types` /
  `-Clink-arg=--initial-memory=65536` / `-Clink-arg=-zstack-size=8192`
- `apps/*/asconfig.json` — `runtime: stub` / `disable: [..., reference-types]` /
  `enable: [sign-extension, nontrapping-f2i, bulk-memory, mutable-globals]`

`wasmicon new` の雛形がこれを写すと **3 重化**する。食い違うと次の形で静かに壊れる:
`reference-types` が混ざれば `call_indirect` のテーブル索引で弾かれ
（abi-spec §6.1 の注）、`--initial-memory` が増えれば RP2040 の 2 ページを超える。

**`wasmicon-gen --check` と同じ形にした**（2026-10-04 実装）。真実は
`tools/wasmicon-cli/src/flags.rs` が持つ **ファイルの中身そのもの**で
（値の表ではない）、`wasmicon new` はそれを書き出すだけ。`apps/` の実物との
一致は `tools/wasmicon-cli/tests/flags.rs` が**バイトで**見る。

- **コメント 1 文字の差でも落ちる。** 緩めると「値は合っているがどちらが
  真実か分からない」状態に戻る
- 検査は既に CI にある `cargo test` に乗るので、新しいジョブは要らない
- `[profile.release]` はここに入れない。食い違ってもアプリが**大きくなる
  だけ**で、静かには壊れない
- あわせて **Rust 版と AS 版で初期メモリがページ単位で一致**していること、
  **一番きついボード（`ports/rp2040` の 2 ページ）に収まる**ことも見る
  （`profile::PROFILES` から引く）

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
ok  rustc     1.97.1 (8bab26f4f 2026-07-14) + wasm32-unknown-unknown
ok  asc       Version 0.28.20
ok  picotool  v2.3.1 (Darwin, ...)
ok  espflash  4.5.0
--  probe-rs  無い（任意。デバッガを使うなら）

→ ファームを焼けるボード: rp2040 / rp2350 / esp32s3
  Rust のアプリはビルドできる
  AssemblyScript のアプリはビルドできる
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
$ wasmicon new room-monitor --lang rust --board rp2350
room-monitor/
  wasmicon.toml        [requirements] pin-roles は空（使う役割を書き足す）
  Cargo.toml           単体で立つ workspace + cdylib + profile.release
  .cargo/config.toml   -reference-types / --initial-memory=65536 / -zstack-size=8192
  rust-toolchain.toml
  src/lib.rs           run() の雛形（log だけ使う。役割の例はコメント）
  .gitignore

次にやること:
    cd room-monitor && cargo build --release
    wasmicon check <出力された .wasm> --board <ボード>
```

`.cargo/config.toml` の中身は `apps/.cargo/config.toml` と**バイト一致**。
一致は `cargo test` が見る（§4.4）。`--lang as` なら `asconfig.json` と
`package.json` と `assembly/index.ts` が出る。

**`wasmicon-hal` の依存はパスで書く。** crates.io / npm に出していないので
（`docs/TODO.md` §5 の「`new` と bindings の配布」）、`--hal <bindings の
置き場所>` を渡すか、省略時は**上に向かって `bindings/` を探す**。
見つからなければ版指定を書いて**その旨を出す**（配布の決定を先取りしない）。

`Cargo.toml` に `[workspace]` を入れてあるのは、既存の workspace の中に
置いたときに cargo が「workspace に入っていない」で止まるのを避けるため。
その場所では `.cargo/config.toml` の `rustflags` が**連結**されて同じ値が
2 回効くので、`new` がそれも言う（害は無い）。

#### 4. 内側のループ — 実機を触らない（host、1〜2 秒）

**`wasmicon build` は作らない。** `cargo` / `npm` を言語で振り分けて包むだけの
薄いラッパになり、ビルドフラグは `.cargo/config.toml` と `asconfig.json` が
既に持っている（§4.4）ので、足せるものが無い。

```
$ cargo build --release
    Finished `release` profile [optimized] target(s) in 0.4s

$ wasmicon check target/wasm32-unknown-unknown/release/room_monitor.wasm --board rp2350
room_monitor.wasm  4,549 B

abi      wasmicon:hal@0.1.0                          import 名で強制される
imports  13 件すべて一致                             (board 1 / gpio 3 / i2c 4 / log 1 / spi 3 / time 1)
exports  run, memory                                 あり
memory   初期 1 ページ
table    なし

[rp2350]
validate 初期 1 ページ ≤ 4                           ok
roles    led, lcd-cs, lcd-dc, lcd-rst                rp2350 にある（参考）
→ 通る

$ wasmicon run target/wasm32-unknown-unknown/release/room_monitor.wasm --trace --i2c-replay sht4x-replay.txt > host.log
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
$ wasmicon check target/wasm32-unknown-unknown/release/room_monitor.wasm --board rp2040
[rp2040]
validate 初期 1 ページ ≤ 2                           ok
i2c      このポートは未実装（実機では unsupported）  ← 落ちる
spi      このポートは未実装（実機では unsupported）  ← 落ちる
roles    lcd-cs, lcd-dc, lcd-rst                     rp2040 の配線表にある
→ 落ちる（このボードで 2 件）
```

`roles` の行は、`wasmicon.toml` の `requirements.pin-roles`（宣言）が、そのボードの
配線表（`[board.rp2040.roles]`）に揃っているかを見ている（§4.7）。宣言が無ければ
役割は照合しない（`.wasm` から確実に列挙できないため。§4.3）。

**これを言えるのはボードプロファイルが「実装済みインターフェース」を持つから**
（§3.8）。持たせなければ `check` は通り、実機で `i2c.bus.open` が
`unsupported` を返して初めて分かる。初期ページ数が収まり、配線表も揃って
いるので、**他に落ちる要素が無い。**

#### 5. 外側のループ — 実機に送る（数秒）

```
$ wasmicon deploy target/wasm32-unknown-unknown/release/room_monitor.wasm --monitor > pico.log
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
$ wasmicon deploy target/wasm32-unknown-unknown/release/room_monitor.wasm --board esp32s3 --monitor > esp32s3.log
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
$ wasmicon deploy target/wasm32-unknown-unknown/release/room_monitor.wasm --persist
フラッシュスロット 0x1010_0000 に 4,549 B 書いた (crc32 1f3a9c21, format 1)
次の電源投入から走る
```

#### 2 つのループ

| ループ | 何を回すか | 頻度 |
|---|---|---|
| 内側 | `cargo build` → `check` → `run`（host、mock HAL とリプレイ） | 常時 |
| 外側 | `deploy --monitor` → `trace diff` | 内側が通ったら |
| ファーム | `flash` | 稀（版を上げるときだけ） |

#### どのコマンドがどの段か（§5）

| 段 | 使えるようになるもの |
|---|---|
| 0 | `doctor` / `new` / `check` / `run` / `trace diff` / `monitor`（identity 無し） |
| 1 | `flash`（ファーム）/ `deploy --persist`（既存フラッシャ経由。**Pico は BOOTSEL 押下が残る**） |
| 2 | `info` / `deploy`（USB、ボタン不要）/ `deploy --monitor` / `monitor` の identity / `fw list` |
| 3 | `deploy --via http`（ESP32-S3） |

---

### 4.7 `wasmicon.toml`

プロジェクトに 1 つ置く。**ここに書くのは「アプリの性質」と「配線」。**
配線表は `deploy` が設定スロットに書き、ファームはその表だけで役割に答える（§3.9）。

```toml
# wasmicon.toml — プロジェクトに 1 つ
version = 1

[requirements]
# このアプリが `board.pin-by-role` で引く役割名（名前は自由）。番号は下に書く
pin-roles = ["lcd-cs", "lcd-dc", "lcd-rst"]

[defaults]
board = "rp2350"
i2c-replay = "fixtures/sht4x.txt"

# ボードごとの配線表。deploy が設定スロットに書く（§3.9）
[board.rp2350.roles]
lcd-cs = 17
lcd-dc = 20
lcd-rst = 21

[board.esp32s3.roles]
lcd-cs = 10
lcd-dc = 14
lcd-rst = 15
```

#### 何がどこの真実か

| 情報 | 真実はどこか | toml の役割 |
|---|---|---|
| 必要な役割名（`requirements.pin-roles`） | **toml**。アプリの性質 | 真実そのもの |
| 役割 → GPIO | **toml**（`deploy` が設定スロットに書く。§3.9） | 真実そのもの。デバイスはそれを読むだけ |
| ABI 準拠のビルドフラグ | **`.cargo/config.toml` / `asconfig.json`** | **書かせない**（下記） |

#### `requirements.pin-roles` と配線表を突き合わせる

§4.3 のとおり、役割名は実行時に文字列で渡るので `.wasm` から確実に列挙できない。
宣言があれば列挙が確定するので、**送る前に**「このボードの配線表に `lcd-rst` が
無いのでこのアプリは動かない」と言える。

**節に分けてあるのは、同じ性質の未宣言項目がもう 1 つあるから。**
`i2c.bus.open` / `spi.bus.open` の `index` も実行時の整数引数で、`.wasm` から
静的に列挙できない（バスが 1 本のボードに index 1 を開くアプリを送ると実機で
`unsupported`）。v0.1 では書かないが、必要になれば
`[requirements]` に `i2c-buses = [0]` を足すだけで済む。
**インターフェースの実装状況は import から分かる**ので宣言は要らない（§4.3）。

名前は `requirements.pin-roles`。`requires` だと何が必要なのか読めず、
`required-pin-roles` のような平坦なキーは項目が増えるたびに長くなる。

**タイポはデバイスに繋がなくても止まる。** 語彙は無いので名前の書式だけを見るが、
宣言（`pin-roles`）と配線表のどちらかでタイポすれば両者が食い違い、`check` /
`deploy` が「配線表に無い」と言う。

#### スキーマの規則

| 決め | 内容 |
|---|---|
| **導出できるものは書かない** | アプリ名と言語は `Cargo.toml` の `package.name` / `asconfig.json` の有無から取る。toml に書くのは上書きとしてだけ。§4.4 と同じ理由で、二重に持つと必ず drift する |
| **1 アプリに 1 つ** | workspace でも**アプリごとに置く**。この repo の `apps/` をドッグフードするなら 5 つになる。`wasmicon new` が 1 つ出す形と揃う |
| **必須のみ** | `pin-roles` は「無ければ動かない」ものだけを並べる。`led` が無くても動く degradation は v0.1 では表現しない（`optional-pin-roles` は実例が出てから） |
| **「この役割は無い」は `"none"`** | `lcd-rst = "none"`。キーを省略したのと同じで、配らない（ファームに既定の表は無い）。`false` や `0` より誤読しにくい |
| **パスは toml のあるディレクトリ基準** | `i2c-replay = "fixtures/sht4x.txt"`。CLI の cwd 基準にすると、どこから呼んだかで壊れる |
| **未知のキーはエラー** | 黙って無視すると「設定したのに効いていない」に気付けない。タイポはここで止める |
| **`version`** | CLI の対応より新しければエラー、古ければ受ける |
| **toml は任意（ただし実機で役割を使うなら要る）** | `check` / `run` は **toml が無くても動く**。ただし実機ボードは既定の表を持たないので、toml が無いまま `deploy` したアプリは役割を 1 つも引けない |

- **プロジェクトの中で `check` / `deploy`** → 宣言と配線表を突き合わせる
- **`.wasm` 単体を受け取って** → 役割は照合しない（列挙できないので）

`wasmicon.toml` は**カレントから上に向かって探す**（cargo と同じ）。
宣言と配線表が食い違えば、**デバイスに繋がなくても焼く前に止まる**:

```
$ wasmicon check app.wasm --board rp2350
...
roles    lcd-cs, lcd-dc, lcd-rst    rp2350 の配線表に無い: lcd-rst  ← 落ちる（[board.rp2350.roles] に書く）
```

バイナリと一緒に運びたくなったら、そこで初めてカスタムセクション
（`wasmicon.roles`）か `wasmicon pack` で束ねる話になる。ランタイムは custom
セクションを読み飛ばすので実行には影響しないが（`runtime/src/decode.rs:79`）、
**`.wasm` のバイト列が変わる**ので記録済みハッシュ（`fc470947…`）を取り直すことに
なる。それが要るまで入れない。

#### `deploy` は配線表を毎回書き、書く前に表示する

> 2026-10-07 に方針を変えた。以前は「デバイスの表と toml を照合し、食い違えば止めて
> `config apply` で明示的に押し込む」としていたが、**ファームが既定の表を持たず、
> 表の真実が toml になった**ので、`deploy` が毎回書く（§3.9）。

```
$ wasmicon deploy app.wasm --board rp2350
app.bin → rp2350 のスロット（+0x100000）  4565 B（wasm 4549 B、crc32 c1cbe3c1）
  配線表: lcd-cs=17 lcd-dc=20 lcd-rst=21
```

**役割マップを書き換えると実際に駆動されるピンが変わる**ので、繋いでいる物に
よっては物理的に危ない、という懸念は残る。今の扱い:

- 表は**そのアプリ自身の `wasmicon.toml`** から来る（他人の設定が混ざらない）
- `deploy` は**書く前に表を 1 行で表示する**
- 予約ピン・バスのピン・重複は CLI とファームの両方が弾く
- 表が壊れていれば**どのピンも駆動しない**側に倒れる

デバイスにある今の表との差分を出して確認を求める（以前の「照合」）には、
デバイスから表を読み戻す手段（2 段の USB 制御チャネルの `info`）が要る。それまでは
上の表示で代える。

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
| 0 | CLI の骨 + `new` / `check` / `run` / `monitor` / `trace diff` / `doctor` | **ゼロ** | 不要 |
| 1 | §3.2 の掃除 → スロット形式 → 「スロットを読んで走る」ファーム → **内蔵アプリの撤去**（§3.3）→ 既存フラッシャで `deploy` | ローダの芯 | 要 |
| 2 | USB 制御チャネル + §3.6 のプロトコル。`info` と `probe`、役割マップの設定（§3.9）が入る | USB スタック | 要 |
| 3 | HTTP（ESP32-S3） | Wi-Fi + RAM 予算の判断 | 要 |

- **0 段は TODO §1.3 の SHT40 作業にそのまま効く**（取り込みと突き合わせ）
- **1 段の時点で ESP32-S3 はボタン操作不要**（espflash が DTR/RTS でリセットする）。
  Pico は BOOTSEL 押下が残り、2 段で消える
- **bindings の配布**（crates.io / npm）はどの段とも独立。ここで決め打ちしない。
  `new` は 0 段に入ったが、依存はパスで書いている（§4.6 の 3）

---

## 6. 変えないもの / 触ると壊れるもの

- **ABI**: import 名、シグネチャ、エラーコード（discriminant + 1）、ハンドル 0 = 無効
  （handoff §2）。必要になったら実装せずオーナーに確認する
- **`wit/` が唯一の真実**、生成物はジェネレータ出力のみ（CLAUDE.md）
- **記録済みの `.wasm` ハッシュとトレース**（verification-report §6 / §7、TODO §3）。
  `new` の雛形は `apps/` と同じフラグを持つので同じバイト列が出る（§4.4）。
  ローダはトレース行を増やさない
  （§3.7）
- ~~内蔵アプリと `guest-lcd-demo` feature は 1 段まで触らない。~~ → 3 ポートが
  スロットを読めるようになったので 2026-10-07 に外し、CI も同時に直した（§3.3）
- **handoff §5 の Phase 4 / 5 / 6 の完了条件**（ESP32-S3 と Pico WH の 2 ボード定義）
- **トラップ後に同じアプリを自動再実行しない**（handoff §3 #4）。ローダは
  「新しいアプリを受け付ける」だけを足す（§3.1）

---

## 7. 未決

**`docs/TODO.md` §5 に集約した。** 判断が要るのは、トラップ後の idle 復帰の確定、
ESP32-S3 の RAM 予算（`max_memory_pages` を落とすか PSRAM か）、HTTP の向き、
Pico W の Wi-Fi（embassy 移行）、ボードの品種（フラッシュ容量と PSRAM の有無）、
ファーム版の振り方。
