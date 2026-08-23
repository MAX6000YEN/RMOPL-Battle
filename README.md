# RMOPL Battle

**R**usty **M**eat **O**bliteration **P**ro **L**GBTQIA+ — a 2D multiplayer physics brawler where
players walk on every surface, including walls, ceilings and each other's heads, and try to throw
one another out of the level. It is built on a deterministic fixed-point simulation with lockstep
networking, which means a match plays identically on every machine and only player inputs cross the
wire. Local players and online players share the same match: four people on one couch can play
against four people somewhere else. Visuals are primitive geometry, deliberately — this is a game
about movement, not about art.

## Building

You need a [Rust](https://rustup.rs) toolchain; nothing else. On Linux you also need X11, OpenGL and
ALSA development headers (`libx11-dev libxi-dev libgl1-mesa-dev libasound2-dev` on Debian and
Ubuntu). Then:

```sh
cargo run --release
```

Escape closes the window.

## Status

Pre-alpha. At the time of writing this repository opens a window and does nothing else. There is no
gameplay, no netcode and no release, the version number is not meaningful, and everything including
the crate layout is subject to change without notice. Do not depend on it.
