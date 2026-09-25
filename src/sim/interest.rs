//! Spatial interest management: which parts of the world a client needs to hear about.
//!
//! A map's `zones` are its rooms and its `portals` say which rooms connect (a doorway is a portal between two zones,
//! `open` unless it says otherwise). A client needs its own room and every room within `interest.hops` open portals of it
//! (default 1): what happens further away cannot be seen, so the server does not send it. Things standing in no zone
//! (outdoors) are always relevant. Map authors write only zones and portals — the relevance sets are computed here, never by
//! hand — and the same data drives lint, tours and `reach`.
//!
//! ```json
//! "zones":   [ { "id": "hall", "rect": [-24,-8,-16,8], "y": 0 }, { "id": "lab", "rect": [-16,-8,-8,8], "y": 0 } ],
//! "portals": [ { "id": "door_0", "between": ["hall", "lab"], "center": [-16, 0], "width": 1.0, "height": 2.2, "open": true } ],
//! "interest": { "hops": 1 }
//! ```
//!
//! The map representation is versioned with the scene (`schema_version`): zone ids, portal connectivity, spawn points/groups
//! and the optional `interest` block are all plain scene data, validated at load ([`validate_sections`]).

use glam::Vec2;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// A room: a zone's floor rectangle and height.
#[derive(Debug, Clone, PartialEq)]
pub struct Room {
    /// The zone id.
    pub id: String,
    /// Lowest x/z corner.
    pub min: Vec2,
    /// Highest x/z corner.
    pub max: Vec2,
    /// Floor height.
    pub y: f32,
}

/// The rooms of a map and which are near which.
#[derive(Debug, Clone, PartialEq)]
pub struct InterestMap {
    /// The rooms, in declaration order.
    pub rooms: Vec<Room>,
    /// How many open portals away still counts as relevant.
    pub hops: u32,
    /// `near[a][b]`: room `b` is within `hops` open portals of room `a`.
    near: Vec<Vec<bool>>,
}

impl InterestMap {
    /// Builds the map from the scene's raw JSON text. `None` when the scene declares no zones (no interest management:
    /// everyone hears about everything). Errors name the zone/portal at fault.
    pub fn parse(scene_json: &str) -> Result<Option<InterestMap>, String> {
        let v: Value = serde_json::from_str(scene_json).map_err(|e| format!("json: {e}"))?;
        let Some(root) = v.as_object() else { return Err("root: scene must be a JSON object".to_string()) };
        let errs = validate_sections(root);
        if !errs.is_empty() {
            return Err(errs.join("\n"));
        }
        Ok(Self::from_root(root))
    }

    fn from_root(root: &Map<String, Value>) -> Option<InterestMap> {
        let zones = root.get("zones")?.as_array()?;
        let rooms: Vec<Room> = zones
            .iter()
            .filter_map(|z| {
                let r = z.get("rect")?.as_array()?;
                let n: Vec<f32> = r.iter().filter_map(|v| v.as_f64()).map(|v| v as f32).collect();
                (n.len() == 4).then(|| Room {
                    id: z.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
                    min: Vec2::new(n[0].min(n[2]), n[1].min(n[3])),
                    max: Vec2::new(n[0].max(n[2]), n[1].max(n[3])),
                    y: z.get("y").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                })
            })
            .collect();
        if rooms.is_empty() {
            return None;
        }
        let index: HashMap<&str, usize> = rooms.iter().enumerate().map(|(i, r)| (r.id.as_str(), i)).collect();
        let hops = root.get("interest").and_then(|i| i.get("hops")).and_then(Value::as_u64).unwrap_or(1).min(16) as u32;
        let mut adj = vec![Vec::new(); rooms.len()];
        for p in root.get("portals").and_then(Value::as_array).into_iter().flatten() {
            if p.get("open").and_then(Value::as_bool) == Some(false) {
                continue;
            }
            let between = p.get("between").and_then(Value::as_array).map(|b| b.iter().filter_map(Value::as_str).collect::<Vec<_>>()).unwrap_or_default();
            if let [a, b] = between.as_slice() {
                if let (Some(&ia), Some(&ib)) = (index.get(a), index.get(b)) {
                    adj[ia].push(ib);
                    adj[ib].push(ia);
                }
            }
        }
        // Breadth-first from every room, `hops` deep.
        let n = rooms.len();
        let mut near = vec![vec![false; n]; n];
        for (start, row) in near.iter_mut().enumerate() {
            let mut frontier = vec![start];
            row[start] = true;
            for _ in 0..hops {
                let mut next = Vec::new();
                for r in frontier {
                    for &o in &adj[r] {
                        if !row[o] {
                            row[o] = true;
                            next.push(o);
                        }
                    }
                }
                frontier = next;
            }
        }
        Some(InterestMap { rooms, hops, near })
    }

    /// The room a point stands in: among zones whose rectangle contains `(x, z)`, the highest whose floor is at or below
    /// `y + 0.5` (so someone on a platform is in the platform's zone, someone under it in the room below).
    pub fn room_at(&self, x: f32, y: f32, z: f32) -> Option<usize> {
        let inside = |r: &Room| x >= r.min.x && x <= r.max.x && z >= r.min.y && z <= r.max.y;
        let candidates = self.rooms.iter().enumerate().filter(|(_, r)| inside(r));
        let below = candidates.clone().filter(|(_, r)| r.y <= y + 0.5).max_by(|a, b| a.1.y.total_cmp(&b.1.y)).map(|(i, _)| i);
        below.or_else(|| candidates.min_by(|a, b| (a.1.y - y).abs().total_cmp(&(b.1.y - y).abs())).map(|(i, _)| i))
    }

    /// Whether something in room `thing` is relevant to a viewer in room `viewer` (`None` = in no room: always relevant).
    pub fn relevant(&self, viewer: Option<usize>, thing: Option<usize>) -> bool {
        match (viewer, thing) {
            (Some(v), Some(t)) => self.near[v][t],
            _ => true,
        }
    }

    /// One line per room saying which rooms it hears (`describe`/debugging).
    pub fn summary(&self) -> String {
        let mut out = String::new();
        for (i, r) in self.rooms.iter().enumerate() {
            let heard: Vec<&str> = self.rooms.iter().enumerate().filter(|(j, _)| self.near[i][*j] && *j != i).map(|(_, o)| o.id.as_str()).collect();
            out.push_str(&format!("{} hears {}\n", r.id, if heard.is_empty() { "only itself".to_string() } else { heard.join(", ") }));
        }
        out
    }
}

/// Validates the map-structure sections that interest management reads: every zone has an id and a 4-number `rect`,
/// ids are unique, every portal joins two existing zones, and `interest.hops` is a small whole number. Returns
/// `path: message` problems (empty when fine); `parse_scene` reports them with the rest.
pub fn validate_sections(root: &Map<String, Value>) -> Vec<String> {
    let mut errs = Vec::new();
    let mut ids: Vec<&str> = Vec::new();
    for (i, z) in root.get("zones").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let id = z.get("id").and_then(Value::as_str).unwrap_or("");
        if id.is_empty() {
            errs.push(format!("zones[{i}].id: missing"));
        } else if ids.contains(&id) {
            errs.push(format!("zones[{i}] ({id}).id: duplicate zone id"));
        } else {
            ids.push(id);
        }
        let ok = z.get("rect").and_then(Value::as_array).is_some_and(|r| r.len() == 4 && r.iter().all(Value::is_number));
        if !ok {
            errs.push(format!("zones[{i}] ({id}).rect: must be [x0, z0, x1, z1] (four numbers)"));
        }
    }
    let mut portal_ids: Vec<&str> = Vec::new();
    for (i, p) in root.get("portals").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let id = p.get("id").and_then(Value::as_str).unwrap_or("");
        let at = if id.is_empty() { format!("portals[{i}]") } else { format!("portals[{i}] ({id})") };
        if id.is_empty() {
            errs.push(format!("{at}.id: missing"));
        } else if portal_ids.contains(&id) {
            errs.push(format!("{at}.id: duplicate portal id"));
        } else {
            portal_ids.push(id);
        }
        let between = p.get("between").and_then(Value::as_array).map(|b| b.iter().map(|v| v.as_str().unwrap_or("")).collect::<Vec<_>>());
        match between.as_deref() {
            Some([a, b]) => {
                for (k, name) in [a, b].into_iter().enumerate() {
                    if !ids.contains(name) {
                        let near = crate::prefabs::suggest(name, ids.iter().copied());
                        let hint = near.first().map(|n| format!(" — did you mean `{n}`?")).unwrap_or_default();
                        errs.push(format!("{at}.between[{k}]: no zone `{name}`{hint} (zones: {})", ids.join(", ")));
                    }
                }
            }
            _ => errs.push(format!("{at}.between: must be two zone ids like [\"hall\", \"lab\"]")),
        }
    }
    if let Some(h) = root.get("interest").and_then(|i| i.get("hops")) {
        if !h.as_u64().is_some_and(|h| h <= 16) {
            errs.push("interest.hops: must be a whole number from 0 to 16 (portals away that still count as relevant)".to_string());
        }
    }
    errs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(extra: &str) -> InterestMap {
        let text = format!(
            r#"{{"zones":[{{"id":"a","rect":[0,0,10,10],"y":0}},{{"id":"b","rect":[10,0,20,10],"y":0}},{{"id":"c","rect":[20,0,30,10],"y":0}},{{"id":"d","rect":[30,0,40,10],"y":0}},
                 {{"id":"loft","rect":[2,2,6,6],"y":3}}],
                "portals":[{{"id":"ab","between":["a","b"]}},{{"id":"bc","between":["b","c"]}},{{"id":"cd","between":["c","d"],"open":false}}]{extra}}}"#
        );
        InterestMap::parse(&text).unwrap().unwrap()
    }

    #[test]
    fn a_client_hears_its_room_and_the_rooms_one_open_portal_away() {
        let m = map("");
        let (a, b, c, d) = (m.room_at(5.0, 0.0, 5.0), m.room_at(15.0, 0.0, 5.0), m.room_at(25.0, 0.0, 5.0), m.room_at(35.0, 0.0, 5.0));
        assert!(m.relevant(a, a) && m.relevant(a, b), "own room and the next one");
        assert!(!m.relevant(a, c), "two portals away is out of earshot");
        assert!(m.relevant(b, a) && m.relevant(b, c), "portals work both ways");
        assert!(!m.relevant(c, d) && !m.relevant(d, c), "a closed portal connects nothing");
    }

    #[test]
    fn hops_widens_the_set_and_a_thing_in_no_room_is_always_relevant() {
        let m = map(r#","interest":{"hops":2}"#);
        let (a, c) = (m.room_at(5.0, 0.0, 5.0), m.room_at(25.0, 0.0, 5.0));
        assert!(m.relevant(a, c), "two hops reach room c");
        assert!(m.relevant(a, None) && m.relevant(None, c), "outdoors is always relevant");
    }

    #[test]
    fn height_picks_the_floor_you_are_standing_on() {
        let m = map("");
        let (ground, loft) = (m.room_at(4.0, 0.0, 4.0), m.room_at(4.0, 3.0, 4.0));
        assert_eq!(m.rooms[ground.unwrap()].id, "a");
        assert_eq!(m.rooms[loft.unwrap()].id, "loft");
        assert!(m.room_at(-5.0, 0.0, 5.0).is_none());
    }

    #[test]
    fn a_scene_without_zones_has_no_interest_management() {
        assert!(InterestMap::parse(r#"{"objects":[]}"#).unwrap().is_none());
    }

    #[test]
    fn broken_zones_and_portals_are_reported_with_a_fix() {
        let e = InterestMap::parse(
            r#"{"zones":[{"id":"a","rect":[0,0,1,1]},{"id":"a","rect":[0,0,1]}],"portals":[{"id":"p","between":["a","bb"]},{"id":"q","between":["a"]}],"interest":{"hops":99}}"#,
        )
        .unwrap_err();
        for needle in [
            "zones[1] (a).id: duplicate",
            "zones[1] (a).rect: must be",
            "portals[0] (p).between[1]: no zone `bb`",
            "portals[1] (q).between: must be two zone ids",
            "interest.hops",
        ] {
            assert!(e.contains(needle), "missing `{needle}` in:\n{e}");
        }
    }
}
