# RMOPL Battle

An original 2D multiplayer physics brawler in Rust. Deterministic fixed-point simulation,
peer-hosted lockstep netcode, mixed local and online players in one match, primitive geometry
visuals with no art assets.

Crate: `rmopl`. Native-first: macOS, Windows, Linux.

---

## Invariants — every phase, no exceptions

1. **No `f32`/`f64` in simulation code.** Rendering and the camera may use floats. A float in a
   type that feeds `Sim::step` is a bug.
2. **The simulation is a pure function of (previous state, inputs for this tick).** No wall-clock
   time, no `rand()`, no `HashMap` iteration order, no threads inside the sim.
3. **Deterministic iteration order everywhere.** Sort by a stable id when order could vary.
4. **RNG is seeded per match and advanced only inside the sim tick.**
5. **`PlayerId` is not `PeerId` is not `DeviceId` is not `EntityId`.** Separate newtypes, no `From`
   impls, never converted with `as`.
6. **Input is the only thing that crosses the network.** If you want to send game state, the design
   is wrong — say so instead of sending it.
7. **No `unwrap()` on anything derived from network or file input.**
8. **The simulation tick rate is fixed at 120 Hz and never coupled to the frame rate.** A rendered
   frame may drive zero, one, or several ticks. "One tick per frame" is a bug, and so is any sim
   behaviour that changes at 60 vs 144 vs 240 FPS.
9. **Authoritative elapsed time is a `u64` tick count**, never a running sum of `FIXED_DELTA_TIME`.
   1/120 is not representable in Q32.32, so any seconds value derived from it is systematically
   short. Converting ticks to seconds is a presentation step, never a simulation one.
10. **Logical players, local players, peers and host status are four independent quantities.**
    Never size a buffer, a loop or an estimate by one when the real quantity is another.
11. `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` all pass before a phase
    is done. Never silence a lint without a written justification.

## Product decisions

| | Decision |
|---|---|
| **120 TPS** | Authoritative simulation runs at exactly 120 fixed ticks per second. Rendering runs at whatever rate the display allows and never changes the number or order of sim ticks. |
| **16 players** | Up to 16 simultaneous players, in any mix of local and online. Player count and machine count are independent: one machine may hold twelve couch players while another holds one. |
| **One shared camera** | All players on a machine see the same world through **one shared gameplay view**, in the spirit of Worms. **There is no split-screen, at any local player count, ever.** Gameplay rendering is always one viewport. Never solve a large local-player configuration by adding viewports; zoom, framing and readability with many players are camera and game-design problems. |
| **Up to 16 local players** | on a single machine is a product target. Never bake in a smaller ceiling — in particular, **do not treat XInput's four-controller limit as the game's limit.** |
| **Q32.32 fixed point** | The simulation uses `math::Fix` throughout. Precondition for the netcode. |
| **No third-party physics engine** | Float-based engines cannot interoperate with a fixed-point sim. The collision code needed here is small and is being written anyway. |

## Non-goals

Host migration, mod runtimes and scripting, online join-in-progress, Steam integration, and
matchmaking are all deliberately out of scope. Local join mid-match is in scope. Note them as debt;
do not build them.

---

## Phase 03 API — what exists today

- **`platform`** — `PlatformShape { extents, radius, kind }` is what a spawn carries;
  `Platform { center, extents, radius, rotation, kind }` is what queries run against, built on
  demand by `Entity::platform()` from the entity's own position and rotation so a centre is never
  stored twice. `PlatformKind::{Normal, Ice}` with a stable `tag()`. **`extents` excludes the
  corner radius.**
  Queries: `right()`, `up()`, `to_local()`, `to_world()`, `normal_at()` (all eight zones),
  `perimeter()`, `top_face_end()`, `surface_point()`, `top_face_local_pos()`,
  `top_face_crossing()`, `top_face_point()`, `local_pos_of_top_x()`.
- **`sim::Entity`** — `position`, `self_imposed_velocity`, `external_velocity`, `rotation`,
  `scale`, `owner`, `shape`, `grounded`, plus `Entity::platform()`.
- **`sim::Grounded { platform: EntityId, local_pos: Fix }`** — where a body is standing.
- **`sim::Spawn`** — gained `self_imposed_velocity`, `rotation`, `scale`, `platform`. Use
  `Spawn::BODY` as the base: `Spawn { position, ..Spawn::BODY }`.
- **`player_physics`** — `add_gravity`, `velocity_based_raycasts` returning `GroundHit`,
  `attach_to_ground`, `unground`, and `step`, which runs from tick slot 5.

### Rules this phase established that later phases inherit

- **Surface positions run clockwise from the left end of the top face.** The top face is segment
  zero, spanning `[0, top_face_end()]`, so moving right along the ground increases the coordinate.
  The other seven segments are Phase 05 and currently answer `None`.
- **`local_pos` is a fraction of the platform's own perimeter**, and the body's centre is then
  offset outward by `RADIUS` along the normal. That is not the only possible parameterisation:
  walking round a corner means the centre traces a longer path than the surface does, so Phase 05
  must either divide by a centre-path perimeter (`perimeter() + 2*PI*RADIUS`) or account for the
  difference at the corners. On a flat face the two agree, which is why Phase 04 can ignore it.
  **Phase 05 owns that decision.**
- **Zone boundaries resolve in favour of the flat faces**, and a landing past the end of the flat
  top attaches at the end. That is what stops `normal_at` and the landing query from disagreeing
  about a body sitting exactly on a corner. It means a body can catch a ledge from up to one radius
  past it; Phase 05 replaces that with the arc.
- **Three ground rays, not one**, run `|v| * dt + RADIUS` from the body's centre and offset
  `± RADIUS` perpendicular to the direction of travel. The extension by the radius is what makes a
  body land when its underside touches rather than when its centre arrives. Ties resolve by
  platform slotmap order, then centre ray, then the two offsets.
- **A grounded platform can stop existing.** The lookup returns `Option` and is handled; the
  generational key fails rather than addressing whatever took the slot. Never unwrap it.
- **Optional fields hash to a fixed number of words** whether present or not, so no arrangement of
  entities can make two different worlds produce the same word stream.
- **An unrotated platform uses exact axis vectors** rather than CORDIC, which would leave every
  landing on every axis-aligned platform a few raw units askew.

### Accuracy facts you must not rediscover the hard way

- **The terminal-velocity test does not guard the tick rate.** The drag term is derived from the
  acceleration, so `g == v * (g / v_max)` solves to `v_max` for every `g`: the halved and unhalved
  constants converge to the same limit and differ only in how long they take. The tests that guard
  the conversion measure the approach — the tick a fraction of terminal is reached, and the distance
  fallen — never the limit.
- **Terminal velocity settles 27 raw units *below* `GRAVITY_MAX_FALL_SPEED`**, at tick 740, because
  the last increments underflow the multiply. That is arithmetic, not tuning, and it is pinned
  rather than snapped away.
- **A grounded body's derived world position is about 7e-9 off** its landing position: turning a
  distance along the perimeter into a fraction and back truncates. It does not accumulate. Assert
  positions with a tolerance, not bit-exactly.
- **`Fix::frac()` has floor semantics** — `frac(-1.25)` is `0.75` — so a surface position wraps into
  `[0, 1)` correctly in both directions with no guard.
- **A local/world round trip on a rotated platform is not exact.** The basis is a few raw units off
  unit length and the error scales with distance from the centre. Unrotated platforms do round-trip
  exactly.

## Phase 02 API — what exists today

- **`ids`** — `PeerId(u16)`, `PlayerId(u8)`, `DeviceId(u32)`, `Tick(u64)` and `EntityId` (a slotmap
  key). No conversions between them, by construction; the module's doctests are compile-fail cases
  proving so. `Player { id, peer, device: Option<DeviceId>, entity: Option<EntityId> }` and
  `Roster`, which keeps players sorted by id and can answer `of_peer`.
- **`input::ActionState`** — one stick and six buttons; the whole game's input and the wire format.
  `with_stick` applies `INPUT_DEADZONE` and quantises to a byte of angle plus a byte of magnitude.
  `KEYBOARD_BINDINGS` and `poll_keyboard` cover two keyboard players.
- **`sim::Sim`** — `new(seed)`, `step(&[(PlayerId, ActionState)])`, `request_spawn`, `state_hash`,
  and read-only accessors. `Pcg32`, `Spawn`, `Entity`.
- **`sim::Accumulator`** — `ticks_due(frame_nanos)` and `alpha()`. Driver-side, not simulation
  state.

### Rules this phase established that later phases inherit

- **`Sim::step` is the only mutation path**, and it reads nothing but its arguments. Anything that
  wants to change the world does it from inside a tick.
- **Inputs are sorted inside `step`**, so the caller may hand them over in arrival order.
- **Spawn ordering keys are a total order over the spawn's fields**, never a counter, an arrival
  index, or a hash. A counter would decide entity ids before the sort ran, so two peers that
  generated the same spawns in a different order would build different worlds. A hash is not
  injective, so two distinct spawns could collide and hand the tie back to arrival order — the same
  bug, hidden in the one case nobody thinks to test. Compare the fields themselves.
- **Hit-stop freezes the world but still advances the tick**, so the tick count and the number of
  `step` calls never come apart. The netcode indexes inputs by tick.
- **The action set carries a magnitude byte that nothing reads yet.** It is reserved rather than
  used: adding it after the netcode ships would mean changing the wire format on every peer at once.
- **`ActionState` stays ability-agnostic and three bytes.** Every bandwidth figure scales with it.
- **`MAX_TICKS_PER_FRAME` bounds catch-up.** A stalled machine runs briefly in slow motion instead
  of spiralling.

## Phase 01 API — what exists today

- **`math::Fix`** — `fixed::types::I32F32`. Build constants with `Fix::lit("1.6")`, which is const
  and keeps float literals out of the source.
- **`math::FVec2`** — `Add`, `Sub`, `Neg`, `Mul<Fix>`, `Div<Fix>` and assigning forms; `dot`,
  `sqr_magnitude`, `magnitude`, `distance`, `normalized_safe`, `perp` (90 deg CCW), `perp_cw`;
  consts `ZERO`, `ONE`, `UP`, `DOWN`, `LEFT`, `RIGHT`.
- **`math::sqrt`** — exact truncated root via 128-bit integer isqrt. Returns zero for negative
  input rather than panicking.
- **`math::sin` / `cos` / `atan2`** — integer CORDIC, deterministic across platforms.
- **`constants`** — tick rate, physics tuning, gameplay, level bounds, session values.

### Accuracy facts you must not rediscover the hard way

- `atan2` answers a near-vertical vector directly rather than through CORDIC, which would form
  `y / x` and panic on overflow once `x` is small. A stick held straight up hits this, so it is
  ordinary input, not a corner case. Fixed at the root in `math::atan2`; call sites need no guard.
- `sin` and `cos` are within **32 raw units** (`math::TRIG_ERROR_BOUND`) of the true value and may
  land up to **2 raw units outside -1..=1**. Anything feeding one into `acos`, `sqrt(1 - x^2)` or a
  similar domain-restricted operation **must clamp at that call site**. There is deliberately no
  global clamp.
- `normalized_safe` returns components up to 1 raw unit low; `sqr_magnitude` of a normalised vector
  is up to 8 units under one. Always under, never over — safe for a later square root, but do not
  assume exact round-tripping.
- `FIXED_DELTA_TIME * 120` is 16 raw units short of one, and no fixed-point value can do better.
  This is why invariant 9 exists.
- Several physics constants were tuned against a 60 Hz tick and are applied per tick without a
  delta-time factor. They fall into groups that convert **differently** — additions halve,
  multipliers take a square root, delta-time-scaled values need nothing. The classification is in
  `src/constants.rs`; read it before using any of them in a per-tick update.

---

## Working method

Work one phase at a time. The current phase brief is in `docs/`. Before starting, verify the actual
repo state and report: Objective / Current state verified / Planned changes / Validation / Risks /
Questions with your recommended answers. Then **stop and wait for approval.** After the phase,
report: Result / Changes / Validation actually run with real pasted output / Known limitations /
Deferred debt / Next phase. Then **stop.**

Do not expand a phase without saying so. Record unrelated technical debt instead of fixing it.
Investigate before asking; when uncertain, investigate; when testable, test; when measurable,
measure.

**Versions.** Every crate, toolchain and CI action is the newest stable release when added. Check
crates.io before writing a version requirement. Never a bare major. CI updates rustup itself rather
than trusting the runner image, because the images lag.

**Git.** Branch per phase (`feat/phase-05-surface-walking`). Atomic commits, English, written like a
human engineer wrote them. Never `fix`, `wip`, `update`, `stuff`. Never force-push. Never mix
unrelated work.

**Never** fabricate repo state, test results or benchmark numbers. Never claim something works
without having run it. Never hide a warning or a failure. Never trust bytes off the network. Never
send simulation state over the wire.
