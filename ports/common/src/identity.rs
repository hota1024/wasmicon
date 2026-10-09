//! ファームの identity（`docs/app-workflow.md` §3.8）。
//!
//! 起動時にバナーの次の行として 1 行で名乗る:
//!
//! ```text
//! wasmicon id rp2350 fw=0.1.0 git=ba2ce81-dirty abi=wasmicon:hal@0.1.0 trace=on pages=4 if=gpio,i2c,spi,time,log,board slot=v1/65536 roles=v1/4096
//! ```
//!
//! **互換は版 1 本では表せない**（§3.8）ので、軸ごとに並べる。`fw` と `git` は
//! 由来（どのビルドか）を示すためだけのもので、互換の判定には使わない。
//!
//! この行はトレース行（`>` / `<`）ではないので、`trace diff` は比べない。
//! 代わりに `trace diff` は 2 つのログのこの行が食い違えば警告する。

use crate::fmt::Buf;
use crate::profile::Profile;

/// 行の先頭。`trace diff` はこれで identity の行を見つける。
pub const PREFIX: &str = "wasmicon id ";

/// 名乗る内容。ポートの `main.rs` が組む。
pub struct Identity<'a> {
    /// ボードのプロファイル（名前、メモリの上限、実装済みインターフェース、スロット）。
    pub profile: &'static Profile,
    /// ファームの版（ポートの `CARGO_PKG_VERSION`）。
    pub fw: &'a str,
    /// ビルド時の `git describe --always --dirty`（`build.rs` が埋める）。
    pub git: &'a str,
    /// `trace` feature を有効にしてビルドしたか。**切ったファームは `monitor` に
    /// 何も出さない**ので、`trace=off` と言えれば配線から疑わずに済む。
    pub trace: bool,
}

/// 1 行に組む（改行は含まない）。`out` は 192 バイトあれば収まる。
pub fn describe(id: &Identity<'_>, out: &mut Buf<'_>) {
    let p = id.profile;
    out.str(PREFIX);
    out.str(p.name);
    out.str(" fw=");
    out.str(id.fw);
    out.str(" git=");
    out.str(id.git);
    out.str(" abi=");
    out.str(wasmicon_core::generated::PACKAGE);
    out.str(if id.trace { " trace=on" } else { " trace=off" });
    out.str(" pages=");
    out.u32(p.config.max_memory_pages);
    out.str(" if=");
    let i = p.interfaces;
    let mut first = true;
    for (on, name) in [
        (i.gpio, "gpio"),
        (i.i2c, "i2c"),
        (i.spi, "spi"),
        (i.time, "time"),
        (i.log, "log"),
        (i.board, "board"),
    ] {
        if on {
            if !first {
                out.byte(b',');
            }
            out.str(name);
            first = false;
        }
    }
    if let Some(s) = p.slot {
        out.str(" slot=v");
        out.u32(u32::from(crate::slot::FORMAT_VERSION));
        out.byte(b'/');
        out.u32(s.len);
    }
    if let Some(r) = p.role_slot {
        out.str(" roles=v");
        out.u32(u32::from(crate::roles::FORMAT_VERSION));
        out.byte(b'/');
        out.u32(r.len);
    }
}
