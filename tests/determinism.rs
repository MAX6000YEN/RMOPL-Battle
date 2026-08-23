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

/// A recorded match, replayed to a single number.
///
/// This is the seed of the determinism harness. It is not testing that the
/// simulation does anything interesting — it does almost nothing yet — but that
/// a fixed seed and a fixed input sequence produce the same bits on Linux and
/// on macOS, which is the property the netcode is built on. CI runs it on both,
/// which is what makes the pinned number below mean something.
///
/// If this value changes, something changed the meaning of a tick. That is
/// sometimes intentional, and it is never something to update without knowing
/// why. It has moved once so far: spawn insertion was reordered when the
/// ordering key stopped being a hash of the spawn's fields and became the
/// fields themselves, which changed which entity got which id.
#[test]
fn a_recorded_match_replays_to_the_same_checksum() {
    use rmopl::ids::PlayerId;
    use rmopl::input::ActionState;
    use rmopl::sim::{Sim, Spawn};

    let mut sim = Sim::new(0x1234_5678_9abc_def0);
    let mut inputs = Vec::new();

    for tick in 0..600u64 {
        inputs.clear();
        for n in 1..=8u8 {
            let angle = Fix::from_num(((tick * 7 + u64::from(n) * 13) % 360) as i32);
            let radians = angle * Fix::PI / Fix::lit("180");
            let action = ActionState {
                jump: (tick + u64::from(n)) % 11 == 0,
                ab1: tick % 5 == 0,
                ab2: n % 3 == 0,
                start: false,
                select: tick == 300,
                ..ActionState::NEUTRAL
            }
            .with_stick(FVec2::new(cos(radians), sin(radians)));
            inputs.push((PlayerId::new(n), action));
        }

        // Spawn requests are made in an order that changes tick to tick; the
        // resulting world must not.
        if tick % 50 == 0 {
            for n in 0..4u8 {
                let offset = Fix::from_num(i32::from((tick as u8).wrapping_add(n)));
                sim.request_spawn(Spawn {
                    position: FVec2::new(offset / Fix::lit("7"), -offset / Fix::lit("3")),
                    owner: Some(PlayerId::new(n)),
                });
            }
        }

        sim.step(&inputs);
    }

    assert_eq!(sim.tick().get(), 600);
    assert_eq!(sim.entity_count(), 48);
    assert_eq!(sim.state_hash(), 11_755_374_171_786_398_137);
}
