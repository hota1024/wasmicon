//! `core` に無い浮動小数演算。
//!
//! `no_std` では `floor` / `ceil` / `trunc` / `nearest` / `sqrt` が使えないので
//! 自前で書く（docs/handoff.md §6）。依存クレートを増やさないための実装であり、
//! spec テストの `f32.wast` / `f64.wast` / `float_misc.wast` が正しさを検査する。
//!
//! 丸めは全て IEEE 754 の最近接偶数。`sqrt` は仮数を整数平方根で求めてから
//! sticky ビットつきで丸めるので、正しく丸められた結果になる。

/// f32 の正準 NaN。
pub const NAN_F32: u32 = 0x7fc0_0000;
/// f64 の正準 NaN。
pub const NAN_F64: u64 = 0x7ff8_0000_0000_0000;

// ---------------------------------------------------------------- 符号

pub fn abs_f32(x: f32) -> f32 {
    f32::from_bits(x.to_bits() & 0x7fff_ffff)
}

pub fn abs_f64(x: f64) -> f64 {
    f64::from_bits(x.to_bits() & 0x7fff_ffff_ffff_ffff)
}

pub fn neg_f32(x: f32) -> f32 {
    f32::from_bits(x.to_bits() ^ 0x8000_0000)
}

pub fn neg_f64(x: f64) -> f64 {
    f64::from_bits(x.to_bits() ^ 0x8000_0000_0000_0000)
}

pub fn copysign_f32(x: f32, y: f32) -> f32 {
    f32::from_bits((x.to_bits() & 0x7fff_ffff) | (y.to_bits() & 0x8000_0000))
}

pub fn copysign_f64(x: f64, y: f64) -> f64 {
    f64::from_bits((x.to_bits() & 0x7fff_ffff_ffff_ffff) | (y.to_bits() & 0x8000_0000_0000_0000))
}

/// NaN を quiet 化する（仕様の NaN 伝播）。
fn quiet_f32(x: f32) -> f32 {
    f32::from_bits(x.to_bits() | 0x0040_0000)
}

fn quiet_f64(x: f64) -> f64 {
    f64::from_bits(x.to_bits() | 0x0008_0000_0000_0000)
}

// ---------------------------------------------------------------- 整数への丸め

/// 0 方向への切り捨て。指数から小数部のビット位置を求めて落とす。
pub fn trunc_f32(x: f32) -> f32 {
    if x.is_nan() {
        return quiet_f32(x);
    }
    let bits = x.to_bits();
    let exp = ((bits >> 23) & 0xff) as i32 - 127;
    if exp < 0 {
        // |x| < 1 → ±0
        return f32::from_bits(bits & 0x8000_0000);
    }
    if exp >= 23 {
        // 既に整数（Inf / NaN もここ）
        return x;
    }
    let mask = (1u32 << (23 - exp)) - 1;
    f32::from_bits(bits & !mask)
}

pub fn trunc_f64(x: f64) -> f64 {
    if x.is_nan() {
        return quiet_f64(x);
    }
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32 - 1023;
    if exp < 0 {
        return f64::from_bits(bits & 0x8000_0000_0000_0000);
    }
    if exp >= 52 {
        return x;
    }
    let mask = (1u64 << (52 - exp)) - 1;
    f64::from_bits(bits & !mask)
}

pub fn floor_f32(x: f32) -> f32 {
    if x.is_nan() {
        return quiet_f32(x);
    }
    let t = trunc_f32(x);
    if t == x {
        return x;
    }
    if x < 0.0 { t - 1.0 } else { t }
}

pub fn floor_f64(x: f64) -> f64 {
    if x.is_nan() {
        return quiet_f64(x);
    }
    let t = trunc_f64(x);
    if t == x {
        return x;
    }
    if x < 0.0 { t - 1.0 } else { t }
}

pub fn ceil_f32(x: f32) -> f32 {
    if x.is_nan() {
        return quiet_f32(x);
    }
    let t = trunc_f32(x);
    if t == x {
        return x;
    }
    if x > 0.0 { t + 1.0 } else { t }
}

pub fn ceil_f64(x: f64) -> f64 {
    if x.is_nan() {
        return quiet_f64(x);
    }
    let t = trunc_f64(x);
    if t == x {
        return x;
    }
    if x > 0.0 { t + 1.0 } else { t }
}

/// 最近接偶数への丸め（`f32.nearest`）。
pub fn nearest_f32(x: f32) -> f32 {
    if x.is_nan() {
        return quiet_f32(x);
    }
    let t = trunc_f32(x);
    if t == x {
        return x;
    }
    let diff = abs_f32(x - t);
    let away = if x < 0.0 { t - 1.0 } else { t + 1.0 };
    // 端数が半分ちょうどのときは偶数側へ（最近接偶数）。
    let odd = trunc_f32(t * 0.5) * 2.0 != t;
    if diff > 0.5 || (diff == 0.5 && odd) {
        away
    } else {
        t
    }
}

pub fn nearest_f64(x: f64) -> f64 {
    if x.is_nan() {
        return quiet_f64(x);
    }
    let t = trunc_f64(x);
    if t == x {
        return x;
    }
    let diff = abs_f64(x - t);
    let away = if x < 0.0 { t - 1.0 } else { t + 1.0 };
    let odd = trunc_f64(t * 0.5) * 2.0 != t;
    if diff > 0.5 || (diff == 0.5 && odd) {
        away
    } else {
        t
    }
}

// ---------------------------------------------------------------- min / max

pub fn min_f32(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return quiet_f32(a);
    }
    if b.is_nan() {
        return quiet_f32(b);
    }
    if a == b {
        // ±0 の区別。min は符号ビットが立っている方。
        return f32::from_bits(a.to_bits() | b.to_bits());
    }
    if a < b { a } else { b }
}

pub fn max_f32(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return quiet_f32(a);
    }
    if b.is_nan() {
        return quiet_f32(b);
    }
    if a == b {
        return f32::from_bits(a.to_bits() & b.to_bits());
    }
    if a > b { a } else { b }
}

pub fn min_f64(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return quiet_f64(a);
    }
    if b.is_nan() {
        return quiet_f64(b);
    }
    if a == b {
        return f64::from_bits(a.to_bits() | b.to_bits());
    }
    if a < b { a } else { b }
}

pub fn max_f64(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return quiet_f64(a);
    }
    if b.is_nan() {
        return quiet_f64(b);
    }
    if a == b {
        return f64::from_bits(a.to_bits() & b.to_bits());
    }
    if a > b { a } else { b }
}

// ---------------------------------------------------------------- 平方根

/// 整数平方根。`(floor(sqrt(n)), 余りが 0 か)` を返す。
fn isqrt_u128(n: u128) -> (u128, bool) {
    if n == 0 {
        return (0, true);
    }
    // n 以下で最大の 4 のべき乗から始める。
    let msb = 127 - n.leading_zeros();
    let mut bit = 1u128 << (msb & !1);
    let mut num = n;
    let mut res = 0u128;
    while bit != 0 {
        if num >= res + bit {
            num -= res + bit;
            res = (res >> 1) + bit;
        } else {
            res >>= 1;
        }
        bit >>= 2;
    }
    (res, num == 0)
}

/// 正しく丸められた平方根。
///
/// 仮数を 2 進で整数平方根に落とし、余り（sticky）を見て最近接偶数へ丸める。
/// 入力が正規化数でも非正規化数でも、結果は必ず正規化数になる
/// （最小の正の入力 2^-1074 でも sqrt は 2^-537）。
pub fn sqrt_f64(x: f64) -> f64 {
    let bits = x.to_bits();
    let sign = bits >> 63;
    let exp = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & 0x000f_ffff_ffff_ffff;

    if exp == 0x7ff {
        if frac != 0 {
            return quiet_f64(x); // NaN
        }
        if sign == 1 {
            return f64::from_bits(NAN_F64); // sqrt(-inf)
        }
        return x; // +inf
    }
    if bits << 1 == 0 {
        return x; // ±0
    }
    if sign == 1 {
        return f64::from_bits(NAN_F64); // 負の数
    }

    // x = m * 2^e （m は 53 ビット、2^52 <= m < 2^53）
    let (mut m, mut e) = if exp == 0 {
        let s = frac.leading_zeros() - 11;
        (frac << s, -1074 - s as i32)
    } else {
        (frac | 0x0010_0000_0000_0000, exp - 1075)
    };
    // 指数を偶数にする。
    if e & 1 != 0 {
        m <<= 1;
        e -= 1;
    }

    // 2^74 倍してから整数平方根を取ると 64 ビットの商が得られる。
    let (r, exact) = isqrt_u128((m as u128) << 74);
    let r = r as u64;

    // 上位 53 ビットを取り、残りで最近接偶数に丸める。
    let mut q = r >> 11;
    let rem = r & 0x7ff;
    let half = 0x400;
    if rem > half || (rem == half && (!exact || (q & 1) == 1)) {
        q += 1;
    }
    let mut unbiased = e / 2 - 26;
    if q >> 53 != 0 {
        q >>= 1;
        unbiased += 1;
    }

    let biased = (unbiased + 52 + 1023) as u64;
    f64::from_bits((biased << 52) | (q & 0x000f_ffff_ffff_ffff))
}

/// f32 の平方根。
///
/// f64 で計算してから f32 に丸める。f64 の 53 ビットは f32 の 2×24+2 ビットを
/// 超えるので、二重丸めにならず正しく丸められた結果になる。
pub fn sqrt_f32(x: f32) -> f32 {
    let bits = x.to_bits();
    let exp = (bits >> 23) & 0xff;
    if exp == 0xff {
        if bits & 0x007f_ffff != 0 {
            return quiet_f32(x);
        }
        if bits >> 31 == 1 {
            return f32::from_bits(NAN_F32);
        }
        return x;
    }
    if bits << 1 == 0 {
        return x;
    }
    if bits >> 31 == 1 {
        return f32::from_bits(NAN_F32);
    }
    sqrt_f64(f64::from(x)) as f32
}
