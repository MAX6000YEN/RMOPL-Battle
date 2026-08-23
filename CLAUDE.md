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
