# Phase 03 — Gravity, platforms and grounding

**Prerequisite:** Phase 02 (identity types and the tick loop) is complete and on `main`.
**Estimate:** 2-3 days.
**Read first:** [`../CLAUDE.md`](../CLAUDE.md) — the invariants there are non-negotiable.

---

## 1. What this phase implements

A body falls under gravity, meets a platform, and attaches to it. The first thing in this project
you can actually watch.

Only the **flat top face** of a platform is handled. Corners, walls, ceilings and rotation are
Phase 05, and doing them now is a trap: the zone system is far easier to get right once there is
movement to test it with, and much harder to debug when nothing moves yet.

### `src/platform.rs`

```rust
pub struct Platform {
    pub center: FVec2,
    pub extents: FVec2,      // HALF width and height of the INNER rectangle, corner radius excluded
    pub radius: Fix,         // corner radius
    pub rotation: Fix,
    pub kind: PlatformKind,  // Normal | Ice
}
```

**`extents` excludes the corner radius.** Settle that convention now and write it on the field: it
decides every perimeter length in Phase 05, and discovering it is wrong there means redoing the
surface parameterisation rather than editing a constant.

Build, for the flat top only:

- `up()` / `right()` — the platform's local basis, derived from `rotation`. Every geometric query
  projects into this basis, which is what makes rotation nearly free when Phase 05 needs it.
- `normal_at(point) -> FVec2` — the full eight-zone version, about thirty lines. Write all of it now
  even though only the up-facing zone gets exercised. Half of it is more work than all of it, and
  the eight-case test is the safety net Phase 05 will lean on hard.
- A flat-top surface query for the raycast.

### `src/body.rs`

```rust
pub struct Body {
    pub position: FVec2,
    pub self_imposed_velocity: FVec2,  // the body's own movement and jumps
    pub external_velocity: FVec2,      // knockback and explosions; Phase 07
    pub rotation: Fix,
    pub scale: Fix,
}
```

Two velocity channels, kept apart. They decay differently, and abilities in Phase 11 need to tell
"I jumped" from "I was thrown". Merging them now costs nothing and cannot be undone cheaply.

### `src/player_physics.rs`

- `add_gravity` — **no `dt` multiplication.** See the tick-rate note in section 4.
- `velocity_based_raycasts` — cast along the velocity vector, length `|v| * dt`, using **three
  parallel rays**: one at the centre-front and two offset by plus and minus the body radius,
  perpendicular to the velocity. One ray lets a fast body slip past a platform corner. Three is not
  belt-and-braces, it is the difference between working and tunnelling.
- `attach_to_ground` — set grounded, record the platform, compute the initial surface position from
  the contact point.
- `unground` — clear grounded state, nudge the body `UNGROUND_NUDGE` along the surface normal so the
  next tick's ground check does not immediately re-attach, and zero the grounded speed.

Grounded state:

```rust
struct Grounded { platform: EntityId, local_pos: Fix }   // local_pos in [0, 1)
```

**Store the scalar position along the perimeter, not an (x, y).** The world position is derived from
it each tick. Storing coordinates works perfectly for flat tops and then has to be thrown away in
Phase 05, taking every call site with it.

### Rendering

Rounded rectangles for platforms, a circle for the body. Then a **debug overlay behind a key
toggle**: the surface normal at the body, the three raycasts, and grounded state as a colour.

This is not gold-plating. Phases 04 and 05 are geometry work whose failures are invisible in a
still frame, and this overlay is the cheapest debugging in the project.

---

## 2. What this phase must NOT implement

- **No physics engine.** Float-based engines cannot interoperate with a fixed-point simulation, and
  the collision code needed here is small.
- **No corner walking, wall walking or ceiling walking.** Phase 05.
- **No ice friction, moving platforms or platform hopping.** Phase 05.
- **No `dt` multiplication on the gravity constants.** Section 4.
- **No knockback.** `external_velocity` exists as a field and stays zero. Phase 07.
- **No camera work.** Phase 06. A fixed view is fine.

---

## 3. Phase 02 API to build on

- `Sim::step` is the only mutation path, and it reads nothing but its arguments. Gravity, raycasts
  and grounding all run inside a tick, in the numbered slots the tick order already reserves —
  physics is step 5, and the late update after it is step 6.
- `Sim::request_spawn` queues; the entity appears at the start of the next tick. `Spawn::order_key`
  is a total order over the spawn's fields — not a hash, deliberately, because a hash collision
  between distinct spawns would silently hand the tie back to arrival order. **If `Spawn` grows
  fields, extend `order_key` with them**, or two spawns differing only in a new field compare equal
  and get treated as interchangeable when they are not.
- `Sim::state_hash` must cover every new field that is part of simulation state. A field left out is
  a desync the harness cannot see.
- `ActionState` carries a stick angle, a reserved magnitude byte and six buttons. Nothing in this
  phase reads the stick; movement is Phase 04.
- `math::Fix` and `math::FVec2` for everything spatial. Never `f32` or `f64`.

---

## 4. The tick rate, and the constant that has to change

`GRAVITY_ACCEL` is a **per-tick addition tuned against a 60 Hz tick**, and the simulation now runs
at 120. Using it unchanged applies it twice as often and doubles gravity.

Halving it preserves terminal velocity: the drag coefficient is
`GRAVITY_ACCEL / GRAVITY_MAX_FALL_SPEED`, so halving the numerator halves the drag and leaves the
equilibrium at 27. **The approach curve still differs**, because this is a discrete exponential and
not the continuous solution it approximates. Re-derive it, pin the result with a test, then confirm
by feel — the number that makes the maths agree and the number that makes falling feel right are
not guaranteed to be the same, and the second one wins.

`GRAVITY_MAX_FALL_SPEED` is an absolute limit and does not change. The full classification of which
constants convert how is in `src/constants.rs`; read it before touching any of them.

---

## 5. Tests and validation expected

- Terminal velocity: apply gravity 600 times from rest, assert the speed converges to
  `GRAVITY_MAX_FALL_SPEED` exactly and never exceeds it.
- A body dropped from a known height attaches within the expected number of ticks, and ends within
  its radius of the surface.
- A body moving fast horizontally past a platform edge does not tunnel. Pick a velocity large
  enough that a single centre ray would miss, so the test actually exercises why there are three.
- `normal_at` returns the right zone and normal for all eight cases, including on a rotated
  platform.
- The replay checksum in `tests/determinism.rs` is extended to cover a falling body, and repinned.
- Manual: a body falls, lands and stays. Toggle the overlay and confirm the normal points away from
  the surface.

---

## 6. Known edge cases and risks

- **Tunnelling at speed.** The reason for three rays. Test it rather than trusting it.
- **Re-attaching on the tick after ungrounding.** What `UNGROUND_NUDGE` exists to prevent; a jump
  that sticks to the floor is this bug.
- **Landing exactly on a corner**, where the flat-top query and the zone the normal reports
  disagree. Only the top face is in scope, so decide what happens and write it down rather than
  letting it fall out of the arithmetic.
- **A grounded platform that is destroyed.** `Grounded` holds an `EntityId`; generational keys mean
  a stale one fails a lookup rather than silently addressing whatever took the slot. Handle the
  `None`.
- **The gravity constant.** Getting the conversion wrong is not a crash, it is a game that feels
  subtly wrong for months.

---

## 7. Open questions — resolve deliberately, do not just pick

1. **Where platforms live.** They could be entities in the slotmap, or a separate level structure.
   Entities give them ids, deterministic ordering and destruction for free, and `Grounded` already
   wants an `EntityId`. **Recommendation: entities.** Moving platforms in Phase 05 make this the
   cheaper answer, not just the tidier one.
2. **Whether `Body` is a component of `Entity` or replaces it.** `Entity` is currently a position and
   an owner and nothing else, deliberately. **Recommendation: fold `Body` into `Entity` rather than
   nesting it**, until there is a second kind of entity that needs different fields. Nesting one
   struct inside another with a one-to-one relationship buys nothing yet.
3. **The re-derived gravity constant.** Halving is the analytically correct starting point, not
   necessarily the shipping value. Pin whatever is chosen with a test and record why.

---

## 8. Deferred work and which phase owns it

| Phase | Owns |
|---|---|
| 04 | Movement and jump. Re-deriving air acceleration, which also sets the air speed cap through its drag term, and the retained-speed multipliers, which convert by square root rather than division. |
| 05 | Surface walking: corners, walls, ceilings, rotation, ice. Clamping trig results at call sites needing a restricted domain. |
| 06 | Camera behaviour, render-only float constants, tick-to-frame interpolation using the alpha the driver already exposes. |
| 07 | Collision and knockback; `external_velocity` starts being written. Damage-source modelling. |
| 08 | The determinism harness and the golden cross-platform checksum. |
| 09 | Lockstep netcode. The driver's input-gathering step is the only thing it replaces. |
| 10 | Session, rounds, spawn points, and the real input backend including gamepad enumeration. |
| 11 | Abilities. |

---

## 9. Completion criteria

- [ ] A body falls, lands on a flat platform, and stays there.
- [ ] Terminal velocity is exactly `GRAVITY_MAX_FALL_SPEED`, verified by test.
- [ ] No tunnelling at high speed, verified by a test that would fail with one ray.
- [ ] Several platforms at different positions work.
- [ ] `normal_at` is correct for all eight zones, rotated included, by test.
- [ ] Grounded position is stored as a scalar along the perimeter, not as coordinates.
- [ ] Every new piece of simulation state is included in `Sim::state_hash`.
- [ ] `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` all pass.
- [ ] CI green on Linux and macOS.
