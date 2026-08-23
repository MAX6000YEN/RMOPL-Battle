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

use rmopl::ids::Roster;
use rmopl::ids::{DeviceId, PeerId, Player, PlayerId};
use rmopl::input::{ActionState, KEYBOARD_BINDINGS, poll_keyboard};
use rmopl::sim::{Accumulator, Sim};

const BACKGROUND: Color = Color::new(0.07, 0.07, 0.09, 1.0);
const NANOS_PER_SECOND: f64 = 1_000_000_000.0;

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

#[macroquad::main("RMOPL Battle")]
async fn main() {
    let (roster, keyboards) = local_roster();
    let mut sim = Sim::new(0);
    let mut accumulator = Accumulator::new();
    let mut inputs = Vec::new();

    while !is_key_pressed(KeyCode::Escape) {
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
        draw_state(&sim, &roster);
        next_frame().await;
    }
}

/// Draws what the simulation currently holds.
///
/// One shared view of the world for everyone at this machine, whatever the
/// local player count. Rendering only ever reads.
fn draw_state(sim: &Sim, roster: &Roster) {
    let lines = [
        format!("tick {}", sim.tick().get()),
        format!("players {}", roster.len()),
        format!("entities {}", sim.entity_count()),
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
