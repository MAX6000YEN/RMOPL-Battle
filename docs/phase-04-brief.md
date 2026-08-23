# Phase 04 — Movement and jump

**Prerequisite:** Phase 03 is complete and on `main`.
**Estimate:** 2-3 days.
**Read first:** [`../CLAUDE.md`](../CLAUDE.md) — the invariants there are non-negotiable.

This brief is self-contained. It was written against `main` and every API name it uses was checked
to exist there. Every number in it was computed from the constants actually in `src/constants.rs`.
You should not need any other document.

---

## 1. What this phase implements

A player walks, and a player jumps. This is where it starts feeling like a game rather than a
physics demo.

**Still only the flat top face of a platform.** Section 2 says this again in more detail, because
it is the boundary most likely to erode once a body can move sideways.

The jump is the single most important piece of feel in the project. Get the algorithm right before
touching a number.

---

## 2. Scope, exactly

### In scope

- Stick input driving a grounded body along the surface it is standing on.
- Ground acceleration, and the friction that stops a body when input is released.
- Ice, which is the same friction with a different coefficient. The field already exists.
- Air control: weak horizontal acceleration with a drag term that caps air speed.
- The jump, edge-triggered.
- Walking off the end of a platform, which ungrounds and carries momentum with it.

### Out of scope — do not build

| Not now | Owner |
|---|---|
| Walking on walls, ceilings or corners; anything at a non-top face | Phase 05 |
| Moving platforms, platform hopping | Phase 05 |
| Coyote time, jump buffering, double jump, wall jump | nobody yet — a deliberate design decision later, not an accident of this phase |
| Knockback. `external_velocity` exists and stays zero | Phase 07 |
| Camera behaviour. A fixed view is fine | Phase 06 |
| Analogue movement speed from `stick_magnitude` | see §3, Q3 |
| Any physics engine | permanently excluded |

---

## 3. Resolved design questions

Recommendations, not commandments. If one turns out wrong, say so rather than working around it.

### Q1 — Grounded speed is a new field on `Entity`, and it goes in the hash. ✅

A grounded body's motion is one scalar: how fast it is sliding along the surface, signed. It is not
a vector, because the direction is whatever the surface says it is at that point.

Put it on `Entity` as `grounded_speed: Fix`, next to the two velocity channels rather than inside
`Grounded`. A body that has just left the ground still needs its speed for one more moment — the
jump reads it after `Grounded` is cleared — and burying it inside an `Option` that the jump has
just set to `None` makes that awkward for no gain.

**It is simulation state, so it goes into `state_hash`, in the same edit that adds the field.**
Phase 03 left you a test that will catch it if you forget:
`sim::tests::every_entity_field_reaches_the_checksum` needs one new variant per new field.

### Q2 — Jump is edge-triggered from a `bool` on `Entity`. ✅

A held jump button must produce exactly one jump. That needs last tick's button state.

Store `jump_held: bool` on `Entity`, set it from this tick's input at the end of the movement step,
and fire a jump only on the rising edge. **It is simulation state and goes in the hash.**

The alternative — keeping the previous tick's whole input vector on `Sim` — is more state, more
hashing, and it makes the netcode's job harder for no benefit.

### Q3 — `stick_magnitude` stays unread. ✅

Movement is uniform-speed this phase: full deflection or nothing, decided by `has_stick()`. The
magnitude byte remains reserved.

Analogue movement speed is a feel decision that wants a controller in someone's hands, and there
isn't one until Phase 10. The byte is already on the wire, so adding it later costs nothing.

### Q4 — Do not apply `INPUT_DEADZONE` again. ✅

**It is already applied**, in `ActionState::with_stick`, at the input boundary where the wire
format is decided. A stick inside the deadzone arrives with `stick_magnitude == 0`.

Test for input with `has_stick()`. Applying the deadzone a second time in the movement code would
be a second, differently-placed threshold that nobody can find later.

---

## 4. The trap in the tick-rate conversions — read this before writing the test

Phase 03's trap was a test that passed whether or not the conversion was done. This phase has two
more of the same shape, and one claim in the repo that was simply wrong.

### 4.1 The air speed cap depends on the *order* of the two terms, not on the coefficient

Air movement is an acceleration and then a drag proportional to current speed:

```
v.x += direction * ACCEL_AIR
drag = ACCEL_AIR / (MAX_SPEED + ACCEL_AIR)
v.x -= drag * v.x
```

Solve the fixed point **with the drag applied after the acceleration**, as written above:

    v = (v + a) * M / (M + a)   =>   v = M

The cap is exactly `MAX_SPEED`, for **every** value of `a`. Halving `ACCEL_AIR` does not move it.

Apply the drag *before* the acceleration instead and the fixed point is `M + a` — 29 — and halving
the coefficient then drags the cap down to 24. **That ordering is the hazard.** Verified by
simulation in exact fixed point:

| | cap |
|---|---|
| accelerate, then drag — `a = 10` | 19.000000010 |
| accelerate, then drag — `a = 5` | 19.000000009 |
| drag, then accelerate — `a = 10` | 29.0 |
| drag, then accelerate — `a = 5` | 24.0 |

An earlier note in `src/constants.rs` claimed the coefficient had to be re-derived to preserve the
cap. That was backwards and has been corrected. **Halve `ACCEL_AIR` to 5** and recompute the drag
from whatever the constant is; do not hard-code 29 anywhere.

The one honest cost: `a / (M + a)` is a large fraction of the speed per tick, so applying it twice
as often is not exactly tick-rate invariant even though the cap is. Reaching 95% of the cap takes
0.133 s at the original 60 Hz and 0.108 s at 120 Hz halved — about 20% snappier. That is a feel
question, not a correctness one. Note it and move on unless it feels wrong.

### 4.2 Friction is a multiplier, so it converts by square root — and the obvious test misses it

When input is released, grounded speed is **multiplied** each tick — no delta time:

```
grounded_speed *= if ice { SLIPPERINESS_ICE } else { SLIPPERINESS_DEFAULT }
```

Multipliers compound. Applying 0.5 twice as often is 0.25 per unit of real time, so the
tick-rate-invariant value is the **square root**, not the half. `math::sqrt` is exact, so this
costs nothing.

**A test asserting "speed decays toward zero", or "ice is slipperier than normal ground", passes on
both the converted and the unconverted value.** What separates them is how many ticks the decay
takes. Measured, from `MAX_SPEED` down to a quarter of it:

| | ticks | wall clock |
|---|---|---|
| original, at 60 Hz | 2 (normal), 10 (ice) | 0.033 s, 0.167 s |
| unconverted, at 120 Hz | 2, 10 | 0.017 s, 0.083 s — **twice as slippery** |
| square-rooted, at 120 Hz | 4, 20 | 0.033 s, 0.167 s ✓ |

So pin the **tick count**, which differs, not the fact that it decays, which does not.

The converted values are `sqrt(0.5) = 0.707106781` (raw `3037000499`) and
`sqrt(0.87) = 0.932737905` (raw `4006078799`). Derive them with `math::sqrt` at the call site or
record them as new constants — either is fine, but pin the raw bits in a test.

### 4.3 `ACCEL_GROUND` needs nothing, and you can prove it

It is already scaled by delta time where it is applied. At 120 Hz that is `180 / 120 = 1.5` per
tick and a body reaches `MAX_SPEED` at tick 13 (0.108 s); at 60 Hz it was 3.0 per tick and tick 7
(0.117 s). Same wall clock, to within one tick of integer rounding. Pin the tick count anyway — it
is the cheapest guard there is against someone "fixing" the delta-time factor later.

---

## 5. Inherited invariants — the ones this phase is most likely to break

All of `CLAUDE.md` applies. These have teeth here.

### 5.1 One `Sim::step` is exactly one tick, always

Hit-stop is not an exception: it freezes the world, still advances the tick, still records the
inputs. If you add anything that can early-return from `step` — and a "nobody is holding anything"
fast path is exactly that temptation — it must still advance the tick.

### 5.2 The two velocity channels stay distinct

`self_imposed_velocity` is what this phase writes: air control and the jump. `external_velocity` is
knockback and **stays zero**. In particular, **do not apply air drag to `external_velocity`** — a
body that was thrown should not have the throw sanded off by its own movement code. Phase 07 owns
how knockback decays.

### 5.3 Every new state field goes into `state_hash`

`grounded_speed` and `jump_held` are both simulation state. A field missing from the hash is a
desync nobody can see until Phase 08, and expensive then. Add them to `entity_words` in the same
edit that adds them to `Entity`, and add a variant for each to
`every_entity_field_reaches_the_checksum`.

### 5.4 `Spawn::order_key` stays a total ordering over every deterministic field

If `Spawn` grows a field this phase — it probably does not need to — extend `order_key` with it.
Never a hash, never a counter.

### 5.5 No floats in the simulation

`Fix` and `FVec2` for everything. Rendering may use floats; `to_screen` in `src/main.rs` is the
only place the conversion happens and it only runs in one direction.

### 5.6 Deterministic iteration order

You now need to find the entity a player owns. Do it by walking `entities()` and matching
`owner == Some(player)` — slotmap order is a function of insert and remove history alone and is
identical on every peer. Do not build a `HashMap` from `PlayerId` to `EntityId` to speed this up:
at 16 players and a handful of entities the scan is free, and a hash map's iteration order is
exactly the kind of thing that desyncs.

---

## 6. Phase 03 API to build on

Verified present on `main`.

- `sim::Entity { position, self_imposed_velocity, external_velocity, rotation, scale, owner, shape,
  grounded }`, and `Entity::platform() -> Option<Platform>`.
- `sim::Grounded { platform: EntityId, local_pos: Fix }`.
- `platform::Platform` — `right()`, `up()`, `to_local()`, `to_world()`, `normal_at()`,
  `perimeter()`, `top_face_end()`, `surface_point(local_pos) -> Option<FVec2>`,
  `top_face_local_pos(point) -> Option<Fix>`, `top_face_crossing(from, delta) -> Option<Fix>`,
  `top_face_point(x)`, `local_pos_of_top_x(x)`.
- `platform::PlatformShape { extents, radius, kind }`, `platform::PlatformKind { Normal, Ice }`
  with `tag()`.
- `player_physics` — `add_gravity(&mut FVec2)`, `velocity_based_raycasts(...) -> Option<GroundHit>`,
  `attach_to_ground(&mut Entity, &GroundHit)`, `unground(&mut Entity, normal: FVec2)`,
  `step(&mut SlotMap<EntityId, Entity>)`.
- `input::ActionState` — `stick_angle`, `stick_magnitude`, `jump`, `ab1`, `ab2`, `ab3`, `start`,
  `select`; `has_stick()`, `stick_radians() -> Option<Fix>`, `with_stick()`. **`INPUT_DEADZONE` is
  already applied.**
- `math` — `Fix`, `FVec2` (`dot`, `sqr_magnitude`, `magnitude`, `distance`, `normalized_safe`,
  `perp` 90° CCW, `perp_cw` 90° CW), `sqrt`, `sin`, `cos`, `atan2`, `TRIG_ERROR_BOUND`.
- `Sim::step`'s numbered tick slots. **Slot 1 is where players act on their inputs; slot 5 is
  physics.**

### Accuracy facts you must not rediscover the hard way

- `sin`/`cos` are within 32 raw units of true and **may sit up to 2 raw units outside -1..=1**.
  Turning `stick_radians()` into a direction vector is exactly the call site that must clamp.
  `Platform::right()` already does this; copy the pattern.
- `atan2` handles near-vertical vectors without panicking; call sites need no guard.
- `normalized_safe` returns components up to 1 raw unit low and `FVec2::ZERO` for a zero vector.
- `Fix::frac()` has floor semantics — `frac(-1.25) == 0.75` — so wrapping a surface position into
  `[0, 1)` needs no guard in either direction.
- A grounded body's derived world position is about 7e-9 off its landing position. Assert positions
  with a tolerance, never bit-exactly.
- Terminal velocity settles 27 raw units below `GRAVITY_MAX_FALL_SPEED`, at tick 740.
- `FIXED_DELTA_TIME * 120` is 16 raw units short of one. Elapsed time is a tick count.

---

## 7. What to build

All of it in `src/player_physics.rs` unless noted.

### Routing input to bodies

`player_physics::step` currently takes only the entity map. It now needs the tick's inputs too, or
a second function that runs from slot 1 and takes both. Prefer the second: **movement intent is
slot 1, physics is slot 5**, and the existing tick-order comments already say so.

```rust
pub fn apply_inputs(entities: &mut SlotMap<EntityId, Entity>, inputs: &[(PlayerId, ActionState)])
```

### `move_grounded`

The direction a body walks is defined by the surface, not by world space. That is what will make
walking round a corner work in Phase 05 without this code changing.

1. `forward = normal.perp_cw()` — check the sign against `math::tests::perpendiculars_turn_the_right_way`, which pins all eight cases. On a flat top the normal is `UP` and `UP.perp_cw()` is `RIGHT`, so a stick held right walks right. Assert that in a test; a flipped perpendicular is very hard to see from behaviour.
2. `alignment = normalized_safe(input).dot(forward)`
3. `into_ground = normal.dot(input)`
4. `dir = if |alignment| >= GROUND_ALIGNMENT_DEADZONE || (into_ground <= 0 && |alignment| >= 0.01) { sign(alignment) } else { 0 }`
5. If `dir == 0`, apply friction: `grounded_speed *= slipperiness` — a multiply, per tick, no delta time. §4.2.
6. Otherwise accelerate toward `dir * MAX_SPEED` (see `increment_towards` below).
7. Advance the surface position: `local_pos = (local_pos + grounded_speed * FIXED_DELTA_TIME / perimeter).frac()`

**The two-part test in step 4 is not redundant.** The second clause is what lets you keep walking
when the surface tilts away from the direction you are holding. Do not simplify it away — it will
look like dead code until Phase 05, when removing it becomes a bug nobody can reproduce.

On the denominator in step 7: divide by `platform.perimeter()`. On a flat face the body's centre
travels exactly as far as the surface point does, so this is correct. It stops being correct at a
corner, where the centre traces a longer path — that is Phase 05's problem and `CLAUDE.md` records
it. Do not try to solve it here.

### `increment_towards`

```
if current * target < 0 { current = 0 }          // instant turnaround
next = current + dt * sign(target) * ACCEL_GROUND
if |next| > |target| { next = target }
```

The turnaround branch is why a body changing direction does not have to decelerate through its old
speed first. Test it directly; it is one comparison and it is easy to leave out.

### `move_aerial`

§4.1. Accelerate `self_imposed_velocity.x` by `ACCEL_AIR`, **then** apply the drag to the result.
`external_velocity` is not touched.

### `jump`

Exactly this, in this order:

```
normal   = if grounded { platform.normal_at(position) } else { UP }
horizontal = FVec2::new(normal.x, ZERO)
b        = clamp((normal.y + 1) / JUMP_NORMAL_SCALE_FACTOR, 0, 1)
up       = UP * b + horizontal * JUMP_EXTRA_X_STRENGTH
tangent  = FVec2::new(normal.y, -normal.x) * grounded_speed
self_imposed_velocity = tangent * JUMP_KEPT_MOMENTUM + up * JUMP_STRENGTH
position += self_imposed_velocity * JUMP_EXTRA_TELEPORT_FACTOR
unground(...)
```

Three things that are easy to get subtly wrong:

- **`b` scales the jump by how upward-facing the surface is**, so a jump off flat ground is full
  strength and a jump off a ceiling is nothing. On a flat top `normal.y` is 1, so `b` is 1.
- **The tangent term is kept, not discarded.** A running jump carries its speed. This is most of
  what makes movement feel good.
- **The teleport nudge is not a hack to delete.** It is what stops the jump being eaten by the
  ground check on the same tick. `unground`'s own nudge along the normal handles the re-attach; this
  one makes the jump visibly leave the ground on its first frame. Both exist, both are needed.

`unground` already clears `grounded`, nudges along the normal, and zeroes `self_imposed_velocity` —
so **call it before writing the new velocity, or change it**. Read it before you use it. If you
change its contract, say so; Phase 03's tests depend on the current one.

### Walking off the end

`Platform::surface_point` returns `None` once `local_pos` leaves the top face, and
`player_physics::step` currently reacts by clearing `grounded` and nothing else. That was correct
when nothing could walk; it is not correct now. A body that walks off a ledge should leave with the
speed it had, as horizontal velocity, rather than dropping straight down.

Convert `grounded_speed` along the surface tangent into `self_imposed_velocity` at that moment.

### Rendering

The debug overlay is behind F1 already. Add the grounded speed and the walk direction to it if it
helps you; it is a debugging tool, so it is yours to extend. Nothing render-side may become
simulation state.

---

## 8. Validation plan

### Ground movement

1. **Acceleration timing.** From rest, holding one direction, `grounded_speed` reaches `MAX_SPEED`
   at **tick 13**. Pin the tick, not just the limit. §4.3.
2. **Friction timing.** Releasing input, speed falls below a quarter of `MAX_SPEED` in **4 ticks**
   on normal ground and **20 ticks** on ice. Pin both; they are 2 and 10 on the unconverted
   constants. §4.2.
3. **`increment_towards` zeroes speed on reversal** — the `current * target < 0` branch, tested
   directly.
4. **`perp_cw` is the right way round**: on a flat top, a stick held right moves the body in +x.
5. **Walking off the end ungrounds and carries momentum** — the body leaves with horizontal speed,
   not straight down.

### Air

6. **The air speed cap is exactly `MAX_SPEED`**, and holding a direction forever does not exceed
   it. §4.1.
7. **The cap is independent of `ACCEL_AIR`** — run the same convergence with a different
   coefficient and assert the same cap. This is the test that catches the ordering mistake, and it
   is the whole reason §4.1 exists.
8. **Air control is weaker than ground control**, by comparing the tick counts.
9. **`external_velocity` is untouched by air drag.**

### Jump

10. **Apex height is pinned to an exact fixed-point value.** From flat ground, standing still, the
    apex is **2.598273 units (raw `11159497521`) at tick 24**, landing back at tick 57. Recompute
    this from your own implementation and pin whatever it actually produces — but if it is far from
    2.6, something is wrong. **This is the regression alarm for every later change to gravity or the
    tick order**, and it is the single most valuable test in the phase.
11. **A standing jump from flat ground gives `self_imposed_velocity.y == JUMP_STRENGTH`** and no
    horizontal component.
12. **A running jump carries horizontal speed**, pinned exactly.
13. **A held jump button produces exactly one jump** — hold it for 60 ticks across a landing and
    assert one takeoff.
14. **Jumping off a rotated platform is weaker and angled**, which is `b` and the horizontal term
    doing their job. The platform may be rotated; nothing walks around it.

### Determinism

15. **`state_hash` covers `grounded_speed` and `jump_held`** — one new variant each in
    `every_entity_field_reaches_the_checksum`.
16. **Same inputs, same result**; different inputs, different result. The existing tests cover the
    shape, but they now have something real to move.
17. **The replay checksum** in `tests/determinism.rs` is re-pinned, with the reason recorded next to
    the value as the existing note does. Consider giving the replay a body that actually walks and
    jumps rather than only falling.

### Manual

18. Walk, jump, land, walk off an edge, land again. Toggle F1 and watch the normal and the rays.
    Confirm a held jump does not bunny-hop.
19. Confirm ice is visibly slipperier than normal ground.
20. **Thirty seconds owed since Phase 02:** confirm by hand that the two keyboard players do not
    steal each other's keys. There is a test proving the bindings do not overlap, but no human has
    pressed a key. Now that something moves, it is worth doing.

---

## 9. Known traps

- **The air cap ordering.** §4.1. The most likely way this phase ships a silently wrong feel.
- **Friction converted by halving instead of square root.** §4.2, and the obvious test misses it.
- **A flipped `perp_cw`.** Inverts "forward" for everything that walks, and looks like an input bug.
- **A held jump bunny-hopping**, because the edge trigger reads this tick's button instead of the
  change since last tick.
- **A jump eaten by the ground check on the same tick.** Both nudges exist for this. If you remove
  either, the jump sticks to the floor and it will look like a Phase 03 bug.
- **Deleting the second clause of the direction test** because it appears to do nothing. It does
  nothing *yet*.
- **`unground` zeroing the velocity you just set**, because it was called after the jump wrote it.
- **Air drag eating knockback**, by touching `external_velocity`.
- **A new state field missing from `state_hash`.** Invisible until Phase 08.
- **Tuning a constant to fix a feel problem that is really an algorithm problem.** If it feels
  wrong, check the algorithm first. Record any deviation you do make, with the reason.

---

## 10. Deferred work and which phase owns it

| Phase | Owns |
|---|---|
| 05 | Surface walking: walls, ceilings, corners, rotation behaviour. **The centre-path perimeter decision** — see `CLAUDE.md`. Ice friction as a surface property rather than a coefficient. Moving platforms. |
| 06 | Camera behaviour, render-only float constants, tick-to-frame interpolation using the alpha the driver already exposes. |
| 07 | Collision and knockback; `external_velocity` starts being written. A destroy path for entities. Damage source as `Option<PlayerId>` or a small enum, never a sentinel in the id range. |
| 08 | The determinism harness and the golden cross-platform checksum. |
| 09 | Lockstep netcode. It replaces the driver's input-gathering step and nothing else. |
| 10 | Session, rounds, spawn points, per-level gravity (`GRAVITY_MODIFIER`), and the real input backend including gamepad enumeration and `stick_magnitude`. |
| 11 | Abilities. |

---

## 11. Completion criteria

- [ ] A body walks along a flat platform under stick input, and stops when it is released.
- [ ] Acceleration and friction timings are pinned by tick count, and would fail on an unconverted
      constant.
- [ ] Ice is measurably and visibly slipperier.
- [ ] The air speed cap is exactly `MAX_SPEED`, by test, and independent of `ACCEL_AIR`.
- [ ] The jump apex is pinned to an exact value.
- [ ] A running jump carries horizontal speed; a standing jump does not.
- [ ] A held jump button produces exactly one jump.
- [ ] Walking off an edge ungrounds cleanly and carries momentum.
- [ ] Landing from a jump re-attaches.
- [ ] `grounded_speed` and `jump_held` are in `state_hash`, with a test that proves it.
- [ ] `self_imposed_velocity` and `external_velocity` remain distinct; the latter stays zero.
- [ ] One `step` is still exactly one tick, hit-stop included.
- [ ] Nothing in this phase walks on a wall, ceiling or corner.
- [ ] `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` all pass.
- [ ] CI green on Linux and macOS.
