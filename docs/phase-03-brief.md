# Phase 03 — Gravity, platforms and grounding

**Prerequisite:** Phase 02 is complete and on `main`.
**Estimate:** 2-3 days.
**Read first:** [`../CLAUDE.md`](../CLAUDE.md) — the invariants there are non-negotiable.

This brief is self-contained. It was written against `main` and every API it names was checked to
exist there. You should not need any other document.

---

## 1. What this phase implements

A body falls under gravity, meets a platform, and attaches to it. The first thing in this project
you can actually watch.

**Only the flat top face of a platform is handled.** Walls, ceilings, corners and rotation
behaviour are Phase 05. Section 2 says this again, in more detail, because it is the scope boundary
most likely to erode.

---

## 2. Scope, exactly

### In scope

- Gravity applied per tick to a body.
- Platforms as rectangles with rounded corners, at arbitrary position and rotation.
- Collision and attachment against **the flat top face only**.
- Grounded state, stored as a scalar position along the surface.
- Ungrounding.
- Rendering of platforms and bodies, plus a debug overlay.

### Out of scope — do not build

| Not now | Owner |
|---|---|
| Walking on walls, ceilings or corners; anything that happens when a body meets a non-top face | Phase 05 |
| Rotation *behaviour* — a rotated platform must be representable and `normal_at` must be correct on one, but nothing walks around it | Phase 05 |
| Ice friction, moving platforms, platform hopping | Phase 05 |
| Player movement input, jumping | Phase 04 |
| Knockback. `external_velocity` exists and stays zero | Phase 07 |
| Camera behaviour. A fixed view is fine | Phase 06 |
| Any physics engine | permanently excluded |

**`normal_at` is the one deliberate exception to "flat top only".** Write the full eight-zone
version now — about thirty lines — even though only the up-facing zone is exercised. Half of it is
more work than all of it, and the eight-case test is the safety net Phase 05 leans on hard.

---

## 3. Resolved design questions

These were open at the end of Phase 02 and are now decided. They are recommendations, not
commandments: if one turns out wrong, say so rather than working around it.

### Q1 — Platforms are entities. ✅

Put platforms in the slotmap alongside everything else, rather than in a separate level structure.

They get stable generational ids, deterministic iteration order and destruction for free, and
`Grounded` needs an `EntityId` to refer to its platform anyway. Moving platforms in Phase 05 make
this the cheaper answer rather than merely the tidier one.

A consequence to plan for: a body can be grounded on a platform that is later removed. Generational
keys mean the stale id fails a lookup instead of silently addressing whatever took the slot —
**handle the `None`**, do not `unwrap` it.

### Q2 — `Body` may be folded into `Entity` for Phase 03. ✅

`Entity` is currently `{ position, owner }` and deliberately near-empty. Add the body fields to it
rather than nesting a `Body` struct inside it.

Nesting a struct in a one-to-one relationship buys nothing until a second kind of entity needs
different fields, and that moment has not arrived. It is a cheap refactor when it does.

**This does not merge the two velocity channels** — see section 5.

### Q3 — Gravity is re-derived for 120 Hz, and validated on the curve, not the limit. ✅

`GRAVITY_ACCEL` is a **per-tick addition tuned against a 60 Hz tick**, and the simulation runs at
120. Applied unchanged it fires twice as often and doubles gravity.

Halving it is the analytically correct starting point. **It is not automatically the shipping
value** — pin whatever is chosen with a test, record why, then confirm by feel. The number that
makes the arithmetic agree and the number that makes falling feel right are not guaranteed to be
the same, and the second one wins.

`GRAVITY_MAX_FALL_SPEED` is an absolute limit and does not change.

The full classification of which constants convert how — additions halve, multipliers take a square
root, delta-time-scaled values need nothing — is in `src/constants.rs`. Read it before touching any
of them.

---

## 4. The trap in the gravity validation — read this before writing the test

The obvious validation is "terminal velocity converges to `GRAVITY_MAX_FALL_SPEED`". **That test
passes whether or not you do the 120 Hz conversion, so on its own it proves nothing.**

The drag term is derived from the acceleration: velocity settles where
`g == v * (g / v_max)`, which solves to `v == v_max` **for every value of `g`**. Halving the
acceleration halves the drag with it and the equilibrium does not move. Simulated to convergence,
the halved and unhalved constants both give exactly 27.

What the conversion actually changes is the **approach curve**:

| | reaches 95% of terminal at |
|---|---|
| constant left unhalved | tick 50 — **0.42 s** of wall clock |
| constant halved | tick 100 — **0.83 s** of wall clock |

So the validation must include **at least one timing assertion**, not just the limit:

- from rest, the number of ticks to reach a chosen fraction of terminal velocity, **and**
- from rest, the distance fallen after a fixed number of ticks.

Both are pinned to exact fixed-point values, computed from the model actually implemented. Either
one catches a wrong conversion; the terminal-velocity assertion catches neither. Keep the terminal
velocity test as well — it guards the drag derivation — but do not mistake it for a guard on the
tick rate.

Sanity check on the numbers above: they come from the model `v += g * (1 - v / v_max)`, which is
what `src/constants.rs` describes. If the implemented model differs, re-derive rather than trusting
this table.

---

## 5. Inherited invariants — the ones this phase is most likely to break

All of `CLAUDE.md` applies. These are the ones with teeth here.

### 5.1 One `Sim::step` is exactly one tick, always

Every call to `step` advances the tick counter exactly once and consumes exactly one tick's inputs.
**Hit-stop is not an exception**: it freezes the world but still advances the tick and still records
the inputs.

This is deliberate and load-bearing. The netcode indexes inputs by tick number, so a call that
advanced the world without advancing the counter — or the reverse — would slide every later tick's
inputs one slot over. If you add anything that can early-return from `step`, it must still advance
the tick.

### 5.2 `Spawn::order_key` stays a canonical total ordering over every deterministic field

Insertion order comes from a lexicographic comparison of the spawn's fields, currently
`(position.x.to_bits(), position.y.to_bits(), owner)`.

**It must not become a hash, and it must not become a counter.**

- A **counter** fixes entity ids before the sort runs, so two peers that produced the same spawns in
  a different order build different worlds and sorting afterwards cannot rescue them.
- A **hash** is not injective. Two spawns differing in some field can collide, and the sort then
  falls back to arrival order — the same bug, hidden in the one case nobody thinks to test. This
  was a real defect caught in review at the end of Phase 02, not a hypothetical.

**If `Spawn` grows fields, extend `order_key` with all of them.** A field left out means two spawns
differing only in that field compare equal and get treated as interchangeable when they are not.

Keep the forward/reverse test: build a world from a spawn list, build it again from the reversed
list, and assert identical `(EntityId, Entity)` pairs — not merely an identical set.

### 5.3 Every deterministic state field goes into `state_hash`

`Sim::state_hash` is what the Phase 08 harness and the eventual desync detector compare. **A field
that is part of simulation state but missing from the hash is a desync nobody can see.**

This phase adds a lot of state — velocities, rotation, scale, grounded status, platform geometry.
Every one of them belongs in the hash. Add them as you add the fields, not afterwards.

Fields that are *not* simulation state — anything render-side, anything timing-related, anything
derived from the local machine — must stay out of both `Sim` and the hash.

### 5.4 The two velocity channels stay distinct

`self_imposed_velocity` (the body's own movement and jumps) and `external_velocity` (knockback and
explosions) are separate fields and stay separate.

They decay differently, and abilities in Phase 11 need to distinguish "I jumped" from "I was
thrown". This phase only ever writes the first one; the second exists, stays zero, and must survive
the temptation to collapse them because nothing uses it yet. Merging them costs nothing now and is
expensive to undo later.

### 5.5 No floats in the simulation

`math::Fix` and `math::FVec2` for everything spatial. `f32`/`f64` are for rendering and the camera
only. A float in a type that feeds `step` is a bug.

### 5.6 Deterministic iteration order

Sort by a stable id anywhere order could vary. The slotmap's own iteration order is a function of
the insert and remove history alone, so it is safe to depend on.

---

## 6. Phase 02 API to build on

Verified present on `main`.

- `Sim::step(&[(PlayerId, ActionState)])` — the only mutation path; reads nothing but its
  arguments. Gravity, raycasts and grounding all run inside a tick, in the numbered slots the tick
  order already reserves as comments. **Physics is slot 5; the late update after it is slot 6.**
- `Sim::request_spawn(Spawn)` — queues; the entity appears at the start of the next tick.
- `Sim::apply_hitstop(u32)`, `Sim::tick()`, `Sim::hitstop()`, `Sim::entity()`, `Sim::entities()`,
  `Sim::entity_count()`, `Sim::inputs()`, `Sim::state_hash()`.
- `sim::Accumulator` — `ticks_due(frame_nanos)` and `alpha()`. Driver-side, **not** simulation
  state. `MAX_TICKS_PER_FRAME` bounds catch-up.
- `sim::Pcg32` — seeded per match, advanced only inside `step`. Nothing in this phase needs it yet.
- `ids` — `PeerId`, `PlayerId`, `DeviceId`, `Tick(u64)`, `EntityId`, `Player`, `Roster`. No
  conversions between id types.
- `input::ActionState` — stick angle, a reserved magnitude byte, six buttons. **Nothing in this
  phase reads the stick**; movement is Phase 04.
- `math` — `Fix`, `FVec2` (`dot`, `sqr_magnitude`, `magnitude`, `distance`, `normalized_safe`,
  `perp`, `perp_cw`), `sqrt`, `sin`, `cos`, `atan2`, `TRIG_ERROR_BOUND`.

### Accuracy facts you must not rediscover the hard way

- `atan2` already handles near-vertical vectors without panicking; call sites need no guard. This
  was fixed during Phase 02 because the underlying routine divides `y / x` and overflows.
- `sin`/`cos` are within 32 raw units of true and **may land up to 2 raw units outside -1..=1**.
  Anything feeding one into `acos`, `sqrt(1 - x^2)` or another domain-restricted operation **must
  clamp at that call site**. There is deliberately no global clamp. Surface normals are exactly
  where this bites.
- `normalized_safe` returns components up to 1 raw unit low; `sqr_magnitude` of a normalised vector
  is up to 8 units under one. Always under, never over.
- `FIXED_DELTA_TIME * 120` is 16 raw units short of one. Elapsed time is a tick count, never a sum
  of delta times.

---

## 7. What to build

### `src/platform.rs`

```rust
pub struct Platform {
    pub center: FVec2,
    pub extents: FVec2,      // HALF width and height of the INNER rectangle, corner radius EXCLUDED
    pub radius: Fix,         // corner radius
    pub rotation: Fix,
    pub kind: PlatformKind,  // Normal | Ice
}
```

**`extents` excludes the corner radius.** Settle that convention now and write it on the field: it
decides every perimeter length in Phase 05, and finding it wrong there means redoing the surface
parameterisation rather than editing a constant.

- `up()` / `right()` — the platform's local basis from `rotation`. Every geometric query projects
  into this basis, which is what makes rotation nearly free in Phase 05.
- `normal_at(point) -> FVec2` — full eight zones. See section 2.
- A flat-top surface query for the raycast.

### Body fields (folded into `Entity`, per Q2)

```rust
position: FVec2,
self_imposed_velocity: FVec2,   // own movement and jumps
external_velocity: FVec2,       // knockback; stays zero this phase
rotation: Fix,
scale: Fix,
```

### `src/player_physics.rs`

- `add_gravity` — **no `dt` multiplication.** Section 3.
- `velocity_based_raycasts` — cast along the velocity vector, length `|v| * dt`, using **three
  parallel rays**: centre-front, plus two offset by ± the body radius perpendicular to the velocity.
  One ray lets a fast body slip past a platform corner. Three is not belt-and-braces; it is the
  difference between working and tunnelling.
- `attach_to_ground` — set grounded, record the platform, compute the initial surface position from
  the contact point.
- `unground` — clear grounded state, nudge the body `UNGROUND_NUDGE` along the surface normal so the
  next tick's ground check does not immediately re-attach, zero the grounded speed.

### Grounded state

```rust
struct Grounded { platform: EntityId, local_pos: Fix }   // local_pos in [0, 1)
```

**Store the scalar position along the perimeter, not an (x, y).** The world position is derived from
it each tick. Coordinates work perfectly for flat tops and then have to be thrown away in Phase 05,
taking every call site with them.

### Rendering

Rounded rectangles for platforms, a circle for the body. Then a **debug overlay behind a key
toggle**: the surface normal at the body, the three raycasts, and grounded state as a colour.

Not gold-plating. Phases 04 and 05 are geometry work whose failures are invisible in a still frame,
and this overlay is the cheapest debugging in the project.

---

## 8. Validation plan

### Gravity

1. **Terminal velocity.** From rest, apply gravity until convergence; assert the speed equals
   `GRAVITY_MAX_FALL_SPEED` exactly and never exceeds it. *Guards the drag derivation. Does not
   guard the tick rate — see section 4.*
2. **Approach timing.** From rest, assert the exact tick at which a chosen fraction of terminal
   velocity is reached.
3. **Fall distance.** From rest, assert the exact distance fallen after a fixed number of ticks.

Tests 2 and 3 are what actually catch a wrong 60→120 conversion. Pin them to exact fixed-point
values.

### Anti-tunnelling — all three cases

4. **High-speed downward.** A body falling fast enough to cross the platform in a single tick must
   land on it, not pass through.
5. **Diagonal edge.** A body moving diagonally at the platform's top edge resolves to a defined
   outcome rather than to whichever ray happened to fire.
6. **Three-ray necessity.** A case that a single centre ray would miss and three rays catch —
   typically a fast pass near a corner. **Choose the velocity so the test fails with one ray.** If
   it passes with one ray it is not testing why there are three; verify that by temporarily
   disabling the outer two.

### Geometry

7. **`normal_at` across all eight zones**, on an axis-aligned platform and on a rotated one.
8. **Landing.** A body dropped from a known height attaches within the expected tick count and rests
   within its radius of the surface.
9. **Several platforms** at different positions and rotations all work.

### Determinism

10. **Grounded-then-destroyed.** Remove the platform a body is grounded on; the stale `EntityId`
    lookup returns `None` and is handled, not unwrapped.
11. **`state_hash` covers the new fields.** Mutating any one of them changes the hash. Cheap to
    write, and it is the only thing standing between a forgotten field and an invisible desync.
12. **Forward/reverse spawn ordering** still holds with the new `Spawn` fields.
13. **The replay checksum** in `tests/determinism.rs` is extended to include a falling body and
    re-pinned. Record *why* it moved next to the value, as the existing note does.

### Manual

14. A body falls, lands and stays. Toggle the overlay and confirm the normal points away from the
    surface.

---

## 9. Known traps

- **The terminal-velocity test proving nothing.** Section 4. The single most likely way this phase
  ships a silently wrong constant.
- **Tunnelling at speed.** The reason for three rays. Test it rather than trusting it.
- **Re-attaching on the tick after ungrounding.** What `UNGROUND_NUDGE` prevents. A jump that sticks
  to the floor is this bug, and it will look like a Phase 04 problem when it is not.
- **Landing exactly on a corner**, where the flat-top query and the zone `normal_at` reports
  disagree. Only the top face is in scope, so decide what happens and write it down rather than
  letting it fall out of the arithmetic.
- **A grounded platform that is destroyed.** Handle the `None`.
- **A new state field missing from `state_hash`.** Invisible until Phase 08, and then expensive.
- **A new `Spawn` field missing from `order_key`.** Same shape of bug, same invisibility.
- **Clamping trig output.** `sin`/`cos` can exceed 1. Surface normals feed exactly the kind of
  domain-restricted operation that cares.

---

## 10. Deferred work and which phase owns it

| Phase | Owns |
|---|---|
| 04 | Movement and jump. Re-deriving air acceleration, which also sets the air speed cap through its drag term, and the retained-speed multipliers, which convert by square root rather than division. |
| 05 | Surface walking: walls, ceilings, corners, rotation behaviour, ice. Clamping trig at domain-restricted call sites. |
| 06 | Camera behaviour, render-only float constants, and tick-to-frame interpolation using the alpha the driver already exposes but does not consume. |
| 07 | Collision and knockback; `external_velocity` starts being written. Damage-source modelling — as `Option<PlayerId>` or a small enum, never a sentinel inside the id range. |
| 08 | The determinism harness and the golden cross-platform checksum. |
| 09 | Lockstep netcode. The driver's input-gathering step is the only thing it replaces; if that swap looks like it needs changes elsewhere, the seam has eroded. |
| 10 | Session, rounds, spawn points, and the real input backend including gamepad enumeration. |
| 11 | Abilities. |

Also outstanding, from Phase 02: nobody has yet confirmed by hand that the two keyboard players
don't steal each other's keys. There is a test proving the bindings don't overlap, but no human has
pressed a key. Worth thirty seconds once something moves.

---

## 11. Completion criteria

- [ ] A body falls, lands on a flat platform, and stays there.
- [ ] Terminal velocity is exactly `GRAVITY_MAX_FALL_SPEED`, by test.
- [ ] Fall timing and fall distance are pinned by test, and would fail on an unconverted constant.
- [ ] No tunnelling: high-speed downward, diagonal-edge, and a case that fails with one ray.
- [ ] `normal_at` correct for all eight zones, rotated included, by test.
- [ ] Several platforms at different positions work.
- [ ] Grounded position stored as a scalar along the perimeter, not as coordinates.
- [ ] `self_imposed_velocity` and `external_velocity` remain distinct; the latter stays zero.
- [ ] Every new deterministic state field is in `state_hash`, with a test that proves it.
- [ ] `Spawn::order_key` still a total ordering over all deterministic fields; forward/reverse test
      passes.
- [ ] One `step` is still exactly one tick, hit-stop included.
- [ ] Nothing in this phase walks on a wall, ceiling or corner.
- [ ] `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` all pass.
- [ ] CI green on Linux and macOS.
