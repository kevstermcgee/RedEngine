//! What a headless script may do to the game (`press: "key:NAME"`; ADR 2026-09-28-seeing-what-the-player-sees) and the game's part of the
//! state dump it asserts on (`/redcraft/...`).
//!
//! Keys: `new:SEED[:creative]`, `give:ITEM[:COUNT]`, `slot:N`, `mine` / `stop`, `use`, `inventory`, `close`, `click|rclick|shift:inv:N|craft:N|
//! chest:N|out|in|fuel|result`, `fly`, `save`, `time:DAY_FRACTION`, `tp:X,Y,Z`, `debug`, `pause`, `title`, `spawn:pig|zombie|creeper|tnt[:METRES]`,
//! `throw`.

use super::{seed_of, Game, Mode, Screen};
use crate::item::{Item, Stack};
use crate::play::Play;
use crate::screens::click_slot;
use crate::ui;
use glam::Vec3;
use red_engine2::client::ModeContext;

impl Game {
    pub(super) fn script(&mut self, cx: &mut ModeContext, name: &str) -> bool {
        let g = self;
        let mut parts = name.split(':');
        let verb = parts.next().unwrap_or("");
        let rest: Vec<&str> = parts.collect();
        match verb {
            "new" => {
                let seed = rest.first().and_then(|s| seed_of(s));
                let mode = if rest.get(1) == Some(&"creative") { Mode::Creative } else { Mode::Survival };
                g.create_world(cx, "Scripted World", seed, mode);
            }
            "give" => {
                let item = rest.first().and_then(|n| Item::by_name(n));
                let count = rest.get(1).and_then(|c| c.parse::<u8>().ok()).unwrap_or(1);
                match (item, g.play.as_mut()) {
                    (Some(i), Some(p)) => {
                        p.inv.add(Stack::new(i, count));
                    }
                    _ => return false,
                }
            }
            "slot" => g.select(rest.first().and_then(|n| n.parse::<usize>().ok()).unwrap_or(1).saturating_sub(1)),
            "mine" => g.script_mine = true,
            "stop" => g.script_mine = false,
            "use" => g.use_pressed = true,
            "inventory" => {
                let s = if g.play.as_ref().is_some_and(Play::creative) { Screen::Creative { page: 0 } } else { Screen::Inventory };
                g.open(cx, s);
            }
            "close" => g.close(cx),
            "click" | "rclick" | "shift" => {
                // click:inv:N | click:craft:N | click:out | click:chest:N | click:in | click:fuel | click:result
                let slot = match (rest.first().copied(), rest.get(1).and_then(|n| n.parse::<usize>().ok())) {
                    (Some("inv"), Some(n)) => ui::SlotRef::Inv(n),
                    (Some("craft"), Some(n)) => ui::SlotRef::Craft(n),
                    (Some("chest"), Some(n)) => ui::SlotRef::Chest(n),
                    (Some("out"), _) => ui::SlotRef::CraftOut,
                    (Some("in"), _) => ui::SlotRef::FurnaceIn,
                    (Some("fuel"), _) => ui::SlotRef::FurnaceFuel,
                    (Some("result"), _) => ui::SlotRef::FurnaceOut,
                    _ => return false,
                };
                let screen = g.screen.clone();
                let items = g.creative_items.clone();
                if let Some(p) = g.play.as_mut() {
                    click_slot(p, &screen, slot, verb == "rclick", verb == "shift", &items);
                }
            }
            "fly" => {
                if let Some(p) = g.play.as_mut() {
                    p.body.flying = !p.body.flying;
                }
            }
            "save" => g.save(cx),
            "time" => {
                if let Some(t) = rest.first().and_then(|t| t.parse::<f32>().ok()) {
                    cx.set_time_of_day(t);
                }
            }
            "tp" => {
                let v: Vec<f32> = rest.first().map(|s| s.split(',').filter_map(|n| n.parse().ok()).collect()).unwrap_or_default();
                if let (3, Some(p)) = (v.len(), g.play.as_mut()) {
                    p.body.pos = Vec3::new(v[0], v[1], v[2]);
                    p.prev = p.body.pos;
                    p.body.vel = Vec3::ZERO;
                    p.body.fall_from = p.body.pos.y;
                }
            }
            "debug" => g.debug = !g.debug,
            "spawn" => {
                // spawn:pig|zombie|creeper|tnt:DISTANCE in front of the player
                let kind = match rest.first().copied() {
                    Some("pig") => crate::entities::Kind::Pig,
                    Some("zombie") => crate::entities::Kind::Zombie,
                    Some("creeper") => crate::entities::Kind::Creeper,
                    Some("tnt") => crate::entities::Kind::Tnt,
                    _ => return false,
                };
                let d = rest.get(1).and_then(|d| d.parse::<f32>().ok()).unwrap_or(4.0);
                let (yaw, _) = cx.camera_angles();
                if let Some(p) = g.play.as_mut() {
                    let at = p.body.pos + crate::tool::look_dir(yaw, 0.0) * d + Vec3::Y * 0.5;
                    p.spawn(kind, at);
                }
            }
            "throw" => {
                let (yaw, pitch) = cx.camera_angles();
                if let Some(p) = g.play.as_mut() {
                    p.throw(false, yaw, pitch);
                }
            }
            "pause" => {
                if g.play.is_some() {
                    g.save(cx);
                    g.open(cx, Screen::Pause);
                }
            }
            "title" => g.back_to_title(cx),
            _ => return false,
        }
        true
    }

    pub(super) fn dump_json(&self) -> serde_json::Value {
        let g = self;
        let Some(p) = &g.play else {
            return serde_json::json!({"screen": g.screen.name(), "worlds": g.worlds.len()});
        };
        let inv: Vec<serde_json::Value> =
            p.inv.slots.iter().enumerate().filter(|(_, s)| !s.is_empty()).map(|(i, s)| serde_json::json!([i, s.item.name(), s.count])).collect();
        let counts: serde_json::Map<String, serde_json::Value> = p
            .inv
            .slots
            .iter()
            .filter(|s| !s.is_empty())
            .fold(std::collections::BTreeMap::<String, u32>::new(), |mut m, s| {
                *m.entry(s.item.name().to_string()).or_default() += s.count as u32;
                m
            })
            .into_iter()
            .map(|(k, v)| (k, serde_json::json!(v)))
            .collect();
        let st = p.terrain.stats();
        serde_json::json!({
            "screen": g.screen.name(),
            "world": p.name,
            "seed": p.seed,
            "mode": p.mode.name(),
            "ready": p.ready,
            "pos": [p.body.pos.x, p.body.pos.y, p.body.pos.z],
            "on_ground": p.body.on_ground,
            "flying": p.body.flying,
            "in_water": p.body.in_water,
            "health": p.health,
            "air": p.air,
            "dead": p.dead,
            "selected": p.inv.selected,
            "held": p.inv.held_item().map(|i| i.name()),
            "inventory": inv,
            "count": counts,
            "cursor": (!p.cursor.is_empty()).then(|| p.cursor.item.name()),
            "target": p.target.map(|t| serde_json::json!({"block": t.block.info().name, "pos": [t.pos.x, t.pos.y, t.pos.z], "distance": t.distance})),
            "mined": p.mined,
            "placed": p.placed,
            "days": p.days,
            "time": p.time,
            "chunks": {"loaded": st.loaded, "lit": st.lit, "meshed": st.meshed},
            "block_under": p.terrain.world.block_at((p.body.pos - Vec3::Y * 0.5).floor().as_ivec3()).info().name,
            "entities": {
                "items": p.entities.iter().filter(|e| matches!(e.kind, crate::entities::Kind::Item(_))).count(),
                "pigs": p.entities.iter().filter(|e| e.kind == crate::entities::Kind::Pig).count(),
                "zombies": p.entities.iter().filter(|e| e.kind == crate::entities::Kind::Zombie).count(),
                "creepers": p.entities.iter().filter(|e| e.kind == crate::entities::Kind::Creeper).count(),
            },
            "kills": p.kills,
            "furnaces": p.furnaces.len(),
            "chests": p.chests.len(),
        })
    }
}
