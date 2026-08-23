//! Cross-platform determinism tests.
//!
//! Every assertion here is on exact raw bits. Approximate comparison would
//! defeat the purpose: the point is not that the arithmetic is roughly right,
//! it is that Linux and macOS produce the identical bit pattern. CI runs this
//! file on both, which is what makes it meaningful.

use rmopl::constants::*;
use rmopl::math::{FVec2, Fix, atan2, cos, sin, sqrt};

/// A long chain of mixed arithmetic, pinned to its exact result.
///
/// This is the canary for a dependency quietly introducing float math or
/// changing rounding: any such change moves these bits.
#[test]
fn arithmetic_chain_is_bit_stable() {
    let mut acc = Fix::lit("1");
    let mut v = FVec2::new(Fix::lit("0.5"), Fix::lit("-1.25"));

    for i in 0..10_000u32 {
        let k = Fix::from_num(i % 97) / Fix::lit("13");
        acc = acc * Fix::lit("1.0001") + k - Fix::lit("0.37");
        if acc > Fix::lit("1000") || acc < Fix::lit("-1000") {
            acc /= Fix::lit("7");
        }
        v += FVec2::new(k, -acc) * FIXED_DELTA_TIME;
        v = v.normalized_safe() * (Fix::lit("1") + k / Fix::lit("64"));
    }

    assert_eq!(acc.to_bits(), 1_106_084_256_143);
    assert_eq!(v.x.to_bits(), 9_761_391);
    assert_eq!(v.y.to_bits(), -4_336_254_071);
}

#[test]
fn trig_is_bit_stable() {
    let mut bits = 0i64;
    for i in -500..=500i64 {
        let a = Fix::from_num(i) / Fix::lit("100");
        bits = bits
            .wrapping_mul(31)
            .wrapping_add(sin(a).to_bits())
            .wrapping_mul(31)
            .wrapping_add(cos(a).to_bits())
            .wrapping_mul(31)
            .wrapping_add(atan2(a, Fix::lit("1.5")).to_bits());
    }
    assert_eq!(bits, -3_525_396_855_101_575_035);
}

#[test]
fn sqrt_is_bit_stable() {
    let mut bits = 0i64;
    for i in 0..10_000i64 {
        bits = bits
            .wrapping_mul(31)
            .wrapping_add(sqrt(Fix::from_num(i) / Fix::lit("7")).to_bits());
    }
    assert_eq!(bits, 3_788_800_590_183_152_348);
}
