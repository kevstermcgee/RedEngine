//! `red_engine2 race-track`: a complete, raceable kart map from a few numbers.
//!
//! Great Outdoors needed a 400-line Python generator to get from "a rounded loop" to a scene a kart race can run on: road pieces, barriers with no gaps on the
//! outside of bends, a start line, gates, item boxes, a grid, a racing line for the bots, the animals, a `match` block and self-checks. Every kart game needs the
//! same things, so this is that generator, built in: `race-track maps/main.json` writes a scene that passes its own lint, and `race-test` shows bots finishing it.
//!
//! The circuit is a rounded rectangle driven counter-clockwise seen from above (east along the south straight, north up the east side, west along the north
//! straight, south down the west side); everything is derived from its centre line. Terrain (mud, a ford, a dirt stretch) and trees are optional.

use serde_json::{json, Map, Value};
use std::f64::consts::{PI, TAU};

/// What to build. [`Default`] is the Great Outdoors size: a lap of about 650 m that a kart takes in 25-35 s.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackSpec {
    /// Half the width of the loop (the centre line of the east and west straights is at `x = +-half_width`), m.
    pub half_width: f64,
    /// Half the height of the loop (the north and south straights are at `z = +-half_height`), m.
    pub half_height: f64,
    /// Radius of the four bends, m.
    pub radius: f64,
    /// Road width, m.
    pub road: f64,
    /// Laps in the race.
    pub laps: u8,
    /// Mud across the east straight, a ford across the north, packed dirt on the west (each animal is affected differently).
    pub terrain: bool,
    /// How many trees to stand along the track (0 = none).
    pub trees: u32,
}

impl Default for TrackSpec {
    fn default() -> Self {
        TrackSpec { half_width: 110.0, half_height: 70.0, radius: 40.0, road: 22.0, laps: 3, terrain: true, trees: 160 }
    }
}

impl TrackSpec {
    /// Why this spec cannot be built, if it cannot.
    pub fn check(&self) -> Result<(), String> {
        let (w, h, r, road) = (self.half_width, self.half_height, self.radius, self.road);
        if !(8.0..=60.0).contains(&road) {
            return Err(format!("--road {road} m: a road between 8 and 60 m wide (karts are about 2 m wide; 22 is roomy for 8)"));
        }
        if r < road * 0.9 {
            return Err(format!(
                "--radius {r} m is tighter than the road is wide ({road} m): the inside of the bend would fold over; use at least {:.0}",
                road * 0.9
            ));
        }
        if w < r + 10.0 || h < r + 10.0 {
            return Err(format!("the loop is too small for its bends: --half-width and --half-height must each be at least radius + 10 = {:.0} m", r + 10.0));
        }
        if !(1..=20).contains(&self.laps) {
            return Err("--laps must be 1..20".into());
        }
        if self.trees > 2000 {
            return Err("--trees at most 2000".into());
        }
        Ok(())
    }
}

/// The centre line, in driving order from the start line at `(0, half_height)`, one point per piece: five straights joined by four quarter-circle bends.
fn centre_line(s: &TrackSpec, step: f64) -> Vec<(f64, f64)> {
    let (w, h, r) = (s.half_width, s.half_height, s.radius);
    let mut out = Vec::new();
    let straight = |out: &mut Vec<(f64, f64)>, a: (f64, f64), b: (f64, f64)| {
        let n = (((b.0 - a.0).hypot(b.1 - a.1) / step) as usize).max(1);
        for i in 0..n {
            let t = i as f64 / n as f64;
            out.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
    };
    let arc = |out: &mut Vec<(f64, f64)>, c: (f64, f64), a0: f64, a1: f64| {
        let n = (((a1 - a0).abs() * r / step) as usize).max(4);
        for i in 0..n {
            let a = a0 + (a1 - a0) * i as f64 / n as f64;
            out.push((c.0 + r * a.cos(), c.1 + r * a.sin()));
        }
    };
    straight(&mut out, (0.0, h), (w - r, h)); // south straight, heading +x
    arc(&mut out, (w - r, h - r), PI / 2.0, 0.0); // south-east bend, then north up the east side
    straight(&mut out, (w, h - r), (w, -h + r));
    arc(&mut out, (w - r, -h + r), 0.0, -PI / 2.0); // north-east bend, then west along the north side
    straight(&mut out, (w - r, -h), (-w + r, -h));
    arc(&mut out, (-w + r, -h + r), -PI / 2.0, -PI); // north-west bend, then south down the west side
    straight(&mut out, (-w, -h + r), (-w, h - r));
    arc(&mut out, (-w + r, h - r), PI, PI / 2.0); // south-west bend, then east back to the line
    straight(&mut out, (-w + r, h), (0.0, h));
    out
}

struct Ring {
    path: Vec<(f64, f64)>,
}

impl Ring {
    fn n(&self) -> usize {
        self.path.len()
    }
    /// Piece `i`: its start, end, unit direction and length.
    fn seg(&self, i: usize) -> ((f64, f64), (f64, f64), f64, f64, f64) {
        let (a, b) = (self.path[i % self.n()], self.path[(i + 1) % self.n()]);
        let (dx, dz) = (b.0 - a.0, b.1 - a.1);
        let l = dx.hypot(dz);
        (a, b, dx / l, dz / l, l)
    }
    /// Yaw in degrees for a plane or box whose long axis is local Z.
    fn heading(&self, i: usize) -> f64 {
        let (_, _, ux, uz, _) = self.seg(i);
        ux.atan2(uz).to_degrees()
    }
    /// The change of heading between piece `i` and the next, wrapped to -pi..pi.
    fn turn(&self, i: usize) -> f64 {
        let d = (self.heading(i + 1) - self.heading(i)).to_radians();
        (d + PI).rem_euclid(TAU) - PI
    }
    /// The centre line pushed sideways by `off` on `side` (-1 or +1) with mitred joins, so pieces built between neighbouring vertices meet exactly.
    fn offset(&self, off: f64, side: f64) -> Vec<(f64, f64)> {
        (0..self.n())
            .map(|j| {
                let (_, _, upx, upz, _) = self.seg((j + self.n() - 1) % self.n());
                let (_, _, unx, unz, _) = self.seg(j);
                let n0 = (-upz * side, upx * side);
                let n1 = (-unz * side, unx * side);
                let (mx, mz) = (n0.0 + n1.0, n0.1 + n1.1);
                let ml = mx.hypot(mz);
                let (mx, mz) = (mx / ml, mz / ml);
                let miter = off / (mx * n1.0 + mz * n1.1).max(0.3);
                (self.path[j].0 + mx * miter, self.path[j].1 + mz * miter)
            })
            .collect()
    }
}

/// Centre, yaw (degrees) and length of the straight piece from `a` to `b`, lengthened by `extra`.
fn piece(a: (f64, f64), b: (f64, f64), extra: f64) -> (f64, f64, f64, f64) {
    let (dx, dz) = (b.0 - a.0, b.1 - a.1);
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0, dx.atan2(dz).to_degrees(), dx.hypot(dz) + extra)
}

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

const DIRT: &str = "#d2b48f";
const DIRT_EDGE: &str = "#e9d6b8";
const HEDGE: [&str; 3] = ["#86c99a", "#9fd6a6", "#78bb8f"];
const LOG: &str = "#b98d62";
const LOG_DARK: &str = "#a47850";
const PINES: [&str; 4] = ["#8fd3b4", "#9bd7a4", "#7fc9ae", "#a6dcc0"];

/// Builds the scene for `spec`.
pub fn build(spec: &TrackSpec) -> Result<Value, String> {
    spec.check()?;
    let (w, h, road) = (spec.half_width, spec.half_height, spec.road);
    let ring = Ring { path: centre_line(spec, 8.0) };
    let n = ring.n();
    let mut objects: Vec<Value> = Vec::new();
    let mut zones: Vec<Value> = Vec::new();

    let floor_w = (w + spec.radius + 200.0) * 2.0;
    let floor_h = (h + spec.radius + 200.0) * 2.0;
    objects.push(
        json!({"id":"floor","type":"plane","size":[floor_w, floor_h],"position":[0,0.0,0],"material":{"color":"#8fcf9b","roughness":1.0},"collide":false}),
    );

    // The road: one plane per piece, longer on the outside of a bend by the wedge that opens between two straight pieces; neighbours sit at slightly different
    // heights so overlapping planes never z-fight.
    for i in 0..n {
        let (a, b, _, _, len) = ring.seg(i);
        let wedge = road * (ring.turn(i) / 2.0).tan().abs() + 0.5;
        objects.push(json!({"id":format!("road_{i}"),"type":"plane","size":[road, r3(len + wedge)],
            "position":[r3((a.0 + b.0) / 2.0), r3(0.03 + 0.01 * (i % 5) as f64), r3((a.1 + b.1) / 2.0)],"rotation":[0, r3(ring.heading(i)), 0],
            "material":{"color":DIRT,"roughness":0.98},"collide":false}));
    }
    for side in [-1.0, 1.0] {
        let vs = ring.offset(road / 2.0 - 0.9, side);
        let tag = if side < 0.0 { "l" } else { "r" };
        for j in 0..n {
            let (cx, cz, yaw, len) = piece(vs[j], vs[(j + 1) % n], 0.3);
            objects.push(
                json!({"id":format!("edge_{j}_{tag}"),"type":"plane","size":[1.8, r3(len)],"position":[r3(cx), r3(0.08 + 0.01 * (j % 5) as f64), r3(cz)],
                "rotation":[0, r3(yaw), 0],"material":{"color":DIRT_EDGE,"roughness":0.98},"collide":false}),
            );
        }
    }

    // Barriers on both edges along the mitred offset curve (no gaps on the outside of a bend): hedges and stacked logs in runs of three.
    for side in [-1.0, 1.0] {
        let vs = ring.offset(road / 2.0 + 0.7, side);
        let tag = if side < 0.0 { "l" } else { "r" };
        for i in 0..n {
            let (px, pz, yaw, len) = piece(vs[i], vs[(i + 1) % n], 0.3);
            let (px, pz) = (r3(px), r3(pz));
            if (i / 3 + usize::from(side > 0.0)) % 2 == 0 {
                objects
                    .push(json!({"id":format!("hedge_{i}_{tag}"),"type":"box","size":[1.3, 1.6, r3(len)],"position":[px, 0.8, pz],"rotation":[0, r3(yaw), 0],
                    "material":{"color":HEDGE[i % 3],"roughness":1.0}}));
            } else {
                objects
                    .push(json!({"id":format!("log_{i}_{tag}"),"type":"box","size":[1.1, 1.5, r3(len)],"position":[px, 0.75, pz],"rotation":[0, r3(yaw), 0],
                    "material":{"color":if i % 2 == 0 { LOG } else { LOG_DARK },"roughness":0.95}}));
                // A log lying along the barrier: a group turned to the piece's heading holding a cylinder laid on its side (nested turns are unambiguous).
                objects.push(json!({"id":format!("logtop_{i}_{tag}"),"type":"group","position":[px, 1.6, pz],"rotation":[0, r3(yaw), 0],"collide":false,
                    "children":[{"id":format!("logtop_{i}_{tag}_c"),"type":"cylinder","radius":0.5,"height":r3(len),"rotation":[90,0,0],
                        "material":{"color":"#c9a173","roughness":0.95},"collide":false}]}));
            }
        }
    }

    // The start / finish line, a chequered strip, and its arch.
    let (line_x, line_z) = (0.0, h);
    let squares = 11;
    for k in 0..squares {
        for row in 0..2 {
            let z = line_z - road / 2.0 + (k as f64 + 0.5) * (road / squares as f64);
            objects.push(json!({"id":format!("chk_{k}_{row}"),"type":"plane","size":[1.2, r3(road / squares as f64)],
                "position":[r3(line_x + (row as f64 - 0.5) * 1.2), 0.14, r3(z)],
                "material":{"color":if (k + row) % 2 == 0 { "#ffffff" } else { "#5b5470" },"roughness":0.9},"collide":false}));
        }
    }
    for (z, tag) in [(line_z - road / 2.0 - 1.6, "a"), (line_z + road / 2.0 + 1.6, "b")] {
        objects.push(
            json!({"id":format!("arch_post_{tag}"),"type":"box","size":[0.9,7.0,0.9],"position":[line_x, 3.5, r3(z)],"material":{"color":LOG},"collide":false}),
        );
    }
    objects.push(
        json!({"id":"arch_bar","type":"box","size":[1.6,1.4,r3(road + 4.2)],"position":[line_x, 7.2, line_z],"material":{"color":LOG_DARK},"collide":false}),
    );
    objects.push(json!({"id":"arch_stripe","type":"box","size":[1.7,0.5,r3(road + 4.3)],"position":[line_x, 7.2, line_z],
        "material":{"color":"#fff1c1","emissive":"#fff1c1"},"collide":false}));

    // Gates: one across each straight, thin along the road; the first is the start line.
    zones.push(json!({"id":"gate_line","rect":[line_x - 1.5, line_z - road / 2.0 - 1.0, line_x + 1.5, line_z + road / 2.0 + 1.0]}));
    zones.push(json!({"id":"gate_east","rect":[w - road / 2.0 - 1.0, -1.5, w + road / 2.0 + 1.0, 1.5]}));
    zones.push(json!({"id":"gate_north","rect":[-1.5, -h - road / 2.0 - 1.0, 1.5, -h + road / 2.0 + 1.0]}));
    zones.push(json!({"id":"gate_west","rect":[-w - road / 2.0 - 1.0, -1.5, -w + road / 2.0 + 1.0, 1.5]}));

    // Terrain across the racing line, only where a straight is long enough to hold it.
    let mut surfaces: Vec<Value> = Vec::new();
    let mut patch = |zones: &mut Vec<Value>, objects: &mut Vec<Value>, id: &str, kind: &str, rect: [f64; 4], color: &str, y: f64| {
        zones.push(json!({"id":id,"rect":rect}));
        surfaces.push(json!({"zone":id,"kind":kind}));
        objects.push(json!({"id":format!("{id}_look"),"type":"plane","size":[r2(rect[2] - rect[0]), r2(rect[3] - rect[1])],
            "position":[r2((rect[0] + rect[2]) / 2.0), y, r2((rect[1] + rect[3]) / 2.0)],"material":{"color":color,"roughness":if kind == "water" { 0.25 } else { 1.0 }},"collide":false}));
    };
    if spec.terrain {
        let east_len = 2.0 * (h - spec.radius);
        let north_len = 2.0 * (w - spec.radius);
        if east_len >= 40.0 {
            let c = -east_len * 0.25;
            patch(&mut zones, &mut objects, "mud_east", "mud", [w - 7.0, c - 11.0, w + 3.0, c + 11.0], "#8a6a4a", 0.20);
        }
        if north_len >= 60.0 {
            let c = north_len * 0.15;
            patch(&mut zones, &mut objects, "ford_north", "water", [c - 16.0, -h - 5.0, c + 16.0, -h + 5.0], "#a9d8f0", 0.21);
        }
        if east_len >= 40.0 {
            let c = east_len * 0.2;
            patch(&mut zones, &mut objects, "dirt_west", "dirt", [-w - 5.0, c - 14.0, -w + 5.0, c + 14.0], "#b98f62", 0.19);
        }
    }

    // Item boxes: three abreast at four places round the lap.
    let mut box_ids: Vec<Value> = Vec::new();
    let rows = [(-55.0_f64.max(-w * 0.5), h, 'x'), (w, -h * 0.35, 'z'), (w * 0.25, -h, 'x'), (-w, h * 0.4, 'z')];
    for (r, (cx, cz, axis)) in rows.iter().enumerate() {
        for k in [-1.0, 0.0, 1.0] {
            let (px, pz) = if *axis == 'x' { (*cx, cz + k * 6.0) } else { (cx + k * 6.0, *cz) };
            let id = format!("box_{r}_{}", (k + 1.0) as i32);
            zones.push(json!({"id":id,"rect":[px - 1.5, pz - 1.5, px + 1.5, pz + 1.5]}));
            objects.push(json!({"id":id,"type":"box","size":[1.5,1.5,1.5],"position":[px, 1.4, pz],"rotation":[0,45,0],"collide":false,
                "material":{"color":if (r as i32 + k as i32).rem_euclid(2) == 0 { "#ffe27a" } else { "#ffb3c8" },"emissive":"#5a4a1e","roughness":0.4}}));
            box_ids.push(json!(id));
        }
    }

    // Trees standing outside the barriers (a small deterministic generator, so the same spec gives the same map).
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    for t in 0..spec.trees {
        let i = (rnd() * n as f64) as usize % n;
        let (a, _, ux, uz, len) = ring.seg(i);
        let side = if rnd() < 0.5 { -1.0 } else { 1.0 };
        let off = road / 2.0 + 6.0 + rnd() * 26.0;
        let along = rnd() * len;
        let (x, z) = (a.0 + ux * along - uz * side * off, a.1 + uz * along + ux * side * off);
        let ht = 9.0 + rnd() * 10.0;
        let col = PINES[(rnd() * 4.0) as usize % 4];
        objects.push(json!({"id":format!("tree_{t}_t"),"type":"cylinder","radius":0.6,"height":r2(ht * 0.35),"position":[r2(x), r2(ht * 0.175), r2(z)],"material":{"color":LOG_DARK},"collide":false}));
        objects.push(json!({"id":format!("tree_{t}_c"),"type":"cone","radius":r2(ht * 0.34),"height":r2(ht * 0.8),"position":[r2(x), r2(ht * 0.55), r2(z)],"material":{"color":col,"roughness":0.9},"collide":false}));
    }

    // The bots' racing line (the centre line, every other point) and eight grid places behind the line, two abreast, facing east.
    let line: Vec<Value> = (0..n).step_by(2).map(|j| json!([r2(ring.path[j].0), r2(ring.path[j].1)])).collect();
    let spawns: Vec<Value> = (0..8)
        .map(|i| {
            let (row, col) = (i / 2, i % 2);
            json!({"id":format!("grid_{}", i + 1),"position":[r2(line_x - 7.0 - 5.0 * row as f64), 0, r2(line_z + if col == 0 { -3.2 } else { 3.2 })],"yaw_deg":90,"group":"race"})
        })
        .collect();

    // The animals: the core `karts` pack, parked out of sight until the client places them.
    for d in crate::sim::kart::Driver::ALL {
        let id = crate::net::fleet::model_id(d);
        objects.push(json!({"id":id,"type":"prefab","prefab":id,"position":[0,-50,0],"collide":false,"lint_ignore":["sunk"]}));
    }

    let mut scene = Map::new();
    scene.insert("camera".into(), json!({"position":[0, h * 0.9, w * 1.3],"target":[0,0,0],"fov":60,"near":0.3,"far":(w.max(h) * 6.0).max(900.0)}));
    scene.insert("background".into(), json!({"sky_top":"#bcdcff","sky_bottom":"#ffe6d0"}));
    scene.insert("ambient".into(), json!({"color":"#f6f0e0","intensity":0.62}));
    scene.insert("lights".into(), json!([{"id":"sun","type":"directional","direction":[-0.4,-1.0,-0.6],"color":"#fff1d6","intensity":1.15}]));
    scene.insert("player".into(), json!({"mode":"peaceful"}));
    scene.insert("music".into(), json!(false));
    scene.insert("zones".into(), Value::Array(zones));
    scene.insert("spawns".into(), Value::Array(spawns));
    let mut race = json!({"laps":spec.laps,"gates":["gate_line","gate_east","gate_north","gate_west"],"countdown_secs":3,"finish_grace_secs":30,
        "item_boxes":box_ids,"item_respawn_secs":6,"line":line});
    if !surfaces.is_empty() {
        race["surfaces"] = Value::Array(surfaces);
    }
    scene.insert("race".into(), race);
    scene.insert("bots".into(), json!({"fill":8,"skill":"normal"}));
    // Lobby (pick your animal, ready up), a one-second hold, the race with its own countdown, the results, and the lobby again.
    scene.insert("match".into(), json!({"min_players":1,"countdown_secs":1,"round_secs":600,"results_secs":20,"join_in_progress":false,"ready_check":true}));
    scene.insert("checks".into(), json!({"lint":{"max_errors":0}}));
    scene.insert("objects".into(), Value::Array(objects));
    Ok(Value::Object(scene))
}

/// A one-paragraph summary of what was built, for the command's output.
pub fn summary(spec: &TrackSpec, scene: &Value) -> String {
    let objects = scene["objects"].as_array().map_or(0, Vec::len);
    let lap = 4.0 * (spec.half_width - spec.radius) + 4.0 * (spec.half_height - spec.radius) + TAU * spec.radius;
    format!(
        "{objects} objects, a lap of about {lap:.0} m ({} laps), road {} m wide, 8 grid places, 12 item boxes{}. Next: `red_engine2 lint`, `race-test` (bots race it headless) and `frame` to look at it.",
        spec.laps,
        spec.road,
        if spec.terrain { ", mud / ford / dirt" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_track_builds_parses_and_lints_clean() {
        let scene = build(&TrackSpec::default()).unwrap();
        let text = serde_json::to_string(&scene).unwrap();
        let parsed = crate::schema::parse_scene(&text).unwrap_or_else(|e| panic!("{e:?}"));
        let race = parsed.race.as_ref().expect("a race block");
        assert_eq!((race.laps, race.gates.len(), race.item_boxes.len(), race.surfaces.len()), (3, 4, 12, 3));
        assert!(race.line.len() > 20, "a racing line for the bots");
        assert_eq!(crate::sim::spawns::parse_spawns(&text).unwrap().len(), 8);
        let world = crate::tools::world::MapWorld::from_text(&text, std::path::Path::new("track.json")).unwrap_or_else(|e| panic!("{e:?}"));
        let reach = crate::tools::reach::compute(&world, &crate::tools::reach::ReachParams { cell: 0.5, ..Default::default() });
        let errors: Vec<_> = crate::tools::lint::lint(&world, &reach).into_iter().filter(|f| f.sev == crate::tools::lint::Severity::Error).collect();
        assert!(errors.is_empty(), "{}", crate::tools::lint::format_report(&errors));
    }

    #[test]
    fn bots_finish_the_default_track_and_a_small_one_without_terrain() {
        for spec in [TrackSpec::default(), TrackSpec { half_width: 70.0, half_height: 55.0, radius: 28.0, road: 16.0, laps: 2, terrain: false, trees: 0 }] {
            let dir = std::env::temp_dir().join(format!("red_race_track_{}_{}", std::process::id(), spec.half_width as u32));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("track.json");
            std::fs::write(&path, serde_json::to_string(&build(&spec).unwrap()).unwrap()).unwrap();
            let report = crate::tools::racetest::run(&path, 8, 0.8, 300.0, &[]).unwrap();
            assert!(report.all_finished(), "{spec:?}\n{}", crate::tools::racetest::render(&report));
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn impossible_tracks_say_why() {
        let bad = |f: fn(&mut TrackSpec)| {
            let mut s = TrackSpec::default();
            f(&mut s);
            build(&s).unwrap_err()
        };
        assert!(bad(|s| s.road = 3.0).contains("--road"));
        assert!(bad(|s| s.radius = 10.0).contains("--radius"));
        assert!(bad(|s| s.half_width = 30.0).contains("too small"));
        assert!(bad(|s| s.laps = 0).contains("--laps"));
    }

    #[test]
    fn the_same_spec_gives_the_same_map() {
        assert_eq!(build(&TrackSpec::default()).unwrap(), build(&TrackSpec::default()).unwrap());
    }
}
