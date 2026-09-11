//! ボード定義。**チップ層 (`crate::chip`) が答えられないものだけ**を持つ。
//!
//! 同じ ESP32-P4 でもボードごとに GPIO の意味が入れ替わるので、ここは
//! 共有できない。実例（いずれも esp-bsp のヘッダで確認した）:
//!
//! | | Function-EV-Board | P4-EYE | Tab5 |
//! |---|---|---|---|
//! | I2C | SDA=G7, SCL=G8 | SDA=G14, SCL=G13 | SDA=G31, SCL=G32 |
//! | 音声 I2S | G9..=G13 | G21, G22 | G26..=G30 |
//! | LCD バックライト | G26 | G20 | G22 |
//! | オンボード LED | 無し | **G23** | 無し |
//!
//! `G23` は P4-EYE では LED、Tab5 では TP_INT、EV-Board ではバックライトになる。
//! 3 ボードで一致したのは microSD の G39..=G44 だけで、これは IO_MUX の SD1
//! 直結ピン（実質チップ由来）。
//!
//! ボード固有の差はすべて定数で表せる（`BoardDef`）。唯一コードが要るのは
//! UART のピンで、esp-hal が型付きシングルトン (`p.GPIO37`) を使っており
//! 番号から実行時に引けないため、ボードごとに `open_serial` を持つ。

use esp_hal::peripherals::Peripherals;
use esp_hal::uart::Uart;
use esp_hal::Blocking;

#[cfg(feature = "tab5")]
mod tab5;

#[cfg(feature = "tab5")]
pub use tab5::DEF;

// ボードはちょうど 1 つ選ぶ。Cargo の feature は加算的なので、
// 「選ばれていない」と「複数選ばれた」の両方をここで弾く。
//
// **2 つ目のボードを足すときは、下に
// `#[cfg(all(feature = "tab5", feature = "<新ボード>"))] compile_error!(...)`
// を必ず足すこと。** 忘れると両方 cfg が通って `DEF` が二重定義になり、
// 分かりにくいエラーになる。
#[cfg(not(feature = "tab5"))]
compile_error!("ボードの feature をちょうど 1 つ選ぶこと（既定は tab5。例: --features tab5）");

/// ボード 1 台ぶんの定義。
pub struct BoardDef {
    /// 起動バナーに出す名前。ログの出所を見分けるためだけのもので、
    /// トレース（abi-spec §9 の `> ` / `< ` 行）には影響しない。
    pub name: &'static str,

    /// 役割名 → GPIO 番号（abi-spec §8）。
    pub roles: &'static [(&'static str, u32)],

    /// ゲストに開放しない GPIO。**閉区間**の並び。
    ///
    /// 塞ぐ理由の中心は「反対側で別のチップが同じネットを駆動している」こと。
    /// ゲストが Low を出して相手が High を出すと両方の出力段を貫通する。
    /// トレース用 UART も必ず含めること（開けられるとトレースが死に、
    /// Phase 6 の測定そのものが成立しなくなる）。
    pub reserved: &'static [(u32, u32)],
}

impl BoardDef {
    pub fn pin_by_role(&self, role: &str) -> Option<u32> {
        self.roles.iter().find(|(r, _)| *r == role).map(|(_, i)| *i)
    }

    pub fn is_reserved(&self, index: u32) -> bool {
        self.reserved
            .iter()
            .any(|&(lo, hi)| lo <= index && index <= hi)
    }
}

/// トレースとログを出す UART を開く。ボードごとにピンが違う。
pub fn open_serial(p: Peripherals) -> Uart<'static, Blocking> {
    #[cfg(feature = "tab5")]
    return tab5::open_serial(p);

    // ボードが選ばれていないときの本当のエラーは上の compile_error! なので、
    // 「戻り値の型が合わない」という無関係なエラーでそれが埋もれないようにする。
    #[cfg(not(feature = "tab5"))]
    {
        let _ = p;
        unreachable!()
    }
}
