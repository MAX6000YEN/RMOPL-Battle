//! Deterministic fixed-point scalar and vector math.
//!
//! Everything the simulation computes with lives here, and all of it is Q32.32.
//! Floats are deliberately absent: the netcode replays the same inputs on every
//! machine and compares checksums, so two platforms disagreeing in the last bit
//! of a multiply is a desync, not a rounding detail. Integer arithmetic has no
//! such freedom, which is the whole reason for the fixed-point choice.

use core::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

/// The one scalar type in the simulation: 32 integer bits, 32 fractional bits.
///
/// Construct compile-time values with [`Fix::lit`], which parses a decimal
/// string during const evaluation and so keeps float literals out of the
/// source entirely.
pub type Fix = fixed::types::I32F32;

/// Square root, exact for every representable non-negative input.
///
/// Widening the raw value to 128 bits before taking an integer square root
/// makes the result the exact truncated root. A CORDIC iteration would be a
/// few units off; there is no reason to accept that when the exact answer
/// costs one shift.
///
/// Negative input returns zero rather than panicking. Magnitudes are the main
/// caller and a negative squared length can only arise from overflow, where
/// killing the whole simulation is the worse outcome.
pub fn sqrt(v: Fix) -> Fix {
    if v <= Fix::ZERO {
        return Fix::ZERO;
    }
    Fix::from_bits(((v.to_bits() as u128) << 32).isqrt() as i64)
}

/// Sine of an angle in radians.
///
/// Integer CORDIC, so it is bit-identical on every platform, but it is not
/// exact: results are within [`TRIG_ERROR_BOUND`] raw units of the true value,
/// and may sit up to two raw units *outside* the range -1..=1. Anything that
/// feeds a result of this into a domain-restricted operation must clamp first.
pub fn sin(radians: Fix) -> Fix {
    cordic::sin(radians)
}

/// Cosine of an angle in radians. Same accuracy caveats as [`sin`].
pub fn cos(radians: Fix) -> Fix {
    cordic::cos(radians)
}

/// Angle in radians from the positive x-axis to `(x, y)`, in -PI..=PI.
///
/// A vector pointing very nearly straight up or straight down is answered
/// directly rather than by CORDIC. The underlying routine forms `y / x`, and
/// once `x` is small enough that quotient does not fit in Q32.32 and the
/// division panics — taking the whole simulation with it. That is not a corner
/// case: a stick held straight up, a wall's surface normal and a vertical
/// knockback all land there, and all three are ordinary gameplay.
///
/// The cutoff is chosen so the shortcut costs nothing measurable. At a ratio of
/// 2^30 the true angle is within about four raw units of a right angle, which
/// is well inside [`TRIG_ERROR_BOUND`] — no caller can distinguish the two
/// answers, and the panic is gone.
pub fn atan2(y: Fix, x: Fix) -> Fix {
    /// How much taller than wide a vector must be to count as vertical.
    const VERTICAL_RATIO: i128 = 1 << 30;

    let tall = i128::from(y.to_bits()).abs();
    let wide = i128::from(x.to_bits()).abs();

    if wide == 0 || tall > wide * VERTICAL_RATIO {
        // At this ratio the sign of x no longer affects the answer to within
        // the error bound, so only the direction of y matters.
        return if y < Fix::ZERO {
            -Fix::FRAC_PI_2
        } else {
            Fix::FRAC_PI_2
        };
    }

    cordic::atan2(y, x)
}

/// Worst-case error of [`sin`], [`cos`] and [`atan2`], in raw Q32.32 units.
///
/// Measured across a dense sweep of the input range; the observed maximum was
/// 22 units, and this is rounded up to leave room. One raw unit is 2^-32, so
/// this is an absolute error below 1e-8.
pub const TRIG_ERROR_BOUND: i64 = 32;

/// A 2D vector in simulation space.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FVec2 {
    pub x: Fix,
    pub y: Fix,
}

impl FVec2 {
    pub const ZERO: Self = Self::new(Fix::ZERO, Fix::ZERO);
    pub const ONE: Self = Self::new(Fix::ONE, Fix::ONE);
    pub const UP: Self = Self::new(Fix::ZERO, Fix::ONE);
    pub const DOWN: Self = Self::new(Fix::ZERO, Fix::NEG_ONE);
    pub const LEFT: Self = Self::new(Fix::NEG_ONE, Fix::ZERO);
    pub const RIGHT: Self = Self::new(Fix::ONE, Fix::ZERO);

    pub const fn new(x: Fix, y: Fix) -> Self {
        Self { x, y }
    }

    pub fn dot(self, other: Self) -> Fix {
        self.x * other.x + self.y * other.y
    }

    /// Squared length. Prefer this to [`FVec2::magnitude`] when comparing
    /// distances: it skips the square root and stays exact.
    pub fn sqr_magnitude(self) -> Fix {
        self.dot(self)
    }

    pub fn magnitude(self) -> Fix {
        sqrt(self.sqr_magnitude())
    }

    pub fn distance(self, other: Self) -> Fix {
        (self - other).magnitude()
    }

    /// Unit vector in the same direction, or [`FVec2::ZERO`] for a zero vector.
    ///
    /// Never divides by zero and never panics: a zero-length direction is an
    /// ordinary situation in gameplay (a stationary player, a contact normal
    /// that cancelled out) and must not be able to halt the simulation.
    pub fn normalized_safe(self) -> Self {
        let len = self.magnitude();
        if len == Fix::ZERO {
            Self::ZERO
        } else {
            self / len
        }
    }

    /// Rotated 90 degrees counter-clockwise.
    pub fn perp(self) -> Self {
        Self::new(-self.y, self.x)
    }

    /// Rotated 90 degrees clockwise.
    pub fn perp_cw(self) -> Self {
        Self::new(self.y, -self.x)
    }
}

impl Add for FVec2 {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for FVec2 {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y)
    }
}

impl Neg for FVec2 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y)
    }
}

impl Mul<Fix> for FVec2 {
    type Output = Self;
    fn mul(self, s: Fix) -> Self {
        Self::new(self.x * s, self.y * s)
    }
}

impl Div<Fix> for FVec2 {
    type Output = Self;
    fn div(self, s: Fix) -> Self {
        Self::new(self.x / s, self.y / s)
    }
}

impl AddAssign for FVec2 {
    fn add_assign(&mut self, o: Self) {
        *self = *self + o;
    }
}

impl SubAssign for FVec2 {
    fn sub_assign(&mut self, o: Self) {
        *self = *self - o;
    }
}

impl MulAssign<Fix> for FVec2 {
    fn mul_assign(&mut self, s: Fix) {
        *self = *self * s;
    }
}

impl DivAssign<Fix> for FVec2 {
    fn div_assign(&mut self, s: Fix) {
        *self = *self / s;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perpendiculars_turn_the_right_way() {
        // A flipped perpendicular silently inverts "forward" for anything
        // walking on a surface, and is very hard to spot from behaviour.
        assert_eq!(FVec2::RIGHT.perp(), FVec2::UP);
        assert_eq!(FVec2::UP.perp(), FVec2::LEFT);
        assert_eq!(FVec2::LEFT.perp(), FVec2::DOWN);
        assert_eq!(FVec2::DOWN.perp(), FVec2::RIGHT);

        assert_eq!(FVec2::RIGHT.perp_cw(), FVec2::DOWN);
        assert_eq!(FVec2::DOWN.perp_cw(), FVec2::LEFT);
        assert_eq!(FVec2::LEFT.perp_cw(), FVec2::UP);
        assert_eq!(FVec2::UP.perp_cw(), FVec2::RIGHT);
    }

    #[test]
    fn perpendiculars_are_inverses_and_orthogonal() {
        let v = FVec2::new(Fix::lit("3.5"), Fix::lit("-8.25"));
        assert_eq!(v.perp().perp_cw(), v);
        assert_eq!(v.dot(v.perp()), Fix::ZERO);
        assert_eq!(v.perp().perp(), -v);
    }

    #[test]
    fn three_four_five_triangle_is_exact() {
        let v = FVec2::new(Fix::lit("3"), Fix::lit("4"));
        assert_eq!(v.sqr_magnitude(), Fix::lit("25"));
        assert_eq!(v.magnitude(), Fix::lit("5"));
        assert_eq!(
            v.dot(FVec2::new(Fix::lit("3"), Fix::lit("4"))),
            Fix::lit("25")
        );
        assert_eq!(
            FVec2::new(Fix::lit("1"), Fix::lit("2"))
                .distance(FVec2::new(Fix::lit("4"), Fix::lit("6"))),
            Fix::lit("5")
        );
    }

    #[test]
    fn dot_of_orthogonal_axes_is_zero() {
        assert_eq!(FVec2::RIGHT.dot(FVec2::UP), Fix::ZERO);
        assert_eq!(FVec2::RIGHT.dot(FVec2::RIGHT), Fix::ONE);
        assert_eq!(FVec2::RIGHT.dot(FVec2::LEFT), -Fix::ONE);
    }

    #[test]
    fn normalizing_zero_yields_zero_and_does_not_panic() {
        assert_eq!(FVec2::ZERO.normalized_safe(), FVec2::ZERO);
        assert_eq!(
            FVec2::new(Fix::ZERO, Fix::ZERO).normalized_safe(),
            FVec2::ZERO
        );
    }

    #[test]
    fn normalizing_an_axis_is_exact() {
        // Axis-aligned vectors divide evenly and come back exact.
        assert_eq!(
            (FVec2::RIGHT * Fix::lit("17")).normalized_safe(),
            FVec2::RIGHT
        );
        assert_eq!(
            (FVec2::DOWN * Fix::lit("1024")).normalized_safe(),
            FVec2::DOWN
        );
    }

    #[test]
    fn normalizing_off_axis_truncates_by_at_most_one_unit() {
        // Three fifths is not a binary fraction, so the components land one raw
        // unit below the nearest representable value. Division truncates toward
        // zero; that is exact and identical everywhere, just not the rounded
        // value, and callers must not assume components round-trip.
        let n = FVec2::new(Fix::lit("3"), Fix::lit("4")).normalized_safe();
        assert_eq!(Fix::lit("0.6").to_bits() - n.x.to_bits(), 1);
        assert_eq!(Fix::lit("0.8").to_bits() - n.y.to_bits(), 1);
    }

    #[test]
    fn sqrt_is_exact_on_perfect_squares() {
        // 46_340 is the largest n whose square still fits in the integer part.
        for n in [0i64, 1, 2, 3, 4, 5, 10, 100, 1_000, 46_340] {
            assert_eq!(sqrt(Fix::from_num(n * n)), Fix::from_num(n));
        }
    }

    #[test]
    fn sqrt_of_negative_is_zero_not_a_panic() {
        assert_eq!(sqrt(Fix::lit("-1")), Fix::ZERO);
        assert_eq!(sqrt(Fix::MIN), Fix::ZERO);
    }

    #[test]
    fn sqrt_returns_the_exact_truncated_root() {
        // The result is the largest representable value whose square does not
        // exceed the input. Checked in 128-bit integers rather than by squaring
        // the result again, because a Q32.32 multiply truncates too and would
        // hide a one-unit error instead of exposing it.
        for raw in [1i64, 2, 3, 7, 12_345, 1 << 32, (1 << 32) * 2, i64::MAX / 4] {
            let v = Fix::from_bits(raw);
            let r = sqrt(v).to_bits() as u128;
            let radicand = (raw as u128) << 32;
            assert!(r * r <= radicand, "root too large for raw {raw}");
            assert!((r + 1) * (r + 1) > radicand, "root too small for raw {raw}");
        }
    }

    #[test]
    fn trig_identities_hold_within_the_documented_bound() {
        // CORDIC is deterministic but not exact. These assert the documented
        // accuracy; the exact bit values are pinned in tests/determinism.rs.
        let near = |a: Fix, b: Fix| (a.to_bits() - b.to_bits()).abs() <= TRIG_ERROR_BOUND;

        assert!(near(sin(Fix::ZERO), Fix::ZERO));
        assert!(near(cos(Fix::ZERO), Fix::ONE));

        let half_pi = Fix::lit("1.5707963267948966");
        assert!(near(sin(half_pi), Fix::ONE));
        assert!(near(cos(half_pi), Fix::ZERO));

        let pi = Fix::lit("3.141592653589793");
        assert!(near(sin(pi), Fix::ZERO));
        assert!(near(cos(pi), -Fix::ONE));

        assert!(near(
            atan2(Fix::ONE, Fix::ONE),
            Fix::lit("0.7853981633974483")
        ));
        assert!(near(atan2(Fix::ZERO, Fix::ONE), Fix::ZERO));
    }

    #[test]
    fn sin_and_cos_can_leave_the_unit_range() {
        // Documented hazard, asserted so it cannot be forgotten: results may sit
        // slightly outside -1..=1, so any caller feeding these into a
        // domain-restricted operation has to clamp at that call site.
        let mut worst = 0i64;
        for i in -2000..=2000i64 {
            let a = Fix::from_num(i) / Fix::lit("100");
            worst = worst.max(sin(a).to_bits().abs() - Fix::ONE.to_bits());
            worst = worst.max(cos(a).to_bits().abs() - Fix::ONE.to_bits());
        }
        assert!(worst > 0, "expected some overshoot beyond 1.0");
        assert!(
            worst <= TRIG_ERROR_BOUND,
            "overshoot {worst} exceeds the documented bound"
        );
    }

    #[test]
    fn normalized_vectors_have_unit_length_to_within_a_few_units() {
        // Truncation in the divide and in the squaring leaves the result just
        // under one. It is always under, never over, which matters for anything
        // that later takes a square root of one minus this.
        for (x, y) in [("3", "4"), ("5", "12"), ("8", "15"), ("7", "24")] {
            let n = FVec2::new(Fix::lit(x), Fix::lit(y)).normalized_safe();
            let err = Fix::ONE.to_bits() - n.sqr_magnitude().to_bits();
            assert!((0..=8).contains(&err), "unit length off by {err} raw units");
        }
    }

    #[test]
    fn operators_agree_with_their_assigning_forms() {
        let a = FVec2::new(Fix::lit("1.5"), Fix::lit("-2.25"));
        let b = FVec2::new(Fix::lit("-0.75"), Fix::lit("3"));
        let s = Fix::lit("2.5");

        let mut t = a;
        t += b;
        assert_eq!(t, a + b);
        let mut t = a;
        t -= b;
        assert_eq!(t, a - b);
        let mut t = a;
        t *= s;
        assert_eq!(t, a * s);
        let mut t = a;
        t /= s;
        assert_eq!(t, a / s);

        assert_eq!(a - b, a + (-b));
        assert_eq!((a * s) / s, a);
    }

    /// A near-vertical vector used to take the whole simulation down: the
    /// CORDIC routine forms `y / x` and that quotient does not fit in Q32.32
    /// once `x` is small enough. A stick held straight up produces exactly
    /// this, so it is ordinary input rather than a corner case.
    #[test]
    fn atan2_survives_vectors_that_are_almost_vertical() {
        let tall = Fix::ONE;
        for wide in [
            Fix::from_bits(1),
            Fix::from_bits(-1),
            Fix::from_bits(7),
            Fix::lit("0.0000000007"),
            Fix::lit("-0.0000000007"),
            Fix::ZERO,
        ] {
            let up = atan2(tall, wide);
            let down = atan2(-tall, wide);
            assert!(
                (up - Fix::FRAC_PI_2).abs().to_bits() <= TRIG_ERROR_BOUND,
                "up was {up} for x = {wide}"
            );
            assert!(
                (down + Fix::FRAC_PI_2).abs().to_bits() <= TRIG_ERROR_BOUND,
                "down was {down} for x = {wide}"
            );
        }
    }

    /// The shortcut must not disturb angles that were never in danger.
    #[test]
    fn atan2_is_unchanged_away_from_the_vertical() {
        assert_eq!(atan2(Fix::ONE, Fix::ONE), cordic::atan2(Fix::ONE, Fix::ONE));
        assert_eq!(
            atan2(Fix::ONE, Fix::lit("-0.001")),
            cordic::atan2(Fix::ONE, Fix::lit("-0.001"))
        );
        assert_eq!(
            atan2(Fix::lit("-3"), Fix::lit("4")),
            cordic::atan2(Fix::lit("-3"), Fix::lit("4"))
        );
    }
}
