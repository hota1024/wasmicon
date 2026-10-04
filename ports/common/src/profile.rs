//! ボードプロファイル。ランタイムの上限、実装済みインターフェース、役割名の
//! 割り当てを 1 箇所に集める（`docs/app-workflow.md` §4.3）。
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
//! - **役割名を 1 箇所にする。** `ROLE_NAMES` から外れた名前を割り当てると、
//!   `pin-by-role` は成功するのにトレースが `role:` に正規化されず、生の GPIO
//!   番号が出る。番号はボードごとに違うので **2 ボードのトレースが食い違う**
//!   （abi-spec §9）。この表が `ports/common` にあるので、
//!   `assert_role_names` がコンパイル時に検査できる
//! - 役割名は `.wasm` から静的に列挙できないので、CLI は
//!   `wasmicon.toml` の `[requirements] pin-roles` と**この表**を突き合わせる
//!   （`docs/app-workflow.md` §4.7）
//!
//! 番号そのものの検査（範囲外・予約ピン）は**デバイス側の責務**で、ここには
//! 置かない。ポートが `gpio_count` と `gpio_reserved` で持っている。

use crate::ROLE_NAMES;
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
    /// 役割名 → GPIO 番号の既定（abi-spec §8）。
    ///
    /// **既定**であって固定ではない。デバイス側で上書きできるようにするのは
    /// 別の話（`docs/app-workflow.md` §3.9、`docs/TODO.md` §5-9）。
    pub roles: &'static [(&'static str, u32)],
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
}

/// アプリスロットの位置と大きさ。
#[derive(Clone, Copy)]
pub struct Slot {
    /// フラッシュの先頭からのオフセット。
    pub offset: u32,
    /// 取ってある大きさ（消去単位の倍数）。ヘッダを含む。
    pub len: u32,
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

/// Pico 系の役割割り当て（abi-spec §8）。
/// RP2040 と RP2350 はヘッダのピン配置が同じなので GP 番号も同じ。
const PICO_ROLES: &[(&str, u32)] = &[("led", 15), ("lcd-cs", 17), ("lcd-dc", 20), ("lcd-rst", 21)];

/// ESP32-S3 DevKitC-1 の役割割り当て（abi-spec §8）。
const ESP32S3_ROLES: &[(&str, u32)] =
    &[("led", 2), ("lcd-cs", 10), ("lcd-dc", 14), ("lcd-rst", 15)];

/// host の mock の役割割り当て（abi-spec §8）。実機とは別の番号でよい
/// （トレースは `role:` に正規化されるので一致する）。
const HOST_ROLES: &[(&str, u32)] = &[("led", 2), ("lcd-cs", 10), ("lcd-dc", 11), ("lcd-rst", 12)];

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
    roles: PICO_ROLES,
    // SRAM 264 KB のうち 160 KB。2 ページ (128 KiB) + ランタイムの構造体。
    arena: 160 * 1024,
    scratch: 8 * 1024,
    // RP2350 と同じ置き方にできる（XIP で読める）が、ポートが
    // スロットを読む実装をまだ持っていない。
    slot: None,
};

/// Raspberry Pi Pico 2 / Pico 2 W。
pub const RP2350: Profile = Profile {
    name: "rp2350",
    config: MCU,
    interfaces: Interfaces::ALL,
    roles: PICO_ROLES,
    // SRAM 520 KB のうち 320 KB。4 ページ (256 KiB) が収まる。
    arena: 320 * 1024,
    scratch: 8 * 1024,
    // フラッシュ 4 MB（Pico 2 W で確定。docs/TODO.md §1.1）。ファームは
    // 先頭から数百 KB なので、1 MiB から先を空けてある。**ファームの末尾と
    // 重ならないことは起動時に検査する**（重なれば自分を壊す）。
    slot: Some(Slot {
        offset: 1 << 20,
        len: 64 * 1024,
    }),
};

/// ESP32-S3 DevKitC-1。
pub const ESP32S3: Profile = Profile {
    name: "esp32s3",
    config: MCU,
    interfaces: Interfaces::ALL,
    roles: ESP32S3_ROLES,
    // DRAM 512 KB のうち 300 KB。増やすとネイティブスタックが削れる
    // （docs/TODO.md §1.4）。
    arena: 300 * 1024,
    scratch: 8 * 1024,
    // **固定オフセットにできない。** espflash の既定のパーティション
    // テーブルは `factory` がフラッシュ末尾まで伸びるので、
    // `partitions.csv` に専用エントリを足す必要がある（§3.3）。
    // フラッシュ容量も未決（§5-2）。
    slot: None,
};

/// PC 上の mock。**全ボードより緩い**ので、これで通っても実機で通るとは限らない。
pub const HOST: Profile = Profile {
    name: "host",
    config: Config::DEFAULT,
    interfaces: Interfaces::ALL,
    roles: HOST_ROLES,
    // PC なので潤沢に取る。
    arena: 16 << 20,
    scratch: 4 << 20,
    // mock にフラッシュは無い。
    slot: None,
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

/// 役割名が全て `ROLE_NAMES` にあることを確かめる。
///
/// **外れるとトレースが壊れる。** `Hal` は `pin-by-role` が返した名前が
/// `ROLE_NAMES` にあるときだけ番号を覚え、トレースを `role:led` の形に
/// 正規化する（abi-spec §9）。無い名前だと正規化されず生の GPIO 番号が出て、
/// 番号はボードごとに違うので **2 ボードのトレースが一致しなくなる**。
///
/// `const fn` なので、各プロファイルの宣言の隣で `const _: () = ...` として
/// **コンパイル時に**検査している。
///
/// # Panics
/// `ROLE_NAMES` に無い役割名があるとき。
pub const fn assert_role_names(roles: &[(&str, u32)]) {
    let mut i = 0;
    while i < roles.len() {
        let mut found = false;
        let mut j = 0;
        while j < ROLE_NAMES.len() {
            if str_eq(roles[i].0, ROLE_NAMES[j]) {
                found = true;
            }
            j += 1;
        }
        assert!(
            found,
            "役割名が ROLE_NAMES に無い。トレースが role: に正規化されず \
             ボード間で食い違う（abi-spec §9）"
        );
        i += 1;
    }
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

/// `const` 文脈で使える文字列比較（`==` は const ではない）。
const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
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

// コンパイル時の検査。役割名を足すときは ROLE_NAMES（crate root）にも足す。
const _: () = assert_role_names(PICO_ROLES);
const _: () = assert_role_names(ESP32S3_ROLES);
const _: () = assert_role_names(HOST_ROLES);
