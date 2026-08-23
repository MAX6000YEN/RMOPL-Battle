//! The action set, which is also the wire format.
//!
//! This is the entire game's input: a stick and six buttons. Everything a
//! player can ever express passes through [`ActionState`], and it stays
//! ability-agnostic — an ability reads the same six buttons every other
//! ability reads. Adding a field here costs bandwidth on every player on every
//! tick forever, so the bar for adding one is very high.
//!
//! **The stick is quantised here, at the input boundary, and nowhere else.**
//! If a local player simulated a full-precision deflection and remote peers
//! reconstructed a quantised one, the two simulations would disagree by a
//! fraction of a unit per tick and diverge with no visible cause. Quantising
//! once, before the value ever reaches the simulation, means the local player
//! runs on exactly the bytes the remote peers will receive.

use crate::constants::INPUT_DEADZONE;
use crate::math::{FVec2, Fix, atan2};

/// Angle quantisation: a full turn divided into 256 steps, so one step is a
/// little over 1.4 degrees. At the speeds involved that is well below what a
/// player can perceive or aim with.
pub const ANGLE_STEPS: u32 = 256;

/// Full deflection. Magnitude is a fraction of this.
pub const MAGNITUDE_MAX: u8 = 255;

/// What one player is asking for on one tick.
///
/// Packs to three bytes: two for the stick, six bits for the buttons.
///
/// Neutral has exactly one representation — `magnitude == 0` — and the angle
/// is forced to zero with it. A second way to say "no input" (a reserved angle
/// value, say) would be a value two peers could encode differently while
/// meaning the same thing, which is the same desync in a different disguise.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct ActionState {
    /// Direction, as a fraction of a full turn measured counter-clockwise from
    /// the positive x-axis. Meaningless, and always zero, when `stick_magnitude`
    /// is zero.
    pub stick_angle: u8,
    /// Deflection, zero for neutral up to [`MAGNITUDE_MAX`] for the edge of the
    /// gate.
    ///
    /// Reserved rather than used: nothing reads it yet, and movement is
    /// currently uniform-speed. It is here because it cannot be added later
    /// without changing the wire format, which means changing it in lockstep on
    /// every peer at once. One byte per player per tick is a cheap price for
    /// keeping analogue movement possible.
    pub stick_magnitude: u8,
    pub jump: bool,
    pub ab1: bool,
    pub ab2: bool,
    pub ab3: bool,
    pub start: bool,
    pub select: bool,
}

impl ActionState {
    /// No stick, no buttons.
    pub const NEUTRAL: Self = Self {
        stick_angle: 0,
        stick_magnitude: 0,
        jump: false,
        ab1: false,
        ab2: false,
        ab3: false,
        start: false,
        select: false,
    };

    /// Quantises a raw stick deflection into the wire representation.
    ///
    /// The deadzone is applied *before* quantisation, so a resting stick is
    /// bit-identical neutral on every machine regardless of how much its
    /// hardware drifts. Deflection beyond full is clamped rather than wrapped.
    ///
    /// Magnitude is not rescaled from the deadzone edge: the smallest non-zero
    /// value a stick can produce is the deadzone itself. Whether that edge
    /// should map to zero movement is a feel question, and it belongs to the
    /// movement code that first reads magnitude.
    pub fn with_stick(mut self, deflection: FVec2) -> Self {
        let magnitude = deflection.magnitude();
        if magnitude < INPUT_DEADZONE {
            self.stick_angle = 0;
            self.stick_magnitude = 0;
            return self;
        }

        self.stick_angle = quantise_angle(atan2(deflection.y, deflection.x));
        self.stick_magnitude = quantise_magnitude(magnitude);
        self
    }

    /// True when the stick is deflected past the deadzone.
    pub const fn has_stick(self) -> bool {
        self.stick_magnitude > 0
    }

    /// The stick direction in radians, or `None` when neutral.
    ///
    /// Reconstructed from the quantised byte, so this returns the same value on
    /// the machine that produced the input and on every machine that receives
    /// it.
    pub fn stick_radians(self) -> Option<Fix> {
        if !self.has_stick() {
            return None;
        }
        let turn = Fix::from_num(self.stick_angle) / Fix::from_num(ANGLE_STEPS);
        Some(turn * Fix::TAU - Fix::PI)
    }
}

/// Maps radians in -PI..=PI onto 0..=255.
///
/// Rounds to nearest rather than truncating, so the quantisation error is
/// symmetric and half the size. The full-turn case wraps to zero: -PI and PI
/// are the same direction and must not be two different bytes.
fn quantise_angle(radians: Fix) -> u8 {
    let turn = (radians + Fix::PI) / Fix::TAU;
    let scaled = turn * Fix::from_num(ANGLE_STEPS) + Fix::lit("0.5");
    // Trig error can push the result a hair outside the range at the ends.
    let step = scaled.to_num::<i64>().clamp(0, ANGLE_STEPS as i64);
    (step % ANGLE_STEPS as i64) as u8
}

/// Maps a deflection of 0..=1 onto 0..=255, saturating past full deflection.
///
/// Never returns zero for a deflection that cleared the deadzone: zero means
/// neutral, and a stick that is being held must not read as released.
fn quantise_magnitude(magnitude: Fix) -> u8 {
    let scaled = magnitude * Fix::from_num(MAGNITUDE_MAX) + Fix::lit("0.5");
    scaled.to_num::<i64>().clamp(1, MAGNITUDE_MAX as i64) as u8
}

/// Which keys drive one keyboard player.
///
/// Two of these is what the game needs today. This is deliberately not a device
/// abstraction layer: gamepads, hot-plug and enumeration arrive with the
/// hardware they can actually be tested against, and building the abstraction
/// before then would be guessing at its shape.
#[derive(Clone, Copy, Debug)]
pub struct KeyboardBinding {
    pub up: KeyCode,
    pub down: KeyCode,
    pub left: KeyCode,
    pub right: KeyCode,
    pub jump: KeyCode,
    pub ab1: KeyCode,
    pub ab2: KeyCode,
    pub ab3: KeyCode,
    pub start: KeyCode,
    pub select: KeyCode,
}

pub use macroquad::input::KeyCode;

/// The keyboard players available on this machine, in order.
///
/// The length of this table is how many keyboard players there are; nothing
/// else in the codebase may assume the number. Local player count, peer count
/// and total player count are independent quantities.
pub const KEYBOARD_BINDINGS: &[KeyboardBinding] = &[
    KeyboardBinding {
        up: KeyCode::W,
        down: KeyCode::S,
        left: KeyCode::A,
        right: KeyCode::D,
        jump: KeyCode::Space,
        ab1: KeyCode::F,
        ab2: KeyCode::G,
        ab3: KeyCode::H,
        start: KeyCode::Tab,
        select: KeyCode::Q,
    },
    KeyboardBinding {
        up: KeyCode::Up,
        down: KeyCode::Down,
        left: KeyCode::Left,
        right: KeyCode::Right,
        jump: KeyCode::RightShift,
        ab1: KeyCode::Kp1,
        ab2: KeyCode::Kp2,
        ab3: KeyCode::Kp3,
        start: KeyCode::Enter,
        select: KeyCode::Kp0,
    },
];

/// Reads one keyboard player's current state.
///
/// A keyboard has no analogue stick, so a held direction is always full
/// deflection. Opposite keys cancel, which keeps a stuck key from producing a
/// direction the player is not asking for.
pub fn poll_keyboard(binding: &KeyboardBinding) -> ActionState {
    use macroquad::input::is_key_down;

    let axis = |negative, positive| match (is_key_down(negative), is_key_down(positive)) {
        (true, false) => Fix::NEG_ONE,
        (false, true) => Fix::ONE,
        _ => Fix::ZERO,
    };

    let deflection = FVec2::new(
        axis(binding.left, binding.right),
        axis(binding.down, binding.up),
    );

    ActionState {
        jump: is_key_down(binding.jump),
        ab1: is_key_down(binding.ab1),
        ab2: is_key_down(binding.ab2),
        ab3: is_key_down(binding.ab3),
        start: is_key_down(binding.start),
        select: is_key_down(binding.select),
        ..ActionState::NEUTRAL
    }
    .with_stick(deflection.normalized_safe())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A resting stick must be bit-identical neutral, or every idle player
    /// contributes drift to the simulation.
    #[test]
    fn deflection_inside_the_deadzone_is_exactly_neutral() {
        let small = INPUT_DEADZONE / Fix::lit("2");
        for direction in [FVec2::UP, FVec2::RIGHT, FVec2::new(small, -small)] {
            let state = ActionState::NEUTRAL.with_stick(direction * small);
            assert_eq!(state.stick_magnitude, 0);
            assert_eq!(state.stick_angle, 0);
            assert!(!state.has_stick());
            assert_eq!(state.stick_radians(), None);
        }
    }

    /// Just past the deadzone must never quantise back down to neutral.
    #[test]
    fn deflection_just_past_the_deadzone_is_not_neutral() {
        let just_past = INPUT_DEADZONE + Fix::lit("0.001");
        let state = ActionState::NEUTRAL.with_stick(FVec2::RIGHT * just_past);
        assert!(state.has_stick());
        assert!(state.stick_magnitude >= 1);
    }

    /// The cardinal directions are the ones a keyboard produces, so they are
    /// the ones most worth pinning exactly.
    #[test]
    fn cardinal_directions_quantise_to_exact_quarters() {
        let cases = [
            (FVec2::RIGHT, 128u8),
            (FVec2::UP, 192),
            (FVec2::LEFT, 0),
            (FVec2::DOWN, 64),
        ];
        for (direction, expected) in cases {
            let state = ActionState::NEUTRAL.with_stick(direction);
            assert_eq!(state.stick_angle, expected, "direction {direction:?}");
            assert_eq!(state.stick_magnitude, MAGNITUDE_MAX);
        }
    }

    /// -PI and PI are the same direction and must not encode as two bytes.
    #[test]
    fn the_wrap_point_has_a_single_encoding() {
        let above = ActionState::NEUTRAL.with_stick(FVec2::new(Fix::NEG_ONE, Fix::lit("0.0001")));
        let below = ActionState::NEUTRAL.with_stick(FVec2::new(Fix::NEG_ONE, Fix::lit("-0.0001")));
        let exact = ActionState::NEUTRAL.with_stick(FVec2::LEFT);

        // Approaching from either side lands on the same byte, and so does
        // sitting exactly on it. Two encodings of one direction would be two
        // peers disagreeing about a stick that is pointing the same way.
        assert_eq!(above.stick_angle, 0);
        assert_eq!(below.stick_angle, 0);
        assert_eq!(exact.stick_angle, 0);
    }

    /// Every byte must survive the round trip back to an angle within one
    /// quantisation step, or the reconstruction remote peers do is not the
    /// value the local player simulated.
    #[test]
    fn every_angle_byte_round_trips_within_one_step() {
        let step = Fix::TAU / Fix::from_num(ANGLE_STEPS);
        for byte in 0..=255u8 {
            let state = ActionState {
                stick_angle: byte,
                stick_magnitude: MAGNITUDE_MAX,
                ..ActionState::NEUTRAL
            };
            let radians = state.stick_radians().expect("stick is deflected");
            let requantised = ActionState::NEUTRAL
                .with_stick(FVec2::new(
                    crate::math::cos(radians),
                    crate::math::sin(radians),
                ))
                .stick_angle;
            assert_eq!(requantised, byte, "byte {byte} did not survive");
            assert!(radians >= -Fix::PI && radians <= Fix::PI);
            assert!(step > Fix::ZERO);
        }
    }

    /// Deflection past the gate is clamped, not wrapped. An overdriven stick
    /// reading 1.2 must not come back as a fifth of full deflection.
    #[test]
    fn overdriven_deflection_saturates() {
        let state = ActionState::NEUTRAL.with_stick(FVec2::RIGHT * Fix::lit("3"));
        assert_eq!(state.stick_magnitude, MAGNITUDE_MAX);
        assert_eq!(state.stick_angle, 128);
    }

    /// Magnitude is reserved, not yet used, but it must already be carrying a
    /// real value or Phase 09 would ship a wire format with a dead byte in it.
    #[test]
    fn magnitude_tracks_deflection() {
        let half = ActionState::NEUTRAL.with_stick(FVec2::RIGHT * Fix::lit("0.5"));
        let full = ActionState::NEUTRAL.with_stick(FVec2::RIGHT);
        assert!(half.stick_magnitude < full.stick_magnitude);
        assert_eq!(half.stick_magnitude, 128);
    }

    /// Buttons are independent; setting one must not disturb the others or the
    /// stick.
    #[test]
    fn buttons_are_independent_of_the_stick() {
        let state = ActionState {
            jump: true,
            ab2: true,
            ..ActionState::NEUTRAL
        }
        .with_stick(FVec2::UP);

        assert!(state.jump && state.ab2);
        assert!(!state.ab1 && !state.ab3 && !state.start && !state.select);
        assert_eq!(state.stick_angle, 192);
    }

    /// Two keyboard players must not share a single key, or one steals the
    /// other's input.
    #[test]
    fn keyboard_bindings_do_not_overlap() {
        let mut seen: Vec<KeyCode> = Vec::new();
        for binding in KEYBOARD_BINDINGS {
            for key in [
                binding.up,
                binding.down,
                binding.left,
                binding.right,
                binding.jump,
                binding.ab1,
                binding.ab2,
                binding.ab3,
                binding.start,
                binding.select,
            ] {
                assert!(!seen.contains(&key), "{key:?} is bound twice");
                seen.push(key);
            }
        }
        assert_eq!(seen.len(), KEYBOARD_BINDINGS.len() * 10);
    }
}
