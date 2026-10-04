//! アプリスロットの形式（`docs/app-workflow.md` §3.4）。
//!
//! ```text
//! offset  size  内容
//! 0       4     magic "WMCA"（Wasmicon App）
//! 4       2     フォーマット版（u16 LE。= 1）
//! 6       2     予約（0）
//! 8       4     wasm の長さ（u32 LE）
//! 12      4     wasm の CRC-32（u32 LE）
//! 16      n     wasm 本体
//! ```
//!
//! **wasm が 16 バイト境界から始まる**ので、読んだスライスをそのまま
//! `decode` に渡せる（RP2350 は XIP のスライス、ESP32-S3 は RAM への写し）。
//!
//! **CRC は「転送の事故」と「アプリのバグ」を切り分けるために入れる。**
//! 入れないと、壊れたバイト列を decode させて「対応外の命令」のような
//! 無関係な理由が出る（`docs/TODO.md` §1.4 の「無言で止まる」を 1 つ減らす）。
//!
//! 書く側（CLI）と読む側（ファーム）で同じ形を 2 回書かないよう、
//! ここに置いて両方が使う。

use crate::crc32;
use crate::profile::Slot;

/// スロットの先頭に置く識別子。
pub const MAGIC: [u8; 4] = *b"WMCA";

/// フォーマット版。**上げるのは配置を変えるときだけ**（互換の軸の 1 つ。
/// `docs/app-workflow.md` §3.6）。
pub const FORMAT_VERSION: u16 = 1;

/// ヘッダの大きさ。wasm はこの直後から始まる。
pub const HEADER_LEN: usize = 16;

/// スロットを読めない理由。
///
/// `core::fmt` を使わないので（`ports/common` はポートと同じ制約で書く）、
/// 文面は `&'static str` で返し、数値はポートが自分の `Buf` で出す。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SlotError {
    /// 消去済み（`0xff`）か、まだ何も書かれていない（`0x00`）。
    /// **失敗ではなく「空」**。ファームはこれを見て idle に入る。
    Empty,
    /// magic が違う。別のものが書かれている。
    BadMagic,
    /// この版を読めない。
    UnsupportedVersion { found: u16 },
    /// 長さがスロットの外を指している。
    Truncated { need: usize, have: usize },
    /// CRC が合わない。転送か書き込みの事故。
    BadCrc { want: u32, got: u32 },
    /// ファームの末尾がスロットに食い込んでいる。**読んではいけない**
    /// （自分のコードを wasm として食わせてしまう）。
    Overlap { fw_end: usize, slot_start: usize },
}

impl SlotError {
    /// ログに出す文面。
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            SlotError::Empty => "slot empty",
            SlotError::BadMagic => "slot magic mismatch",
            SlotError::UnsupportedVersion { .. } => "slot format version not supported",
            SlotError::Truncated { .. } => "slot truncated",
            SlotError::BadCrc { .. } => "slot crc mismatch",
            SlotError::Overlap { .. } => "firmware overlaps the app slot",
        }
    }

    /// 空（まだ何も配っていない）か。**これだけは異常ではない。**
    #[must_use]
    pub fn is_empty(self) -> bool {
        self == SlotError::Empty
    }
}

/// ヘッダから読み取れること。
#[derive(Clone, Copy)]
pub struct Header {
    /// wasm の長さ。
    pub len: usize,
    /// ヘッダが持っている CRC-32。
    pub crc: u32,
}

/// ヘッダだけを読む。
///
/// **XIP で読めないボード（ESP32-S3）はこれを先に読む。** 長さが分かって
/// から、その分だけ arena を取って本体を読む（64 KiB のスロットに対して
/// アプリは数 KB なので、スロットの大きさぶんの RAM は要らない。DRAM は
/// ほぼ使い切っている。`docs/TODO.md` §1.4）。
///
/// # Errors
/// 空、magic 違い、版違いのとき。**長さと CRC はまだ検査しない**
/// （本体を読んでから `verify`）。
pub fn parse_header(head: &[u8]) -> Result<Header, SlotError> {
    let Some(head) = head.get(..HEADER_LEN) else {
        return Err(SlotError::Empty);
    };
    let magic: &[u8] = &head[..4];
    if magic != MAGIC {
        // 消去済みのフラッシュは 0xff、まだ書いていない RAM は 0x00。
        // どちらも「空」で、magic 違いとは区別する（ログの意味が変わる）。
        if magic.iter().all(|b| *b == 0xff) || magic.iter().all(|b| *b == 0) {
            return Err(SlotError::Empty);
        }
        return Err(SlotError::BadMagic);
    }
    let version = u16::from_le_bytes([head[4], head[5]]);
    if version != FORMAT_VERSION {
        return Err(SlotError::UnsupportedVersion { found: version });
    }
    Ok(Header {
        len: u32::from_le_bytes([head[8], head[9], head[10], head[11]]) as usize,
        crc: u32::from_le_bytes([head[12], head[13], head[14], head[15]]),
    })
}

/// 読んだ本体がヘッダと合っているか。
///
/// # Errors
/// 長さが足りない、CRC が合わないとき。
pub fn verify(wasm: &[u8], h: &Header) -> Result<(), SlotError> {
    if wasm.len() != h.len {
        return Err(SlotError::Truncated {
            need: h.len,
            have: wasm.len(),
        });
    }
    let got = crc32(wasm);
    if got != h.crc {
        return Err(SlotError::BadCrc { want: h.crc, got });
    }
    Ok(())
}

/// スロットの中の wasm を取り出す。
///
/// `bytes` はスロット領域全体でよい（長さはヘッダが持つので、余りは無視する）。
///
/// # Errors
/// 空、magic 違い、版違い、長さ不足、CRC 不一致のとき。
pub fn parse(bytes: &[u8]) -> Result<&[u8], SlotError> {
    let h = parse_header(bytes)?;
    let need = HEADER_LEN + h.len;
    let Some(wasm) = bytes.get(HEADER_LEN..need) else {
        return Err(SlotError::Truncated {
            need,
            have: bytes.len(),
        });
    };
    verify(wasm, &h)?;
    Ok(wasm)
}

/// XIP にマップされたフラッシュからスロットを読む。
///
/// RP2040 / RP2350 はフラッシュが memory-mapped なので、**RAM に写さず
/// スライスのまま返す**（design-notes §4）。両ポートで同じことをするので
/// ここに置く（生スライスを作る `unsafe` を 1 箇所にする）。
///
/// `fw_end` はリンカが置く「ファームの末尾」のアドレス。**スロットに
/// 食い込んでいたら読まずに `Overlap` を返す。**
///
/// # Errors
/// 重なっているとき、および `parse` が失敗したとき。
///
/// # Safety
/// `xip_base + slot.offset` から `slot.len` バイトが、読み出し可能な
/// memory-mapped flash であること（範囲外を読むとバスフォルトになる）。
pub unsafe fn read_xip(
    xip_base: usize,
    slot: Slot,
    fw_end: usize,
) -> Result<&'static [u8], SlotError> {
    let slot_start = xip_base + slot.offset as usize;
    if fw_end > slot_start {
        // **ここで返るので、重なっているときはスライスを作らない。**
        return Err(SlotError::Overlap { fw_end, slot_start });
    }
    // SAFETY: 呼び出し側の契約（読み出し可能な memory-mapped flash）。
    // 上でファームの末尾より後ろだと確かめてある。
    let bytes = unsafe { core::slice::from_raw_parts(slot_start as *const u8, slot.len as usize) };
    parse(bytes)
}

/// ヘッダを組む（書く側が使う）。
#[must_use]
pub fn header(wasm: &[u8]) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[..4].copy_from_slice(&MAGIC);
    h[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    // 6..8 は予約（0）。
    let len = u32::try_from(wasm.len()).unwrap_or(u32::MAX);
    h[8..12].copy_from_slice(&len.to_le_bytes());
    h[12..16].copy_from_slice(&crc32(wasm).to_le_bytes());
    h
}

/// スロットに置くときの全体の大きさ。
#[must_use]
pub fn image_len(wasm_len: usize) -> usize {
    HEADER_LEN + wasm_len
}
