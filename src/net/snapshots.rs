//! Building the world state a client is sent: the players it can see, plus the props whose pose changed since the client
//! last *confirmed* them. Pure functions of the [`MatchSim`], what the client has acknowledged and (optionally) the map's
//! [`InterestMap`], so they are tested — and bandwidth-budgeted — without a socket.
//!
//! Per-prop acknowledgement: the server remembers, for each client and each moving prop, the generation of the pose the client
//! last acknowledged. A prop is (re)sent when it changed after that and is relevant to the client. That makes interest
//! management cheap and exact: a prop that changed while out of earshot is simply still "unconfirmed" when the client walks
//! into the room, and is sent then; nothing is ever sent twice once acknowledged.

use super::protocol::{character_to_wire, PlayerSnap, PropSnap, MAX_PLAYERS_PER_SNAPSHOT, MAX_PROPS_PER_SNAPSHOT, NO_PROP};
use crate::sim::change::Generation;
use crate::sim::interest::InterestMap;
use crate::sim::match_sim::MatchSim;

/// Every connected player as a snapshot record (at most [`MAX_PLAYERS_PER_SNAPSHOT`]).
pub(super) fn player_snaps(sim: &MatchSim, out: &mut Vec<PlayerSnap>) {
    out.clear();
    out.extend(sim.players().take(MAX_PLAYERS_PER_SNAPSHOT).map(|(slot, p)| PlayerSnap {
        id: slot as u8,
        character: character_to_wire(p.state.character),
        flags: (p.crouching as u8) | ((p.combat.is_swinging() as u8) << 1) | ((p.combat.is_dead() as u8) << 2),
        pos: [p.state.pos.x, p.state.foot_y, p.state.pos.y],
        yaw: p.state.yaw,
        pitch: p.state.pitch,
        speed: p.speed,
        vy: p.state.vy,
        weapon: p.combat.weapon.wire(),
        held: sim.props().held_by(slot).map_or(NO_PROP, |h| h as u16),
        hp: p.combat.hp.min(255) as u8,
    }));
}

/// The room a player stands in, if the map has rooms.
pub(super) fn room_of_player(sim: &MatchSim, interest: Option<&InterestMap>, slot: usize) -> Option<usize> {
    let p = sim.player(slot)?;
    interest?.room_at(p.state.pos.x, p.state.foot_y, p.state.pos.y)
}

/// The players `viewer` should be told about: itself always, everyone else if they are in a relevant room.
pub(super) fn visible_players(sim: &MatchSim, interest: Option<&InterestMap>, viewer: usize, all: &[PlayerSnap], out: &mut Vec<PlayerSnap>) {
    out.clear();
    let Some(map) = interest else {
        out.extend_from_slice(all);
        return;
    };
    let vroom = room_of_player(sim, interest, viewer);
    out.extend(all.iter().filter(|p| p.id as usize == viewer || map.relevant(vroom, room_of_player(sim, interest, p.id as usize))).copied());
}

/// The moving props `viewer_room` should be told about: those that changed after the generation the client last confirmed
/// (`known[entity slot]`) and lie in a relevant room, oldest-unconfirmed first, at most [`MAX_PROPS_PER_SNAPSHOT`]. `sent`
/// receives `(entity slot, generation)` for each so a later acknowledgement can confirm them. `scratch` is a reusable buffer.
pub(super) fn props_to_send(
    sim: &MatchSim,
    interest: Option<&InterestMap>,
    viewer_room: Option<usize>,
    known: &[Generation],
    scratch: &mut Vec<(Generation, usize)>,
    out: &mut Vec<PropSnap>,
    sent: &mut Vec<(usize, Generation)>,
) {
    scratch.clear();
    out.clear();
    sent.clear();
    let entities = sim.props().entities();
    for slot in 0..entities.len() {
        let changed = entities.transforms.changed_at(slot);
        let confirmed = known.get(slot).copied().unwrap_or(Generation(0));
        if changed <= confirmed {
            continue;
        }
        if let Some(map) = interest {
            let at = entities.transforms.get(slot).position;
            if !map.relevant(viewer_room, map.room_at(at.x, at.y, at.z)) {
                continue;
            }
        }
        scratch.push((confirmed, slot));
    }
    scratch.sort_unstable();
    for &(_, slot) in scratch.iter().take(MAX_PROPS_PER_SNAPSHOT) {
        let t = entities.transforms.get(slot);
        out.push(PropSnap { id: sim.props().prop_of_entity(slot) as u16, pos: t.position.to_array(), rot: t.rotation.to_array() });
        sent.push((slot, entities.transforms.changed_at(slot)));
    }
}
