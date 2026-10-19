//! Floating-point operations that `core` lacks.
//!
//! `no_std` has no `floor` / `ceil` / `trunc` / `nearest` / `sqrt`, so they are
//! written by hand rather than pulled in from a dependency crate
//! (docs/decisions.md P-03). The spec tests `f32.wast` / `f64.wast` /
//! `float_misc.wast` check them.
//!
//! All rounding is IEEE 754 round-to-nearest-even. `sqrt` computes the
//! significand with an integer square root and rounds with a sticky bit, so
//! the result is correctly rounded.

/// Canonical NaN for f32.
pub const NAN_F32: u32 = 0x7fc0_0000;
/// Canonical NaN for f64.
pub const NAN_F64: u64 = 0x7ff8_0000_0000_0000;

// ---------------------------------------------------------------- sign

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

/// Quiets a NaN (the spec's NaN propagation).
fn quiet_f32(x: f32) -> f32 {
    f32::from_bits(x.to_bits() | 0x0040_0000)
}

fn quiet_f64(x: f64) -> f64 {
    f64::from_bits(x.to_bits() | 0x0008_0000_0000_0000)
}

// ---------------------------------------------------------------- rounding to integer

/// Truncation toward zero. Finds the fraction bits from the exponent and clears them.
pub fn trunc_f32(x: f32) -> f32 {
    if x.is_nan() {
        return quiet_f32(x);
    }
    let bits = x.to_bits();
    let exp = ((bits >> 23) & 0xff) as i32 - 127;
    if exp < 0 {
        // |x| < 1 -> zero with the same sign
        return f32::from_bits(bits & 0x8000_0000);
    }
    if exp >= 23 {
        // Already an integer (Inf and NaN land here too)
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

/// Round to nearest even (`f32.nearest`).
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
    // A tie goes to the even side (round half to even).
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
        // Signed zeros: min is the one with the sign bit set.
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

// ---------------------------------------------------------------- square root

/// Integer square root. Returns `(floor(sqrt(n)), remainder == 0)`.
fn isqrt_u128(n: u128) -> (u128, bool) {
    if n == 0 {
        return (0, true);
    }
    // Start from the largest power of 4 that is <= n.
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

/// Correctly rounded square root.
///
/// Reduces the significand to a binary integer square root and rounds to
/// nearest even using the remainder (sticky). The result is always a normal
/// number, whether the input is normal or subnormal (even the smallest
/// positive input 2^-1074 has sqrt 2^-537).
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
        return x; // signed zero
    }
    if sign == 1 {
        return f64::from_bits(NAN_F64); // negative
    }

    // x = m * 2^e (m has 53 bits, 2^52 <= m < 2^53)
    let (mut m, mut e) = if exp == 0 {
        let s = frac.leading_zeros() - 11;
        (frac << s, -1074 - s as i32)
    } else {
        (frac | 0x0010_0000_0000_0000, exp - 1075)
    };
    // Make the exponent even.
    if e & 1 != 0 {
        m <<= 1;
        e -= 1;
    }

    // Scaling by 2^74 before the integer square root yields a 64-bit quotient.
    let (r, exact) = isqrt_u128((m as u128) << 74);
    let r = r as u64;

    // Take the top 53 bits and round to nearest even with the rest.
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

/// Square root for f32.
///
/// Computed in f64 and rounded to f32. The 53 bits of f64 exceed the 2*24+2
/// bits f32 needs, so there is no double rounding and the result is correctly
/// rounded.
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
