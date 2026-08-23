//! The identity model: four unrelated concepts that must never be confused.
//!
//! A machine, a logical participant, a physical input device and a thing in the
//! world have four different lifecycles. One machine hosts several couch
//! players; a player outlives the gamepad that was unplugged mid-match; a
//! spectator has no entity at all. Collapsing any two of these into one number
//! works right up until the netcode, at which point every signature in the
//! codebase is wrong.
//!
//! So they are separate newtypes with no conversions between them: no `From`
//! impls, no `as` casts, no shared integer width by coincidence. The compiler
//! is the thing enforcing this, not a convention.
//!
//! These are the mistakes, and each one is a compile error rather than a
//! runtime surprise. The doctests below fail to build on purpose; that is the
//! test.
//!
//! One id assigned to another:
//!
//! ```compile_fail
//! use rmopl::ids::{PeerId, PlayerId};
//! let peer = PeerId::new(1);
//! let player: PlayerId = peer;
//! ```
//!
//! One id compared to another:
//!
//! ```compile_fail
//! use rmopl::ids::{DeviceId, PlayerId};
//! let _ = PlayerId::new(1) == DeviceId::new(1);
//! ```
//!
//! An id passed where a different one is expected:
//!
//! ```compile_fail
//! use rmopl::ids::{PeerId, Player, PlayerId};
//! let _ = Player::new(PeerId::new(1), PlayerId::new(1));
//! ```
//!
//! An id used as the integer it happens to wrap:
//!
//! ```compile_fail
//! use rmopl::ids::Tick;
//! let _: u64 = Tick::ZERO + 1;
//! ```

use slotmap::new_key_type;

/// Defines an opaque id newtype over `$repr`.
///
/// Deliberately generates no `From`, no arithmetic and no `Default`. `raw` is
/// the only way out, and exists for the wire format and for debug output —
/// never for turning one kind of id into another.
macro_rules! id_newtype {
    ($(#[$meta:meta])* $name:ident($repr:ty)) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        pub struct $name($repr);

        impl $name {
            pub const fn new(raw: $repr) -> Self {
                Self(raw)
            }

            /// The underlying integer. For serialisation and diagnostics only.
            pub const fn raw(self) -> $repr {
                self.0
            }
        }
    };
}

id_newtype! {
    /// A machine taking part in the session. One peer may hold many players.
    PeerId(u16)
}

id_newtype! {
    /// A logical participant in a match, local or remote.
    ///
    /// Distinct from the peer that hosts it and from the entity it controls.
    /// There is no reserved value inside the range: a damage source that no
    /// player caused is modelled as `Option<PlayerId>` or an enum, never as a
    /// sentinel id, because a sentinel inside the value range is exactly the
    /// class of bug this module exists to prevent.
    PlayerId(u8)
}

id_newtype! {
    /// A physical input device. Comes and goes independently of the player
    /// using it.
    DeviceId(u32)
}

new_key_type! {
    /// A thing in the world. Generational, so a stale id never silently
    /// addresses whatever was allocated in the same slot afterwards.
    pub struct EntityId;
}

/// A simulation tick number, and the only authoritative measure of elapsed
/// time in the project.
///
/// Sixty-four bits because seconds are never accumulated: converting ticks to
/// seconds is a presentation step, and a `u32` would wrap after about 414 days
/// of continuous simulation at 120 Hz.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Tick(u64);

impl Tick {
    pub const ZERO: Self = Self(0);

    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// A logical participant and everything currently attached to it.
///
/// Both attachments are optional and both are normal. A player whose gamepad
/// was unplugged keeps their entity standing in the world; a player waiting to
/// spawn, or spectating, has no entity. Neither is an error state, so neither
/// is asserted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Player {
    pub id: PlayerId,
    /// The machine this player sits at. Several players share one peer.
    pub peer: PeerId,
    /// `None` while no device is attached.
    pub device: Option<DeviceId>,
    /// `None` while the player has nothing in the world.
    pub entity: Option<EntityId>,
}

impl Player {
    pub const fn new(id: PlayerId, peer: PeerId) -> Self {
        Self {
            id,
            peer,
            device: None,
            entity: None,
        }
    }
}

/// Every player in the match, kept sorted by [`PlayerId`].
///
/// Sorted because iteration order feeds the simulation, and anything the
/// simulation iterates must have an order that does not depend on insertion
/// history or on a hash seed.
#[derive(Clone, Default, Debug)]
pub struct Roster {
    players: Vec<Player>,
}

impl Roster {
    pub const fn new() -> Self {
        Self {
            players: Vec::new(),
        }
    }

    /// Adds a player, or replaces the existing one with the same id.
    pub fn insert(&mut self, player: Player) {
        match self.players.binary_search_by_key(&player.id, |p| p.id) {
            Ok(at) => self.players[at] = player,
            Err(at) => self.players.insert(at, player),
        }
    }

    pub fn remove(&mut self, id: PlayerId) -> Option<Player> {
        let at = self.players.binary_search_by_key(&id, |p| p.id).ok()?;
        Some(self.players.remove(at))
    }

    pub fn get(&self, id: PlayerId) -> Option<&Player> {
        let at = self.players.binary_search_by_key(&id, |p| p.id).ok()?;
        self.players.get(at)
    }

    pub fn get_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        let at = self.players.binary_search_by_key(&id, |p| p.id).ok()?;
        self.players.get_mut(at)
    }

    /// All players, ascending by id.
    pub fn iter(&self) -> impl Iterator<Item = &Player> {
        self.players.iter()
    }

    /// The players hosted by one machine, ascending by id. Usually more than
    /// one: that is the whole point of separating the two id types.
    pub fn of_peer(&self, peer: PeerId) -> impl Iterator<Item = &Player> {
        self.players.iter().filter(move |p| p.peer == peer)
    }

    pub fn len(&self) -> usize {
        self.players.len()
    }

    pub fn is_empty(&self) -> bool {
        self.players.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::MAX_PLAYERS;

    /// The mapping the whole identity model exists to express: one machine,
    /// several players, each with their own device and entity.
    #[test]
    fn one_peer_holds_several_players() {
        let laptop = PeerId::new(1);
        let desktop = PeerId::new(2);

        let mut roster = Roster::new();
        for (n, peer) in [laptop, laptop, desktop, desktop, desktop]
            .into_iter()
            .enumerate()
        {
            let mut player = Player::new(PlayerId::new(n as u8 + 1), peer);
            player.device = Some(DeviceId::new(n as u32 + 100));
            roster.insert(player);
        }

        assert_eq!(roster.of_peer(laptop).count(), 2);
        assert_eq!(roster.of_peer(desktop).count(), 3);
        assert_eq!(roster.len(), 5);

        // Two players on one machine still hold two distinct devices.
        let laptop_devices: Vec<_> = roster.of_peer(laptop).map(|p| p.device).collect();
        assert_eq!(
            laptop_devices,
            vec![Some(DeviceId::new(100)), Some(DeviceId::new(101))]
        );
    }

    /// Unplugging a controller must not delete the player or its entity. The
    /// character keeps standing in the world; it simply stops receiving input.
    #[test]
    fn a_player_outlives_its_device() {
        let mut roster = Roster::new();
        let id = PlayerId::new(1);
        let mut player = Player::new(id, PeerId::new(1));
        player.device = Some(DeviceId::new(7));
        player.entity = Some(EntityId::default());
        roster.insert(player);

        let entity_before = roster.get(id).and_then(|p| p.entity);
        roster.get_mut(id).expect("player was inserted").device = None;

        let after = roster.get(id).expect("player must survive the unplug");
        assert_eq!(after.device, None);
        assert_eq!(after.entity, entity_before);
        assert_eq!(roster.len(), 1);
    }

    /// A spectator, or a player between rounds, holds no entity at all.
    #[test]
    fn a_player_without_an_entity_is_valid() {
        let player = Player::new(PlayerId::new(3), PeerId::new(1));
        assert_eq!(player.entity, None);
        assert_eq!(player.device, None);
    }

    /// Iteration order must come from the ids, never from insertion order.
    #[test]
    fn roster_iteration_is_sorted_by_player_id() {
        let peer = PeerId::new(1);
        let mut roster = Roster::new();
        for n in [9u8, 2, 14, 1, 7] {
            roster.insert(Player::new(PlayerId::new(n), peer));
        }

        let ids: Vec<u8> = roster.iter().map(|p| p.id.raw()).collect();
        assert_eq!(ids, vec![1, 2, 7, 9, 14]);
    }

    #[test]
    fn inserting_the_same_id_replaces_rather_than_duplicates() {
        let mut roster = Roster::new();
        let id = PlayerId::new(4);
        roster.insert(Player::new(id, PeerId::new(1)));
        roster.insert(Player::new(id, PeerId::new(2)));

        assert_eq!(roster.len(), 1);
        assert_eq!(roster.get(id).map(|p| p.peer), Some(PeerId::new(2)));
    }

    /// The product target is sixteen simultaneous players in any mix of local
    /// and online, so the roster must hold that many without a peer count
    /// having anything to do with it.
    #[test]
    fn the_roster_holds_the_full_player_count_on_a_single_peer() {
        let peer = PeerId::new(1);
        let mut roster = Roster::new();
        for n in 1..=MAX_PLAYERS {
            roster.insert(Player::new(PlayerId::new(n as u8), peer));
        }

        assert_eq!(roster.len(), MAX_PLAYERS);
        assert_eq!(roster.of_peer(peer).count(), MAX_PLAYERS);
    }

    #[test]
    fn ticks_count_up_exactly() {
        let mut tick = Tick::ZERO;
        for _ in 0..600 {
            tick = tick.next();
        }
        assert_eq!(tick.get(), 600);
        assert_eq!(tick, Tick::new(600));
        assert!(Tick::new(1) < Tick::new(2));
    }
}
