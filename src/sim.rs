//! The simulation and the loop that drives it.
//!
//! [`Sim::step`] is the only way the world ever changes. It takes the inputs
//! for one tick as an argument and reads nothing else — no clock, no input
//! device, no global. That is what makes the netcode possible: a peer that
//! receives the inputs for tick T can reproduce tick T exactly, and a peer that
//! disagrees about any of it has a bug that shows up as a checksum mismatch
//! rather than as a slow drift nobody notices until the match is unplayable.
//!
//! There is no gameplay here yet. What is here is the shape everything else
//! will be poured into, and the ordering guarantees that would be very
//! expensive to retrofit.

use slotmap::SlotMap;

use crate::constants::{MAX_PLAYERS, TICKS_PER_SECOND};
use crate::ids::{EntityId, PlayerId, Tick};
use crate::input::ActionState;
use crate::math::{FVec2, Fix};
use crate::platform::{Platform, PlatformShape};
use crate::player_physics;

// --- Random numbers --------------------------------------------------------

/// PCG-XSH-RR 64/32: sixty-four bits of state, thirty-two bits out.
///
/// Written out rather than pulled from a crate on purpose. A general-purpose
/// random number crate makes no promise that its algorithm produces the same
/// bits in the next minor version, and a silent change to that would be a
/// desync between two builds of this game that differ only in a lockfile. Ten
/// lines of arithmetic that will never change are worth more here than a
/// dependency, and the output is pinned by a test either way.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pcg32 {
    state: u64,
    increment: u64,
}

const PCG_MULTIPLIER: u64 = 6_364_136_223_846_793_005;

impl Pcg32 {
    /// Seeds a stream. `stream` selects one of 2^63 distinct sequences, so two
    /// generators seeded identically but on different streams do not correlate.
    pub const fn new(seed: u64, stream: u64) -> Self {
        let mut rng = Self {
            state: 0,
            // Must be odd; this is the standard construction.
            increment: (stream << 1) | 1,
        };
        rng.state = rng
            .state
            .wrapping_mul(PCG_MULTIPLIER)
            .wrapping_add(rng.increment);
        rng.state = rng.state.wrapping_add(seed);
        rng.state = rng
            .state
            .wrapping_mul(PCG_MULTIPLIER)
            .wrapping_add(rng.increment);
        rng
    }

    pub fn next_u32(&mut self) -> u32 {
        let previous = self.state;
        self.state = previous
            .wrapping_mul(PCG_MULTIPLIER)
            .wrapping_add(self.increment);

        let xorshifted = (((previous >> 18) ^ previous) >> 27) as u32;
        let rotation = (previous >> 59) as u32;
        xorshifted.rotate_right(rotation)
    }

    pub fn next_u64(&mut self) -> u64 {
        let high = self.next_u64_half();
        (high << 32) | self.next_u64_half()
    }

    fn next_u64_half(&mut self) -> u64 {
        u64::from(self.next_u32())
    }

    /// A value in `0..bound`, without modulo bias.
    ///
    /// The rejection loop matters: a plain modulo would make low values very
    /// slightly more likely, and "very slightly" is enough to be visible across
    /// a whole match's worth of spawns and item rolls.
    pub fn below(&mut self, bound: u32) -> u32 {
        assert!(bound > 0, "an empty range has no values to pick from");
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let candidate = self.next_u32();
            if candidate >= threshold {
                return candidate % bound;
            }
        }
    }

    /// The generator's own state, for the simulation checksum. Two peers whose
    /// worlds match but whose generators do not are one lucky tick away from
    /// diverging, so this is part of the state that gets compared.
    const fn checksum_words(self) -> [u64; 2] {
        [self.state, self.increment]
    }
}

// --- Entities --------------------------------------------------------------

/// A thing in the world.
///
/// One type for every kind of entity, bodies and platforms alike. Nesting a
/// separate body struct inside this one would buy nothing while the
/// relationship is one to one, and splitting it out is a cheap refactor on the
/// day a second kind of entity needs different fields.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entity {
    pub position: FVec2,
    /// The body's own movement: walking, and jumps.
    ///
    /// Kept separate from [`Entity::external_velocity`] permanently. The two
    /// decay differently, and an ability that needs to know whether a body
    /// jumped or was thrown cannot recover that from a single summed vector.
    pub self_imposed_velocity: FVec2,
    /// Knockback and explosions. Written by the collision phase; zero until
    /// then, and deliberately not merged into the channel above.
    pub external_velocity: FVec2,
    pub rotation: Fix,
    pub scale: Fix,
    /// The player this belongs to, if any. Scenery and projectiles owned by
    /// nobody are normal, which is why this is an `Option` rather than a
    /// reserved id value.
    pub owner: Option<PlayerId>,
    /// Set when this entity is a platform rather than a body.
    ///
    /// Platforms live in the same slotmap as everything else, which hands them
    /// stable generational ids, a deterministic iteration order and destruction
    /// for nothing. Grounded state has to name a platform by [`EntityId`]
    /// anyway.
    pub shape: Option<PlatformShape>,
    /// Set while this body is standing on a platform.
    pub grounded: Option<Grounded>,
}

impl Entity {
    /// The platform this entity is, if it is one.
    ///
    /// Built from the entity's own position and rotation rather than stored, so
    /// there is exactly one source of truth for where a platform sits. Storing
    /// a centre twice is how a moving platform ends up drawn in one place and
    /// collided with in another.
    pub fn platform(&self) -> Option<Platform> {
        self.shape
            .map(|shape| Platform::new(self.position, self.rotation, shape))
    }
}

/// A body standing on a platform.
///
/// The position along the surface is a **scalar fraction of the platform's
/// perimeter**, not a pair of coordinates, and the world position is derived
/// from it every tick. Coordinates work perfectly for a flat top face and then
/// have to be thrown away when bodies start walking round corners, taking every
/// call site with them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Grounded {
    pub platform: EntityId,
    /// Distance around the platform's perimeter, in `0..1`. See
    /// [`Platform::top_face_end`] for which part of that range is the top face.
    pub local_pos: Fix,
}

/// A request to put something in the world, honoured at the start of the next
/// tick.
///
/// Deferred because inserting into the world while iterating it makes the
/// result depend on where the iterator had got to, which is precisely the kind
/// of order dependence the netcode cannot survive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Spawn {
    pub position: FVec2,
    pub self_imposed_velocity: FVec2,
    pub rotation: Fix,
    pub scale: Fix,
    pub owner: Option<PlayerId>,
    /// Present when what is being spawned is a platform. Its centre and
    /// rotation come from `position` and `rotation` above, so there is only
    /// ever one place either is written.
    pub platform: Option<PlatformShape>,
}

/// Every field of a [`Spawn`], as raw bits, in a fixed order.
///
/// A named alias rather than an inline tuple only because it is long; the
/// meaning is exactly what it looks like.
type SpawnKey = (
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    Option<(i64, i64, i64, u8)>,
    Option<PlayerId>,
);

impl Spawn {
    /// A motionless, unrotated, unowned body at the origin.
    ///
    /// Exists so that `Spawn { position, ..Spawn::BODY }` stays readable as the
    /// struct grows, and so that a new field gets a considered default in one
    /// place instead of at every call site.
    pub const BODY: Self = Self {
        position: FVec2::ZERO,
        self_imposed_velocity: FVec2::ZERO,
        rotation: Fix::ZERO,
        scale: Fix::ONE,
        owner: None,
        platform: None,
    };

    /// The key insertion is ordered by: every field, in a fixed order,
    /// compared as raw bits.
    ///
    /// Derived from *what is being spawned*, never from when the request
    /// arrived or from a counter. That distinction is the whole point: a
    /// counter would make the resulting entity ids depend on request order, so
    /// two peers that generated the same set of spawns in a different order
    /// would build different worlds — and sorting afterwards would not save
    /// them, because the ids were already wrong before the sort ran.
    ///
    /// This is a total order over the fields rather than a hash of them, and
    /// the difference matters. A hash is not injective: two spawns that differ
    /// somewhere can still land on the same 64-bit value, and the sort would
    /// then leave them in whatever order they arrived in — which is the very
    /// dependence the key exists to remove, reintroduced in the one case
    /// nobody would think to test. Comparing the fields themselves cannot
    /// collide, so two keys are equal only when every field is equal, and such
    /// spawns really are interchangeable.
    ///
    /// `Option` orders `None` before `Some`, so an unowned spawn and one owned
    /// by player zero are distinct here rather than needing a sentinel.
    /// Every deterministic field appears here. A field left out would make two
    /// spawns that differ only in it compare equal, and the sort would then
    /// treat them as interchangeable when they are not.
    fn order_key(&self) -> SpawnKey {
        (
            self.position.x.to_bits(),
            self.position.y.to_bits(),
            self.self_imposed_velocity.x.to_bits(),
            self.self_imposed_velocity.y.to_bits(),
            self.rotation.to_bits(),
            self.scale.to_bits(),
            self.platform.map(|shape| {
                (
                    shape.extents.x.to_bits(),
                    shape.extents.y.to_bits(),
                    shape.radius.to_bits(),
                    shape.kind.tag(),
                )
            }),
            self.owner,
        )
    }
}

// --- The simulation --------------------------------------------------------

/// The whole simulated world, and nothing else.
///
/// Every field here is part of the state two peers must agree on. Nothing
/// render-side, nothing timing-related and nothing derived from the local
/// machine belongs in this struct.
#[derive(Clone, Debug)]
pub struct Sim {
    tick: Tick,
    rng: Pcg32,
    /// Ticks of hit-stop remaining. Counted in ticks, never in milliseconds:
    /// wall-clock time is not something peers agree on.
    hitstop: u32,
    entities: SlotMap<EntityId, Entity>,
    /// Spawn requests waiting for the next tick.
    pending: Vec<Spawn>,
    /// The inputs this tick was given, sorted by player.
    inputs: Vec<(PlayerId, ActionState)>,
}

impl Sim {
    /// A fresh match. The seed is agreed between peers before the match starts
    /// and never changes during it.
    pub fn new(seed: u64) -> Self {
        Self {
            tick: Tick::ZERO,
            rng: Pcg32::new(seed, 0),
            hitstop: 0,
            entities: SlotMap::with_key(),
            pending: Vec::new(),
            inputs: Vec::with_capacity(MAX_PLAYERS),
        }
    }

    /// Advances the world by exactly one tick.
    ///
    /// The only mutation path there is. Inputs arrive as an argument in any
    /// order and are sorted here, so a caller that iterates a hash map or
    /// collects packets as they land cannot change the outcome.
    ///
    /// The ordering of the steps below is load-bearing. Changing it changes
    /// results, and two peers running different orderings desync.
    pub fn step(&mut self, inputs: &[(PlayerId, ActionState)]) {
        // Inputs are recorded before anything can bail out, so tick T always
        // holds the inputs for tick T. The netcode indexes inputs by tick
        // number; a tick that consumed nothing would put every later tick's
        // inputs one slot out.
        self.inputs.clear();
        self.inputs.extend_from_slice(inputs);
        self.inputs.sort_unstable_by_key(|(player, _)| *player);

        // Hit-stop freezes the world for a few ticks on a heavy hit. The tick
        // counter still advances: it is the shared clock peers agree on, and
        // stopping it would mean the netcode's tick numbers no longer line up
        // with the number of times `step` was called.
        if self.hitstop > 0 {
            self.hitstop -= 1;
            self.tick = self.tick.next();
            return;
        }

        // 1. Players act on their inputs.
        //    Movement is Phase 04; for now inputs are recorded and no more.

        // 2. Insert everything spawned last tick, ordered by their fields.
        self.insert_pending();

        // 3. Remove destroyed entities. Nothing destroys anything yet.

        // 4. Per-entity simulation update. Phase 03 onwards.

        // 5. Physics: gravity, the ground raycasts, and grounding.
        player_physics::step(&mut self.entities);

        // 6. Per-entity late update, after physics has moved everything.

        // 7. Constraint fixup, for anything physics left in an illegal state.

        // 8. The tick is over.
        self.tick = self.tick.next();
    }

    /// Queues something to appear in the world at the start of the next tick.
    ///
    /// Queues only: nothing is inserted and no [`EntityId`] exists until
    /// [`Sim::step`] runs. Calling this does not hand out access to the world,
    /// and the order of calls does not affect the result.
    pub fn request_spawn(&mut self, spawn: Spawn) {
        self.pending.push(spawn);
    }

    fn insert_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }

        // Equal keys now mean spawns identical in every field, so how the sort
        // arranges them among themselves cannot be observed and stability buys
        // nothing.
        self.pending.sort_unstable_by_key(Spawn::order_key);
        for spawn in self.pending.drain(..) {
            self.entities.insert(Entity {
                position: spawn.position,
                self_imposed_velocity: spawn.self_imposed_velocity,
                external_velocity: FVec2::ZERO,
                rotation: spawn.rotation,
                scale: spawn.scale,
                owner: spawn.owner,
                shape: spawn.platform,
                grounded: None,
            });
        }
    }

    /// Puts the world on hold for `ticks` ticks.
    pub fn apply_hitstop(&mut self, ticks: u32) {
        self.hitstop = self.hitstop.max(ticks);
    }

    pub fn tick(&self) -> Tick {
        self.tick
    }

    pub fn hitstop(&self) -> u32 {
        self.hitstop
    }

    pub fn entity(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(id)
    }

    /// Every entity, in the order the simulation itself walks them.
    ///
    /// That order is a function of the insert and remove history alone, which
    /// is identical on every peer, so it is safe for gameplay to depend on.
    pub fn entities(&self) -> impl Iterator<Item = (EntityId, &Entity)> {
        self.entities.iter()
    }

    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    /// The inputs the current tick was given, sorted by player.
    pub fn inputs(&self) -> &[(PlayerId, ActionState)] {
        &self.inputs
    }

    /// A single number standing for the entire simulation state.
    ///
    /// Two peers exchanging this every tick find a desync on the tick it
    /// happens rather than a minute later, when the cause is unrecoverable.
    /// This is the seed of the determinism harness; it hashes raw bits, because
    /// the claim being tested is bit-for-bit equality and nothing weaker.
    pub fn state_hash(&self) -> u64 {
        let mut words = vec![self.tick.get(), u64::from(self.hitstop)];
        words.extend_from_slice(&self.rng.checksum_words());

        for (id, entity) in self.entities.iter() {
            words.push(slotmap::Key::data(&id).as_ffi());
            entity_words(entity, &mut words);
        }

        for (player, action) in &self.inputs {
            words.push(u64::from(player.raw()));
            words.push(u64::from(action.stick_angle));
            words.push(u64::from(action.stick_magnitude));
            words.push(u64::from(
                u8::from(action.jump)
                    | u8::from(action.ab1) << 1
                    | u8::from(action.ab2) << 2
                    | u8::from(action.ab3) << 3
                    | u8::from(action.start) << 4
                    | u8::from(action.select) << 5,
            ));
        }

        fnv1a(&words)
    }
}

/// Every deterministic field of an entity, as raw bits.
///
/// Split out from [`Sim::state_hash`] so a test can assert, field by field,
/// that changing one changes the words. A field that is part of simulation
/// state but missing from here is a desync nobody can see until two machines
/// have already disagreed for a minute.
///
/// Optional fields contribute a fixed number of words whether they are present
/// or not, so the word stream of an entity carrying a platform can never
/// coincide with that of one that does not.
fn entity_words(entity: &Entity, words: &mut Vec<u64>) {
    let bits = |v: Fix| v.to_bits() as u64;

    words.extend_from_slice(&[
        bits(entity.position.x),
        bits(entity.position.y),
        bits(entity.self_imposed_velocity.x),
        bits(entity.self_imposed_velocity.y),
        bits(entity.external_velocity.x),
        bits(entity.external_velocity.y),
        bits(entity.rotation),
        bits(entity.scale),
        match entity.owner {
            None => u64::MAX,
            Some(player) => u64::from(player.raw()),
        },
    ]);

    words.extend_from_slice(&match entity.shape {
        None => [0; 5],
        Some(shape) => [
            1,
            bits(shape.extents.x),
            bits(shape.extents.y),
            bits(shape.radius),
            u64::from(shape.kind.tag()),
        ],
    });

    words.extend_from_slice(&match entity.grounded {
        None => [0; 3],
        Some(grounded) => [
            1,
            slotmap::Key::data(&grounded.platform).as_ffi(),
            bits(grounded.local_pos),
        ],
    });
}

/// FNV-1a over 64-bit words.
///
/// Not a cryptographic hash and does not need to be: it is comparing two
/// machines that are supposed to be running identical code, not defending
/// against one crafting a collision. Hand-written so its output cannot change
/// underneath a golden checksum.
fn fnv1a(words: &[u64]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = OFFSET;
    for word in words {
        for byte in word.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    hash
}

// --- Driving the simulation ------------------------------------------------

const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// The most ticks one frame may run.
///
/// Without a ceiling a machine that stalls asks for the ticks it missed, takes
/// longer than a frame to simulate them, and asks for even more next time: it
/// never catches up and the game locks solid. With a ceiling it runs briefly in
/// slow motion instead, which is survivable and self-correcting.
///
/// Eight ticks is 66 ms of catch-up per frame, so anything rendering above
/// about 15 FPS keeps real time.
pub const MAX_TICKS_PER_FRAME: u32 = 8;

/// Turns elapsed wall-clock time into a number of ticks to run.
///
/// This is the only part of the project that knows what a second is, and it is
/// not simulation state — it lives on the machine, not in the match.
///
/// It answers "how much time has passed", which under lockstep is a different
/// question from "may the simulation advance": there the answer is "when the
/// inputs for the next tick have arrived", which may be sooner or later than
/// the clock says. Keeping the two apart is what lets the netcode supply its
/// own answer without reshaping the loop around it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Accumulator {
    /// Time owed to the simulation, in units of one billionth of a tick.
    ///
    /// Scaled rather than stored in nanoseconds because one tick is 8333333.33
    /// nanoseconds, and rounding that would run the game a hair fast forever.
    /// In these units a tick is exactly `NANOS_PER_SECOND`, so nothing rounds.
    owed: u64,
}

impl Accumulator {
    pub const fn new() -> Self {
        Self { owed: 0 }
    }

    /// Ticks to run for a frame that took `frame_nanos`.
    ///
    /// Returns zero when a frame was shorter than a tick, which is normal above
    /// 120 FPS, and several when it was longer, which is normal below. It is
    /// never "one per frame": that would tie the simulation to the display and
    /// make the game run at different speeds on different monitors.
    pub fn ticks_due(&mut self, frame_nanos: u64) -> u32 {
        // Saturating rather than wrapping: a clock that jumps backwards or
        // reports something absurd should cost one clamped frame, not wrap the
        // accumulator around to a tiny value and stall the game.
        self.owed = self
            .owed
            .saturating_add(frame_nanos.saturating_mul(u64::from(TICKS_PER_SECOND)));

        let due = self.owed / NANOS_PER_SECOND;
        self.owed %= NANOS_PER_SECOND;

        if due > u64::from(MAX_TICKS_PER_FRAME) {
            // Drop the surplus outright. Remembering it would just move the
            // spiral one frame later.
            MAX_TICKS_PER_FRAME
        } else {
            due as u32
        }
    }

    /// How far the next tick already is, in 0..1.
    ///
    /// Rendering between two ticks needs this to place things at the moment the
    /// frame is actually being shown rather than at the last tick boundary. A
    /// float because it never reaches the simulation — it exists purely to draw
    /// with, and the drawing code is allowed floats.
    ///
    /// Computed and exposed but not yet used: interpolation belongs to the
    /// camera and rendering work, and doing it here would be guessing at what
    /// that needs.
    pub fn alpha(&self) -> f32 {
        self.owed as f32 / NANOS_PER_SECOND as f32
    }
}

/// One tick's worth of seconds, for scaling a per-second rate.
///
/// Never for measuring elapsed time — see the note on
/// `constants::FIXED_DELTA_TIME`.
pub const fn tick_seconds() -> Fix {
    crate::constants::FIXED_DELTA_TIME
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::ActionState;
    use crate::math::FVec2;
    use crate::platform::{PlatformKind, PlatformShape};

    fn spawn_at(x: &str, y: &str) -> Spawn {
        Spawn {
            position: FVec2::new(Fix::lit(x), Fix::lit(y)),
            ..Spawn::BODY
        }
    }

    fn recorded_inputs(tick: u64) -> Vec<(PlayerId, ActionState)> {
        (1..=4u8)
            .map(|n| {
                let action = ActionState {
                    jump: (tick + u64::from(n)).is_multiple_of(7),
                    ab1: tick.is_multiple_of(3),
                    ..ActionState::NEUTRAL
                }
                .with_stick(FVec2::new(
                    Fix::from_num(((tick + u64::from(n)) % 5) as i32) - Fix::lit("2"),
                    Fix::from_num((tick % 3) as i32) - Fix::lit("1"),
                ));
                (PlayerId::new(n), action)
            })
            .collect()
    }

    #[test]
    fn six_hundred_steps_land_on_tick_six_hundred() {
        let mut sim = Sim::new(0xfeed);
        for tick in 0..600 {
            sim.step(&recorded_inputs(tick));
        }
        assert_eq!(sim.tick(), Tick::new(600));
    }

    /// The property the entire netcode rests on.
    #[test]
    fn the_same_seed_and_inputs_produce_the_same_state() {
        let mut a = Sim::new(0x5eed);
        let mut b = Sim::new(0x5eed);

        for tick in 0..600 {
            let inputs = recorded_inputs(tick);
            a.step(&inputs);
            b.step(&inputs);
            assert_eq!(a.state_hash(), b.state_hash(), "diverged on tick {tick}");
        }
    }

    #[test]
    fn a_different_seed_produces_a_different_state() {
        let mut a = Sim::new(1);
        let mut b = Sim::new(2);
        a.step(&[]);
        b.step(&[]);
        assert_ne!(a.state_hash(), b.state_hash());
    }

    /// Inputs must reach the simulation, or the equality test above would pass
    /// on a simulation that ignores them entirely.
    #[test]
    fn different_inputs_produce_a_different_state() {
        let mut a = Sim::new(7);
        let mut b = Sim::new(7);

        a.step(&[(PlayerId::new(1), ActionState::NEUTRAL)]);
        b.step(&[(
            PlayerId::new(1),
            ActionState::NEUTRAL.with_stick(FVec2::RIGHT),
        )]);

        assert_ne!(a.state_hash(), b.state_hash());
    }

    /// The caller may hand inputs over in whatever order they arrived in.
    #[test]
    fn input_order_does_not_affect_the_result() {
        let inputs = recorded_inputs(3);
        let mut reversed = inputs.clone();
        reversed.reverse();

        let mut a = Sim::new(11);
        let mut b = Sim::new(11);
        a.step(&inputs);
        b.step(&reversed);

        assert_eq!(a.state_hash(), b.state_hash());
        assert_eq!(a.inputs(), b.inputs());
    }

    /// Entity ids must come out of the content of the spawns, not out of the
    /// order the requests happened to be made in. Sorting at insertion time is
    /// not enough on its own — this is what proves the ordering key itself does
    /// not carry arrival order into the result.
    #[test]
    fn shuffled_spawn_requests_build_an_identical_world() {
        let spawns = [
            spawn_at("1", "2"),
            spawn_at("-3.5", "0"),
            spawn_at("0", "0"),
            spawn_at("97.25", "-11.5"),
            Spawn {
                owner: Some(PlayerId::new(0)),
                ..Spawn::BODY
            },
        ];

        let mut forwards = Sim::new(42);
        for spawn in spawns {
            forwards.request_spawn(spawn);
        }
        forwards.step(&[]);

        let mut backwards = Sim::new(42);
        for spawn in spawns.iter().rev() {
            backwards.request_spawn(*spawn);
        }
        backwards.step(&[]);

        assert_eq!(forwards.entity_count(), spawns.len());
        assert_eq!(forwards.state_hash(), backwards.state_hash());

        // Not just the same set: the same entity under the same id.
        let forwards_ids: Vec<_> = forwards.entities().map(|(id, e)| (id, *e)).collect();
        let backwards_ids: Vec<_> = backwards.entities().map(|(id, e)| (id, *e)).collect();
        assert_eq!(forwards_ids, backwards_ids);
    }

    /// Two spawns share an ordering key only when they are equal in every
    /// field. A hash could not promise this, and a collision between distinct
    /// spawns would silently hand the tie back to arrival order.
    ///
    /// There is one variant per field, and each differs from the base by a
    /// single raw unit wherever it can. A field left out of the key shows up
    /// here as two variants comparing equal.
    #[test]
    fn only_identical_spawns_share_an_ordering_key() {
        let base = Spawn::BODY;
        let a_shape = PlatformShape {
            extents: FVec2::ONE,
            radius: Fix::ONE,
            kind: PlatformKind::Normal,
        };
        let variants = [
            base,
            Spawn {
                owner: Some(PlayerId::new(0)),
                ..base
            },
            Spawn {
                owner: Some(PlayerId::new(1)),
                ..base
            },
            Spawn {
                position: FVec2::new(Fix::from_bits(1), Fix::ZERO),
                ..base
            },
            Spawn {
                position: FVec2::new(Fix::ZERO, Fix::from_bits(1)),
                ..base
            },
            Spawn {
                position: FVec2::new(Fix::from_bits(-1), Fix::ZERO),
                ..base
            },
            Spawn {
                self_imposed_velocity: FVec2::new(Fix::from_bits(1), Fix::ZERO),
                ..base
            },
            Spawn {
                self_imposed_velocity: FVec2::new(Fix::ZERO, Fix::from_bits(1)),
                ..base
            },
            Spawn {
                rotation: Fix::from_bits(1),
                ..base
            },
            Spawn {
                scale: Fix::ONE + Fix::from_bits(1),
                ..base
            },
            Spawn {
                platform: Some(a_shape),
                ..base
            },
            Spawn {
                platform: Some(PlatformShape {
                    extents: FVec2::new(Fix::ONE + Fix::from_bits(1), Fix::ONE),
                    ..a_shape
                }),
                ..base
            },
            Spawn {
                platform: Some(PlatformShape {
                    extents: FVec2::new(Fix::ONE, Fix::ONE + Fix::from_bits(1)),
                    ..a_shape
                }),
                ..base
            },
            Spawn {
                platform: Some(PlatformShape {
                    radius: Fix::ONE + Fix::from_bits(1),
                    ..a_shape
                }),
                ..base
            },
            Spawn {
                platform: Some(PlatformShape {
                    kind: PlatformKind::Ice,
                    ..a_shape
                }),
                ..base
            },
        ];

        for (i, a) in variants.iter().enumerate() {
            for (j, b) in variants.iter().enumerate() {
                assert_eq!(
                    a.order_key() == b.order_key(),
                    i == j,
                    "variants {i} and {j} disagree"
                );
            }
        }

        // Identical spawns do share one, which is what makes them safe to
        // reorder.
        assert_eq!(base.order_key(), base.order_key());
    }

    /// The ordering must be a total order: consistent, and never dependent on
    /// which of the two happens to be asked first.
    #[test]
    fn the_ordering_is_antisymmetric_and_transitive() {
        let spawns = [
            spawn_at("-1", "5"),
            spawn_at("-1", "-5"),
            spawn_at("0", "0"),
            spawn_at("3.25", "0"),
            Spawn {
                owner: Some(PlayerId::new(2)),
                ..Spawn::BODY
            },
            Spawn {
                self_imposed_velocity: FVec2::new(Fix::lit("-3"), Fix::lit("8")),
                rotation: Fix::lit("0.5"),
                ..Spawn::BODY
            },
            Spawn {
                platform: Some(PlatformShape {
                    extents: FVec2::new(Fix::lit("4"), Fix::lit("1")),
                    radius: Fix::lit("0.5"),
                    kind: PlatformKind::Ice,
                }),
                ..Spawn::BODY
            },
        ];

        for a in &spawns {
            for b in &spawns {
                assert_eq!(
                    a.order_key().cmp(&b.order_key()),
                    b.order_key().cmp(&a.order_key()).reverse()
                );
                for c in &spawns {
                    if a.order_key() <= b.order_key() && b.order_key() <= c.order_key() {
                        assert!(a.order_key() <= c.order_key());
                    }
                }
            }
        }
    }

    /// Every deterministic field of an entity must reach the checksum. A field
    /// that is part of simulation state but missing from the hash is a desync
    /// two peers cannot see: their worlds differ and their checksums agree.
    ///
    /// One variant per field, each differing from the base by the smallest
    /// change that field can express. A field left out of `entity_words` shows
    /// up as two variants producing identical words.
    #[test]
    fn every_entity_field_reaches_the_checksum() {
        let base = Entity {
            position: FVec2::ZERO,
            self_imposed_velocity: FVec2::ZERO,
            external_velocity: FVec2::ZERO,
            rotation: Fix::ZERO,
            scale: Fix::ONE,
            owner: None,
            shape: None,
            grounded: None,
        };
        let a_shape = PlatformShape {
            extents: FVec2::ONE,
            radius: Fix::ONE,
            kind: PlatformKind::Normal,
        };
        let a_key = {
            let mut map: SlotMap<EntityId, Entity> = SlotMap::with_key();
            map.insert(base)
        };
        let one = Fix::from_bits(1);

        let variants = [
            base,
            Entity {
                position: FVec2::new(one, Fix::ZERO),
                ..base
            },
            Entity {
                position: FVec2::new(Fix::ZERO, one),
                ..base
            },
            Entity {
                self_imposed_velocity: FVec2::new(one, Fix::ZERO),
                ..base
            },
            Entity {
                self_imposed_velocity: FVec2::new(Fix::ZERO, one),
                ..base
            },
            Entity {
                external_velocity: FVec2::new(one, Fix::ZERO),
                ..base
            },
            Entity {
                external_velocity: FVec2::new(Fix::ZERO, one),
                ..base
            },
            Entity {
                rotation: one,
                ..base
            },
            Entity {
                scale: Fix::ONE + one,
                ..base
            },
            Entity {
                owner: Some(PlayerId::new(0)),
                ..base
            },
            Entity {
                owner: Some(PlayerId::new(1)),
                ..base
            },
            Entity {
                shape: Some(a_shape),
                ..base
            },
            Entity {
                shape: Some(PlatformShape {
                    extents: FVec2::new(Fix::ONE + one, Fix::ONE),
                    ..a_shape
                }),
                ..base
            },
            Entity {
                shape: Some(PlatformShape {
                    extents: FVec2::new(Fix::ONE, Fix::ONE + one),
                    ..a_shape
                }),
                ..base
            },
            Entity {
                shape: Some(PlatformShape {
                    radius: Fix::ONE + one,
                    ..a_shape
                }),
                ..base
            },
            Entity {
                shape: Some(PlatformShape {
                    kind: PlatformKind::Ice,
                    ..a_shape
                }),
                ..base
            },
            Entity {
                grounded: Some(Grounded {
                    platform: a_key,
                    local_pos: Fix::ZERO,
                }),
                ..base
            },
            Entity {
                grounded: Some(Grounded {
                    platform: a_key,
                    local_pos: one,
                }),
                ..base
            },
        ];

        let words = |entity: &Entity| {
            let mut out = Vec::new();
            entity_words(entity, &mut out);
            out
        };

        for (i, a) in variants.iter().enumerate() {
            for (j, b) in variants.iter().enumerate() {
                assert_eq!(
                    words(a) == words(b),
                    i == j,
                    "variants {i} and {j} hash the same"
                );
            }
        }
    }

    /// An entity carrying a platform contributes the same number of words as
    /// one that does not, so no arrangement of entities can make two different
    /// worlds produce the same stream of words.
    #[test]
    fn an_entity_contributes_a_fixed_number_of_words() {
        let mut plain = Vec::new();
        entity_words(
            &Entity {
                position: FVec2::ZERO,
                self_imposed_velocity: FVec2::ZERO,
                external_velocity: FVec2::ZERO,
                rotation: Fix::ZERO,
                scale: Fix::ONE,
                owner: None,
                shape: None,
                grounded: None,
            },
            &mut plain,
        );

        let mut world = Sim::new(1);
        world.request_spawn(Spawn {
            platform: Some(PlatformShape {
                extents: FVec2::ONE,
                radius: Fix::ONE,
                kind: PlatformKind::Ice,
            }),
            ..Spawn::BODY
        });
        world.step(&[]);

        let mut furnished = Vec::new();
        let (_, entity) = world.entities().next().expect("the platform");
        entity_words(entity, &mut furnished);

        assert_eq!(plain.len(), furnished.len());
    }

    /// The checksum has to actually move when the world does. Gravity alone is
    /// enough to prove it: an entity that fell is not where it was.
    #[test]
    fn the_checksum_follows_a_falling_body() {
        let mut sim = Sim::new(5);
        sim.request_spawn(spawn_at("0", "0"));
        sim.step(&[]);

        let resting = sim.state_hash();
        sim.step(&[]);
        assert_ne!(sim.state_hash(), resting, "the body should have fallen");
    }

    #[test]
    fn spawns_appear_on_the_following_tick_not_immediately() {
        let mut sim = Sim::new(1);
        sim.request_spawn(spawn_at("1", "1"));
        assert_eq!(sim.entity_count(), 0, "queued, not inserted");

        sim.step(&[]);
        assert_eq!(sim.entity_count(), 1);

        sim.step(&[]);
        assert_eq!(sim.entity_count(), 1, "inserted once, not once per tick");
    }

    /// Hit-stop must freeze the world without stalling the clock the netcode
    /// counts on.
    #[test]
    fn hitstop_freezes_the_world_but_not_the_tick_counter() {
        let mut sim = Sim::new(3);
        sim.request_spawn(spawn_at("0", "0"));
        sim.apply_hitstop(3);

        for expected in 1..=3u64 {
            sim.step(&[]);
            assert_eq!(sim.tick(), Tick::new(expected));
            assert_eq!(sim.entity_count(), 0, "the world is on hold");
        }

        assert_eq!(sim.hitstop(), 0);
        sim.step(&[]);
        assert_eq!(sim.tick(), Tick::new(4));
        assert_eq!(sim.entity_count(), 1, "the spawn survived the freeze");
    }

    /// Hit-stop freezes physics too, and still costs exactly one tick. A tick
    /// that advanced the world without advancing the counter, or the reverse,
    /// would slide every later tick's inputs one slot over once the netcode
    /// indexes them by tick number.
    #[test]
    fn hitstop_holds_a_falling_body_still_without_skipping_a_tick() {
        let mut sim = Sim::new(3);
        sim.request_spawn(spawn_at("0", "10"));
        sim.step(&[]);

        let (id, _) = sim.entities().next().expect("the body");
        let dropped_from = sim.entity(id).expect("still there").position;

        sim.apply_hitstop(4);
        for expected in 2..=5u64 {
            sim.step(&[]);
            assert_eq!(sim.tick(), Tick::new(expected));
            assert_eq!(
                sim.entity(id).expect("still there").position,
                dropped_from,
                "the world is on hold"
            );
        }

        sim.step(&[]);
        assert!(sim.entity(id).expect("still there").position.y < dropped_from.y);
    }

    /// The phase, through the public API: a body falls, lands, and stays.
    #[test]
    fn a_body_falls_onto_a_platform_and_stays_there() {
        let mut sim = Sim::new(0xf00d);
        sim.request_spawn(Spawn {
            position: FVec2::new(Fix::ZERO, Fix::lit("-10")),
            platform: Some(PlatformShape {
                extents: FVec2::new(Fix::lit("10"), Fix::lit("1")),
                radius: Fix::lit("0.5"),
                kind: PlatformKind::Normal,
            }),
            ..Spawn::BODY
        });
        sim.request_spawn(Spawn {
            position: FVec2::new(Fix::lit("2"), Fix::lit("10")),
            owner: Some(PlayerId::new(1)),
            ..Spawn::BODY
        });

        for _ in 0..300 {
            sim.step(&[]);
        }

        let body = sim
            .entities()
            .find(|(_, e)| e.owner == Some(PlayerId::new(1)))
            .map(|(_, e)| *e)
            .expect("the body");

        let grounded = body.grounded.expect("should have landed");
        assert_eq!(body.self_imposed_velocity, FVec2::ZERO);
        assert_eq!(
            body.external_velocity,
            FVec2::ZERO,
            "nothing writes this yet"
        );

        // Resting on the top face, one radius above it.
        let surface = Fix::lit("-10") + Fix::lit("1") + Fix::lit("0.5");
        let expected = surface + crate::constants::RADIUS;
        assert!((body.position.y - expected).abs() < Fix::lit("0.00001"));
        assert!(grounded.local_pos > Fix::ZERO && grounded.local_pos < Fix::ONE);

        // And it is still there a hundred ticks later.
        let settled = body.position;
        for _ in 0..100 {
            sim.step(&[]);
        }
        let still = sim
            .entities()
            .find(|(_, e)| e.owner == Some(PlayerId::new(1)))
            .map(|(_, e)| *e)
            .expect("the body");
        assert_eq!(still.position, settled);
    }

    /// Two overlapping hits should not stack into a much longer freeze.
    #[test]
    fn hitstop_takes_the_longer_of_two_hits() {
        let mut sim = Sim::new(3);
        sim.apply_hitstop(5);
        sim.apply_hitstop(2);
        assert_eq!(sim.hitstop(), 5);
    }

    // --- The generator -----------------------------------------------------

    /// The reference output of PCG-XSH-RR 64/32. If these bits ever move, every
    /// replay and every golden checksum in the project has silently changed
    /// meaning.
    #[test]
    fn the_generator_produces_its_documented_bits() {
        let mut rng = Pcg32::new(42, 54);
        let got: Vec<u32> = (0..6).map(|_| rng.next_u32()).collect();
        assert_eq!(
            got,
            vec![
                0xa1_5c_02_b7,
                0x7b_47_f4_09,
                0xba_1d_33_30,
                0x83_d2_f2_93,
                0xbf_a4_78_4b,
                0xcb_ed_60_6e
            ]
        );
    }

    #[test]
    fn the_generator_is_reproducible_and_stream_separated() {
        let mut a = Pcg32::new(9, 0);
        let mut b = Pcg32::new(9, 0);
        let mut other_stream = Pcg32::new(9, 1);

        for _ in 0..64 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
        assert_ne!(Pcg32::new(9, 0).next_u32(), other_stream.next_u32());
    }

    #[test]
    fn bounded_values_stay_in_range_and_cover_it() {
        let mut rng = Pcg32::new(0xabc, 0);
        let mut seen = [false; 6];
        for _ in 0..10_000 {
            let value = rng.below(6);
            assert!(value < 6);
            seen[value as usize] = true;
        }
        assert!(seen.iter().all(|&hit| hit), "some values never came up");
        assert_eq!(rng.below(1), 0);
    }

    #[test]
    fn sixty_four_bit_draws_use_the_whole_width() {
        let mut rng = Pcg32::new(1, 1);
        let values: Vec<u64> = (0..8).map(|_| rng.next_u64()).collect();
        assert!(values.iter().any(|v| v >> 32 != 0), "high half never set");
        assert!(values.iter().any(|v| *v as u32 != 0), "low half never set");
    }

    // --- The accumulator ---------------------------------------------------

    /// One tick is 8333333.33 nanoseconds. Rounding that to a whole number of
    /// nanoseconds would drift, so the accumulator must land exactly 120 ticks
    /// in exactly one second, however that second is chopped up.
    #[test]
    fn a_second_is_exactly_a_hundred_and_twenty_ticks() {
        // Frame rates that divide a second exactly, so the test is measuring
        // the accumulator rather than the rounding in its own arithmetic.
        for frame_nanos in [
            NANOS_PER_SECOND / 1000,
            NANOS_PER_SECOND / 250,
            NANOS_PER_SECOND / 125,
            NANOS_PER_SECOND / 40,
        ] {
            let frames = NANOS_PER_SECOND / frame_nanos;
            let mut accumulator = Accumulator::new();
            let ticks: u32 = (0..frames)
                .map(|_| accumulator.ticks_due(frame_nanos))
                .sum();
            assert_eq!(ticks, TICKS_PER_SECOND, "at {frame_nanos} ns per frame");
        }
    }

    /// The same wall-clock second must produce the same number of ticks no
    /// matter what the display is doing. A game that runs faster on a better
    /// monitor is the bug this test exists to catch.
    #[test]
    fn tick_count_does_not_depend_on_frame_rate() {
        let five_seconds = 5 * NANOS_PER_SECOND;
        for fps in [30u64, 60, 75, 120, 144, 240] {
            let frame_nanos = NANOS_PER_SECOND / fps;
            let mut accumulator = Accumulator::new();
            let ticks: u32 = (0..fps * 5)
                .map(|_| accumulator.ticks_due(frame_nanos))
                .sum();

            let expected = (five_seconds * u64::from(TICKS_PER_SECOND)) / NANOS_PER_SECOND;
            // Frame durations that do not divide a second evenly leave a
            // fraction of a tick unpaid at the end; never more than one.
            assert!(
                expected - u64::from(ticks) <= 1,
                "{fps} FPS produced {ticks} ticks, expected about {expected}"
            );
        }
    }

    #[test]
    fn a_fast_frame_can_produce_no_ticks_at_all() {
        let mut accumulator = Accumulator::new();
        // A 1000 FPS frame is an eighth of a tick.
        assert_eq!(accumulator.ticks_due(NANOS_PER_SECOND / 1000), 0);
        assert!(accumulator.alpha() > 0.0 && accumulator.alpha() < 1.0);
    }

    /// The catch-up spiral, which is the failure this loop is most likely to
    /// have. A half-second stall asks for sixty ticks; running sixty would take
    /// longer than the frame it is trying to catch up on.
    #[test]
    fn a_long_stall_is_capped_rather_than_burst() {
        let mut accumulator = Accumulator::new();
        let half_a_second = NANOS_PER_SECOND / 2;

        assert_eq!(accumulator.ticks_due(half_a_second), MAX_TICKS_PER_FRAME);

        // And the surplus is gone, not saved up to be inflicted on the next
        // frame. Otherwise the cap only delays the spiral.
        assert_eq!(accumulator.ticks_due(0), 0);
        // 25 ms is three ticks exactly, so this asserts the debt is zero rather
        // than a fraction left over from the rounding.
        assert_eq!(accumulator.ticks_due(25_000_000), 3);
    }

    /// A clock that reports something absurd should cost one clamped frame, not
    /// wrap the accumulator round to a small value.
    #[test]
    fn an_absurd_frame_duration_does_not_wrap() {
        let mut accumulator = Accumulator::new();
        assert_eq!(accumulator.ticks_due(u64::MAX), MAX_TICKS_PER_FRAME);
        assert_eq!(accumulator.ticks_due(25_000_000), 3);
    }

    #[test]
    fn alpha_reports_progress_towards_the_next_tick() {
        let mut accumulator = Accumulator::new();
        accumulator.ticks_due(NANOS_PER_SECOND / 240);
        // Half a tick in.
        assert!((accumulator.alpha() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn one_tick_of_seconds_is_the_shared_constant() {
        assert_eq!(tick_seconds(), crate::constants::FIXED_DELTA_TIME);
    }
}
