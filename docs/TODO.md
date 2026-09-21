# 残作業

最終更新: 2026-09-11

**全 6 フェーズのソフトウェア側は完了**し、CI も green。残っているものをここに集約する。
散らばると更新漏れで嘘になるので、**残作業はこのファイルだけに書く**。
決定済みの事項と経緯は `docs/handoff.md`、検証の現状は `docs/verification-report.md`。

---

## 1. 実機が要るもの

実機（ESP32-S3 DevKitC-1 / M5Stack Tab5 / Raspberry Pi Pico WH）が
手元に来るまで進められない。

### 1.1 オーナーに聞くこと

- [ ] **実機の配線**。`docs/abi-spec.md` §8 の表（I2C/SPI のピン、役割名 → GPIO 番号）が実機と合っているか
- [ ] **シリアルの接続方法**。RP2040 は UART0 (GP0/GP1)、ESP32-S3 は UART0 (GPIO43/44)、Tab5 は UART0 (G37/G38 = M5-Bus 13/14) を前提にしている
- [ ] **モジュールの型番**。ILI9341 は 3.3V ロジックの SPI 版、SHT31 は I2C アドレス 0x44 を前提にしている。
      **Tab5 の内部 I2C には PI4IOE5V6408-2 が 0x44 で載っている**ので、SHT31 を内部バスに
      繋ぐならアドレスを 0x45 にする必要がある（現在は PORT.A に出しているので衝突しない）
- [ ] **役割名**。`led` / `lcd-cs` / `lcd-dc` / `lcd-rst` を既定のまま確定扱いで進めている。変えるなら 3 箇所（`wit/board.wit` のコメント、abi-spec §8 の表、各ポートの `ROLES`）
- [ ] **`led` に外付け LED を充てている**。どのボードもオンボード LED が素の GPIO ではないため（Pico W/WH は CYW43439、DevKitC-1 は WS2812、**Tab5 はユーザーが振れる LED を持たない**）
- [ ] **PORT.A の SDA/SCL（ESP32-P4 / Tab5）**。Tab5 の PinMap は色と GPIO
      （Yellow=G53, White=G54）までで、どちらが SDA かを書いていない。
      M5Stack の通例（Yellow=SCL, White=SDA）に従って **SCL=G53 / SDA=G54** と
      しているが、**逆だとする二次情報もある**。実機かテスタで確定させる。
      **P4 の I2C は実装済みになったので、ここが逆だと PORT.A に繋いだ
      ユニットが動かない**（内部 I2C は G31/G32 で別系統なので影響しない）
- [x] **トレースの経路は UART0 で 3 ポート揃えた**（2026-09-11）。一度
      USB-Serial-JTAG に変更したが、この個体で出力が取れず戻した（§1.1.5）
- [ ] **M5-Bus のラベル付きピンが何か**。PB_IN (G17) / PB_OUT (G52) は名前から
      電源ボタン系と推定して `reserved` に入れてある。違うならゲストに開けてよい。
      PC_RX (G7) / PC_TX (G6) は逆に**開けたまま**にしてあるが、書き込み経路の
      UART なら塞ぐべき
- [ ] **Tab5 の PinMap に出てこない GPIO（0, 1, 24, 25, 33, 46, 49, 50）の扱い**。
      周辺の表にも M5-Bus にも PORT.A にも出てこない。塞いでいないが、未接続なのか
      内部で使われているのかが分からない。なお 33 と 35 は P4 の strapping ピン
      (32..=38) なので、開けてはいるが出力に使うと起動に影響しうる

### 1.1.5 シリコンリビジョン（解決済み）と、次に出たトレース取り込みの問題

#### 解決済み: Tab5 の ESP32-P4 v1.0 で起動する

2026-09-11 の最初の書き込みは 2nd stage bootloader に弾かれた:

```
E (79) boot_comm: chip revision check failed. Required >= v3.0, found v1.0.
```

**原因は esp-hal の既定値**で、ポートの不具合ではなかった。

- 手元の個体は **ESP32-P4 v1.0**（ROM `esp32p4-eco2-20240710`）
- espflash はイメージヘッダの `min_chip_rev_full` を ELF のメタデータ
  (`build_info.MIN_CHIP_REVISION`) から読み、`flash_data.min_chip_rev.max(metadata)`
  で下から clamp する。だから espflash の `--min-chip-rev 0.0` は効かない
- その metadata を書いているのは esp-hal の esp-config オプション
  `min-chip-revision`。**P4 の既定が 300 (v3.0)** になっている

**解決**: `ports/esp32p4/.cargo/config.toml` の `[env]` で

```toml
ESP_HAL_CONFIG_MIN_CHIP_REVISION = "100"
```

これは **ESP-IDF の `CONFIG_ESP32P4_REV_MIN_100`（"Rev v1.0"）に対応する正規の
設定**で、迂回ではない。設定後はイメージヘッダが min=100 / max=199 になり、
この max=199 は ESP-IDF の `ESP32P4_REV_MAX_FULL = 199 if ESP32P4_SELECTS_REV_LESS_V3`
と一致する。`--force` なしで書き込め、`boot: Loaded app from partition at
offset 0x10000` まで到達する。

v3.x の個体しか使わなくなったら 300 に戻してよい。

#### 解決済み: トレースの取り込み（2026-09-21）

**Tab5 実機からトレースを取り込めるようになった。** 経路は UART0 (G37) →
3.3V USB シリアル変換（Adafruit CP2102N）→ PC。配線は 2 本だけ:

| CP2102N | Tab5 M5-Bus |
|---|---|
| GND | pin 1（または 3 / 5） |
| RXD | **pin 14 = G37 (TXD0)** |
| 5V / 3V | **繋がない**（Tab5 は自前で給電済み） |

115200 8N1。ピン 14 の隣が pin 12 = 3V3 なので挿し間違いに注意。

取り込み側の落とし穴が 2 つあった。

1. **macOS では `cu.*` をクローズすると termios が既定へ戻る。** 別プロセスで
   `stty` を打ってから `cat` で開き直しても効かず、既定の 9600 で聴いてしまう。
   **fd を開いたままボーレートを設定する**こと（`tcsetattr` 後に読み返して検証する）
2. **USB-C を PC に繋いだまま espflash からリセットすると必ず download モードに
   落ちる**（`rst:0x17 CHIP_USB_UART_RESET` / `boot:0x204 DOWNLOAD`）。
   `--after hard-reset` / `watchdog-reset` / `--before no-reset-no-sync` のどれでも同じ。
   **電源ボタン（ダブルプレスで OFF → 1 回押しで ON）なら通常起動する**
   （`rst:0x1 POWERON` / `boot:0x20e SPI_FAST_FLASH_BOOT`）。
   `espflash reset` を 2 回叩くと 2 回目が通常起動になることもある

#### 解決済み: esp-hal 1.2 が P4 v3.x / ECO5 を前提にしている（2026-09-21）

手元の Tab5 は **P4 v1.0 / ROM `esp32p4-eco2-20240710`**。esp-hal 1.2 系は
より新しいシリコンと ROM を前提にしており、**同じ構図の不整合が 3 つ**出た。
シリコンリビジョン（上記）を含めると 4 つ。

1. **esp-sync の Zcmp 回避が不正命令になる。**
   `esp-sync 0.3.0` の `raw.rs` は `SingleCoreInterruptLock::enter` で
   `csrrw a0, 0x347, t0`（`mintthresh`）を書く。これは **P4 v3.2/ECO7 の Zcmp
   ハードウェアバグ回避**（IDF-14279 / DIG-661）だが、分岐条件が
   `cfg(esp32p4)` だけで**リビジョンも Zcmp の有無も見ていない**
   （上流自身が `// TODO: any with zcmp` と書いている）。v1.0 では CSR 0x347 が
   不正命令になり、`esp_hal::init` の中で例外 → RWDT リセットの無限ループ。
   しかも我々のターゲット `riscv32imafc` は **Zcmp を含まない**ので元々不要。
   → `ports/esp32p4/vendor/esp-sync` に写しを置き、`[patch.crates-io]` で
   差し替えて P4 分岐だけ削除（汎用 riscv 経路へフォールバック）。
   差分は `WASMICON LOCAL PATCH` の 2 箇所だけ。

2. **ROM 関数のアドレス表が ECO5 固定。**
   `esp-rom-sys 0.1.5` の `ld/esp32p4/rom-functions.x` は
   `esp32p4.rom.eco5.*.ld` をハードコードしている（選択肢が無い）。
   非 ECO5 版は同じディレクトリに同梱されているのに使われない。
   実害: `__ashldi3` が ECO5=`0x4fc00744` / 非 ECO5=`0x4fc00750` と **12 バイト**
   ずれ、64bit 可変シフトが別の関数に当たる。`u64_leb` が 1 ではなく
   `0x0000_0001_0000_0001` を返し、`decode` が
   「memory size must be at most 65536 pages」で失敗していた。
   **クラッシュせずもっともらしい誤値を返す**ので発見が難しい。
   → `ports/esp32p4/rom-pre-eco5.x` で非 ECO5 の表を読み直し、
   linkall.x の**後**に渡してシンボル代入の後勝ちで上書きする。

3. **`memcpy` / `memset` / `memmove` / `memcmp` も ECO5 のアドレス。**
   `ld/esp32p4/rom/additional.ld`（冒頭に `Ref: esp-idf esp32p4.rom.libc.ld (eco5)`）
   にあり、非 ECO5 版のアドレスは同梱されていない。
   → ROM を使わず `ports/esp32p4/src/mem.rs` の自前実装へ向ける。
   LLVM が「バイトコピーのループ」を `memcpy` 呼び出しへ畳み込んで自分自身を
   無限再帰で呼ぶのを防ぐため、読み書きに volatile を使っている。

**上流が ROM リビジョンとチップリビジョンを選べるようになったら、
2 と 3 と `vendor/esp-sync` は消すこと。** 上流への報告は未実施。

#### 解決済み 4: スタックが実在しない RAM に置かれていた（2026-09-21）

**これが最後の、そして最も厄介な 1 件。** 上の 3 つを直すと
`decode` / `validate` / `Exec::new` は実機で完全に正しく動くようになったが、
`instantiate` を通ると `Module` が丸ごと 0 に潰れた。

```
after decode:   types=7 imports=7 funcs=3 mems=1 exports=2 code=3
after validate: types=7 imports=7 funcs=3 mems=1 exports=2 code=3
after exec:     types=7 imports=7 funcs=3 mems=1 exports=2 code=3
after inst:     types=0 imports=0 funcs=0 mems=0 exports=0 code=0
```

**原因**: esp-hal は P4 の L2MEM を 768KB (0x4FF00000..0x4FFC0000) とみなして
`_stack_start = 0x4FFADFC0` を置く。しかし実測すると
**0x4FF9E000 は生きていて 0x4FFA0000 は死んでいる**。使えるのは
**0x4FF00000..0x4FFA0000 の 640KB だけ**で、上位 128KB は L2 キャッシュに
割り当てられていて RAM として存在しない（128KB は P4 の L2 キャッシュ容量）。

実在しない領域のスタックは**キャッシュに載っている間だけ正しく見える**。
`instantiate` が線形メモリを 64KB ゼロ埋めするとキャッシュラインが追い出され、
書き戻し先が無いのでスタックの内容が失われる。クラッシュせず静かに壊す。

**解決**: `ports/esp32p4/rom-pre-eco5.x` で `_stack_start = 0x4FFA0000;`。
（`_stack_start_cpu0` は上書きできないが、esp-riscv-rt が実際に使うのは
`_stack_start` の方だったので効いた。）

切り分けに効いた観測（再開するとき参考になる）:

- fill のサイズを振ると **32KB は無傷、48KB で壊れる**。サイズ依存＝
  キャッシュの追い出し量依存だった
- スタック上のカナリアは **volatile で読み書きしないと意味がない**。
  普通の代入だと最適化でレジスタに載り、スタックが壊れても検出できない
- `0x4FF00000` から一定間隔で目印を書き、arena を大量に触って追い出してから
  読み戻すと、実 RAM の上端が 1 回で出る

**上流が L2 キャッシュ設定を見て RAM 長を決めるようになったら消すこと。**

#### 到達点: blink が実機で完走（2026-09-21）

```
$ sh verify/diff-traces.sh blink-host.log blink-tab5.log
一致: 20 行
```

**同じ `.wasm` が PC と Tab5 実機で完全に同一の host call 列を出す。**
GPIO のレジスタ直叩きも含めて動作しており、docs/handoff.md §5 Phase 4 の
完了条件を満たした。

- [ ] 再開するときの足場: `chip::early_write` と `EarlyOut`（`core::fmt` の
      出力先）を残してあるので、`write!(EarlyOut, ...)` を刻めばよい
- [ ] **u64 を `core::fmt` で 10 進整形しない**。`<u64 as Display>::fmt` の
      スタックバッファが esp-hal のスタックガードに当たって panic する。
      u32 2 本に割るか 16 進で出すこと
- [ ] `ARENA` は 300KB のままだが、スタックは
      `__ebss (0x4FF902C4) .. 0x4FFA0000` の **約 63KB** に減った。
      sensor-display のような深い経路を通すときは足りるか確認すること

#### 経緯: USB-Serial-JTAG にトレースが出てこない（2026-09-21 に UART0 で解決）

アプリはロードされ実行に入るが、**トレースを取り込めていない**。

**USB-C の接続先は確定した（疑いは晴れた）。** `ioreg` で見ると
`USB JTAG/serial debug unit / Espressif` として列挙されるので、**Tab5 の USB-C は
P4 の USB_DEVICE (USB-Serial-JTAG) に繋がっている**。ペリフェラルの選択は正しい。

それでも出てこない。分かっている事実:

- ブートローダの `Loaded app from partition at offset 0x10000` までは必ず届く。
  その直後にホスト側が `Broken pipe` になる
- **アプリ起動時に USB が再列挙される**（`ioreg` の registry id が変わる）。
  `esp_hal::init` が `disable_peripherals()` で一度落とし、こちらが
  `UsbSerialJtag::new` で入れ直すため
- 再列挙後に `/dev/cu.usbmodem*` を開き直しても **0 バイト**。20 秒待っても何も出ない
- 保存 PC は `esp_sync::GenericRawMutex::acquire`（`UsbSerialJtag::write` の
  ホスト待ちビジーウェイトの中）を指す
- `open_serial` に 2 秒の待ちを入れるとリセットループ自体は止まった（改善）。
  ただし出力は出ないまま
- `--before no-reset-no-sync --after no-reset` でリセットせず聴いても出ない
- `cat` でポートを開くと `rst:0x17 CHIP_USB_UART_RESET` が起き、ROM バナーの
  31 バイトだけ取れて切れる（開くと DTR が動いてリセットされる）

**→ 下の「最小再現で切り分けた結果」で実証した。wasmicon のコードは無関係。**

**バックライトによる目視確認は成立しない（2026-09-11 に実機で確認）。**
`led-backlight` feature で `led` を G22 に向けても光らなかった。原因は
**Tab5 のバックライトが G22 だけでは点かない**こと:

```c
#define BSP_LCD_BACKLIGHT  (GPIO_NUM_22)            // P4 の GPIO
#define BSP_LCD_EN         (IO_EXPANDER_PIN_NUM_4)  // I2C エキスパンダ
```

esp-bsp の `bsp_feature_enable(BSP_FEATURE_LCD)` は PI4IOE5V6408（内部 I2C,
0x43）のピン 4 を立てて LCD の電源を入れている。内部 I2C (G31/G32) は
`reserved` で I2C 自体も未実装なので、**エキスパンダを叩けるようになるまで
画面は目視確認に使えない**。したがって「光らなかった」はアプリが動いて
いない証拠にならない。

- [x] **内部 I2C + PI4IOE5V6408 は実装済み**（ポート側の初期化として扱い、
      ゲストにも `Board` にも渡していない）。2026-09-21 に実機で
      `lcd: power on` を確認した = **エキスパンダ 0x43 が ACK を返している**。
      これで内部 I2C の実通信が動いていることは確定した。
      ただし**画面が見えるかはまだ別**（バックライトの明滅は未確認）

**2026-09-11 の追試で分かったこと**（いずれも実機）:

- `Serial::write` に上限を付けた（50ms）。**esp-hal の `UsbSerialJtag::write` は
  ホストがドレインするまで無限にビジーウェイトする**ので、モニタ未接続の実機では
  最初の 1 行で止まる。保存 PC がその待ちループを指していたのはこれ。
  上限を付けた副作用として、**ホスト未接続時の出力は捨てられる**
- 1 秒ごとのハートビートを出しても**ホスト側で 0 バイト**。後から繋いでも拾えない
- 内部 I2C + エキスパンダで LCD 電源を入れ、G22 を明滅させる診断を入れたが
  **画面に変化なし**。ただし I2C 成否と画面可視性を分離できていない
- 投機的に入れていた 2 秒待機は外した。タイマー依存の待ちを起動経路の先頭に
  置くと、タイマーが動いていない場合にそこで全部止まり、切り分けを妨げる

#### 最小再現で切り分けた結果（2026-09-11、決着）

**この Tab5（ESP32-P4 v1.0）では、アプリからの USB-Serial-JTAG 出力が取れない。
wasmicon のコードは無関係。**

wasmicon を一切含まない 30 行の esp-hal アプリで再現した:

```rust
#![no_std] #![no_main]
esp_bootloader_esp_idf::esp_app_desc!();
#[esp_hal::main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default());
    let mut usb = UsbSerialJtag::new(p.USB_DEVICE);
    loop {
        let _ = usb.write(b"p4-min alive\r\n");
        let _ = usb.flush_tx();
        for _ in 0..20_000_000u32 { core::hint::spin_loop(); }  // タイマー非依存
    }
}
```

- I2C もタイマー依存の待ちもゲスト実行も含まない
- それでも `boot: Loaded app from partition` の直後に `Broken pipe` になり、
  **`p4-min alive` は 1 行も出ない**
- アプリ実行中にポートを開き直しても **0 バイト**（1 秒ごとに出しているのに）
- ROM とブートローダの出力は同じ USB から出ている。
  **ROM 側の USB は動いていて、アプリが握った瞬間に止まる**

ESP-IDF が v0.x〜v1.x と v3.x を別レンジ (`REV_LESS_V3`) にして既定を v3.1 に
していることと整合する。**シリコンの errata という仮説を強く支持する**（未確定）。

**判断ミスの記録**: トレースを UART0 から USB-Serial-JTAG に変えたのは
「部品不要」を優先した判断だったが、**この個体では機能しない経路だった**。
その結果、観測手段が無いまま仮説ベースの変更を重ねることになった。
UART のままなら USB シリアル変換 1 個で最初から観測できていた。

#### USB-OTG も試したが列挙されなかった（2026-09-11）

arduino-esp32 の Tab5 定義が `build.usb_mode=0`（TinyUSB / USB-OTG）なので、
**Serial-JTAG ではなく OTG が正しい経路**と考えて試した。オーナー環境では
「USB CDC On Boot = Enabled」で `/dev/cu.usbmodem14401` にログが出ている。

- esp-hal の OTG ドライバは embassy-usb 前提。executor を持たずに手で
  ポーリングする形で最小再現を書いた（`Usb::new_hs` + `CdcAcmClass`）
- **最初の版は非同期ブロックの中で数え上げスピンしていて、その間
  `device.run()` がポーリングされず列挙が成立しなかった。** これは自分のバグ。
  待ちを入れるなら必ず await する形にすること
- 直した版でも**列挙されない**。アプリ実行後も `USB JTAG/serial debug unit`
  のままで、こちらの CDC は現れない
- 「USB-C が Serial-JTAG のパッド側で HS は未接続」なのか「アプリが USB 初期化
  前に落ちている」のか、**観測手段が無いので区別できていない**

**教訓: 観測手段を確保する前にハードの試行錯誤を続けない。** このセッションで
同じ失敗を 2 度繰り返した（Serial-JTAG と OTG）。トレースを UART0 から動かした
判断が、そもそもの原因。

未取得の決定的情報:

- [ ] **Arduino 動作中の USB 識別情報**（VID/PID・Product Name）。
      `ioreg -c IOUSBHostDevice -w0 | grep -B2 -A6 -i "vendor name"` 等で取れる。
      これが分かれば、どの USB コントローラが USB-C に繋がっているかが確定する

- [x] **トレースを UART0 (G37/G38) に戻した**（2026-09-11、オーナー判断）。
      3 ポートで経路が揃う。UART は受け手がいなくても送信が詰まらないので、
      `UsbSerialJtag` のようにホスト未接続で止まることもない
- [ ] **3.3V の USB シリアル変換を用意し、M5-Bus 13/14 に繋ぐ。**
      これで初めて実機のトレースが読める。**ここが Phase 4 の前提**
- [ ] USB-C 経由の観測を諦めたわけではない。**追加部品は技術的な必要条件ではなく**、
      Arduino は USB-C でログを出せている（CDC On Boot = Enabled）。
      決定的な未取得情報は「Arduino 動作中の USB の Product Name / VID・PID」で、
      これが分かればどの USB コントローラが USB-C に繋がっているか確定する

選択肢:

- [ ] **v3.x シリコンの P4 で試す。** 上の仮説の検証も兼ねる。一番情報量が多い
- [ ] **トレースを UART0 (G37/G38) に戻す。** USB を経由しないので確実。
      ただし USB シリアル変換と M5-Bus 13/14 への配線（＝部品）が要る
- [ ] `UsbSerialJtag::write` のブロックに上限を付ける。ハングはしなくなるが
      **トレースが黙って欠ける**。検証の測定器としては最悪の壊れ方なので、
      入れるなら欠けたことを検出できる形にすること

### 1.1.7 `wasmicon:device` の切り出し（ABI 面は完了、実装が残り）

方針は `docs/abi-spec.md` §11（2026-09-11 承認）。`wasmicon:hal` は凍結し、
内容を変えない。**hal の import 表は 20 件のまま 1 行も動いていない**
（組み替え前後で `--sigs` を diff して確認済み）。

済み:

- [x] `wit/` を 3 パッケージに組み替えた（`wasmicon:app` / `hal` / `device`）。
      world を hal に残したまま device を import すると、device が hal の
      `error-code` を `use` した時点で**依存が循環して弾かれる**ため
- [x] `wit/deps/device/display.wit` と `world app-display` を追加
- [x] ジェネレータと `tools/wit2sig.py` のモジュール名をインターフェースの
      所属パッケージから引くようにした
- [x] ジェネレータが 2 world を扱う（import は上位集合 `app-display` から採り、
      群の順序を hal → device に固定する）
- [x] `ImportDesc` に `group` を持たせ、`ports/common` は **hal 群だけを登録**する。
      `world app-display` のゲストは display を持たないポートでリンクエラーになる
- [x] `check-sigs.sh` が 27 imports で一致。**独立実装の `wit2sig.py` が device の
      lowering にも同じ結論を出している**
- [x] abi-spec §11.6 に device の正規表を追加し、`tests/abi_spec.rs` が転記して検査

残り:

- [ ] バインディング（Rust / AssemblyScript）に device のモジュールを足す。
      現状 `bindings/` は生成されるが、ゲストから使う薄いラッパが無い
- [ ] `verify/diff-traces.sh` に device 行を落とす処理を足す。**方針は決定済み**
      （§11.4、2026-09-11）: device の呼び出しは §9 の書式でトレースに出し、
      比較時に diff-traces 側で落とす。`[wasm]` 行と同じ扱い。
      `--self-test` にも device 行が落ちることの検査を足すこと
- [ ] display を提供するポート側の実装（Tab5 の MIPI-DSI）。
      **その前に §1.1.5 の「内部 I2C + エキスパンダで LCD の電源を入れる」が要る**

### 1.2 実装

- [ ] **`ports/rp2040` の I2C / SPI**。現在は `unsupported` を返す。これが無いと sensor-display は実機で動かない
- [ ] **`ports/esp32s3` の I2C / SPI**。同上
- [x] **`ports/esp32p4` の I2C**。PORT.A を `i2c.bus` index 0 として実装した（SPI は未実装）
- [ ] **`ports/esp32p4` の 2 つ目のボード定義**。チップ層 (`src/chip.rs`) と
      ボード定義 (`src/boards/`) は分けてあるが、**定義は Tab5 の 1 つだけ**。
      M5Stamp ESP32P4 はオンボードの GPIO 割り当てが非公開で書けない。
      Function-EV-Board と P4-EYE なら esp-bsp のヘッダから今すぐ書ける
      （実機がある場合）。**2 つ目を足すときは `src/boards/mod.rs` の
      「複数選択」ガードを一緒に足すこと**（コメントに書いてある）

### 1.3 検証（Phase 4 / 5 / 6 の完了条件）

Phase 4 / 5 / 6 の完了条件は **ESP32-S3 と Pico WH の 2 ボード**で定義されている
（`docs/handoff.md` §5）。ESP32-P4 は後から足したボードなので完了条件は変えていない。
P4 の分は下に別項として置く。

- [ ] 両ボードで `blink-rs` / `blink-as` が動き、シリアルのトレースが host 版と一致（Phase 4）
- [ ] 4 通り（Rust/AS × 2 ボード）で表示が出る（Phase 5）
- [ ] 同一 `.wasm` を両ボードで走らせ、`time` を除くトレースと SPI ピクセル CRC が完全一致（Phase 6）
  - 手順は `docs/verification-report.md` §5
  - 突き合わせは `sh verify/diff-traces.sh a.log b.log`
- [ ] **ボード間の浮動小数の一致**。sensor-display が唯一 f32 を使う温度バーの計算。RP2040 はソフトフロート、ESP32-S3 は f32 のみハード FPU（非正規化数の扱いに設定依存あり）。ここが Phase 6 の本来の実測対象
- [ ] `verify/sht31-replay.txt` を**実機から記録した応答**に差し替える（現在は合成データ）
- [ ] 結果を `docs/verification-report.md` に反映する

ESP32-P4 の分（Phase 4 / 5 / 6 と同じことを 3 ボード目にも通す）。
**2026-09-21 に `blink-rs` が完走し、host 版とトレース差分ゼロを確認した（§1.1.5）**:

- [x] ESP32-P4 で `blink-rs` が動き、トレースが host 版と一致（20 行）
- [x] ESP32-P4 で `blink-as` が動き、トレースが host 版と一致（20 行）。
      **実機で Rust 版と AS 版も 20 行一致**。ゲストの切り替えは
      `cargo build --release --features guest-as`
- [x] **`start` セクションの呼び出しが `ports/esp32p4` から抜けていたのを修正**。
      Rust 版は start を持たないので blink-rs では気づけず、AS 版で
      `Index out of range`（`~lib/typedarray.ts:878`、`out32` が未初期化）
      として露見した。**AS 版は「Rust 版で通った」だけでは代替できない**
- [ ] ESP32-P4 で sensor-display の表示が出る（Rust / AS）
- [ ] 同一 `.wasm` を 3 ボードで走らせ、`time` を除くトレースが完全一致
  （`sh verify/diff-traces.sh esp32s3.log esp32p4.log` を追加で回す）
- [ ] **P4 の f32 の一致**。P4 の HP コアは RV32IMA**F**C でハード FPU（単精度）。
      RP2040 のソフトフロートと ESP32-S3 の Xtensa FPU に加えて 3 つ目の実装になるので、
      温度バーの計算がここでも一致するかは実測対象

### 1.4 実機で最初に疑うところ

**ESP32-P4 は 2026-09-21 に blink が完走し、GPIO の直叩きも観測できた。**
RP2040 / ESP32-S3 はまだ一度も観測していないので、
「blink が光らない」を最初の期待値として想定すること。

- RP2040: SIO / IO_BANK0 / PADS_BANK0（FUNCSEL=5）
- ESP32-S3: GPIO / IO_MUX（MCU_SEL=1、GPIO マトリクスの out_sel=128）
- ESP32-P4: GPIO / IO_MUX（MCU_SEL=1、GPIO マトリクスの **out_sel=256**）。
  S3 と定数が違うのはここと GPIO 本数（0..=54）だけで、レジスタ名は同じ

**ただし GPIO を疑う前に、そもそもアプリが起動したかを先に確認すること。**
2026-09-11 の Tab5 では、書き込みは成功したのにブートローダで弾かれていて、
ランタイムのコードに一度も到達していなかった（§1.1.5）。起動していれば
バナー（`wasmicon esp32p4/tab5`）が出る。**バナーが出ないうちは GPIO の話ではない。**

#### Tab5 で部品なしに目視する（`led-backlight` feature）

```sh
cd ports/esp32p4 && cargo run --release --features led-backlight
```

`led` 役割を LCD のバックライト (G22) に向けるビルド。外付け LED も M5-Bus への
配線も要らず、blink が画面の明滅として見える。副作用として G22 を `reserved` から
外す（他の予約は既定と同一で、差分は 22 の 1 本だけ）。

**トレースは既定ビルドと完全に同一。** `pin-by-role` で引いた番号は
`wasmicon-port` の `write_pin` が `role:led` に正規化するので（abi-spec §9）、
`diff-traces.sh` の比較結果は変わらない。

- [ ] **ただし今の blink は速すぎて見えない。** `BLINKS = 3` / `INTERVAL_MS = 1`
      なので全体が約 6ms で終わる。目視するには `apps/blink-rs` と
      `apps/blink-as` の `INTERVAL_MS` を上げる必要がある（両方揃えること）。
      **`time` はトレースに出ない**ので、変えてもトレースの一致検証には影響しない。
      host のテストが 3×2×interval だけ遅くなるのが唯一のコスト
- 画像は出ない。パネルを初期化していないので点灯・消灯が見えるだけ
- LEDA の極性は未確認なので、反転して見えるかもしれない（点滅自体は見える）

---

## 2. 実機なしで判断できること

- [ ] **toolchain を固定するか**。`rust-toolchain.toml` は `channel = "stable"` の浮動。clippy の新しい lint や rustfmt の出力変化で CI が突然落ちる（初回 CI がまさにそれ: 手元 1.97.1 / CI 1.98.0）。特に「生成物 diff ゼロ」の検査は rustfmt の出力に依存するので、手元で通って CI で落ちる形で効く。固定すると手動でのバージョン上げが要る
- [ ] **`docs/abi-spec.md` §10 の未決 2〜5 を確定にするか**。いずれも既定のまま実装済みで動いている
  - #2 `sleep-ms` 中の挙動 → 各ポートの HAL に委ねる（実装済み）
  - #3 トラップ後の挙動 → ログを出して停止、再起動しない（実装済み）
  - #4 `log` の UTF-8 検証 → しない（実装済み）
  - #5 `spi.transfer` を v0.1 に残すか → 残している（実装済み）
- [ ] **旧 `main` ブランチをどうするか**。リモートの `main` は旧実装（wasm decoder / llvm 試行）のまま。`v2` を `main` にするか、`main` を残すか

---

## 3. 分かっている制限（今は困っていない）

直す必要が出たときのために書いておく。

- **`spi.transfer` と `i2c.write-read` は 128 バイトまで**（`ports/common` の `SCRATCH`）。送信元と受信先がどちらもゲストメモリにあり範囲が重なりうるので、送信側を一度写している。超えると `unsupported`。v0.1 の用途（SHT31 の 6 バイト、ILI9341 の ID 読み）には十分
- **`draw_text` は 12 文字まで**（`apps/README.md` §2）。超えると描かずに失敗を返す
- **テキスト形式の Wasm を読めない**。spec テストの `(module quote ...)` 538 件はこれでスキップしている（スキップ 548 件の内訳はランナーが実行時に出す）
- **複数モジュールのリンクをしない**。`linking.wast` / `imports.wast` 系は対象外
- **`panic` の理由を実機のシリアルに出せない（RP2040 / ESP32-S3）**。シリアルはボードが持っていて panic handler から届かない。ランタイム由来の失敗は `main` が捕まえて出すので、ここに来るのはポート自身のバグに限られる。
  **ESP32-P4 は解決済み**: `chip::early_write` がブートローダ設定済みの UART0 へ生レジスタで書くので、クロック未設定でも panic の全文（例外コード・`mepc`・`mtval`）が出る。他の 2 ポートにも同じ手が使える

---

## 4. 今後やるとしたら（MVP 後）

`docs/design-notes.md` §6 のロードマップにある、v0.1 の範囲外のもの。

- AoT コンパイル（`wasmicon_compiler`）
- HTTP での動的ロード
- Component Model の完全採用（resource type / async / component binary）
- インタプリタの最適化。今は `match` ループのまま。RP2040 で ILI9341 のテキスト描画が
  1 秒以内という目標は未計測（`docs/handoff.md` §5 Phase 2）
