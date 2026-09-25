//! `red_engine2 build`: the blueprint compiler, the "framework" layer between an AI's intent and a finished map.
//!
//! An AI that wants "three rooms in a row, a door between each, two players spawning in the first, some crates in the
//! others" should write about twenty lines, not nine hundred objects. A **blueprint** is that description; [`compile`]
//! turns it into a complete scene the rest of the toolchain already understands:
//!
//! * walls with doors, derived from the room rectangles (shared edges become partitions, free edges the perimeter,
//!   so corners and door positions are arithmetic the AI never does);
//! * a floor and (optionally) a ceiling per room, lamps, a sun, a camera, `zones`, `spawns`, `portals` and
//!   `interest` (what the multiplayer server needs for room-based interest management);
//! * seeded prop fill that keeps door approaches, room-to-room aisles and spawn pads clear (so a crate never seals a
//!   route, the failure that cost the Cheddar project most of its debugging time);
//! * a `checks` block that already passes: lint budget, one reachability check per room, and one **auto-planned walk**
//!   from the first spawn to every other room (`walk --auto`), so the map defends itself against later edits.
//!
//! The output is a normal scene: edit it with `add/set/move`, or change the blueprint and rebuild. `build --check`
//! fails when the committed map no longer equals what the blueprint produces, which catches a hand-edited or stale map.
//! Blueprint format: see [`example`] (printed by `build --example`) and SPEC.md "Blueprints".

use super::edit::{format_scene, num};
use super::gen::{scatter, ScatterParams};
use super::lint::{self, Severity};
use super::reach::{self, ColliderGrid, ReachParams};
use super::world::MapWorld;
use crate::props::PropKind;
use crate::strict::check_keys;
use glam::Vec2;
use serde_json::{json, Map, Value};
use std::path::Path;

/// The blueprint format version this compiler understands (`"blueprint": 1`).
pub const BLUEPRINT_VERSION: u64 = 1;

const EPS: f32 = 1e-3;
const WALL_EXT: f32 = 0.24;
const WALL_PART: f32 = 0.15;
/// Doors narrower than this trip the `door` lint and squeeze a 0.7 m body.
const MIN_DOOR: f32 = 0.9;
/// Point lights the renderer supports, minus the sun's slot.
const MAX_LAMPS: usize = 15;
const FLOOR_COLORS: [&str; 6] = ["#8f9aa8", "#a89f8f", "#8fa895", "#a88f9d", "#9fa8b8", "#b0a688"];

/// The keys this level of a blueprint accepts (a test checks each is documented in SPEC.md).
pub const TOP_KEYS: &[&str] = &["blueprint", "name", "height", "ceiling", "rooms", "doors", "spawns", "fill", "keep_clear", "extra", "prefab_files", "scene"];
/// The keys this level of a blueprint accepts (a test checks each is documented in SPEC.md).
pub const ROOM_KEYS: &[&str] = &["id", "rect", "floor", "lamp"];
/// The keys this level of a blueprint accepts (a test checks each is documented in SPEC.md).
pub const DOOR_KEYS: &[&str] = &["between", "width", "at", "kind"];
/// The keys this level of a blueprint accepts (a test checks each is documented in SPEC.md).
pub const SPAWN_KEYS: &[&str] = &["room", "group", "count", "id"];
/// The keys this level of a blueprint accepts (a test checks each is documented in SPEC.md).
pub const FILL_KEYS: &[&str] = &["room", "kind", "count", "seed", "scale", "colors", "min_gap", "clearance", "id"];

/// What a compile produced: the scene text plus a report of what was made and how it linted.
#[derive(Debug)]
pub struct Built {
    /// The finished scene, in the canonical layout the edit tools write.
    pub scene_text: String,
    /// The scene as a value (for `--json` and tests).
    pub scene: Value,
    /// Human-readable summary lines (rooms, doors, objects, spawn groups, checks).
    pub summary: Vec<String>,
    /// `lint` findings on the result: (severity, code, message). Errors mean the blueprint needs changing.
    pub findings: Vec<(Severity, &'static str, String)>,
}

impl Built {
    /// Number of lint errors in the built map.
    pub fn errors(&self) -> usize {
        self.findings.iter().filter(|f| f.0 == Severity::Error).count()
    }
}

struct Room {
    id: String,
    min: Vec2,
    max: Vec2,
    floor: String,
    lamp: bool,
}

impl Room {
    fn center(&self) -> Vec2 {
        (self.min + self.max) * 0.5
    }
    fn size(&self) -> Vec2 {
        self.max - self.min
    }
}

struct Door {
    a: usize,
    b: usize,
    width: f32,
    kind: String,
    center: Vec2,
    /// The wall runs along Z (constant x) when true, along X (constant z) when false.
    vertical: bool,
}

fn f32s(v: &Value, n: usize) -> Option<Vec<f32>> {
    let a = v.as_array()?;
    (a.len() == n).then(|| a.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect::<Vec<_>>()).filter(|x| x.len() == n)
}

fn r3(v: f32) -> Value {
    num((v as f64 * 1000.0).round() / 1000.0 + 0.0)
}

fn pt(p: Vec2) -> Value {
    json!([r3(p.x), r3(p.y)])
}

fn obj<'a>(v: &'a Value, path: &str, allowed: &[&str], errs: &mut Vec<String>) -> Option<&'a Map<String, Value>> {
    match v.as_object() {
        Some(o) => {
            check_keys(errs, path, o, allowed);
            Some(o)
        }
        None => {
            errs.push(format!("{path}: expected an object"));
            None
        }
    }
}

/// Overlap of two rooms' facing edges: `(vertical wall?, coordinate, lo, hi)` along the wall.
fn shared_edge(a: &Room, b: &Room) -> Option<(bool, f32, f32, f32)> {
    for (l, r) in [(a, b), (b, a)] {
        if (l.max.x - r.min.x).abs() < EPS {
            let (lo, hi) = (l.min.y.max(r.min.y), l.max.y.min(r.max.y));
            if hi - lo > EPS {
                return Some((true, l.max.x, lo, hi));
            }
        }
    }
    for (l, r) in [(a, b), (b, a)] {
        if (l.max.y - r.min.y).abs() < EPS {
            let (lo, hi) = (l.min.x.max(r.min.x), l.max.x.min(r.max.x));
            if hi - lo > EPS {
                return Some((false, l.max.y, lo, hi));
            }
        }
    }
    None
}

fn rooms_overlap(a: &Room, b: &Room) -> bool {
    a.min.x < b.max.x - EPS && b.min.x < a.max.x - EPS && a.min.y < b.max.y - EPS && b.min.y < a.max.y - EPS
}

/// Room-edge intervals on one wall line, keyed by `(vertical, coordinate in mm)`: `(lo, hi, belongs to the room's max side)`.
type EdgeLines = std::collections::BTreeMap<(bool, i64), Vec<(f32, f32, bool)>>;

/// One straight wall run derived from the room edges.
struct Run {
    vertical: bool,
    coord: f32,
    lo: f32,
    hi: f32,
    partition: bool,
}

/// Splits every room edge into partition (shared) and exterior runs, merged where contiguous.
fn wall_runs(rooms: &[Room]) -> Vec<Run> {
    // (vertical, coord in mm) -> intervals with the side of the room they belong to (true = the room's max side).
    let mut lines = EdgeLines::new();
    let key = |c: f32| (c * 1000.0).round() as i64;
    for r in rooms {
        lines.entry((true, key(r.min.x))).or_default().push((r.min.y, r.max.y, false));
        lines.entry((true, key(r.max.x))).or_default().push((r.min.y, r.max.y, true));
        lines.entry((false, key(r.min.y))).or_default().push((r.min.x, r.max.x, false));
        lines.entry((false, key(r.max.y))).or_default().push((r.min.x, r.max.x, true));
    }
    let mut out = Vec::new();
    for ((vertical, k), ivs) in lines {
        let coord = k as f32 / 1000.0;
        let mut cuts: Vec<f32> = ivs.iter().flat_map(|i| [i.0, i.1]).collect();
        cuts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        cuts.dedup_by(|a, b| (*a - *b).abs() < EPS);
        let mut cur: Option<Run> = None;
        for w in cuts.windows(2) {
            let (p, q) = (w[0], w[1]);
            let mid = (p + q) * 0.5;
            let below = ivs.iter().any(|i| !i.2 && i.0 <= mid && mid <= i.1);
            let above = ivs.iter().any(|i| i.2 && i.0 <= mid && mid <= i.1);
            let kind = match (below, above) {
                (true, true) => Some(true),
                (false, false) => None,
                _ => Some(false),
            };
            match (kind, cur.as_mut()) {
                (Some(part), Some(run)) if run.partition == part && (run.hi - p).abs() < EPS => run.hi = q,
                (Some(part), _) => {
                    if let Some(run) = cur.take() {
                        out.push(run);
                    }
                    cur = Some(Run { vertical, coord, lo: p, hi: q, partition: part });
                }
                (None, _) => {
                    if let Some(run) = cur.take() {
                        out.push(run);
                    }
                }
            }
        }
        if let Some(run) = cur.take() {
            out.push(run);
        }
    }
    out
}

/// Clear pad in front of a door on both sides plus the aisle between the doors of one room, as `(min, max)` rects.
fn keep_clear_rects(rooms: &[Room], doors: &[Door], spawn_pts: &[(usize, Vec2)], extra: &[(Vec2, Vec2)]) -> Vec<Vec<(Vec2, Vec2)>> {
    let mut per_room: Vec<Vec<(Vec2, Vec2)>> = vec![Vec::new(); rooms.len()];
    for (i, room) in rooms.iter().enumerate() {
        let mut anchors: Vec<Vec2> = Vec::new();
        for d in doors.iter().filter(|d| d.a == i || d.b == i) {
            // The point just inside this room in front of the door.
            let toward = room.center() - d.center;
            let n = if d.vertical { Vec2::new(toward.x.signum(), 0.0) } else { Vec2::new(0.0, toward.y.signum()) };
            let pad = d.center + n * 0.9;
            anchors.push(pad);
            let half = if d.vertical { Vec2::new(1.1, d.width * 0.5 + 0.6) } else { Vec2::new(d.width * 0.5 + 0.6, 1.1) };
            per_room[i].push((pad - half, pad + half));
        }
        for (_, p) in spawn_pts.iter().filter(|s| s.0 == i) {
            anchors.push(*p);
            per_room[i].push((*p - Vec2::splat(1.1), *p + Vec2::splat(1.1)));
        }
        // Aisles: every anchor pair in the room must stay connected, so keep the box between them (inflated) empty.
        for x in 0..anchors.len() {
            for y in x + 1..anchors.len() {
                let (lo, hi) = (anchors[x].min(anchors[y]) - Vec2::splat(0.75), anchors[x].max(anchors[y]) + Vec2::splat(0.75));
                per_room[i].push((lo, hi));
            }
        }
        per_room[i].extend(extra.iter().copied());
    }
    per_room
}

/// Compiles a blueprint (parsed JSON) into a scene. Errors are `path: message` lines with a fix where one is known.
/// `prefab_files` need the blueprint's location: use [`compile_in`] for those.
pub fn compile(bp: &Value) -> Result<Built, Vec<String>> {
    compile_in(bp, None)
}

/// [`compile`] for a blueprint stored in `base` (a directory): `prefab_files` are read relative to it.
pub fn compile_in(bp: &Value, base: Option<&Path>) -> Result<Built, Vec<String>> {
    let mut errs = Vec::new();
    let Some(root) = obj(bp, "", TOP_KEYS, &mut errs) else { return Err(errs) };
    match root.get("blueprint").and_then(Value::as_u64) {
        Some(BLUEPRINT_VERSION) => {}
        Some(v) => errs.push(format!("blueprint: version {v} is not supported (this engine reads version {BLUEPRINT_VERSION})")),
        None => errs.push(format!("blueprint: missing \"blueprint\": {BLUEPRINT_VERSION} (the format version)")),
    }
    let name = root.get("name").and_then(Value::as_str).unwrap_or("map").to_string();
    let height = root.get("height").and_then(Value::as_f64).unwrap_or(2.8) as f32;
    if !(2.3..=8.0).contains(&height) {
        errs.push(format!("height: {height} m is outside 2.3..8 (doors are 2.2 m tall and the player needs headroom)"));
    }
    let ceiling = root.get("ceiling").and_then(Value::as_bool).unwrap_or(false);

    // ---- rooms
    let mut rooms: Vec<Room> = Vec::new();
    for (i, r) in root.get("rooms").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let path = format!("rooms[{i}]");
        let Some(o) = obj(r, &path, ROOM_KEYS, &mut errs) else { continue };
        let id = o.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            errs.push(format!("{path}.id: needs a short id of letters, digits, _ or - (it names zones, walls and checks)"));
            continue;
        }
        if rooms.iter().any(|x| x.id == id) {
            errs.push(format!("{path}.id: '{id}' is used twice"));
            continue;
        }
        let Some(rc) = o.get("rect").and_then(|v| f32s(v, 4)) else {
            errs.push(format!("{path}.rect: needs [x0, z0, x1, z1] in metres (e.g. [-8, -6, 0, 6])"));
            continue;
        };
        let (min, max) = (Vec2::new(rc[0].min(rc[2]), rc[1].min(rc[3])), Vec2::new(rc[0].max(rc[2]), rc[1].max(rc[3])));
        if max.x - min.x < 2.0 || max.y - min.y < 2.0 {
            errs.push(format!("{path}.rect: {:.1} x {:.1} m is too small (rooms need at least 2 x 2 m)", max.x - min.x, max.y - min.y));
            continue;
        }
        let floor = o.get("floor").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| FLOOR_COLORS[rooms.len() % FLOOR_COLORS.len()].to_string());
        rooms.push(Room { id, min, max, floor, lamp: o.get("lamp").and_then(Value::as_bool).unwrap_or(true) });
    }
    if rooms.is_empty() && errs.is_empty() {
        errs.push("rooms: needs at least one room, e.g. {\"id\": \"hall\", \"rect\": [-8, -6, 8, 6]}".into());
    }
    for i in 0..rooms.len() {
        for j in i + 1..rooms.len() {
            if rooms_overlap(&rooms[i], &rooms[j]) {
                errs.push(format!("rooms: '{}' and '{}' overlap; rooms may touch along an edge but not overlap", rooms[i].id, rooms[j].id));
            }
        }
    }
    let room_ix = |id: &str| rooms.iter().position(|r| r.id == id);
    let room_names = || rooms.iter().map(|r| r.id.as_str()).collect::<Vec<_>>().join(", ");

    // ---- doors
    let mut doors: Vec<Door> = Vec::new();
    for (i, d) in root.get("doors").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let path = format!("doors[{i}]");
        let Some(o) = obj(d, &path, DOOR_KEYS, &mut errs) else { continue };
        let pair: Vec<&str> = o.get("between").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        if pair.len() != 2 {
            errs.push(format!("{path}.between: needs two room ids, e.g. [\"hall\", \"store\"]"));
            continue;
        }
        let (Some(a), Some(b)) = (room_ix(pair[0]), room_ix(pair[1])) else {
            errs.push(format!("{path}.between: unknown room in [{}, {}] (rooms: {})", pair[0], pair[1], room_names()));
            continue;
        };
        let Some((vertical, coord, lo, hi)) = shared_edge(&rooms[a], &rooms[b]) else {
            errs.push(format!("{path}.between: '{}' and '{}' do not share a wall (rooms must touch along an edge; move a rect so they do)", pair[0], pair[1]));
            continue;
        };
        let width = o.get("width").and_then(Value::as_f64).unwrap_or(1.4) as f32;
        if width < MIN_DOOR {
            errs.push(format!("{path}.width: {width} m is narrower than {MIN_DOOR} m (the 0.7 m player body needs slack; `lint` flags these as `door`)"));
            continue;
        }
        let offset = o.get("at").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let c = (lo + hi) * 0.5 + offset;
        if c - width * 0.5 < lo + 0.1 || c + width * 0.5 > hi - 0.1 {
            errs.push(format!("{path}: a {width} m door at offset {offset} does not fit on the {:.1} m of wall the two rooms share", hi - lo));
            continue;
        }
        let center = if vertical { Vec2::new(coord, c) } else { Vec2::new(c, coord) };
        let kind = o.get("kind").and_then(Value::as_str).unwrap_or("door").to_string();
        if kind != "door" && kind != "arch" {
            errs.push(format!("{path}.kind: '{kind}' must be \"door\" or \"arch\""));
            continue;
        }
        for other in doors.iter().filter(|x| x.vertical == vertical && (if vertical { x.center.x } else { x.center.y } - coord).abs() < EPS) {
            let gap = if vertical { (other.center.y - center.y).abs() } else { (other.center.x - center.x).abs() };
            if gap < (other.width + width) * 0.5 + 0.3 {
                errs.push(format!("{path}: overlaps another door on the same wall (keep 0.3 m of wall between doors)"));
            }
        }
        doors.push(Door { a, b, width, kind, center, vertical });
    }

    // ---- spawns
    let mut spawns: Vec<(String, String, [f32; 3], f32, usize)> = Vec::new(); // id, group, pos, yaw, room
    for (i, s) in root.get("spawns").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let path = format!("spawns[{i}]");
        let Some(o) = obj(s, &path, SPAWN_KEYS, &mut errs) else { continue };
        let room = o.get("room").and_then(Value::as_str).unwrap_or("");
        let Some(ri) = room_ix(room) else {
            errs.push(format!("{path}.room: unknown room '{room}' (rooms: {})", room_names()));
            continue;
        };
        let count = o.get("count").and_then(Value::as_u64).unwrap_or(1).clamp(1, 16) as usize;
        let group = o.get("group").and_then(Value::as_str).unwrap_or("default").to_string();
        let prefix = o.get("id").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("spawn_{room}"));
        let r = &rooms[ri];
        let radius = (r.size().x.min(r.size().y) * 0.5 - 1.2).max(0.0);
        for k in 0..count {
            let ang = std::f32::consts::TAU * k as f32 / count as f32 + std::f32::consts::PI;
            let p = if count == 1 { r.center() } else { r.center() + Vec2::new(ang.cos(), ang.sin()) * radius };
            // Face the room's centre (a lone spawn faces the first door, else north).
            let face =
                if count == 1 { doors.iter().find(|d| d.a == ri || d.b == ri).map(|d| d.center).unwrap_or(p + Vec2::new(0.0, -1.0)) } else { r.center() };
            let d = face - p;
            let yaw = (d.x).atan2(-d.y).to_degrees();
            let yaw = if yaw < 0.0 { yaw + 360.0 } else { yaw };
            spawns.push((if count == 1 { prefix.clone() } else { format!("{prefix}_{}", k + 1) }, group.clone(), [p.x, 0.0, p.y], yaw.round(), ri));
        }
    }

    // ---- fill requests (parsed now, placed later)
    struct Fill {
        room: usize,
        kinds: Vec<PropKind>,
        count: usize,
        seed: u64,
        scale: (f32, f32),
        colors: Vec<String>,
        min_gap: f32,
        clearance: f32,
        id: String,
    }
    let mut fills: Vec<Fill> = Vec::new();
    for (i, f) in root.get("fill").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let path = format!("fill[{i}]");
        let Some(o) = obj(f, &path, FILL_KEYS, &mut errs) else { continue };
        let room = o.get("room").and_then(Value::as_str).unwrap_or("");
        let Some(ri) = room_ix(room) else {
            errs.push(format!("{path}.room: unknown room '{room}' (rooms: {})", room_names()));
            continue;
        };
        let names: Vec<String> = match o.get("kind") {
            Some(Value::String(s)) => vec![s.clone()],
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
            _ => Vec::new(),
        };
        let mut kinds = Vec::new();
        for n in &names {
            match PropKind::from_name(n) {
                Some(k) => kinds.push(k),
                None => errs.push(format!("{path}.kind: '{n}' is not a prop (run `red_engine2 props` for the list, e.g. crate, barrel, chair, bookshelf)")),
            }
        }
        if names.is_empty() {
            errs.push(format!("{path}.kind: needs a prop name or a list of them, e.g. \"crate\""));
        }
        let scale = o.get("scale").and_then(|v| f32s(v, 2)).map(|s| (s[0], s[1])).unwrap_or((0.9, 1.15));
        fills.push(Fill {
            room: ri,
            kinds,
            count: o.get("count").and_then(Value::as_u64).unwrap_or(6).min(400) as usize,
            seed: o.get("seed").and_then(Value::as_u64).unwrap_or(i as u64 + 1),
            scale,
            colors: o.get("colors").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default(),
            min_gap: o.get("min_gap").and_then(Value::as_f64).unwrap_or(0.9) as f32,
            clearance: o.get("clearance").and_then(Value::as_f64).unwrap_or(0.7) as f32,
            id: o
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("{}_{}", rooms[ri].id, names.first().cloned().unwrap_or_else(|| "fill".into()))),
        });
    }
    let extra_clear: Vec<(Vec2, Vec2)> = root
        .get("keep_clear")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| f32s(v, 4).map(|r| (Vec2::new(r[0].min(r[2]), r[1].min(r[3])), Vec2::new(r[0].max(r[2]), r[1].max(r[3])))))
        .collect();
    let extra_objects: Vec<Value> = root.get("extra").and_then(Value::as_array).cloned().unwrap_or_default();
    let scene_merge: Map<String, Value> = root.get("scene").and_then(Value::as_object).cloned().unwrap_or_default();
    // A game's own prefab libraries (same format as `assets/*.json`) are merged into the scene's `prefabs`, so the built map is
    // self-contained: the server and clients need only the map, and the engine never has to be modified for a game's props.
    let mut prefab_defs: Vec<Value> = Vec::new();
    for (i, f) in root.get("prefab_files").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let path = format!("prefab_files[{i}]");
        let Some(rel) = f.as_str() else {
            errs.push(format!("{path}: expected a file path"));
            continue;
        };
        let Some(base) = base else {
            errs.push(format!("{path}: prefab files are read relative to the blueprint; compile it with `build FILE` (or `compile_in`)"));
            continue;
        };
        let text = match std::fs::read_to_string(base.join(rel)) {
            Ok(t) => t,
            Err(e) => {
                errs.push(format!("{path}: {}: {e}", base.join(rel).display()));
                continue;
            }
        };
        let defs: Vec<Value> = match serde_json::from_str::<Value>(&text) {
            Ok(Value::Array(a)) => a,
            Ok(Value::Object(o)) => o
                .into_iter()
                .map(|(name, mut d)| {
                    d["name"] = Value::String(name);
                    d
                })
                .collect(),
            Ok(_) => {
                errs.push(format!("{path}: {rel} must be a JSON array of prefab definitions (each with a \"name\") or an object {{name: definition}}"));
                continue;
            }
            Err(e) => {
                errs.push(format!("{path}: {rel} is not valid JSON: {e}"));
                continue;
            }
        };
        for d in defs {
            let name = d.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            if name.is_empty() {
                errs.push(format!("{path}: a prefab in {rel} has no \"name\""));
            } else if prefab_defs.iter().any(|p| p.get("name").and_then(Value::as_str) == Some(name.as_str())) {
                errs.push(format!("{path}: prefab '{name}' is defined twice across prefab_files"));
            } else {
                prefab_defs.push(d);
            }
        }
    }
    if !errs.is_empty() {
        return Err(errs);
    }

    // ---- base scene: floors, walls, lights
    let mut objects: Vec<Value> = Vec::new();
    for r in &rooms {
        objects.push(json!({"id": format!("floor_{}", r.id), "type": "plane", "size": [r3(r.size().x), r3(r.size().y)],
            "position": [r3(r.center().x), 0.01, r3(r.center().y)], "material": {"color": r.floor, "roughness": 0.9}}));
    }
    let (mut n_ext, mut n_part) = (0, 0);
    for run in wall_runs(&rooms) {
        let (from, to) = if run.vertical {
            (Vec2::new(run.coord, run.lo), Vec2::new(run.coord, run.hi))
        } else {
            (Vec2::new(run.lo, run.coord), Vec2::new(run.hi, run.coord))
        };
        let (id, thickness, color) = if run.partition {
            n_part += 1;
            (format!("part_{}", n_part - 1), WALL_PART, "#cfcfd6")
        } else {
            n_ext += 1;
            (format!("wall_ext_{}", n_ext - 1), WALL_EXT, "#b8b8c0")
        };
        let mut w = json!({"id": id, "type": "wall", "from": pt(from), "to": pt(to), "height": r3(height), "thickness": thickness,
            "material": {"color": color, "roughness": 0.9}});
        let openings: Vec<Value> = doors
            .iter()
            .filter(|d| run.partition && d.vertical == run.vertical && (if d.vertical { d.center.x } else { d.center.y } - run.coord).abs() < EPS)
            .filter(|d| {
                let along = if d.vertical { d.center.y } else { d.center.x };
                along >= run.lo - EPS && along <= run.hi + EPS
            })
            .map(|d| {
                let along = if d.vertical { d.center.y } else { d.center.x };
                json!({"kind": d.kind, "at": r3(along - run.lo), "width": r3(d.width)})
            })
            .collect();
        if !openings.is_empty() {
            w["openings"] = Value::Array(openings);
        }
        objects.push(w);
    }
    if ceiling {
        for r in &rooms {
            objects.push(json!({"id": format!("ceiling_{}", r.id), "type": "box", "size": [r3(r.size().x), 0.2, r3(r.size().y)],
                "position": [r3(r.center().x), r3(height + 0.1), r3(r.center().y)], "material": {"color": "#e8e4dc", "roughness": 0.95}}));
        }
    }
    let mid = rooms.iter().fold(Vec2::ZERO, |a, r| a + r.center()) / rooms.len() as f32;
    let mut lights: Vec<Value> = vec![json!({"id": "sun", "type": "directional", "direction": [-0.4, -1, -0.3], "color": "#ffffff", "intensity": 1.0,
        "cast_shadows": true, "shadow_center": [r3(mid.x), 0, r3(mid.y)]})];
    let mut notes: Vec<String> = Vec::new();
    for r in rooms.iter().filter(|r| r.lamp) {
        let (nx, nz) = ((r.size().x / 8.0).ceil().max(1.0) as usize, (r.size().y / 8.0).ceil().max(1.0) as usize);
        for ix in 0..nx {
            for iz in 0..nz {
                if lights.len() > MAX_LAMPS {
                    if !notes.iter().any(|n| n.contains("lamps capped")) {
                        notes.push(format!("lamps capped at {MAX_LAMPS}: the renderer supports 16 lights; set \"lamp\": false on rooms that do not need one"));
                    }
                    continue;
                }
                let p = Vec2::new(r.min.x + r.size().x * (ix as f32 + 0.5) / nx as f32, r.min.y + r.size().y * (iz as f32 + 0.5) / nz as f32);
                lights.push(json!({"id": format!("lamp_{}_{}", r.id, lights.len()), "type": "point", "position": [r3(p.x), r3(height - 0.3), r3(p.y)],
                    "color": "#fff4e0", "intensity": 10, "range": 10}));
            }
        }
    }
    objects.extend(extra_objects);

    // ---- fill, one request at a time so later fills avoid earlier ones
    let spawn_pts: Vec<(usize, Vec2)> = spawns.iter().map(|s| (s.4, Vec2::new(s.2[0], s.2[2]))).collect();
    let clear = keep_clear_rects(&rooms, &doors, &spawn_pts, &extra_clear);
    let base_doc = |objects: &[Value], lights: &[Value]| -> String {
        json!({"camera": {"position": [0, 1.7, 0], "target": [0, 1.5, 5]}, "lights": lights, "prefabs": prefab_defs, "objects": objects}).to_string()
    };
    let mut placed_total = 0usize;
    for f in &fills {
        let world = MapWorld::from_text(&base_doc(&objects, &lights), Path::new("blueprint.json"))
            .map_err(|e| e.into_iter().map(|m| format!("internal: {m}")).collect::<Vec<_>>())?;
        let r = &rooms[f.room];
        let inset = Vec2::splat(0.7);
        let params = ScatterParams {
            kinds: f.kinds.clone(),
            count: f.count,
            rects: vec![(r.min + inset, r.max - inset)],
            excludes: clear[f.room].clone(),
            seed: f.seed,
            id_prefix: f.id.clone(),
            colors: f.colors.clone(),
            scale: f.scale,
            min_gap: f.min_gap,
            clearance: f.clearance,
            y: 0.0,
            random_yaw: true,
            lint_ignore: Vec::new(),
        };
        let placed = scatter(&world, &params).map_err(|e| vec![format!("fill in '{}': {e}", r.id)])?;
        if placed.len() < f.count {
            notes.push(format!(
                "fill '{}' placed {} of {} (the room is too crowded with the doors/spawns kept clear: use a bigger room or fewer items)",
                f.id,
                placed.len(),
                f.count
            ));
        }
        placed_total += placed.len();
        objects.extend(placed);
    }

    // ---- assemble the scene
    let first = spawns.first().map(|s| Vec2::new(s.2[0], s.2[2])).unwrap_or_else(|| rooms[0].center());
    let look = rooms[0].center();
    let mut scene = Map::new();
    scene.insert("x-blueprint".into(), json!({"name": name, "version": BLUEPRINT_VERSION, "note": "generated by `red_engine2 build`; edit the blueprint and rebuild, or edit this map directly and drop the blueprint"}));
    scene.insert("meta".into(), json!({"fps": 30, "duration": 6, "resolution": [1280, 720]}));
    scene.insert("background".into(), json!({"sky_top": "#9fb4c8", "sky_bottom": "#dfe6ee"}));
    scene.insert("ambient".into(), json!({"color": "#ffffff", "intensity": 0.5}));
    scene.insert("camera".into(), json!({"fov": 75, "position": [r3(first.x), 1.7, r3(first.y)], "target": [r3(look.x), 1.5, r3(look.y)]}));
    scene.insert("post".into(), json!({"ao": 0.6, "outline": 0.4}));
    scene.insert("lights".into(), Value::Array(lights));
    scene.insert(
        "zones".into(),
        Value::Array(
            rooms
                .iter()
                .map(|r| json!({"id": r.id, "rect": [r3(r.min.x + 0.12), r3(r.min.y + 0.12), r3(r.max.x - 0.12), r3(r.max.y - 0.12)], "y": 0, "kind": "room"}))
                .collect(),
        ),
    );
    if !spawns.is_empty() {
        scene.insert(
            "spawns".into(),
            Value::Array(spawns.iter().map(|s| json!({"id": s.0, "position": [r3(s.2[0]), 0, r3(s.2[2])], "yaw_deg": s.3, "group": s.1})).collect()),
        );
    }
    if !doors.is_empty() {
        scene.insert(
            "portals".into(),
            Value::Array(
                doors
                    .iter()
                    .enumerate()
                    .map(|(i, d)| json!({"id": format!("door_{i}"), "between": [rooms[d.a].id, rooms[d.b].id], "center": pt(d.center), "width": r3(d.width), "height": 2.2, "open": true}))
                    .collect(),
            ),
        );
        scene.insert("interest".into(), json!({"cell_size": 8.0}));
    }
    if !prefab_defs.is_empty() {
        scene.insert("prefabs".into(), Value::Array(prefab_defs.clone()));
    }
    scene.insert("objects".into(), Value::Array(objects));
    for (k, v) in &scene_merge {
        if k != "checks" {
            scene.insert(k.clone(), v.clone());
        }
    }

    // ---- self-check: build the world, find real anchors, lint, and write checks that pass today
    let text0 = serde_json::to_string(&Value::Object(scene.clone())).map_err(|e| vec![e.to_string()])?;
    let world = MapWorld::from_text(&text0, Path::new("blueprint.json")).map_err(|e| {
        e.into_iter()
            .map(|m| format!("the compiled scene did not validate (this is a compiler bug or a `scene`/`extra` block problem): {m}"))
            .collect::<Vec<_>>()
    })?;
    let rr = reach::compute(&world, &ReachParams { start: Some(first), ..Default::default() });
    let grid = ColliderGrid::new(&world.colliders, rr.min, rr.min + Vec2::new(rr.nx as f32, rr.nz as f32) * rr.cell);
    let mut anchors: Vec<Option<Vec2>> = Vec::new();
    for r in &rooms {
        let c = r.center();
        let mut best: Option<(f32, Vec2)> = None;
        let (x0, x1) = (((r.min.x - rr.min.x) / rr.cell).ceil().max(0.0) as usize, (((r.max.x - rr.min.x) / rr.cell).floor() as usize).min(rr.nx - 1));
        let (z0, z1) = (((r.min.y - rr.min.y) / rr.cell).ceil().max(0.0) as usize, (((r.max.y - rr.min.y) / rr.cell).floor() as usize).min(rr.nz - 1));
        for iz in z0..=z1 {
            for ix in x0..=x1 {
                let p = rr.cell_center(iz * rr.nx + ix);
                // Stay comfortably inside the room and off anything solid (a prop's collider may be near the centre).
                if p.x < r.min.x + 0.8
                    || p.x > r.max.x - 0.8
                    || p.y < r.min.y + 0.8
                    || p.y > r.max.y - 0.8
                    || grid.blocked_r(p, 0.0, 0.7)
                    || rr.levels_at(p).is_empty()
                {
                    continue;
                }
                let d = (p - c).length();
                if best.is_none_or(|b| d < b.0) {
                    best = Some((d, p));
                }
            }
        }
        anchors.push(best.map(|b| b.1));
    }
    let findings_raw = lint::lint(&world, &rr);
    let warnings = findings_raw.iter().filter(|f| f.sev == Severity::Warn).count();
    let findings: Vec<(Severity, &'static str, String)> =
        findings_raw.iter().filter(|f| f.sev >= Severity::Warn).map(|f| (f.sev, f.code, f.message.clone())).collect();

    let mut checks = Map::new();
    checks.insert("lint".into(), json!({"max_errors": 0, "max_warnings": warnings}));
    let mut reach_checks = Vec::new();
    let mut walk_checks = Vec::new();
    for (i, r) in rooms.iter().enumerate() {
        match anchors[i] {
            Some(a) => {
                reach_checks.push(json!({"to": [r3(a.x), r3(a.y)], "from": pt(first), "why": format!("room {} is reachable", r.id)}));
                if (a - first).length() > 0.5 {
                    walk_checks.push(json!({"name": format!("first spawn to {}", r.id), "from": pt(first), "to": pt(a), "auto": true}));
                }
            }
            None => notes.push(format!("room '{}' has no reachable open floor from the first spawn: no doors connect it, or a fill/extra blocks it", r.id)),
        }
    }
    checks.insert("reach".into(), Value::Array(reach_checks));
    checks.insert("walk".into(), Value::Array(walk_checks));
    checks.insert("objects".into(), json!({"min_count": scene["objects"].as_array().map(Vec::len).unwrap_or(0).saturating_sub(2)}));
    if let Some(user) = scene_merge.get("checks").and_then(Value::as_object) {
        for (k, v) in user {
            match (checks.get_mut(k), v) {
                (Some(Value::Array(a)), Value::Array(b)) => a.extend(b.iter().cloned()),
                (Some(Value::Object(a)), Value::Object(b)) => a.extend(b.clone()),
                _ => {
                    checks.insert(k.clone(), v.clone());
                }
            }
        }
    }
    scene.insert("checks".into(), Value::Object(checks));
    // Keep the long `objects` list last so the short sections (zones, spawns, checks) read first.
    let objects_v = scene.remove("objects").unwrap_or(Value::Null);
    scene.insert("objects".into(), objects_v);
    let scene = Value::Object(scene);

    let n_objects = scene["objects"].as_array().map(Vec::len).unwrap_or(0);
    let mut summary = vec![format!(
        "blueprint '{name}': {} room(s), {} door(s), {} spawn(s){}, {n_objects} objects ({placed_total} fill props), {} check(s)",
        rooms.len(),
        doors.len(),
        spawns.len(),
        if spawns.is_empty() {
            String::new()
        } else {
            format!(" in group(s) {}", {
                let mut g: Vec<&str> = spawns.iter().map(|s| s.1.as_str()).collect();
                g.sort();
                g.dedup();
                g.join(", ")
            })
        },
        1 + scene["checks"]["reach"].as_array().map(Vec::len).unwrap_or(0) + scene["checks"]["walk"].as_array().map(Vec::len).unwrap_or(0),
    )];
    summary.extend(notes);
    let scene_text = format_scene(&scene);
    Ok(Built { scene_text, scene, summary, findings })
}

/// A small working blueprint (`build --example`): three rooms in a row, two spawns, crates and barrels.
pub fn example() -> String {
    r##"{
  "blueprint": 1,
  "name": "three_rooms",
  "height": 2.8,
  "rooms": [
    { "id": "hall",  "rect": [-14, -6, -4, 6] },
    { "id": "store", "rect": [-4, -6, 6, 6] },
    { "id": "yard",  "rect": [6, -6, 14, 6] }
  ],
  "doors": [
    { "between": ["hall", "store"], "width": 1.4 },
    { "between": ["store", "yard"], "width": 2.0, "kind": "arch" }
  ],
  "spawns": [
    { "room": "hall", "group": "duel", "count": 2 }
  ],
  "fill": [
    { "room": "store", "kind": ["crate", "barrel"], "count": 10, "seed": 7 },
    { "room": "yard",  "kind": "crate", "count": 5, "seed": 3 }
  ]
}
"##
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(bp: &str) -> Result<Built, Vec<String>> {
        compile(&serde_json::from_str::<Value>(bp).unwrap())
    }

    #[test]
    fn the_example_builds_a_clean_self_checking_map() {
        let b = build(&example()).unwrap();
        assert_eq!(b.errors(), 0, "{:?}", b.findings);
        let s = &b.scene;
        assert_eq!(s["zones"].as_array().unwrap().len(), 3);
        assert_eq!(s["portals"].as_array().unwrap().len(), 2);
        assert_eq!(s["spawns"].as_array().unwrap().len(), 2);
        assert!(s["checks"]["walk"].as_array().unwrap().iter().all(|w| w["auto"] == true));
        assert!(b.summary[0].contains("3 room(s), 2 door(s), 2 spawn(s)"), "{:?}", b.summary);
    }

    #[test]
    fn the_built_map_passes_its_own_verify() {
        let b = build(&example()).unwrap();
        let dir = std::env::temp_dir().join("re2_blueprint_verify");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("three_rooms.json");
        std::fs::write(&p, &b.scene_text).unwrap();
        let r = super::super::verify::run(&p, &super::super::verify::Options { skip_views: true, out_dir: Some(dir.clone()), ..Default::default() }).unwrap();
        assert_eq!(r.failed(), 0, "{}", r.render());
        assert!(r.results.len() >= 5, "lint + reach + auto walks + objects: {}", r.render());
    }

    #[test]
    fn output_is_deterministic() {
        assert_eq!(build(&example()).unwrap().scene_text, build(&example()).unwrap().scene_text);
    }

    #[test]
    fn a_grid_of_rooms_with_a_perimeter_and_no_leaks() {
        // 2x2 grid; doors link the ring. Shared edges must become single partitions, the outside a closed loop.
        let b = build(
            r##"{"blueprint":1,"rooms":[
              {"id":"nw","rect":[-8,-6,0,0]},{"id":"ne","rect":[0,-6,8,0]},
              {"id":"sw","rect":[-8,0,0,6]},{"id":"se","rect":[0,0,8,6]}],
              "doors":[{"between":["nw","ne"]},{"between":["ne","se"]},{"between":["se","sw"]},{"between":["sw","nw"]}],
              "spawns":[{"room":"nw","group":"a"}],"fill":[{"room":"se","kind":"crate","count":4}]}"##,
        )
        .unwrap();
        assert_eq!(b.errors(), 0, "{:?}", b.findings);
        assert!(!b.findings.iter().any(|f| f.1 == "leak"), "{:?}", b.findings);
        let walls: Vec<&Value> = b.scene["objects"].as_array().unwrap().iter().filter(|o| o["type"] == "wall").collect();
        let parts = walls.iter().filter(|w| w["id"].as_str().unwrap().starts_with("part_")).count();
        assert_eq!(parts, 2, "one vertical and one horizontal partition, each shared by two rooms");
    }

    #[test]
    fn fill_never_seals_a_door() {
        // A crowded room with doors on opposite sides: every seed must leave the route open.
        for seed in 1..=12 {
            let b = build(&format!(
                r##"{{"blueprint":1,"rooms":[{{"id":"a","rect":[-10,-5,0,5]}},{{"id":"b","rect":[0,-5,10,5]}}],
                  "doors":[{{"between":["a","b"],"width":1.2}}],"spawns":[{{"room":"a","group":"g"}}],
                  "fill":[{{"room":"b","kind":["crate","barrel","chair"],"count":40,"seed":{seed}}}]}}"##
            ))
            .unwrap();
            assert_eq!(b.errors(), 0, "seed {seed}: {:?}", b.findings);
            let dir = std::env::temp_dir().join(format!("re2_blueprint_seed{seed}"));
            std::fs::create_dir_all(&dir).unwrap();
            let p = dir.join("m.json");
            std::fs::write(&p, &b.scene_text).unwrap();
            let r = super::super::verify::run(&p, &super::super::verify::Options { skip_views: true, out_dir: Some(dir), ..Default::default() }).unwrap();
            assert_eq!(r.failed(), 0, "seed {seed}:\n{}", r.render());
        }
    }

    #[test]
    fn mistakes_are_readable_errors_with_a_fix() {
        let e = build(r##"{"blueprint":1,"rooms":[{"id":"a","rect":[0,0,6,6]},{"id":"b","rect":[10,0,16,6]}],"doors":[{"between":["a","b"]}]}"##).unwrap_err();
        assert!(e.iter().any(|m| m.contains("do not share a wall")), "{e:?}");
        let e = build(r##"{"blueprint":1,"rooms":[{"id":"a","rect":[0,0,6,6]},{"id":"b","rect":[3,0,9,6]}]}"##).unwrap_err();
        assert!(e.iter().any(|m| m.contains("overlap")), "{e:?}");
        let e = build(r##"{"blueprint":1,"rooms":[{"id":"a","rect":[0,0,6,6],"colour":"red"}]}"##).unwrap_err();
        assert!(e.iter().any(|m| m.contains("unknown field")), "{e:?}");
        let e = build(r##"{"blueprint":1,"rooms":[{"id":"a","rect":[0,0,6,6]},{"id":"b","rect":[6,0,12,6]}],"doors":[{"between":["a","b"],"width":0.6}]}"##)
            .unwrap_err();
        assert!(e.iter().any(|m| m.contains("narrower")), "{e:?}");
        let e = build(r##"{"blueprint":1,"rooms":[{"id":"a","rect":[0,0,6,6]}],"fill":[{"room":"a","kind":"couchh"}]}"##).unwrap_err();
        assert!(e.iter().any(|m| m.contains("not a prop")), "{e:?}");
        assert!(build(r##"{"rooms":[]}"##).is_err());
    }

    #[test]
    fn a_games_own_prefab_library_is_merged_into_the_map_without_touching_the_engine() {
        let dir = std::env::temp_dir().join(format!("re2_blueprint_prefabs_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("shelf.json"),
            r##"[{"name":"my_shelf","tags":["store"],"desc":"a game-specific shelf","params":{"color":"#8a6a3f"},
                "objects":[{"id":"body","type":"box","size":[1.2,1.8,0.4],"position":[0,0.9,0],"material":{"color":"$color"}}]}]"##,
        )
        .unwrap();
        let bp: Value = serde_json::from_str(
            r##"{"blueprint":1,"prefab_files":["shelf.json"],"rooms":[{"id":"a","rect":[-8,-6,8,6]}],
                "extra":[{"id":"shelf_1","type":"prefab","prefab":"my_shelf","position":[5,0,4]}]}"##,
        )
        .unwrap();
        let b = compile_in(&bp, Some(&dir)).unwrap();
        assert_eq!(b.errors(), 0, "{:?}", b.findings);
        assert_eq!(b.scene["prefabs"][0]["name"], "my_shelf");
        // Without a location the blueprint says what is missing; an unknown file is an error naming the path.
        assert!(compile(&bp).unwrap_err().iter().any(|m| m.contains("relative to the blueprint")));
        let missing: Value = serde_json::from_str(r##"{"blueprint":1,"prefab_files":["nope.json"],"rooms":[{"id":"a","rect":[0,0,6,6]}]}"##).unwrap();
        let e = compile_in(&missing, Some(&dir)).unwrap_err();
        assert!(e.iter().any(|m| m.contains("prefab_files[0]") && m.contains("nope.json")), "{e:?}");
    }

    #[test]
    fn scene_block_merges_rules_and_extra_checks() {
        let b = build(
            r##"{"blueprint":1,"rooms":[{"id":"a","rect":[-6,-6,6,6]}],"spawns":[{"room":"a","group":"solo"}],
               "scene":{"vars":{"score":0},"checks":{"objects":{"exist":["floor_a"]}}}}"##,
        )
        .unwrap();
        assert_eq!(b.scene["vars"]["score"], 0);
        assert_eq!(b.scene["checks"]["objects"]["exist"][0], "floor_a");
        assert!(b.scene["checks"]["objects"]["min_count"].is_number(), "user keys merge into, not replace, the generated object");
    }
}
