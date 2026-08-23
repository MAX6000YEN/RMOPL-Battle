//! Tuning constants for the simulation. Values only, no logic.
//!
//! Each constant is justified by the test that pins it or by the comment above
//! its block, in plain language and in the units the simulation actually uses.

use crate::math::Fix;

// --- Tick rate -------------------------------------------------------------

/// Authoritative simulation steps per second.
///
/// Fixed and completely independent of rendering. A renderer running at 60,
/// 144 or 240 FPS produces exactly the same number of simulation steps in the
/// same order; frame rate may change how often the world is drawn and never
/// what the world does. The netcode depends on this, since peers agree on tick
/// numbers rather than on wall-clock time.
pub const TICKS_PER_SECOND: u32 = 120;

/// Length of one simulation tick, in seconds.
///
/// One-hundred-and-twentieth has no exact representation in Q32.32 (2^32 / 120
/// is 35791394.13...), so this is the truncated value and multiplying it by 120
/// lands 16 raw units short of one.
///
/// Fixed-point addition does not round, so summing this constant N times gives
/// exactly the same answer as multiplying it by N: unlike floats, there is no
/// stochastic accumulation of error here. The shortfall is instead systematic,
/// a steady 0.1333 raw units per tick, so any elapsed time expressed in seconds
/// drifts linearly and predictably below the truth.
///
/// The fix is not to accumulate more carefully, it is not to convert at all.
/// Authoritative elapsed time is a `u64` tick count, which is exact, is what
/// peers actually agree on, and never needs this constant. Use this only to
/// scale a per-second rate into one tick's worth of change.
pub const FIXED_DELTA_TIME: Fix = Fix::lit("1").strict_div(Fix::lit("120"));

// --- Player physics --------------------------------------------------------
//
// IMPORTANT — these were tuned for a 60 Hz tick and several of them are applied
// once per tick with no delta-time factor, which means they are not rates but
// per-tick deltas. At the 120 Hz tick rate above, using them unchanged in a
// per-tick update would apply them twice as often and double the effect.
//
// Resolving that belongs to the phases that write the movement and gravity
// code, not here; this module only records the values. The classification is
// noted so the conversion is a deliberate decision rather than an accident:
//
//   per-tick additions, halve:      ACCEL_AIR, GRAVITY_ACCEL (converted),
//                                   MIN_TURNSPEED
//   per-tick multipliers, take the  SLIPPERINESS_ICE, SLIPPERINESS_DEFAULT
//     square root, not the half:
//   already scaled by delta time:   ACCEL_GROUND
//   absolute limits, unaffected:    MAX_SPEED, GRAVITY_MAX_FALL_SPEED
//   one-shot impulses, unaffected:  JUMP_STRENGTH, UNGROUND_NUDGE,
//                                   JUMP_EXTRA_TELEPORT_FACTOR
//
// Halving is not universally right even within the first group.
//
// Air acceleration is paired with a drag term of ACCEL_AIR / (MAX_SPEED +
// ACCEL_AIR), and where the cap lands depends on the order the two are applied
// in within one tick. Accelerate first and then apply the drag to the result,
// and the equilibrium is exactly MAX_SPEED for *every* value of the
// coefficient, so halving moves nothing. Apply the drag first and the
// equilibrium is MAX_SPEED + ACCEL_AIR instead, and then halving does move it.
// The hazard is the ordering, not the halving; an earlier note here had that
// backwards.
//
// Retained-speed multipliers compound, so applying 0.5 twice as often is 0.25
// per unit of real time, and the tick-rate-invariant value is its square root.

/// Horizontal speed cap while grounded.
pub const MAX_SPEED: Fix = Fix::lit("19");
/// Upward impulse applied on the tick a jump starts.
pub const JUMP_STRENGTH: Fix = Fix::lit("30");
/// Ground acceleration, scaled by delta time where it is applied.
pub const ACCEL_GROUND: Fix = Fix::lit("180");
/// Air acceleration, applied per tick.
pub const ACCEL_AIR: Fix = Fix::lit("10");
/// Collision radius of a player.
pub const RADIUS: Fix = Fix::lit("0.76");
/// Downward acceleration, applied per tick.
///
/// **Converted for the 120 Hz tick.** The tuned value was 1.6 against a 60 Hz
/// tick, applied once per tick with no delta-time factor, so it is a per-tick
/// addition rather than a rate: used unchanged at 120 Hz it would fire twice as
/// often and fall twice as hard. Halving is the analytically correct conversion
/// for this group.
///
/// The conversion is *not* guarded by the terminal-velocity test. The drag term
/// is derived from this constant, so halving the acceleration halves the drag
/// with it and the equilibrium `g == v * (g / v_max)` still solves to `v_max`
/// for every `g`. What actually changes is how long the fall takes to get
/// there, which is why `player_physics` pins the tick count and the distance
/// fallen instead.
pub const GRAVITY_ACCEL: Fix = Fix::lit("0.8");
/// Terminal velocity; the gravity drag term is derived so falling settles here.
pub const GRAVITY_MAX_FALL_SPEED: Fix = Fix::lit("27");
/// Per-level gravity scale. One is normal gravity.
pub const GRAVITY_MODIFIER: Fix = Fix::lit("1");
/// Gravity scale on low-gravity levels.
pub const GRAVITY_MODIFIER_SPACE: Fix = Fix::lit("0.5");
/// How much of a slope's horizontal normal is added to a jump.
pub const JUMP_EXTRA_X_STRENGTH: Fix = Fix::lit("0.6");
/// Fraction of existing tangential speed carried through a jump.
pub const JUMP_KEPT_MOMENTUM: Fix = Fix::lit("0.65");
/// Divides the surface normal term that weakens jumps off steep surfaces.
pub const JUMP_NORMAL_SCALE_FACTOR: Fix = Fix::lit("1");
/// Fraction of the new velocity applied as an instant position offset on jump,
/// so a jump visibly leaves the ground on its first tick.
pub const JUMP_EXTRA_TELEPORT_FACTOR: Fix = Fix::lit("0.02");
/// Inverse mass used when resolving player-to-player pushes.
pub const INVERSE_MASS: Fix = Fix::lit("35");
/// Retained tangential speed per tick on ice.
pub const SLIPPERINESS_ICE: Fix = Fix::lit("0.87");
/// Retained tangential speed per tick on ordinary ground.
pub const SLIPPERINESS_DEFAULT: Fix = Fix::lit("0.5");
/// Pushed this far along the surface normal when leaving the ground, so the
/// next tick's ground check does not immediately re-attach.
pub const UNGROUND_NUDGE: Fix = Fix::lit("0.05");

// --- Gameplay --------------------------------------------------------------

/// Below this, a surface counts as flat for the purpose of aligning to it.
pub const GROUND_ALIGNMENT_DEADZONE: Fix = Fix::lit("0.2");
/// Stick deflection below this reads as no input.
pub const INPUT_DEADZONE: Fix = Fix::lit("0.4");
/// Floor on turn rate, so alignment always completes instead of asymptoting.
pub const MIN_TURNSPEED: Fix = Fix::lit("0.02");

/// Maximum simultaneous players in a match, local and online combined.
pub const MAX_PLAYERS: usize = 16;
/// Attributed as the source of damage that no player caused.
pub const ENVIRONMENTAL_PLAYER_ID: u32 = 1000;
pub const MAX_CLONES: usize = 16;
pub const MAX_BEAMS: usize = 64;

// --- Level bounds ----------------------------------------------------------

pub const CAMERA_X_MIN: Fix = Fix::lit("-97.27");
pub const CAMERA_X_MAX: Fix = Fix::lit("97.6");
pub const CAMERA_Y_MIN: Fix = Fix::lit("-26");
pub const CAMERA_Y_MAX: Fix = Fix::lit("40");
pub const BLASTZONE_X_MIN: Fix = Fix::lit("-105");
pub const BLASTZONE_X_MAX: Fix = Fix::lit("105");
pub const BLASTZONE_Y_MAX: Fix = Fix::lit("58");
/// There is no lower blast zone; falling out the bottom means hitting water.
pub const WATER_HEIGHT: Fix = Fix::lit("-11.3");
pub const WATER_HEIGHT_SPACE: Fix = Fix::lit("-50");

// --- Session ---------------------------------------------------------------

/// Round length before sudden death begins, in seconds.
pub const SECONDS_BEFORE_SUDDEN_DEATH: u32 = 120;
/// Number of distinct team spawn points a level provides.
pub const TEAM_SPAWNS: usize = 4;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Fix;

    /// One over one hundred and twenty is not representable in binary fixed
    /// point, so this pins the exact truncated value instead of asserting an
    /// identity that cannot hold. Multiplying it by the tick rate is 16 raw
    /// units short of one, which is why elapsed time is counted in ticks.
    #[test]
    fn fixed_delta_time_is_the_expected_truncation() {
        assert_eq!(FIXED_DELTA_TIME.to_bits(), 35_791_394);
        assert_eq!(TICKS_PER_SECOND, 120);

        let a_second = FIXED_DELTA_TIME * Fix::from_num(TICKS_PER_SECOND);
        assert_ne!(a_second, Fix::ONE, "1/120 cannot be exact in Q32.32");
        assert_eq!(Fix::ONE.to_bits() - a_second.to_bits(), 16);
    }

    /// Summing delta time is exactly as accurate as multiplying by it, and both
    /// are systematically short of the truth. Ten minutes of ticks lands 9600
    /// raw units below 600 seconds either way, which is the concrete reason
    /// elapsed time is counted in ticks rather than converted to seconds.
    #[test]
    fn seconds_derived_from_delta_time_are_short_however_they_are_computed() {
        let ticks = 600 * TICKS_PER_SECOND;
        let mut summed = Fix::ZERO;
        for _ in 0..ticks {
            summed += FIXED_DELTA_TIME;
        }
        let multiplied = Fix::from_num(ticks) * FIXED_DELTA_TIME;

        // Fixed-point addition is exact, so these two agree bit for bit.
        assert_eq!(summed, multiplied);

        // And both fall short of the real elapsed time by the same amount.
        assert_eq!(Fix::from_num(600).to_bits() - summed.to_bits(), 9_600);

        // The tick count itself carries no error at all.
        assert_eq!(ticks, 72_000);
    }

    #[test]
    fn tuning_constants_have_their_expected_bits() {
        // Halved from the 1.6 tuned against a 60 Hz tick, which had
        // 6_871_947_674 raw units. Halving is exact in binary.
        assert_eq!(GRAVITY_ACCEL.to_bits(), 3_435_973_837);
        assert_eq!(GRAVITY_ACCEL * Fix::lit("2"), Fix::lit("1.6"));
        assert_eq!(RADIUS.to_bits(), 3_264_175_145);

        // Whole numbers are exact, so scaling one stays exact.
        assert_eq!(MAX_SPEED * Fix::lit("2"), Fix::lit("38"));

        // Sixty-five hundredths is not a binary fraction, so the product lands
        // 12 raw units below the decimal answer. Pinned rather than rounded, so
        // a change in how literals are parsed shows up here.
        let kept = JUMP_STRENGTH * JUMP_KEPT_MOMENTUM;
        assert_eq!(kept.to_bits(), 83_751_862_260);
        assert_eq!(Fix::lit("19.5").to_bits() - kept.to_bits(), 12);
    }

    /// The gravity drag term is derived so that falling settles at terminal
    /// velocity; this checks the division that derives it stays sane.
    #[test]
    fn gravity_drag_ratio_is_small_and_positive() {
        let drag = GRAVITY_ACCEL / GRAVITY_MAX_FALL_SPEED;
        assert!(drag > Fix::ZERO);
        assert!(drag < Fix::lit("0.1"));
        assert_eq!(drag.to_bits(), 127_258_290);
    }

    #[test]
    fn bounds_are_ordered_and_blastzone_contains_the_camera() {
        assert!(CAMERA_X_MIN < CAMERA_X_MAX);
        assert!(CAMERA_Y_MIN < CAMERA_Y_MAX);
        assert!(BLASTZONE_X_MIN < CAMERA_X_MIN);
        assert!(BLASTZONE_X_MAX > CAMERA_X_MAX);
        assert!(BLASTZONE_Y_MAX > CAMERA_Y_MAX);
        assert!(WATER_HEIGHT > CAMERA_Y_MIN);
        assert!(WATER_HEIGHT_SPACE < WATER_HEIGHT);
    }

    #[test]
    fn player_ids_cannot_collide_with_the_environment_id() {
        assert!(MAX_PLAYERS as u32 <= ENVIRONMENTAL_PLAYER_ID);
        assert_eq!(MAX_PLAYERS, 16);
    }

    #[test]
    fn slipperiness_values_retain_speed_rather_than_add_it() {
        for s in [SLIPPERINESS_ICE, SLIPPERINESS_DEFAULT] {
            assert!(s > Fix::ZERO && s < Fix::ONE);
        }
        assert!(SLIPPERINESS_ICE > SLIPPERINESS_DEFAULT);
    }

    #[test]
    fn deadzones_are_fractions() {
        for d in [GROUND_ALIGNMENT_DEADZONE, INPUT_DEADZONE, MIN_TURNSPEED] {
            assert!(d > Fix::ZERO && d < Fix::ONE);
        }
    }
}
