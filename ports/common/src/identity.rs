//! Firmware identity (docs/app-workflow.md §3.8).
//!
//! At boot the firmware reports itself in one line right after the banner.
//! Compatibility cannot be expressed by a single version
//! (docs/app-workflow.md §3.8), so each axis is listed separately. `fw` and
//! `git` only say which build this is; they play no part in the compatibility
//! check.
//!
//! The line is not a trace line (`>` / `<`), so `trace diff` does not compare
//! it. Instead `trace diff` warns when this line differs between two logs.

use crate::fmt::Buf;
use crate::profile::Profile;

/// Start of the line. `trace diff` uses it to find the identity line.
pub const PREFIX: &str = "wasmicon id ";

/// What the firmware reports. Built by the port's `main.rs`.
pub struct Identity<'a> {
    /// Board profile (name, memory limits, implemented interfaces, slots).
    pub profile: &'static Profile,
    /// Firmware version (the port's `CARGO_PKG_VERSION`).
    pub fw: &'a str,
    /// `git describe --always --dirty` at build time (filled in by `build.rs`).
    pub git: &'a str,
    /// Whether the firmware was built with the `trace` feature. A firmware
    /// without it prints nothing to `monitor`, so saying `trace=off` saves
    /// suspecting the wiring.
    pub trace: bool,
}

/// Builds the line (without a newline). `out` of 256 bytes is enough (the
/// line is under 192 bytes; the rest is headroom for a long `git describe`).
///
/// ```text
/// wasmicon id rp2350 fw=0.1.0 git=ba2ce81-dirty abi=wasmicon:hal@0.1.0 trace=on pages=4 if=gpio,i2c,spi,time,log,board,assets slot=v1/65536 roles=v1/4096 assets=v1/3076096
/// ```
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
        (i.assets, "assets"),
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
    if let Some(a) = p.assets {
        out.str(" assets=v");
        out.u32(u32::from(crate::assets::FORMAT_VERSION));
        out.byte(b'/');
        out.u32(a.len);
    }
}
