//! Cross-checks the hand-written `float.rs` against the std implementations.
//!
//! The core is `no_std`, so these operations are written by hand. A bug here
//! fails `f32.wast` / `f64.wast` / `float_misc.wast` in bulk, so it is caught
//! here first.

use wasmicon_core::float::*;

/// Reproducible pseudo-random numbers (avoids a dependency crate).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
}

fn bits_eq_f64(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

fn bits_eq_f32(a: f32, b: f32) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

#[test]
fn sqrt_f64_matches_std() {
    let mut rng = Lcg(0x1234_5678_9abc_def0);
    let mut checked = 0;
    // Representative values
    for x in [
        0.0f64,
        -0.0,
        1.0,
        2.0,
        4.0,
        0.5,
        1e-300,
        1e300,
        f64::MIN_POSITIVE,
        f64::MIN_POSITIVE / 2.0,
        5e-324,
        f64::MAX,
        3.0,
        1e-10,
        123456.789,
    ] {
        assert!(bits_eq_f64(sqrt_f64(x), x.sqrt()), "sqrt({x:e})");
        checked += 1;
    }
    // Random values (positive only; negative, NaN and Inf are checked separately)
    for _ in 0..200_000 {
        let b = rng.next() & 0x7fff_ffff_ffff_ffff;
        let x = f64::from_bits(b);
        if x.is_nan() || x.is_infinite() {
            continue;
        }
        assert!(
            bits_eq_f64(sqrt_f64(x), x.sqrt()),
            "sqrt({x:e}) bits={b:#x}: {:#x} != {:#x}",
            sqrt_f64(x).to_bits(),
            x.sqrt().to_bits()
        );
        checked += 1;
    }
    assert!(checked > 100_000);
}

#[test]
fn sqrt_f64_special_values() {
    assert!(sqrt_f64(-1.0).is_nan());
    assert!(sqrt_f64(f64::NEG_INFINITY).is_nan());
    assert!(sqrt_f64(f64::INFINITY).is_infinite());
    assert!(sqrt_f64(f64::NAN).is_nan());
    assert_eq!(sqrt_f64(0.0).to_bits(), 0.0f64.to_bits());
    assert_eq!(sqrt_f64(-0.0).to_bits(), (-0.0f64).to_bits());
}

#[test]
fn sqrt_f32_matches_std() {
    let mut rng = Lcg(0xdead_beef_cafe_1234);
    for _ in 0..200_000 {
        let b = (rng.next() as u32) & 0x7fff_ffff;
        let x = f32::from_bits(b);
        if x.is_nan() || x.is_infinite() {
            continue;
        }
        assert!(
            bits_eq_f32(sqrt_f32(x), x.sqrt()),
            "sqrt({x:e}) bits={b:#x}"
        );
    }
}

#[test]
fn rounding_matches_std() {
    let mut rng = Lcg(0x0f0f_0f0f_1234_5678);
    for _ in 0..200_000 {
        let x = f64::from_bits(rng.next());
        if x.is_nan() {
            continue;
        }
        assert!(bits_eq_f64(trunc_f64(x), x.trunc()), "trunc({x:e})");
        assert!(bits_eq_f64(floor_f64(x), x.floor()), "floor({x:e})");
        assert!(bits_eq_f64(ceil_f64(x), x.ceil()), "ceil({x:e})");
        assert!(
            bits_eq_f64(nearest_f64(x), x.round_ties_even()),
            "nearest({x:e})"
        );

        let y = f32::from_bits(rng.next() as u32);
        if y.is_nan() {
            continue;
        }
        assert!(bits_eq_f32(trunc_f32(y), y.trunc()), "trunc({y:e})");
        assert!(bits_eq_f32(floor_f32(y), y.floor()), "floor({y:e})");
        assert!(bits_eq_f32(ceil_f32(y), y.ceil()), "ceil({y:e})");
        assert!(
            bits_eq_f32(nearest_f32(y), y.round_ties_even()),
            "nearest({y:e})"
        );
    }
}

#[test]
fn min_max_follow_the_spec() {
    // Signed zeros (spec: min(-0,+0) = -0, max(-0,+0) = +0)
    assert_eq!(min_f64(-0.0, 0.0).to_bits(), (-0.0f64).to_bits());
    assert_eq!(min_f64(0.0, -0.0).to_bits(), (-0.0f64).to_bits());
    assert_eq!(max_f64(-0.0, 0.0).to_bits(), 0.0f64.to_bits());
    assert_eq!(max_f64(0.0, -0.0).to_bits(), 0.0f64.to_bits());
    assert_eq!(min_f32(-0.0, 0.0).to_bits(), (-0.0f32).to_bits());
    assert_eq!(max_f32(-0.0, 0.0).to_bits(), 0.0f32.to_bits());
    // NaN if either operand is NaN
    assert!(min_f64(f64::NAN, 1.0).is_nan());
    assert!(max_f64(1.0, f64::NAN).is_nan());
    // Ordinary comparisons
    assert_eq!(min_f64(1.0, 2.0), 1.0);
    assert_eq!(max_f64(1.0, 2.0), 2.0);
    assert_eq!(min_f64(f64::NEG_INFINITY, 0.0), f64::NEG_INFINITY);
}

#[test]
fn copysign_and_abs() {
    assert_eq!(copysign_f64(1.0, -2.0).to_bits(), (-1.0f64).to_bits());
    assert_eq!(copysign_f64(-1.0, 2.0).to_bits(), 1.0f64.to_bits());
    assert_eq!(abs_f64(-0.0).to_bits(), 0.0f64.to_bits());
    assert_eq!(neg_f64(0.0).to_bits(), (-0.0f64).to_bits());
}
