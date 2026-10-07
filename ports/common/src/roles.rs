//! 役割マップ（役割名 → GPIO 番号）と、それを置く設定スロットの形式
//! （`docs/app-workflow.md` §3.9）。
//!
//! **役割名はファームの語彙ではない。** アプリ（`wasmicon.toml` の
//! `[requirements] pin-roles`）と配線（`[board.<name>.roles]`）の間の約束で、
//! ファームは `deploy` が設定スロットに書いた表のとおりに `pin-by-role` に
//! 答えるだけ。**ファームは既定の表を持たない**（2026-10-07 オーナー決定）。
//! スロットが空・壊れていれば役割は 1 つも配らず、どのピンも勝手に駆動しない。
//!
//! ```text
//! offset  size  内容
//! 0       4     magic "WMCR"（Wasmicon Roles）
//! 4       2     フォーマット版（u16 LE。= 1）
//! 6       2     予約（0）
//! 8       4     本文の長さ（u32 LE）
//! 12      4     本文の CRC-32（u32 LE）
//! 16      n     本文。1 行 1 項目の `name=gpio\n`（10 進）
//! ```
//!
//! 本文をテキストにするのは、役割 ID という新しい互換の軸を作らずに済み、
//! フラッシュを吸い出しても人が読めるため（§3.9）。
//!
//! 書く側（CLI）と読む側（ファーム）で同じ検査を 2 回書かないよう、
//! ここに置いて両方が使う。`alloc` を使わないので、名前の長さと個数に
//! 上限がある（`MAX_NAME` / `MAX_ROLES`）。

use crate::crc32;
use crate::fmt::Buf;
use crate::profile::{Profile, Slot};

/// スロットの先頭に置く識別子。アプリスロット（`"WMCA"`）と取り違えない。
pub const MAGIC: [u8; 4] = *b"WMCR";

/// フォーマット版。
pub const FORMAT_VERSION: u16 = 1;

/// ヘッダの大きさ。本文はこの直後から始まる。
pub const HEADER_LEN: usize = 16;

/// 役割の最大数。
pub const MAX_ROLES: usize = 8;

/// 役割名の最大長（バイト）。
pub const MAX_NAME: usize = 16;

/// 本文の最大長。`MAX_ROLES` 行 × （名前 16 + `=` + 番号 3 桁 + 改行）に収まる。
pub const MAX_BODY: usize = 256;

/// 設定スロットに取る大きさ（フラッシュの消去単位）。
pub const SLOT_LEN: u32 = 4096;

/// 役割マップを読めない・作れない理由。
///
/// 文面は `&'static str` で返す（`core::fmt` を使わない）。どの項目で
/// 落ちたかは、書く側（CLI）が 1 項目ずつ `insert` して自分で出す。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RolesError {
    /// 消去済み（`0xff`）か、まだ何も書かれていない。**失敗ではなく「空」。**
    Empty,
    /// magic が違う。
    BadMagic,
    /// この版を読めない。
    UnsupportedVersion,
    /// 本文が長すぎる（`MAX_BODY` 超え）か、スロットの外を指している。
    TooLong,
    /// CRC が合わない。
    BadCrc,
    /// `name=gpio` の形になっていない行がある。
    BadLine,
    /// 役割名が使えない文字を含むか、長さが範囲外。
    BadName,
    /// GPIO 番号が 10 進の数として読めない。
    BadNumber,
    /// 役割が `MAX_ROLES` を超えた。
    TooMany,
    /// 同じ役割名が 2 回出てきた。
    DuplicateName,
    /// 同じ GPIO 番号に 2 つの役割が割り当てられている。
    DuplicatePin,
    /// GPIO 番号がボードの本数を超えている。
    OutOfRange,
    /// 予約ピン（トレースの UART、フラッシュ、無線チップなど）。
    Reserved,
    /// ポートが SPI / I2C に使っているピン。割り当てるとバスが黙って壊れる。
    BusPin,
    /// ファームの末尾が設定スロットに食い込んでいる。読んではいけない。
    Overlap,
}

impl RolesError {
    /// ログに出す文面。
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            RolesError::Empty => "roles empty",
            RolesError::BadMagic => "roles magic mismatch",
            RolesError::UnsupportedVersion => "roles format version not supported",
            RolesError::TooLong => "roles too long",
            RolesError::BadCrc => "roles crc mismatch",
            RolesError::BadLine => "roles line is not name=gpio",
            RolesError::BadName => "role name must be 1..16 of a-z 0-9 - starting with a-z",
            RolesError::BadNumber => "role gpio is not a decimal number",
            RolesError::TooMany => "too many roles",
            RolesError::DuplicateName => "role name appears twice",
            RolesError::DuplicatePin => "two roles share one gpio",
            RolesError::OutOfRange => "role gpio out of range",
            RolesError::Reserved => "role gpio is reserved",
            RolesError::BusPin => "role gpio is used by spi / i2c",
            RolesError::Overlap => "firmware overlaps the roles slot",
        }
    }

    /// 空か。**これだけは異常ではない。**
    #[must_use]
    pub fn is_empty(self) -> bool {
        self == RolesError::Empty
    }
}

/// そのボードで割り当ててよい GPIO の制約。
#[derive(Clone, Copy)]
pub struct Limits<'a> {
    /// GPIO の本数。
    pub gpio_count: u32,
    /// 予約ピン。
    pub reserved: &'a [u32],
    /// ポートが SPI / I2C に使うピン。
    pub bus_pins: &'a [u32],
}

impl Limits<'static> {
    /// プロファイルから作る。
    #[must_use]
    pub const fn of(p: &'static Profile) -> Self {
        Limits {
            gpio_count: p.gpio_count,
            reserved: p.reserved,
            bus_pins: p.bus_pins,
        }
    }
}

/// 役割名として使えるか。英小文字で始まる 1..=16 文字の `a-z` / `0-9` / `-`。
///
/// 文字を絞るのは、本文の `=` と改行、トレースの `role:<name>` と
/// 衝突させないため。
#[must_use]
pub fn valid_name(name: &[u8]) -> bool {
    if name.is_empty() || name.len() > MAX_NAME || !name[0].is_ascii_lowercase() {
        return false;
    }
    name.iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// 役割名 → GPIO 番号の表。固定長で、`alloc` を使わない。
#[derive(Clone, Copy)]
pub struct RoleMap {
    names: [[u8; MAX_NAME]; MAX_ROLES],
    lens: [u8; MAX_ROLES],
    pins: [u32; MAX_ROLES],
    n: usize,
}

impl RoleMap {
    /// 役割が 1 つも無い表。**設定スロットが空・壊れているときはこれで走る。**
    pub const EMPTY: RoleMap = RoleMap {
        names: [[0; MAX_NAME]; MAX_ROLES],
        lens: [0; MAX_ROLES],
        pins: [0; MAX_ROLES],
        n: 0,
    };

    /// 役割の数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.n
    }

    /// 空か。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// `i` 番目の役割名。
    #[must_use]
    pub fn name(&self, i: usize) -> &[u8] {
        &self.names[i][..self.lens[i] as usize]
    }

    /// `i` 番目の GPIO 番号。
    #[must_use]
    pub fn pin(&self, i: usize) -> u32 {
        self.pins[i]
    }

    /// 役割名から引く。見つかれば表の中の位置を返す。
    #[must_use]
    pub fn find(&self, name: &[u8]) -> Option<usize> {
        (0..self.n).find(|&i| self.name(i) == name)
    }

    /// 役割名から GPIO 番号を引く。
    #[must_use]
    pub fn get(&self, name: &[u8]) -> Option<u32> {
        self.find(name).map(|i| self.pins[i])
    }

    /// 1 項目足す。ボードの制約もここで見る。
    ///
    /// # Errors
    /// 名前が使えない、重複、上限超え、またはボードで割り当てられない番号のとき。
    pub fn insert(&mut self, name: &[u8], pin: u32, limits: &Limits<'_>) -> Result<(), RolesError> {
        if !valid_name(name) {
            return Err(RolesError::BadName);
        }
        if self.find(name).is_some() {
            return Err(RolesError::DuplicateName);
        }
        if pin >= limits.gpio_count {
            return Err(RolesError::OutOfRange);
        }
        if limits.reserved.contains(&pin) {
            return Err(RolesError::Reserved);
        }
        if limits.bus_pins.contains(&pin) {
            return Err(RolesError::BusPin);
        }
        if self.pins[..self.n].contains(&pin) {
            return Err(RolesError::DuplicatePin);
        }
        if self.n == MAX_ROLES {
            return Err(RolesError::TooMany);
        }
        self.names[self.n][..name.len()].copy_from_slice(name);
        self.lens[self.n] = name.len() as u8;
        self.pins[self.n] = pin;
        self.n += 1;
        Ok(())
    }
}

/// 本文（`name=gpio` の行）を読む。
///
/// # Errors
/// 行の形が崩れている、または `insert` が弾いたとき。**1 項目でも
/// おかしければ表全体を捨てる**（呼び出し側は `RoleMap::EMPTY` で走る）。
pub fn parse_body(body: &[u8], limits: &Limits<'_>) -> Result<RoleMap, RolesError> {
    let mut map = RoleMap::EMPTY;
    for line in body.split(|b| *b == b'\n') {
        if line.is_empty() {
            continue;
        }
        let Some(eq) = line.iter().position(|b| *b == b'=') else {
            return Err(RolesError::BadLine);
        };
        let (name, digits) = (&line[..eq], &line[eq + 1..]);
        map.insert(name, parse_u32(digits)?, limits)?;
    }
    Ok(map)
}

/// 10 進の非負整数。空、10 進以外の文字、u32 を溢れる値は弾く。
fn parse_u32(digits: &[u8]) -> Result<u32, RolesError> {
    if digits.is_empty() {
        return Err(RolesError::BadNumber);
    }
    let mut v: u32 = 0;
    for &d in digits {
        if !d.is_ascii_digit() {
            return Err(RolesError::BadNumber);
        }
        v = v
            .checked_mul(10)
            .and_then(|v| v.checked_add(u32::from(d - b'0')))
            .ok_or(RolesError::BadNumber)?;
    }
    Ok(v)
}

/// ヘッダから読み取れること。
#[derive(Clone, Copy)]
pub struct Header {
    /// 本文の長さ（`MAX_BODY` 以下であることは確かめてある）。
    pub len: usize,
    /// 本文の CRC-32。
    pub crc: u32,
}

/// ヘッダだけを読む（XIP で読めないボードはこれを先に読み、本文の分だけ読む）。
///
/// # Errors
/// 空、magic 違い、版違い、本文が `MAX_BODY` を超えるとき。
pub fn parse_header(head: &[u8]) -> Result<Header, RolesError> {
    let Some(head) = head.get(..HEADER_LEN) else {
        return Err(RolesError::Empty);
    };
    let magic = &head[..4];
    if magic != MAGIC {
        if magic.iter().all(|b| *b == 0xff) || magic.iter().all(|b| *b == 0) {
            return Err(RolesError::Empty);
        }
        return Err(RolesError::BadMagic);
    }
    if u16::from_le_bytes([head[4], head[5]]) != FORMAT_VERSION {
        return Err(RolesError::UnsupportedVersion);
    }
    let len = u32::from_le_bytes([head[8], head[9], head[10], head[11]]) as usize;
    if len > MAX_BODY {
        return Err(RolesError::TooLong);
    }
    Ok(Header {
        len,
        crc: u32::from_le_bytes([head[12], head[13], head[14], head[15]]),
    })
}

/// 本文の CRC を確かめてから読む。
///
/// # Errors
/// 長さか CRC が合わない、または `parse_body` が失敗したとき。
pub fn parse_checked(body: &[u8], h: &Header, limits: &Limits<'_>) -> Result<RoleMap, RolesError> {
    if body.len() != h.len {
        return Err(RolesError::TooLong);
    }
    if crc32(body) != h.crc {
        return Err(RolesError::BadCrc);
    }
    parse_body(body, limits)
}

/// スロット全体（ヘッダ + 本文。余りは無視する）を読む。
///
/// # Errors
/// `parse_header` / `parse_checked` が失敗したとき。
pub fn parse(bytes: &[u8], limits: &Limits<'_>) -> Result<RoleMap, RolesError> {
    let h = parse_header(bytes)?;
    let Some(body) = bytes.get(HEADER_LEN..HEADER_LEN + h.len) else {
        return Err(RolesError::TooLong);
    };
    parse_checked(body, &h, limits)
}

/// 表を本文に書く。書けた長さを返す（書く側が使う）。
///
/// `out` は `MAX_BODY` 以上あれば必ず足りる。
#[must_use]
pub fn encode_body(map: &RoleMap, out: &mut [u8]) -> usize {
    let mut n = 0;
    let mut put = |b: u8, out: &mut [u8]| {
        if n < out.len() {
            out[n] = b;
            n += 1;
        }
    };
    for i in 0..map.len() {
        for &b in map.name(i) {
            put(b, out);
        }
        put(b'=', out);
        let mut digits = [0u8; 10];
        let mut d = 0;
        let mut v = map.pin(i);
        loop {
            digits[d] = b'0' + (v % 10) as u8;
            d += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        while d > 0 {
            d -= 1;
            put(digits[d], out);
        }
        put(b'\n', out);
    }
    n
}

/// ヘッダを組む（書く側が使う）。
#[must_use]
pub fn header(body: &[u8]) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[..4].copy_from_slice(&MAGIC);
    h[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    let len = u32::try_from(body.len()).unwrap_or(u32::MAX);
    h[8..12].copy_from_slice(&len.to_le_bytes());
    h[12..16].copy_from_slice(&crc32(body).to_le_bytes());
    h
}

/// XIP にマップされたフラッシュから設定スロットを読む（RP2040 / RP2350）。
///
/// # Errors
/// ファームと重なっているとき、および `parse` が失敗したとき。
///
/// # Safety
/// `crate::slot::xip_region` と同じ。
pub unsafe fn read_xip(
    xip_base: usize,
    slot: Slot,
    fw_end: usize,
    limits: &Limits<'_>,
) -> Result<RoleMap, RolesError> {
    // SAFETY: 呼び出し側の契約をそのまま渡す。
    let bytes = unsafe { crate::slot::xip_region(xip_base, slot, fw_end) }
        .map_err(|_| RolesError::Overlap)?;
    parse(bytes, limits)
}

/// 起動時にシリアルへ出す 1 行を組む。
///
/// 読めたら `wasmicon: roles lcd-cs=17 lcd-dc=20`（空なら `roles none`）、
/// 読めなければ `wasmicon: <理由>, no roles`。**バナーなのでトレース行
/// （`>` / `<`）ではない**。`trace diff` はこの行を比べない。
///
/// `out` は 192 バイトあれば `MAX_ROLES` 個でも収まる。
pub fn describe(result: &Result<RoleMap, RolesError>, out: &mut Buf<'_>) {
    out.str("wasmicon: ");
    match result {
        Ok(map) if map.is_empty() => out.str("roles none"),
        Ok(map) => {
            out.str("roles");
            for i in 0..map.len() {
                out.byte(b' ');
                out.bytes(map.name(i));
                out.byte(b'=');
                out.u32(map.pin(i));
            }
        }
        Err(e) => {
            out.str(e.reason());
            out.str(", no roles");
        }
    }
}

/// `(名前, 番号)` の表から作る（host の mock が使う）。
///
/// # Errors
/// `RoleMap::insert` が弾いたとき。
pub fn from_table(table: &[(&str, u32)], limits: &Limits<'_>) -> Result<RoleMap, RolesError> {
    let mut map = RoleMap::EMPTY;
    for &(name, pin) in table {
        map.insert(name.as_bytes(), pin, limits)?;
    }
    Ok(map)
}
