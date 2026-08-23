//! The driver: collect input, advance the simulation, draw the result.
//!
//! The split matters more than anything it currently draws. Gathering input,
//! deciding how many ticks are owed, and running them are three separate
//! things, because the netcode replaces exactly one of them: local input
//! becomes the confirmed inputs for a tick, arriving from the host. Everything
//! else here stays as it is. If that swap ever looks like it needs changes
//! elsewhere in this file, the seam has been eroded and wants repairing before
//! the netcode lands on top of it.

use macroquad::prelude::*;

use rmopl::constants::{FIXED_DELTA_TIME, RADIUS};
use rmopl::ids::Roster;
use rmopl::ids::{DeviceId, PeerId, Player, PlayerId};
use rmopl::input::{ActionState, KEYBOARD_BINDINGS, poll_keyboard};
use rmopl::math::{FVec2, Fix};
use rmopl::platform::{Platform, PlatformKind, PlatformShape};
use rmopl::sim::{Accumulator, Entity, Sim, Spawn};

const BACKGROUND: Color = Color::new(0.07, 0.07, 0.09, 1.0);
const NANOS_PER_SECOND: f64 = 1_000_000_000.0;

/// How many pixels one world unit is drawn as.
///
/// A fixed view. Framing, zoom and following the action are camera work and
/// belong to their own phase; this exists so there is something to look at.
const PIXELS_PER_UNIT: f32 = 22.0;
/// World point drawn at the centre of the window.
const VIEW_CENTER: (f32, f32) = (0.0, 2.0);

/// Simulation coordinates are fixed-point; screen coordinates are floats.
///
/// This function is the boundary, and it only ever runs in one direction.
/// Nothing on the right-hand side of it can reach [`Sim::step`].
fn to_screen(point: FVec2) -> Vec2 {
    let x: f32 = point.x.to_num();
    let y: f32 = point.y.to_num();
    vec2(
        screen_width() / 2.0 + (x - VIEW_CENTER.0) * PIXELS_PER_UNIT,
        // Screen y grows downwards and world y grows upwards.
        screen_height() / 2.0 - (y - VIEW_CENTER.1) * PIXELS_PER_UNIT,
    )
}

/// The players sitting at this machine, and the keyboard each one uses.
///
/// Local players are built here and nowhere else. There is one peer — this
/// machine — holding all of them, which is the arrangement the identity model
/// exists for: player count and machine count are unrelated numbers.
fn local_roster() -> (Roster, Vec<(PlayerId, usize)>) {
    let this_machine = PeerId::new(0);
    let mut roster = Roster::new();
    let mut keyboards = Vec::new();

    for (slot, _) in KEYBOARD_BINDINGS.iter().enumerate() {
        let id = PlayerId::new(slot as u8 + 1);
        let mut player = Player::new(id, this_machine);
        player.device = Some(DeviceId::new(slot as u32));
        roster.insert(player);
        keyboards.push((id, slot));
    }

    (roster, keyboards)
}

/// Reads every local player's device once, for one tick.
///
/// This is the function the netcode replaces. Its shape is the contract: given
/// a tick, produce the inputs for it. Where they came from is nobody else's
/// business.
fn gather_local_inputs(keyboards: &[(PlayerId, usize)], out: &mut Vec<(PlayerId, ActionState)>) {
    out.clear();
    for &(id, slot) in keyboards {
        match KEYBOARD_BINDINGS.get(slot) {
            Some(binding) => out.push((id, poll_keyboard(binding))),
            // A device that vanished leaves the player in place, holding
            // nothing. They keep their entity and simply stop acting.
            None => out.push((id, ActionState::NEUTRAL)),
        }
    }
}

/// A few platforms and a few bodies to drop onto them.
///
/// Hard-coded here rather than loaded: levels, spawn points and rounds are a
/// later phase, and this is the driver's business until then.
fn build_scene(sim: &mut Sim) {
    let platform = |x: &str, y: &str, rotation: &str, ex: &str, ey: &str, kind| Spawn {
        position: FVec2::new(Fix::lit(x), Fix::lit(y)),
        rotation: Fix::lit(rotation),
        platform: Some(PlatformShape {
            extents: FVec2::new(Fix::lit(ex), Fix::lit(ey)),
            radius: Fix::lit("0.4"),
            kind,
        }),
        ..Spawn::BODY
    };

    for spawn in [
        platform("0", "-6", "0", "12", "0.8", PlatformKind::Normal),
        platform("-11", "0", "0.25", "4", "0.5", PlatformKind::Normal),
        platform("10", "1", "-0.3", "3.5", "0.5", PlatformKind::Ice),
        platform("0", "3", "0", "2.5", "0.4", PlatformKind::Normal),
    ] {
        sim.request_spawn(spawn);
    }

    for (slot, (x, y)) in [("0.5", "12"), ("-11", "14"), ("10", "16"), ("6", "11")]
        .into_iter()
        .enumerate()
    {
        sim.request_spawn(Spawn {
            position: FVec2::new(Fix::lit(x), Fix::lit(y)),
            owner: Some(PlayerId::new(slot as u8 + 1)),
            ..Spawn::BODY
        });
    }
}

#[macroquad::main("RMOPL Battle")]
async fn main() {
    let (roster, keyboards) = local_roster();
    let mut sim = Sim::new(0);
    build_scene(&mut sim);
    let mut overlay = false;
    let mut accumulator = Accumulator::new();
    let mut inputs = Vec::new();

    while !is_key_pressed(KeyCode::Escape) {
        if is_key_pressed(KeyCode::F1) {
            overlay = !overlay;
        }

        // How much time has passed. Under lockstep this stops being the whole
        // answer — the simulation may also be waiting on inputs that have not
        // arrived — which is why the question is asked separately from the
        // running of the ticks below.
        let frame_nanos = (get_frame_time() as f64 * NANOS_PER_SECOND) as u64;
        let due = accumulator.ticks_due(frame_nanos);

        for _ in 0..due {
            gather_local_inputs(&keyboards, &mut inputs);
            sim.step(&inputs);
        }

        // Available for rendering between two ticks. Nothing consumes it yet;
        // interpolation belongs with the camera work.
        let _alpha = accumulator.alpha();

        clear_background(BACKGROUND);
        draw_world(&sim, overlay);
        draw_state(&sim, &roster, overlay);
        next_frame().await;
    }
}

/// Draws what the simulation currently holds.
///
/// One shared view of the world for everyone at this machine, whatever the
/// local player count. Rendering only ever reads.
fn draw_state(sim: &Sim, roster: &Roster, overlay: bool) {
    let grounded = sim.entities().filter(|(_, e)| e.grounded.is_some()).count();
    let lines = [
        format!("tick {}", sim.tick().get()),
        format!("players {}", roster.len()),
        format!("entities {}", sim.entity_count()),
        format!("grounded {grounded}"),
        format!("F1 overlay {}", if overlay { "on" } else { "off" }),
    ];

    for (row, line) in lines.iter().enumerate() {
        draw_text(line, 20.0, 40.0 + row as f32 * 26.0, 24.0, LIGHTGRAY);
    }

    for (player, action) in sim.inputs() {
        let row = lines.len() + player.raw() as usize;
        let stick = match action.stick_radians() {
            Some(radians) => format!("{:.2} rad @ {}", radians, action.stick_magnitude),
            None => "neutral".to_string(),
        };
        draw_text(
            format!(
                "player {} {stick}{}",
                player.raw(),
                if action.jump { " jump" } else { "" }
            ),
            20.0,
            40.0 + row as f32 * 26.0,
            24.0,
            SKYBLUE,
        );
    }
}

/// Draws the world: platforms, then bodies, then the overlay on top.
fn draw_world(sim: &Sim, overlay: bool) {
    for (_, entity) in sim.entities() {
        if let Some(platform) = entity.platform() {
            draw_platform(&platform);
        }
    }

    for (_, entity) in sim.entities() {
        if entity.shape.is_none() {
            draw_body(entity);
        }
    }

    if overlay {
        for (_, entity) in sim.entities() {
            if entity.shape.is_none() {
                draw_body_overlay(sim, entity);
            }
        }
    }
}

/// A rounded rectangle: the inner rectangle grown by the corner radius along
/// each axis in turn, plus a disc at each corner.
///
/// Two overlapping rectangles and four circles, which is the whole shape and
/// needs no mesh. The extents exclude the radius, so growing by it is exactly
/// what the collision code means by the same numbers.
fn draw_platform(platform: &Platform) {
    let colour = match platform.kind {
        PlatformKind::Normal => Color::new(0.36, 0.38, 0.46, 1.0),
        PlatformKind::Ice => Color::new(0.44, 0.62, 0.76, 1.0),
    };

    let center = to_screen(platform.center);
    let rotation: f32 = platform.rotation.to_num();
    let half_width: f32 = platform.extents.x.to_num::<f32>() * PIXELS_PER_UNIT;
    let half_height: f32 = platform.extents.y.to_num::<f32>() * PIXELS_PER_UNIT;
    let radius: f32 = platform.radius.to_num::<f32>() * PIXELS_PER_UNIT;

    // World y grows upwards and screen y downwards, so a rotation that is
    // counter-clockwise in the simulation is clockwise on screen.
    let params = DrawRectangleParams {
        offset: vec2(0.5, 0.5),
        rotation: -rotation,
        color: colour,
    };
    draw_rectangle_ex(
        center.x,
        center.y,
        (half_width + radius) * 2.0,
        half_height * 2.0,
        params.clone(),
    );
    draw_rectangle_ex(
        center.x,
        center.y,
        half_width * 2.0,
        (half_height + radius) * 2.0,
        params,
    );

    if radius > 0.0 {
        for (sx, sy) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
            let local = FVec2::new(platform.extents.x * sign(sx), platform.extents.y * sign(sy));
            let corner = to_screen(platform.to_world(local));
            draw_circle(corner.x, corner.y, radius, colour);
        }
    }
}

fn sign(of: f32) -> Fix {
    if of < 0.0 { Fix::NEG_ONE } else { Fix::ONE }
}

fn draw_body(entity: &Entity) {
    let colour = if entity.grounded.is_some() {
        Color::new(0.55, 0.82, 0.5, 1.0)
    } else {
        Color::new(0.93, 0.71, 0.38, 1.0)
    };
    let center = to_screen(entity.position);
    draw_circle(
        center.x,
        center.y,
        RADIUS.to_num::<f32>() * PIXELS_PER_UNIT,
        colour,
    );
}

/// The debug overlay: the three ground rays, and the surface normal under a
/// grounded body.
///
/// The rays are recomputed here from the body's public state rather than
/// recorded by the simulation. Anything the simulation stored for the sake of
/// drawing would be state two peers have to agree on, for no gameplay reason.
///
/// Worth its thirty lines: the phases after this one are geometry work whose
/// failures are invisible in a still frame.
fn draw_body_overlay(sim: &Sim, entity: &Entity) {
    const RAY: Color = Color::new(1.0, 0.35, 0.45, 0.9);
    const NORMAL: Color = Color::new(0.45, 0.95, 1.0, 1.0);

    if let Some(grounded) = entity.grounded {
        let surface = sim
            .entity(grounded.platform)
            .and_then(Entity::platform)
            .and_then(|platform| {
                platform
                    .surface_point(grounded.local_pos)
                    .map(|point| (point, platform.normal_at(point)))
            });
        if let Some((point, normal)) = surface {
            let from = to_screen(point);
            let to = to_screen(point + normal * Fix::lit("2"));
            draw_line(from.x, from.y, to.x, to.y, 2.0, NORMAL);
            draw_circle(from.x, from.y, 3.0, NORMAL);
        }
        return;
    }

    // The same three rays the physics casts: along the movement about to be
    // made, extended by the body radius, offset either side by the same.
    let velocity = entity.self_imposed_velocity + entity.external_velocity;
    let delta = velocity * FIXED_DELTA_TIME;
    let direction = delta.normalized_safe();
    if direction == FVec2::ZERO {
        return;
    }
    let reach = delta + direction * RADIUS;
    let offset = direction.perp() * RADIUS;

    for origin in [
        entity.position,
        entity.position + offset,
        entity.position - offset,
    ] {
        let from = to_screen(origin);
        let to = to_screen(origin + reach);
        draw_line(from.x, from.y, to.x, to.y, 1.5, RAY);
    }
}
