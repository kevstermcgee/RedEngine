//! Workshop's part of a snapshot: what a client shows, in a few bytes (`GameMsg` kind [`KIND`], the engine's codec `protocol::W`/`R`).
//!
//! The client loads the same content (the map hash proves it), so ids never travel: machines and stores go in id order. Per machine: its
//! status (0 idle, 1 working, 2 blocked), the ticks left in its cycle, cycles made, and its output's switch position; per shared store:
//! the count of each resource in id order; then units shipped per sink. A client reads it back with [`decode`] (`NetClient::set_game_decoder`).

use crate::content::Catalog;
use crate::game::WorkshopGame;
use crate::machines::Machines;
use red_engine2::net::game_view::GameView;
use red_engine2::net::protocol::{DecodeError, GameMsg, R, W};

/// [`GameMsg::kind`] of Workshop's part.
pub const KIND: u8 = 0x57;

/// Most machines, stores or sinks a part carries (more is refused rather than truncated).
pub const MAX_ITEMS: usize = 4096;

/// What a client knows of the workshop from one snapshot.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorkshopView {
    /// The systems' tick.
    pub tick: u32,
    /// Per machine (id order): status, ticks left, cycles made, switch position (255: no switch).
    pub machines: Vec<(u8, u16, u32, u8)>,
    /// Per shared store (id order): each resource's count (resource id order).
    pub stores: Vec<Vec<u16>>,
    /// Per sink (id order): units shipped.
    pub shipped: Vec<u32>,
}

impl GameView for WorkshopView {}

/// The part for `game`.
pub fn encode(game: &WorkshopGame) -> GameMsg {
    let cat = game.catalog();
    let sys = game.systems();
    let ledger = sys.ledger();
    let mut bytes = Vec::new();
    let mut w = W(&mut bytes);
    w.u32(sys.tick_count() as u32);
    let machines = sys.get::<Machines>();
    let units: Vec<_> = machines.map(|m| m.units().collect()).unwrap_or_default();
    w.u16(units.len() as u16);
    for (iid, u) in units {
        let status = match (machines.and_then(|m| m.blocked(iid, ledger)), u.left) {
            (Some(_), _) => 2,
            (None, Some(_)) => 1,
            (None, None) => 0,
        };
        let pos = cat.switches.iter().find(|(_, s)| s.instance == *iid).and_then(|(_, s)| s.to.iter().position(|t| *t == u.to)).map_or(255, |p| p as u8);
        w.u8(status);
        w.u16(u.left.unwrap_or(0).min(u16::MAX as u32) as u16);
        w.u32(u.made.min(u32::MAX as u64) as u32);
        w.u8(pos);
    }
    let resources: Vec<_> = cat.resources.iter().map(|(r, _)| r.clone()).collect();
    w.u16(cat.stores.len() as u16);
    w.u16(resources.len() as u16);
    for (sid, _) in cat.stores.iter() {
        for r in &resources {
            w.u16(ledger.count(sid, r).min(u16::MAX as u32) as u16);
        }
    }
    w.u16(cat.sinks.len() as u16);
    for (sid, _) in cat.sinks.iter() {
        w.u32(game.shipped(sid).min(u32::MAX as u64) as u32);
    }
    GameMsg { kind: KIND, bytes }
}

/// Reads a part back: `Ok(None)` for another game's kind, an error for a malformed one (the client then drops the snapshot).
pub fn decode(g: &GameMsg) -> Result<Option<Box<dyn GameView>>, DecodeError> {
    if g.kind != KIND {
        return Ok(None);
    }
    let mut r = R::new(&g.bytes);
    let tick = r.u32()?;
    let n = r.u16()? as usize;
    if n > MAX_ITEMS {
        return Err(DecodeError::OutOfRange);
    }
    let mut machines = Vec::with_capacity(n);
    for _ in 0..n {
        machines.push((r.u8()?, r.u16()?, r.u32()?, r.u8()?));
    }
    let (stores, resources) = (r.u16()? as usize, r.u16()? as usize);
    if stores > MAX_ITEMS || resources > MAX_ITEMS {
        return Err(DecodeError::OutOfRange);
    }
    let mut counts = Vec::with_capacity(stores);
    for _ in 0..stores {
        counts.push((0..resources).map(|_| r.u16()).collect::<Result<Vec<_>, _>>()?);
    }
    let sinks = r.u16()? as usize;
    if sinks > MAX_ITEMS {
        return Err(DecodeError::OutOfRange);
    }
    let shipped = (0..sinks).map(|_| r.u32()).collect::<Result<Vec<_>, _>>()?;
    r.finish()?;
    Ok(Some(Box::new(WorkshopView { tick, machines, stores: counts, shipped })))
}

/// The shared stores in the order a part carries them (id order), for a client to label what it decodes.
pub fn store_order(cat: &Catalog) -> Vec<red_engine2::sim::defs::Id> {
    cat.stores.iter().map(|(i, _)| i.clone()).collect()
}
