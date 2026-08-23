# Phase 02 — Identity types and the tick loop

**Prerequisite:** Phase 01 (fixed-point math and constants) is complete and on `main`.
**Estimate:** ~1 day.
**Read first:** [`../CLAUDE.md`](../CLAUDE.md) — the invariants there are non-negotiable.

---

## 1. What this phase implements

The two things that are nearly free now and structurally expensive later: **the identity model**
and a **tick-indexed simulation loop** shaped so lockstep netcode can be dropped in at Phase 09
without touching any gameplay code.

There is little visible output from this phase. Do it anyway. The alternative is discovering in
Phase 09 that every update signature in the codebase is wrong.

### `src/ids.rs` — four distinct newtypes

```rust
pub struct PeerId(u16);      // a machine in the session
pub struct PlayerId(u8);     // a logical participant
pub struct DeviceId(u32);    // a physical input device
pub struct EntityId(..);     // slotmap key; a thing in the world
pub struct Tick(u64);        // simulation tick number
```

No `From` impls between them, no `as` casts. They have different lifecycles and the compiler must
enforce that. This relationship must be expressible from day one:

```
Machine 1 --+-- Player 1 -- Device (gamepad)  -- Entity
            +-- Player 2 -- Device (keyboard) -- Entity
Machine 2 --+-- Player 3 -- Device (gamepad)  -- Entity
            +-- Player 4 -- Device (gamepad)  -- Entity
```

**One peer, N players.** A player with no device is valid (unplugged mid-match — the player still
exists and their entity still stands there). A player with no entity is valid (spectating, waiting
to spawn). Model both as `Option`, not as invariants you assert.

### `src/input.rs` — the action set

```rust
pub struct ActionState {
    pub stick_angle: u8,   // quantised; one value reserved as the neutral sentinel
    pub jump: bool,
    pub ab1: bool, pub ab2: bool, pub ab3: bool,
    pub start: bool, pub select: bool,
}
```

That is the whole game's input. It packs to about **2 bytes per player per tick**, and it must stay
that small and stay ability-agnostic — the entire network budget scales with it.

**Quantise the stick to a byte now, not at send time.** Otherwise the local player simulates a
slightly different value than remote peers reconstruct, and Phase 09 gets a desync with no visible
cause. Apply the input deadzone *before* quantisation.

Keep the layering thin: `device -> DeviceId -> ActionState -> PlayerId`.

### `src/sim.rs` — the tick loop

```rust
pub struct Sim { tick: Tick, rng: DeterministicRng, /* world */ }

impl Sim {
    /// The only way the world ever changes.
    pub fn step(&mut self, inputs: &[(PlayerId, ActionState)]) { .. }
}
```

Non-negotiable, because Phase 09 depends on every one of them:

- `step` takes inputs as an argument. It never reads a device, a clock, or a global.
- `step` is the **only** mutation path. No `&mut world` handed out elsewhere.
- Entity insertion is deferred to the start of the next tick and **sorted by a stable id** before
  insertion.
- `rng` is seeded per match and advanced only inside `step`.
- Hit-stop is a tick counter on `Sim`, checked at the top of `step`. Never wall-clock.

**Tick order**, which matters for determinism:

1. Sample inputs, update players
2. Insert pending entities, sorted by a stable comparator, then initialise each
3. Remove destroyed entities
4. Simulation update on all entities, in order
5. Physics step
6. Late simulation update on all entities, in order
7. Constraint fixup
8. Increment the tick counter

### `src/main.rs` — the driver

An accumulator that gathers inputs, runs zero or more `step()` calls, and renders once.

- Drive the rate from `constants::TICKS_PER_SECOND`, **never a literal**.
- One frame may produce **zero, one, or several** ticks. Never one tick per frame.
- **Bound the catch-up.** Cap how many ticks a single frame may run. Without a ceiling, a machine
  that falls behind tries to simulate more each frame and never recovers — a catch-up spiral.
- Keep **"wall-clock says a tick interval elapsed"** and **"the simulation is allowed to advance"**
  as separate concepts. Under lockstep the sim advances when *inputs for that tick are available*,
  which is a different question. Phase 09 supplies the real predicate. **Leave a clean seam for it**
  rather than hard-wiring the clock as the decider.
- Compute the leftover-time interpolation alpha and expose it, but do not consume it — Phase 06
  owns interpolation.
- Rendering reads the sim; it never writes to it. There is **one shared camera and no split-screen**
  — nothing here should associate a view with a `PlayerId`.

The Phase 09 swap is the test of whether this phase succeeded: replacing "gather local inputs" with
"gather confirmed inputs for tick T from the network" must leave everything else untouched.

---

## 2. What this phase must NOT implement

- **No networking.** `PeerId` exists as a type; that is all. Phase 09 owns transport, packet
  formats, delay buffers and host logic.
- **No rollback.** The design is lockstep with an input-delay buffer. No state snapshots, no
  history buffer.
- **No final input backend.** Two keyboard players is enough today. Gamepads, hot-plug and device
  enumeration are Phase 10. Do not build a device abstraction layer for hardware you cannot test.
- **No camera behaviour.** Phase 06.
- **No plugin or backend registry for input.**
- **No general event bus, ECS scheduler, or system-ordering framework.** The tick order is eight
  fixed steps in a function.
- **No gameplay.** Gravity is Phase 03, movement Phase 04. This phase moves nothing.

Two keyboard players is the requirement *today*, but do not let the number 2 — or 4 — reach a type,
an array size, or a match arm. Size by `constants::MAX_PLAYERS`.

---

## 3. Phase 01 API to build on

Use `math::Fix` and `math::FVec2` for anything spatial; never `f32`/`f64`. Take the tick rate from
`constants::TICKS_PER_SECOND` and the step duration from `constants::FIXED_DELTA_TIME`. Size player
collections by `constants::MAX_PLAYERS`. `CLAUDE.md` lists the full surface and the accuracy
caveats — read the caveats before using `sin`, `cos` or `normalized_safe`.

---

## 4. Invariants to preserve

All of `CLAUDE.md`. The ones this phase is most likely to break:

- Tick rate fixed at 120 Hz, decoupled from frame rate; one frame may drive zero, one or many ticks.
- Authoritative time is a `u64` tick counter; never accumulate `FIXED_DELTA_TIME`.
- `step` is pure: no clock, no devices, no globals.
- Deterministic iteration order; sort by stable id wherever order could vary.
- The four id types never convert into one another.
- Logical players, local players, peers and host are distinct concepts.

---

## 5. Tests and validation expected

- `step()` called 600 times with a recorded input sequence leaves the tick counter at 600.
- Two `Sim`s with the same seed, fed the same input sequence, hold equal state — compare a debug
  hash. This is the seed of the Phase 08 determinism harness.
- Entity insertion order is identical across two runs with shuffled insertion requests.
- The accumulator produces the expected tick counts for a range of frame durations, including a
  frame far longer than one tick, which must be **bounded** rather than producing a huge burst.
- The RNG's output bits are pinned to hard-coded expected values, the way Phase 01 pins arithmetic.
- Manual: two keyboard players reach two distinct `PlayerId`s and neither steals the other's keys.

---

## 6. Known edge cases and risks

- **Catch-up spiral.** The single most likely bug in this phase. A frame that takes 500 ms would
  request 60 ticks; if simulating them takes longer than 500 ms the machine never recovers. Cap it
  and test the cap.
- **A player outliving its device.** Unplugging a controller must not delete the player or its
  entity.
- **A player with no entity.** Spectators and not-yet-spawned players are normal.
- **Quantisation drift.** If the stick is quantised anywhere other than at the input boundary, local
  and remote simulations diverge and it will not be diagnosable until Phase 09.
- **RNG choice.** See the open question below — this is a determinism decision, not a convenience
  one.
- **Frame-rate-dependent behaviour** sneaking in through the driver. Test at more than one frame
  duration.

---

## 7. Open questions — resolve deliberately, do not just pick

1. **`PlayerId(u8)` versus the environmental-source constant.** `constants` carries an
   `ENVIRONMENTAL_PLAYER_ID` of 1000, which does not fit in a `u8`. **Recommendation: do not use
   that sentinel.** Model a damage or effect source as `Option<PlayerId>` or a small enum
   (`Environment` / `Player(PlayerId)`). A sentinel inside the value range is exactly the class of
   bug the separate id types exist to prevent. Phase 07 owns the decision when damage sources first
   appear; this phase only needs to avoid foreclosing it.
2. **Which PRNG.** Unspecified so far. **Recommendation: write a small explicit PCG or xorshift
   inline**, roughly ten lines, rather than depending on `rand` — its algorithms are not guaranteed
   stable across versions, and a silent change would be a cross-version desync that Phase 08 would
   catch only by luck. Pin the output bits in a test either way.
3. **Stick angle carries no magnitude.** This looks deliberate — it forces uniform movement speed
   and halves the input size — but it means analogue "walk slowly" is impossible forever, and it is
   very hard to add later without changing the wire format. **Confirm the intent before building
   on it.**

---

## 8. Deferred work and which phase owns it

| Phase | Owns |
|---|---|
| 03 | Gravity and grounding. Re-deriving the per-tick gravity constant for 120 Hz. |
| 04 | Movement and jump. Re-deriving air acceleration (it also sets the air speed cap) and the retained-speed multipliers (which convert by square root, not division). |
| 05 | Surface walking. Clamping trig results at call sites that need a restricted domain. |
| 06 | Camera behaviour, render-only float constants, and tick-to-frame interpolation. |
| 07 | Collision and knockback. Damage-source modelling. |
| 08 | Determinism harness and the golden cross-platform checksum. |
| 09 | Lockstep netcode: transport, packet format, delay buffers in milliseconds, real bandwidth figures, initial host selection. |
| 10 | Session, rounds, spawn points, and the real input backend including gamepad enumeration. |
| 11 | Ability system; per-player entity capacity limits. |

---

## 9. Completion criteria

- [ ] The four id types cannot be mixed up — try it; it must not compile.
- [ ] One peer holding two players is representable and tested.
- [ ] A player surviving device disconnection is representable and tested.
- [ ] `step()` has no access to time, input devices, or global state.
- [ ] The same-seed-same-input equality test passes.
- [ ] The tick counter is `u64` and never derived from accumulated delta time.
- [ ] The accumulator is frame-rate independent and its catch-up is bounded and tested.
- [ ] Swapping local input for network-supplied input would require no changes outside the driver.
- [ ] `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` all pass.
- [ ] CI green on Linux and macOS.
