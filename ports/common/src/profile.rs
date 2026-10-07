//! ボードプロファイル。ランタイムの上限、実装済みインターフェース、GPIO の
//! 制約、スロットの置き場所を 1 箇所に集める（`docs/app-workflow.md` §4.3）。
//!
//! **値はポートの `main.rs` / `board.rs` にあったものをそのまま移したもの。**
//! `config` は validate の上限なので、変えると通るアプリが変わる。値は
//! `tests/profiles.rs` が移す前の数値で固定している。
//!
//! 集める理由は 3 つ:
//!
//! - **CLI が読めるようにする。** `wasmicon check --board rp2040` は、ボードの
//!   上限と実装状況を知らないと「host では通るのに実機で落ちる」を先に言えない。
//!   `ports/host` は `Config::DEFAULT`（`max_memory_pages` = 65536）で走るので
//!   **全ボードより緩い**
//! - **配線表（役割マップ）を書く前に検査できるようにする。** 実機ボードは
//!   役割の既定の表を持たない（`roles` モジュール、2026-10-07 オーナー決定）。
//!   CLI は `wasmicon.toml` の `[board.<name>.roles]` を、ここにある
//!   `gpio_count` / `reserved` / `bus_pins` で検査してから設定スロットに書き、
//!   ファームは起動時に同じ検査（`roles::Limits`）をもう一度かける
//!
//! ポートは自分の `NUM_GPIO` / 予約ピンを持っていて、ここの値と一致することを
//! コンパイル時に確かめている（`board.rs`）。

use wasmicon_core::Config;

/// そのポートが実装している HAL インターフェース（abi-spec §7）。
///
/// 未実装のものは `unsupported` を返す。**これを持たないと、`ports/rp2040`
/// 向けの静的検査は通ってしまう**（SPI / I2C が `unsupported` を返すのは
/// 実行時なので）。
#[derive(Clone, Copy)]
pub struct Interfaces {
    pub gpio: bool,
    pub i2c: bool,
    pub spi: bool,
    pub time: bool,
    pub log: bool,
    pub board: bool,
}

impl Interfaces {
    /// 全て実装済み。
    pub const ALL: Interfaces = Interfaces {
        gpio: true,
        i2c: true,
        spi: true,
        time: true,
        log: true,
        board: true,
    };
}

/// 1 ボード分のプロファイル。
pub struct Profile {
    /// `--board` で指す名前。ポートのディレクトリ名に揃える。
    pub name: &'static str,
    /// ランタイムの上限。
    pub config: Config,
    /// 実装済みインターフェース。
    pub interfaces: Interfaces,
    /// mock の役割名 → GPIO 番号。**host だけが持つ。**
    ///
    /// 実機ボードは空。配線表は `deploy` が設定スロット（`role_slot`）に書き、
    /// ファームが起動時に読む（`docs/app-workflow.md` §3.9）。host はフラッシュを
    /// 持たない mock なので、ここに表を置いて `wasmicon run` とテストが使う。
    pub roles: &'static [(&'static str, u32)],
    /// GPIO の本数。配線表の範囲検査に使う。
    pub gpio_count: u32,
    /// ゲストに開放しない GPIO（トレースの UART、フラッシュ、無線チップなど）。
    pub reserved: &'static [u32],
    /// ポートが SPI / I2C に使う GPIO。配線表に入れるとバスが黙って壊れる
    /// （`docs/app-workflow.md` §3.9。実行時の `reserved` には入れていない）。
    pub bus_pins: &'static [u32],
    /// ランタイムに渡す arena の大きさ。**ポートの `static ARENA` がこれを使う。**
    ///
    /// 線形メモリは arena の残り全部を取る（`Arena::alloc_rest`）ので、
    /// `max_memory_pages` を満たすだけでは足りない。**decode / validate /
    /// Exec / instantiate が先に取った残りに `min_pages * 64 KiB` が
    /// 収まらなければ、実機は `instantiate` で落ちる。** この余裕は
    /// `Config` からは分からないので、プロファイルが持つ。
    pub arena: usize,
    /// 検証中だけ使う作業領域の大きさ。ポートの `static SCRATCH` がこれを使う。
    pub scratch: usize,
    /// アプリスロットの置き場所（`docs/app-workflow.md` §3.3 / §3.4）。
    ///
    /// **`None` = まだ決まっていない。** ESP32-S3 は固定オフセットではなく
    /// `partitions.csv` のエントリにする必要があり（espflash の既定の
    /// `factory` がフラッシュ末尾まで伸びる）、フラッシュ容量が
    /// `docs/TODO.md` §5-2 の未決。
    pub slot: Option<Slot>,
    /// 設定スロット（配線表）の置き場所。**アプリスロットの直後**に 4 KiB。
    ///
    /// `deploy` はアプリスロットとこれを 1 本の画像にして 1 回で書く
    /// （`pack::build`）。`None` = フラッシュを持たない（host）。
    pub role_slot: Option<Slot>,
}

/// アプリスロットの位置と大きさ、読み方、焼き方。
///
/// **読み方と焼き方もここに置く。** ボード名の文字列で分岐していると、
/// 新しいポートが増えたときに**コンパイルエラーにならず**既定の枝に
/// 落ちる（esp32p4 を足したら `picotool` で焼こうとする、など）。
/// ボードを宣言する 1 箇所で明示させる。
#[derive(Clone, Copy)]
pub struct Slot {
    /// フラッシュの先頭からのオフセット。
    pub offset: u32,
    /// 取ってある大きさ（消去単位の倍数）。ヘッダを含む。
    pub len: u32,
    /// スロットの読み方。**arena を食うかどうかが変わる。**
    pub read: SlotRead,
    /// スロットに書くのに使う外のツール。
    pub flasher: Flasher,
}

/// スロットの読み方。
///
/// **`wasmicon check` がこれを見る。** `Copy` のボードはアプリの分だけ
/// arena が減るので、同じ `.wasm` でも `instantiate` の余裕が違う。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SlotRead {
    /// フラッシュが memory-mapped（XIP）。**RAM に写さない**ので
    /// arena は減らない。RP2040 / RP2350。
    Xip,
    /// RAM に写してから decode する。**アプリの分だけ arena が減る。**
    /// ESP32-S3（`esp-storage` 経由。XIP に任意オフセットを期待しない）。
    Copy,
}

/// スロットに書くのに使う外のツール（1 段。`docs/app-workflow.md` §3.5）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Flasher {
    /// `picotool load -t bin -o <絶対アドレス>`。**BOOTSEL 押下が要る。**
    Picotool,
    /// `espflash write-bin <オフセット>`。DTR/RTS でリセットするので
    /// **ボタン操作が要らない**が、**書き込みとトレースが同じ口**。
    Espflash,
}

/// マイコン共通の上限。ボード間の差は `max_memory_pages` だけ。
///
/// ホストの既定値（`Config::DEFAULT`）のままだと scratch が足りない。
const MCU: Config = Config {
    max_value_stack: 512,
    max_control_depth: 64,
    max_locals: 256,
    max_memory_pages: 4,
    max_table_elems: 256,
    max_call_depth: 32,
    operand_stack_slots: 1024,
};

/// host の mock の役割割り当て。実機とは別の番号でよい
/// （トレースは `role:` に正規化されるので一致する）。
const HOST_ROLES: &[(&str, u32)] = &[("led", 2), ("lcd-cs", 10), ("lcd-dc", 11), ("lcd-rst", 12)];

/// Pico / Pico 2 系（RP2040 / RP2350A）の予約ピン。
/// GP0/GP1 はトレースの UART0、GP23/24/25/29 は CYW43439 または電源まわり。
const PICO_RESERVED: &[u32] = &[0, 1, 23, 24, 25, 29];

/// Pico / Pico 2 系のバスのピン。I2C0 = GP4/GP5、SPI0 = GP16/GP18/GP19。
const PICO_BUS_PINS: &[u32] = &[4, 5, 16, 18, 19];

/// ESP32-S3 の予約ピン。22..=25 は欠番、26..=32 は SPI フラッシュ / PSRAM、
/// 43/44 はトレースの UART0。
const ESP32S3_RESERVED: &[u32] = &[22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 43, 44];

/// ESP32-S3 のバスのピン。I2C0 = GPIO8/9、SPI2 = GPIO11/12/13。
const ESP32S3_BUS_PINS: &[u32] = &[8, 9, 11, 12, 13];

/// アプリスロットの直後に置く設定スロット。
const fn role_slot_after(app: Slot) -> Slot {
    Slot {
        offset: app.offset + app.len,
        len: crate::roles::SLOT_LEN,
        read: app.read,
        flasher: app.flasher,
    }
}

/// Pico 系のアプリスロット（RP2040 / RP2350 で同じ置き方）。
const PICO_SLOT: Slot = Slot {
    offset: 1 << 20,
    len: 64 * 1024,
    read: SlotRead::Xip,
    flasher: Flasher::Picotool,
};

/// Raspberry Pi Pico WH。
pub const RP2040: Profile = Profile {
    name: "rp2040",
    // SRAM 264 KB。線形メモリは 2 ページ（128 KiB）まで。
    config: Config {
        max_memory_pages: 2,
        ..MCU
    },
    // I2C / SPI は未実装で `unsupported` を返す（docs/TODO.md §1.2）。
    interfaces: Interfaces {
        i2c: false,
        spi: false,
        ..Interfaces::ALL
    },
    roles: &[],
    gpio_count: 30,
    reserved: PICO_RESERVED,
    bus_pins: PICO_BUS_PINS,
    // SRAM 264 KB のうち 160 KB。2 ページ (128 KiB) + ランタイムの構造体。
    arena: 160 * 1024,
    scratch: 8 * 1024,
    // フラッシュ 2 MB（Pico / Pico W(H)）。RP2350 と同じ置き方
    // （XIP で読める）で、オフセットも揃えてある。
    slot: Some(PICO_SLOT),
    role_slot: Some(role_slot_after(PICO_SLOT)),
};

/// Raspberry Pi Pico 2 / Pico 2 W。
pub const RP2350: Profile = Profile {
    name: "rp2350",
    config: MCU,
    interfaces: Interfaces::ALL,
    roles: &[],
    gpio_count: 30,
    reserved: PICO_RESERVED,
    bus_pins: PICO_BUS_PINS,
    // SRAM 520 KB のうち 320 KB。4 ページ (256 KiB) が収まる。
    arena: 320 * 1024,
    scratch: 8 * 1024,
    // フラッシュ 4 MB（Pico 2 W で確定。docs/TODO.md §1.1）。ファームは
    // 先頭から数百 KB なので、1 MiB から先を空けてある。**ファームの末尾と
    // 重ならないことは起動時に検査する**（重なれば自分を壊す）。
    slot: Some(PICO_SLOT),
    role_slot: Some(role_slot_after(PICO_SLOT)),
};

/// ESP32-S3 DevKitC-1。
pub const ESP32S3: Profile = Profile {
    name: "esp32s3",
    config: MCU,
    interfaces: Interfaces::ALL,
    roles: &[],
    gpio_count: 49,
    reserved: ESP32S3_RESERVED,
    bus_pins: ESP32S3_BUS_PINS,
    // DRAM 512 KB のうち 300 KB。増やすとネイティブスタックが削れる
    // （docs/TODO.md §1.4）。
    arena: 300 * 1024,
    scratch: 8 * 1024,
    // フラッシュ 8 MB（2026-10-04 に `espflash board-info` で実測。
    // docs/TODO.md §5-2）。Pico 系と同じ 1 MiB から取る。
    //
    // **`partitions.csv` は要らなかった。** 既定のテーブルは `factory` が
    // フラッシュ末尾まで伸びるが、`espflash flash` はアプリのセクタしか
    // 消さない —— 7 MB 地点に目印を書いてファームを焼き、残っていることを
    // 実測した（docs/verification-report.md §10）。
    slot: Some(ESP32S3_SLOT),
    role_slot: Some(role_slot_after(ESP32S3_SLOT)),
};

/// ESP32-S3 のアプリスロット。
const ESP32S3_SLOT: Slot = Slot {
    offset: 1 << 20,
    len: 64 * 1024,
    read: SlotRead::Copy,
    flasher: Flasher::Espflash,
};

/// PC 上の mock。**全ボードより緩い**ので、これで通っても実機で通るとは限らない。
pub const HOST: Profile = Profile {
    name: "host",
    config: Config::DEFAULT,
    interfaces: Interfaces::ALL,
    roles: HOST_ROLES,
    gpio_count: 48,
    reserved: &[],
    bus_pins: &[],
    // PC なので潤沢に取る。
    arena: 16 << 20,
    scratch: 4 << 20,
    // mock にフラッシュは無い。
    slot: None,
    role_slot: None,
};

/// 名前で引くための表。CLI と `info`（`docs/app-workflow.md` §3.8）が使う。
pub const PROFILES: &[&Profile] = &[&RP2040, &RP2350, &ESP32S3, &HOST];

/// `--board <name>` を解決する。
#[must_use]
pub fn by_name(name: &str) -> Option<&'static Profile> {
    let mut i = 0;
    while i < PROFILES.len() {
        if PROFILES[i].name == name {
            return Some(PROFILES[i]);
        }
        i += 1;
    }
    None
}

/// 役割の GPIO 番号がそのボードで開けることを確かめる。
///
/// **`ROLES` を `board.rs` から外に出したので、番号と `NUM_GPIO` /
/// `RESERVED` の隣接が切れた。** 範囲外や予約ピンを割り当てても
/// `pin-by-role` は成功し、ゲストの `pin.open` が実機で初めて
/// `invalid-argument` を返す。各ポートがこれをコンパイル時に呼んで塞ぐ。
///
/// 予約ピンをスライスで取るのは、`const fn` から関数ポインタを呼べない
/// ため（`ports/esp32s3` は範囲で判定しているので、あちらは同じ検査を
/// `board.rs` 側の const ブロックに書いてある）。
///
/// # Panics
/// 番号が `gpio_count` 以上か、`reserved` に含まれるとき。
pub const fn assert_roles_openable(roles: &[(&str, u32)], gpio_count: u32, reserved: &[u32]) {
    let mut i = 0;
    while i < roles.len() {
        let n = roles[i].1;
        assert!(n < gpio_count, "役割の GPIO 番号がボードの本数を超えている");
        let mut j = 0;
        while j < reserved.len() {
            assert!(
                n != reserved[j],
                "役割の GPIO 番号が予約ピンに当たっている（ゲストは開けない）"
            );
            j += 1;
        }
        i += 1;
    }
}

/// 2 つのピンの並びが同じか（`const` 文脈用）。
///
/// ポートは自分の予約ピンを持っていて、配線表の検査（CLI とファーム）は
/// プロファイルの `reserved` を使う。**食い違うと、CLI が通した表で実機の
/// `pin.open` が落ちる**ので、各ポートの `board.rs` がこれでコンパイル時に
/// 突き合わせる。
#[must_use]
pub const fn same_pins(a: &[u32], b: &[u32]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}
