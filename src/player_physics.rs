//! Gravity, ground raycasts and grounding.
//!
//! Everything here runs inside a tick, from the physics slot of [`crate::sim`].
//! Nothing reads a clock, and nothing multiplies by delta time except the one
//! place that turns a velocity into a distance.
//!
//! Only the flat top face of a platform can be landed on. Walls, ceilings and
//! corners are surface-walking work and belong to a later phase.

use slotmap::SlotMap;

use crate::constants::{
    FIXED_DELTA_TIME, GRAVITY_ACCEL, GRAVITY_MAX_FALL_SPEED, RADIUS, UNGROUND_NUDGE,
};
use crate::ids::EntityId;
use crate::math::{FVec2, Fix};
use crate::platform::Platform;
use crate::sim::{Entity, Grounded};

/// Applies one tick of gravity, with the drag term that gives falling a
/// terminal velocity.
///
/// The model is `v += g * (1 - v / v_max)`, so the acceleration fades to
/// nothing as the fall speed approaches the limit, and a body already falling
/// faster than the limit is decelerated instead.
///
/// **No delta-time factor.** [`GRAVITY_ACCEL`] is a per-tick addition, not a
/// per-second rate. It was tuned against a 60 Hz tick and halved for the 120 Hz
/// one this simulation actually runs at; the tests that pin that conversion
/// measure how long the fall takes, because the terminal velocity it converges
/// to is the same either way.
pub fn add_gravity(velocity: &mut FVec2) {
    let fall = -velocity.y;
    velocity.y -= GRAVITY_ACCEL * (Fix::ONE - fall / GRAVITY_MAX_FALL_SPEED);
}

/// Where a body's ground rays first met a platform.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GroundHit {
    pub id: EntityId,
    pub platform: Platform,
    /// Where the body's centre is at the moment of contact.
    pub center: FVec2,
}

/// Casts a body's three ground rays along the movement it is about to make.
///
/// One ray down the centre is not enough: a body moving fast enough can pass a
/// platform's corner with its centre line missing the top face entirely while
/// most of the body went through it. So the centre ray is joined by two more,
/// offset by the body radius either side, perpendicular to the direction of
/// travel. That is the difference between landing and tunnelling, not a
/// belt-and-braces measure.
///
/// The rays run the distance about to be travelled **plus the body radius**, so
/// a body lands when its underside reaches the surface rather than when its
/// centre does. Without the extension a body would sink to its middle before
/// anything noticed.
pub fn velocity_based_raycasts(
    platforms: &[(EntityId, Platform)],
    center: FVec2,
    delta: FVec2,
) -> Option<GroundHit> {
    let direction = delta.normalized_safe();
    if direction == FVec2::ZERO {
        return None;
    }
    let reach = delta + direction * RADIUS;
    let offset = direction.perp() * RADIUS;
    let origins = [center, center + offset, center - offset];

    let mut best: Option<(Fix, EntityId, Platform)> = None;
    for (id, platform) in platforms {
        for origin in origins {
            let Some(t) = platform.top_face_crossing(origin, reach) else {
                continue;
            };
            // Strictly nearer, so a tie is broken by the order the candidates
            // are visited in: platforms in slotmap order, then the centre ray,
            // then the offset ray on the left of the direction of travel, then
            // the one on the right. That order is a function of the insert and
            // remove history alone, so every peer resolves the tie the same way.
            if best.is_none_or(|(nearest, _, _)| t < nearest) {
                best = Some((t, *id, *platform));
            }
        }
    }

    best.map(|(t, id, platform)| GroundHit {
        id,
        platform,
        center: center + reach * t,
    })
}

/// Attaches a body to the top face of the platform it just hit.
///
/// The surface position is stored as a scalar along the platform's perimeter,
/// never as coordinates: coordinates would work perfectly for a flat top and
/// then have to be thrown away when bodies start walking around corners.
///
/// A contact past either end of the flat top attaches at the end instead. Only
/// the flat face is walkable this phase, and clamping keeps the stored position
/// inside the zone [`Platform::normal_at`] answers as up-facing, so the landing
/// query and the normal cannot disagree about the same body.
pub fn attach_to_ground(entity: &mut Entity, hit: &GroundHit) {
    let extent = hit.platform.extents.x;
    let x = hit.platform.to_local(hit.center).x.clamp(-extent, extent);

    entity.grounded = Some(Grounded {
        platform: hit.id,
        local_pos: hit.platform.local_pos_of_top_x(x),
    });
    entity.self_imposed_velocity = FVec2::ZERO;
    entity.position = rest_position(&hit.platform, hit.platform.top_face_point(x));
}

/// Takes a body off the ground.
///
/// The nudge along the surface normal is what stops the next tick's ground
/// check from immediately re-attaching a body that has just left. Without it a
/// jump looks like it sticks to the floor, and the bug looks like it belongs to
/// whatever code asked for the jump.
pub fn unground(entity: &mut Entity, normal: FVec2) {
    entity.grounded = None;
    entity.position += normal * UNGROUND_NUDGE;
    entity.self_imposed_velocity = FVec2::ZERO;
}

/// Where a body's centre sits when resting on `point`.
fn rest_position(platform: &Platform, point: FVec2) -> FVec2 {
    point + platform.normal_at(point) * RADIUS
}

/// One tick of physics for every entity.
///
/// Bodies grounded on a platform have their world position re-derived from
/// their stored surface position; everything else falls, casts its rays, and
/// either lands or moves.
pub fn step(entities: &mut SlotMap<EntityId, Entity>) {
    // Gathered first because the loop below needs the world mutably. Slotmap
    // iteration order is a function of the insert and remove history alone,
    // which is identical on every peer, so this vector is too.
    let platforms: Vec<(EntityId, Platform)> = entities
        .iter()
        .filter_map(|(id, entity)| entity.platform().map(|platform| (id, platform)))
        .collect();

    for (_, entity) in entities.iter_mut() {
        // Platforms do not fall. Moving ones are a later phase.
        if entity.shape.is_some() {
            continue;
        }

        if let Some(grounded) = entity.grounded {
            // A linear scan: a level holds a handful of platforms, and this
            // keeps the deterministic slotmap ordering rather than trading it
            // for a map with an iteration order nobody agreed on.
            let resting = find(&platforms, grounded.platform).and_then(|platform| {
                platform
                    .surface_point(grounded.local_pos)
                    .map(|point| rest_position(&platform, point))
            });

            match resting {
                Some(position) => {
                    entity.position = position;
                    entity.self_imposed_velocity = FVec2::ZERO;
                    continue;
                }
                // The platform this body was standing on is gone. A
                // generational key fails the lookup instead of silently
                // addressing whatever was allocated in the same slot, and the
                // body falls again. Never unwrapped.
                None => entity.grounded = None,
            }
        }

        add_gravity(&mut entity.self_imposed_velocity);
        let delta = (entity.self_imposed_velocity + entity.external_velocity) * FIXED_DELTA_TIME;

        match velocity_based_raycasts(&platforms, entity.position, delta) {
            Some(hit) => attach_to_ground(entity, &hit),
            None => entity.position += delta,
        }
    }
}

fn find(platforms: &[(EntityId, Platform)], id: EntityId) -> Option<Platform> {
    platforms
        .iter()
        .find(|(candidate, _)| *candidate == id)
        .map(|(_, platform)| *platform)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{GRAVITY_MAX_FALL_SPEED, RADIUS};
    use crate::platform::{PlatformKind, PlatformShape};

    fn shape(half_width: &str, half_height: &str) -> PlatformShape {
        PlatformShape {
            extents: FVec2::new(Fix::lit(half_width), Fix::lit(half_height)),
            radius: Fix::lit("0.5"),
            kind: PlatformKind::Normal,
        }
    }

    fn body(x: &str, y: &str) -> Entity {
        Entity {
            position: FVec2::new(Fix::lit(x), Fix::lit(y)),
            self_imposed_velocity: FVec2::ZERO,
            external_velocity: FVec2::ZERO,
            rotation: Fix::ZERO,
            scale: Fix::ONE,
            owner: None,
            shape: None,
            grounded: None,
        }
    }

    fn moving(x: &str, y: &str, vx: &str, vy: &str) -> Entity {
        Entity {
            self_imposed_velocity: FVec2::new(Fix::lit(vx), Fix::lit(vy)),
            ..body(x, y)
        }
    }

    fn platform(x: &str, y: &str, rotation: &str, shape: PlatformShape) -> Entity {
        Entity {
            rotation: Fix::lit(rotation),
            shape: Some(shape),
            ..body(x, y)
        }
    }

    /// A body's world position is derived from its stored surface position
    /// every tick, and turning a distance along the perimeter into a fraction
    /// of it and back truncates. The loss is about 7e-9 of a world unit and it
    /// does not accumulate — the scalar is stored once, so the derived position
    /// is a pure function of it rather than a running sum.
    fn near(a: FVec2, b: FVec2) -> bool {
        a.distance(b) < Fix::lit("0.00001")
    }

    /// A world holding the given entities, in the order they are listed.
    fn world(entities: Vec<Entity>) -> (SlotMap<EntityId, Entity>, Vec<EntityId>) {
        let mut map = SlotMap::with_key();
        let ids = entities.into_iter().map(|e| map.insert(e)).collect();
        (map, ids)
    }

    // --- Gravity -----------------------------------------------------------

    /// Falling settles at terminal velocity and never passes it.
    ///
    /// This guards the drag derivation and **nothing else**. It would pass just
    /// as happily on the unconverted 60 Hz constant: the drag term is derived
    /// from the acceleration, so `g == v * (g / v_max)` solves to `v_max` for
    /// every value of `g`. The two tests after this one are the ones that guard
    /// the tick rate.
    ///
    /// It settles 27 raw units *below* the limit rather than exactly on it,
    /// because the last increments are small enough that a fixed-point multiply
    /// truncates them to nothing. That is a property of the arithmetic, not of
    /// the tuning — the unconverted constant happens to land exactly on 27
    /// because its increments stay larger for longer — and it is 6e-9 of a
    /// world unit. Pinned rather than papered over with a snap to the limit,
    /// which would be tuning the physics to flatter the test.
    #[test]
    fn falling_settles_at_terminal_velocity_and_never_exceeds_it() {
        let mut velocity = FVec2::ZERO;
        let mut settled_at = None;

        for tick in 1..=2_000u32 {
            let previous = velocity.y;
            add_gravity(&mut velocity);
            assert!(
                -velocity.y <= GRAVITY_MAX_FALL_SPEED,
                "overshot terminal velocity on tick {tick}"
            );
            if settled_at.is_none() && velocity.y == previous {
                settled_at = Some(tick - 1);
            }
        }

        assert_eq!(settled_at, Some(740), "convergence moved");
        assert_eq!(velocity.y.to_bits(), -115_964_116_965);
        assert_eq!(
            GRAVITY_MAX_FALL_SPEED.to_bits() + velocity.y.to_bits(),
            27,
            "terminal velocity should rest 27 raw units under the limit"
        );
        assert_eq!(velocity.x, Fix::ZERO, "gravity is not sideways");
    }

    /// **This is the test that guards the 60 to 120 Hz conversion.**
    ///
    /// The unconverted constant reaches this same fraction of terminal velocity
    /// on tick 50 — half a second earlier in wall-clock time — while converging
    /// to exactly the same limit. Nothing about the equilibrium can tell the two
    /// apart; only the approach curve can.
    #[test]
    fn falling_reaches_most_of_terminal_velocity_on_the_expected_tick() {
        let ninety_five_percent = GRAVITY_MAX_FALL_SPEED * Fix::lit("0.95");
        let half = GRAVITY_MAX_FALL_SPEED / Fix::lit("2");

        let mut velocity = FVec2::ZERO;
        let (mut reached_half, mut reached_most) = (None, None);
        for tick in 1..=200u32 {
            add_gravity(&mut velocity);
            if reached_half.is_none() && -velocity.y >= half {
                reached_half = Some(tick);
            }
            if reached_most.is_none() && -velocity.y >= ninety_five_percent {
                reached_most = Some(tick);
            }
        }

        // 0.20 s and 0.83 s at 120 ticks per second. On the unconverted
        // constant these are tick 12 and tick 50.
        assert_eq!(reached_half, Some(24));
        assert_eq!(reached_most, Some(100));
    }

    /// The other half of the tick-rate guard, and the one a player would
    /// actually feel: how far a body falls in a given time.
    #[test]
    fn a_body_falls_a_pinned_distance_in_a_pinned_number_of_ticks() {
        let mut velocity = FVec2::ZERO;
        let mut fallen = Fix::ZERO;
        let mut after_half_a_second = Fix::ZERO;

        for tick in 1..=120u32 {
            add_gravity(&mut velocity);
            fallen -= velocity.y * FIXED_DELTA_TIME;
            if tick == 60 {
                after_half_a_second = fallen;
            }
        }

        // Half a second: 7.34 world units. The unconverted constant falls
        // 10.02 in the same time.
        assert_eq!(after_half_a_second.to_bits(), 31_540_730_797);
        // A full second: 19.83 world units, against 23.43 unconverted.
        assert_eq!(fallen.to_bits(), 85_172_332_062);
    }

    /// Above terminal velocity the drag term reverses and slows the body down,
    /// rather than adding to a fall that is already too fast.
    #[test]
    fn gravity_decelerates_a_body_already_falling_faster_than_terminal() {
        let mut velocity = FVec2::new(Fix::ZERO, Fix::lit("-100"));
        add_gravity(&mut velocity);
        assert!(velocity.y > Fix::lit("-100"), "should have slowed");
        assert!(velocity.y < Fix::ZERO, "should still be falling");
    }

    // --- Landing -----------------------------------------------------------

    /// The whole point of the phase: a body falls, meets a platform, and stays
    /// on it.
    #[test]
    fn a_body_dropped_onto_a_platform_lands_and_rests_there() {
        let (mut entities, ids) = world(vec![
            platform("0", "0", "0", shape("8", "1")),
            body("0", "10"),
        ]);
        let (ground, falling) = (ids[0], ids[1]);

        let mut landed_on = None;
        for tick in 1..=200u32 {
            step(&mut entities);
            if entities[falling].grounded.is_some() && landed_on.is_none() {
                landed_on = Some(tick);
            }
        }

        // The surface is at y = 1.5 and the drop starts at y = 10, so the body
        // centre falls 7.74 world units before its underside touches. The
        // pinned fall curve puts that just past half a second.
        assert_eq!(landed_on, Some(63));

        let resting = entities[falling];
        let grounded = resting.grounded.expect("still on the ground at tick 200");
        assert_eq!(grounded.platform, ground);
        assert_eq!(resting.self_imposed_velocity, FVec2::ZERO);
        assert_eq!(resting.external_velocity, FVec2::ZERO);

        // Resting on the surface, its centre one radius above it.
        assert!(near(
            resting.position,
            FVec2::new(Fix::ZERO, Fix::lit("1.5") + RADIUS)
        ));
        assert!(grounded.local_pos > Fix::ZERO);
        assert!(grounded.local_pos < entities[ground].platform().unwrap().top_face_end());
    }

    /// A landing is stored as a scalar along the surface, and the world
    /// position is derived from it. Two bodies landing at different places on
    /// the same platform must get different surface positions.
    #[test]
    fn where_a_body_lands_is_stored_as_a_position_along_the_surface() {
        let (mut entities, ids) = world(vec![
            platform("0", "0", "0", shape("8", "1")),
            body("-6", "4"),
            body("6", "4"),
        ]);

        for _ in 0..120 {
            step(&mut entities);
        }

        let left = entities[ids[1]].grounded.expect("left body landed");
        let right = entities[ids[2]].grounded.expect("right body landed");
        assert!(
            left.local_pos < right.local_pos,
            "surface positions run left to right along the top face"
        );
        assert!((entities[ids[1]].position.x - Fix::lit("-6")).abs() < Fix::lit("0.00001"));
        assert!((entities[ids[2]].position.x - Fix::lit("6")).abs() < Fix::lit("0.00001"));
    }

    #[test]
    fn several_platforms_at_different_places_and_rotations_all_catch_bodies() {
        let (mut entities, ids) = world(vec![
            platform("0", "0", "0", shape("4", "1")),
            platform("-20", "5", "0.4", shape("5", "0.75")),
            platform("20", "-8", "-0.7", shape("6", "2")),
            body("0", "12"),
            body("-20", "14"),
            body("20", "6"),
        ]);

        for _ in 0..400 {
            step(&mut entities);
        }

        for (body_index, platform_index) in [(3, 0), (4, 1), (5, 2)] {
            let grounded = entities[ids[body_index]]
                .grounded
                .unwrap_or_else(|| panic!("body {body_index} never landed"));
            assert_eq!(grounded.platform, ids[platform_index]);

            // Resting one radius off the surface it is standing on.
            let surface = entities[ids[platform_index]]
                .platform()
                .unwrap()
                .surface_point(grounded.local_pos)
                .expect("on the top face");
            let gap = entities[ids[body_index]].position.distance(surface);
            assert!(
                (gap - RADIUS).abs() < Fix::lit("0.0001"),
                "body {body_index} rests {gap} from the surface"
            );
        }
    }

    #[test]
    fn a_platform_does_not_fall() {
        let (mut entities, ids) = world(vec![platform("0", "0", "0", shape("4", "1"))]);
        for _ in 0..200 {
            step(&mut entities);
        }
        assert_eq!(entities[ids[0]].position, FVec2::ZERO);
        assert_eq!(entities[ids[0]].self_imposed_velocity, FVec2::ZERO);
    }

    // --- Anti-tunnelling ---------------------------------------------------

    /// A body moving fast enough to cross the whole platform in a single tick
    /// must land on it rather than pass through.
    #[test]
    fn a_fast_falling_body_does_not_tunnel_through_a_platform() {
        // At 3000 units per second a tick covers 25 world units, so this body
        // starts above the platform and would end up well below it.
        let (mut entities, ids) = world(vec![
            platform("0", "0", "0", shape("8", "1")),
            moving("0", "10", "0", "-3000"),
        ]);

        step(&mut entities);

        let landed = entities[ids[1]];
        assert!(landed.grounded.is_some(), "tunnelled straight through");
        assert_eq!(landed.position.y, Fix::lit("1.5") + RADIUS);
        // And it stopped there rather than keeping its speed.
        assert_eq!(landed.self_imposed_velocity, FVec2::ZERO);
    }

    /// A body crossing the top edge diagonally at speed resolves to a defined
    /// outcome rather than to whichever ray happened to fire first.
    #[test]
    fn a_fast_diagonal_body_at_the_edge_resolves_the_same_way_every_time() {
        let run = |vx: &str| {
            let (mut entities, ids) = world(vec![
                platform("0", "0", "0", shape("8", "1")),
                moving("-9", "6", vx, "-600"),
            ]);
            step(&mut entities);
            entities[ids[1]]
        };

        // Arriving from outside the left edge and moving inwards: it lands, on
        // the flat top, and its stored surface position is on the top face.
        let inwards = run("400");
        let grounded = inwards.grounded.expect("the inward body should land");
        assert_eq!(inwards.position.y, Fix::lit("1.5") + RADIUS);
        assert!(grounded.local_pos >= Fix::ZERO);

        // Arriving from outside the left edge and moving away: nothing to land
        // on, so it keeps falling. The outcome is decided by the geometry, not
        // by which of the three rays was tested first.
        let outwards = run("-400");
        assert!(outwards.grounded.is_none(), "landed on thin air");
    }

    /// The reason there are three rays and not one.
    ///
    /// The body falls just past the end of the top face, so the centre ray
    /// misses it entirely — its x never enters the flat span. The offset ray on
    /// the inside of the body does cross, because half the body is over the
    /// platform. With a single centre ray this body falls forever; with three
    /// it catches the ledge.
    ///
    /// Verified to fail with the outer two rays disabled, which is the only
    /// thing that makes it a test of why there are three.
    #[test]
    fn a_body_over_a_ledge_is_caught_by_an_offset_ray_and_not_by_the_centre() {
        let ledge = Fix::lit("8");
        let past_the_edge = ledge + RADIUS / Fix::lit("2");
        let (mut entities, ids) = world(vec![
            platform("0", "0", "0", shape("8", "1")),
            Entity {
                position: FVec2::new(past_the_edge, Fix::lit("6")),
                ..body("0", "0")
            },
        ]);

        // The centre ray alone finds nothing: the body's centre is outside the
        // flat span for the whole fall.
        let ground = entities[ids[0]].platform().unwrap();
        let straight_down = FVec2::new(past_the_edge, Fix::lit("1.5"));
        assert_eq!(
            ground.top_face_crossing(straight_down, FVec2::new(Fix::ZERO, Fix::lit("-4"))),
            None,
            "the centre ray must miss, or this test is not testing anything"
        );

        for _ in 0..200 {
            step(&mut entities);
        }

        let caught = entities[ids[1]];
        assert!(caught.grounded.is_some(), "fell past the ledge");
        // Only the flat top is walkable this phase, so a body caught past the
        // end of it is placed at the end. The corner arc it is really touching
        // belongs to the surface-walking phase.
        assert!(near(
            caught.position,
            FVec2::new(ledge, Fix::lit("1.5") + RADIUS)
        ));
    }

    /// A body travelling upwards through a platform passes it, rather than
    /// snapping onto a surface it approached from underneath.
    #[test]
    fn a_body_moving_upwards_does_not_attach_to_a_platform_above_it() {
        let (mut entities, ids) = world(vec![
            platform("0", "0", "0", shape("8", "1")),
            moving("0", "-10", "0", "600"),
        ]);

        // Four ticks carry it from below the platform to above it.
        for _ in 0..4 {
            step(&mut entities);
            assert!(
                entities[ids[1]].grounded.is_none(),
                "attached to a platform it approached from underneath"
            );
        }
        assert!(
            entities[ids[1]].position.y > Fix::lit("1.5"),
            "should be past it by now"
        );
    }

    // --- Leaving the ground ------------------------------------------------

    /// The nudge is what stops the next tick's ground check from re-attaching a
    /// body that has just left. Without it a jump appears to stick to the floor.
    #[test]
    fn ungrounding_nudges_clear_of_the_surface_and_does_not_re_attach() {
        let (mut entities, ids) = world(vec![
            platform("0", "0", "0", shape("8", "1")),
            body("0", "4"),
        ]);
        for _ in 0..200 {
            step(&mut entities);
        }
        assert!(entities[ids[1]].grounded.is_some(), "should have landed");

        let before = entities[ids[1]].position;
        let normal = entities[ids[0]].platform().unwrap().normal_at(before);
        unground(&mut entities[ids[1]], normal);

        let after = entities[ids[1]];
        assert!(after.grounded.is_none());
        assert_eq!(after.self_imposed_velocity, FVec2::ZERO);
        assert_eq!(after.position, before + normal * UNGROUND_NUDGE);

        // The tick straight after leaving must not put it back on the ground.
        step(&mut entities);
        assert!(
            entities[ids[1]].grounded.is_none(),
            "re-attached on the tick after ungrounding"
        );
    }

    // --- A platform that stops existing ------------------------------------

    /// A body can be standing on a platform that is later removed. The
    /// generational key fails the lookup instead of silently addressing
    /// whatever took the slot, and the body falls again. Never unwrapped.
    #[test]
    fn a_body_grounded_on_a_destroyed_platform_falls_instead_of_panicking() {
        let (mut entities, ids) = world(vec![
            platform("0", "0", "0", shape("8", "1")),
            body("0", "4"),
        ]);
        for _ in 0..200 {
            step(&mut entities);
        }
        let stale = entities[ids[1]].grounded.expect("landed").platform;
        assert_eq!(stale, ids[0]);

        entities.remove(ids[0]);
        // Something else takes the freed slot. It is placed well out of the way,
        // so if the stale key were to address it the body would stay grounded on
        // a platform that is nowhere near it.
        let replacement = entities.insert(platform("500", "0", "0", shape("8", "1")));
        assert_ne!(replacement, stale);

        let resting_place = entities[ids[1]].position;
        step(&mut entities);

        assert!(entities[ids[1]].grounded.is_none(), "the platform is gone");
        assert!(
            entities[ids[1]].position.y < resting_place.y,
            "should be falling again"
        );
    }

    /// The same lookup failing for the other reason: a stored surface position
    /// that is no longer on the platform's top face.
    #[test]
    fn a_surface_position_off_the_top_face_ungrounds_rather_than_unwrapping() {
        let (mut entities, ids) = world(vec![
            platform("0", "0", "0", shape("8", "1")),
            body("0", "4"),
        ]);
        for _ in 0..200 {
            step(&mut entities);
        }

        entities[ids[1]].grounded = Some(Grounded {
            platform: ids[0],
            // Somewhere round the side, which this phase cannot resolve.
            local_pos: Fix::lit("0.6"),
        });
        step(&mut entities);

        // The unresolvable position takes the body off the ground rather than
        // being unwrapped. It is directly above the platform, so the same tick
        // then lands it again — at a surface position the platform can actually
        // answer for.
        let recovered = entities[ids[1]].grounded.expect("landed again");
        let ground = entities[ids[0]].platform().unwrap();
        assert_ne!(recovered.local_pos, Fix::lit("0.6"));
        assert!(ground.surface_point(recovered.local_pos).is_some());
    }

    // --- Velocity channels -------------------------------------------------

    /// The two velocity channels both move a body, and stay separate while
    /// doing it. Nothing in this phase writes the external one.
    #[test]
    fn both_velocity_channels_move_a_body_and_remain_distinct() {
        let (mut entities, ids) = world(vec![Entity {
            external_velocity: FVec2::new(Fix::lit("12"), Fix::ZERO),
            ..moving("0", "0", "-12", "0")
        }]);

        step(&mut entities);

        let moved = entities[ids[0]];
        // The two cancel horizontally, so the body only falls.
        assert_eq!(moved.position.x, Fix::ZERO);
        assert!(moved.position.y < Fix::ZERO);
        // Gravity wrote the body's own channel and left the other alone.
        assert_eq!(
            moved.external_velocity,
            FVec2::new(Fix::lit("12"), Fix::ZERO)
        );
        assert_eq!(moved.self_imposed_velocity.x, Fix::lit("-12"));
        assert!(moved.self_imposed_velocity.y < Fix::ZERO);
    }
}
