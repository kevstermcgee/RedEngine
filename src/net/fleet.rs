//! The kart models of a race, on the client: which scene object is which driver, where each kart, Acorn and plank is drawn this frame, and which item boxes
//! are gone (ADR 2026-09-29-great-outdoors-karts-as-a-first-class-engine-feature).
//!
//! Like the avatars, everything that can appear must already be in the scene (hidden) before the renderer is built. A race has at most one kart per driver, so
//! a driver's model is a fixed object: the map's own `kart_<animal>` object when it has one (a game brings its art as prefab instances, e.g. `kart_duck`), else
//! a plain built-in kart in the driver's colour, so any race scene is playable and visible with no art at all. Acorns and planks come from two pools of
//! fallback objects. Item boxes are the map's own objects, named like the zone they sit in (`race.item_boxes`); a taken one is hidden until it respawns.

use crate::net::protocol::{HazardSnap, KartSnap, MAX_HAZARDS_PER_SNAPSHOT};
use crate::scene_pool::{hide_object, ScenePool};
use crate::schema::{Object, Scene};
use crate::sim::kart::Driver;
use crate::track::Track;
use glam::Vec3;

/// The id a game gives a driver's model object.
pub fn model_id(driver: Driver) -> String {
    format!("kart_{}", driver.name().to_lowercase())
}

/// The plain kart a driver wears when the map brings no art of its own: a coloured tub on four wheels with a round head.
fn fallback_kart(driver: Driver) -> Object {
    let (body, head) = match driver {
        Driver::Duck => ("#ffe27a", "#fff2b8"),
        Driver::Bunny => ("#c7f0dc", "#fdeef4"),
        Driver::Deer => ("#c9edd0", "#e8c39e"),
        Driver::Coyote => ("#f4c7b0", "#d9b38c"),
        Driver::Hawk => ("#bfe3ff", "#f3e9dc"),
        Driver::Bear => ("#ffe0b8", "#b98f6f"),
        Driver::Wolf => ("#e3d3f0", "#b8c0d8"),
        Driver::Beaver => ("#b8895a", "#b08560"),
    };
    let id = model_id(driver);
    let wheels: Vec<String> = [(-0.6, -0.65), (0.6, -0.65), (-0.6, 0.65), (0.6, 0.65)]
        .iter()
        .enumerate()
        .map(|(i, (x, z))| format!(r##"{{"id":"{id}_w{i}","type":"cylinder","radius":0.3,"height":0.24,"position":[{x},0.3,{z}],"rotation":[0,0,90],"material":{{"color":"#5b5470"}}}}"##))
        .collect();
    let json = format!(
        r##"{{"camera":{{"position":[0,1,5],"target":[0,0,0]}},"objects":[{{"id":"{id}","type":"group","collide":false,"children":[
            {{"id":"{id}_tub","type":"box","size":[1.0,0.3,1.9],"position":[0,0.4,0],"material":{{"color":"{body}"}}}},
            {{"id":"{id}_body","type":"sphere","radius":0.3,"position":[0,1.0,0.2],"material":{{"color":"{head}"}}}},
            {{"id":"{id}_head","type":"sphere","radius":0.26,"position":[0,1.4,0.1],"material":{{"color":"{head}"}}}},
            {}]}}]}}"##,
        wheels.join(",")
    );
    first_object(&json)
}

fn hazard_object(id: &str, json_object: &str) -> Object {
    first_object(&format!(r##"{{"camera":{{"position":[0,1,5],"target":[0,0,0]}},"objects":[{json_object}]}}"##).replace("$ID", id))
}

#[allow(clippy::panic)] // the JSON is this file's own constant, and `tests` builds every fallback: a mistake fails a test, never a player
fn first_object(json: &str) -> Object {
    let mut scene = crate::schema::parse_scene(json).unwrap_or_else(|e| panic!("built-in kart model: {e:?}"));
    let mut o = scene.objects.remove(0);
    o.collide = false;
    o
}

/// Counters for diagnostics and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FleetStats {
    /// Karts placed this frame.
    pub karts: usize,
    /// Hazards drawn this frame.
    pub hazards: usize,
    /// Hazards the snapshot had that the pools had no object for.
    pub undrawn_hazards: usize,
}

/// The kart models, hazard pools and item boxes of one race.
#[derive(Debug)]
pub struct KartFleet {
    /// `scene.objects` index of each driver's model, by driver wire number.
    models: [usize; 8],
    shown: [bool; 8],
    acorns: ScenePool,
    planks: ScenePool,
    /// Object id of each item box (the zone's id), in `race.item_boxes` order.
    box_ids: Vec<String>,
    boxes_ready: u32,
    stats: FleetStats,
    laps: u8,
    /// `(driver, place, finished)` of every kart placed this frame: the HUD's standings.
    placed: Vec<(u8, u8, bool)>,
}

impl KartFleet {
    /// The fleet for `scene`, added to it hidden (call before the renderer is built); `None` when the scene has no race.
    pub fn build(scene: &mut Scene) -> Option<KartFleet> {
        let course = scene.race.clone()?;
        let mut models = [0usize; 8];
        for driver in Driver::ALL {
            let id = model_id(driver);
            let index = match scene.objects.iter().position(|o| o.id == id) {
                Some(i) => i,
                None => {
                    scene.objects.push(fallback_kart(driver));
                    scene.objects.len() - 1
                }
            };
            hide_object(scene, index);
            models[driver.wire() as usize] = index;
        }
        let acorns = ScenePool::add(scene, MAX_HAZARDS_PER_SNAPSHOT, |k| {
            hazard_object(&format!("acorn_{k}"), r##"{"id":"$ID","type":"sphere","radius":0.3,"material":{"color":"#c98a52"},"collide":false}"##)
        });
        let planks = ScenePool::add(scene, MAX_HAZARDS_PER_SNAPSHOT, |k| {
            hazard_object(&format!("plank_{k}"), r##"{"id":"$ID","type":"box","size":[1.6,0.12,0.6],"material":{"color":"#c79a6b"},"collide":false}"##)
        });
        let box_ids = course.item_boxes.iter().map(|(id, _, _)| id.clone()).collect();
        Some(KartFleet {
            models,
            shown: [false; 8],
            acorns,
            planks,
            box_ids,
            boxes_ready: u32::MAX,
            stats: FleetStats::default(),
            laps: course.laps,
            placed: Vec::new(),
        })
    }

    /// Which item boxes the newest snapshot says are ready (bit `i` = the race's `i`th box).
    pub fn set_boxes_ready(&mut self, mask: u32) {
        self.boxes_ready = mask;
    }

    /// Starts a frame: every model and hazard is hidden until it is placed again.
    pub fn begin_frame(&mut self, scene: &mut Scene) {
        for driver in 0..8 {
            if self.shown[driver] {
                hide_object(scene, self.models[driver]);
                self.shown[driver] = false;
            }
        }
        for slot in 0..self.acorns.capacity() {
            self.acorns.release(scene, slot);
            self.planks.release(scene, slot);
        }
        self.stats = FleetStats::default();
        self.placed.clear();
    }

    /// Laps in the race.
    pub fn laps(&self) -> u8 {
        self.laps
    }

    /// The karts placed this frame, best place first (`(driver, place, finished)`; karts with no place yet go last).
    pub fn standings(&self) -> Vec<(u8, u8, bool)> {
        let mut rows = self.placed.clone();
        rows.sort_by_key(|(driver, place, _)| (if *place == 0 { u8::MAX } else { *place }, *driver));
        rows
    }

    /// Draws the kart of driver `driver` at `pos` (the foot of the kart) facing `yaw` (the kart's heading). `false` for a driver number that does not exist.
    pub fn place_kart(&mut self, scene: &mut Scene, driver: u8, pos: Vec3, yaw: f32, kart: &KartSnap) -> bool {
        let Some(&index) = self.models.get(driver as usize) else { return false };
        let Some(o) = scene.objects.get_mut(index) else { return false };
        o.position = Track::constant(pos);
        // The model faces -Z; a heading of `yaw` (clockwise seen from above) is a rotation about +Y of `-yaw`.
        o.rotation = Track::constant(Vec3::new(0.0, -yaw.to_degrees(), 0.0));
        o.scale = Track::constant(Vec3::ONE);
        self.shown[driver as usize] = true;
        self.stats.karts += 1;
        self.placed.push((driver, kart.place, kart.finished));
        true
    }

    /// Draws the Acorns and planks. `t` is the clock in seconds (an Acorn tumbles as it flies).
    pub fn place_hazards(&mut self, scene: &mut Scene, hazards: &[HazardSnap], t: f32) {
        for h in hazards {
            let (pool, height) = if h.kind == 0 { (&mut self.acorns, 0.45) } else { (&mut self.planks, 0.07) };
            let Some(slot) = pool.claim() else {
                self.stats.undrawn_hazards += 1;
                continue;
            };
            let Some(o) = scene.objects.get_mut(pool.object_index(slot)) else { continue };
            o.position = Track::constant(Vec3::new(h.pos[0], height, h.pos[1]));
            o.rotation = Track::constant(Vec3::new(0.0, if h.kind == 0 { t * 360.0 } else { 15.0 * h.owner as f32 }, 0.0));
            o.scale = Track::constant(Vec3::ONE);
            self.stats.hazards += 1;
        }
    }

    /// The ids the renderer must not draw: models nobody drives, hazards not on the track, and item boxes that have been taken.
    pub fn hidden_ids<'a>(&'a self, scene: &'a Scene) -> Vec<&'a str> {
        let mut ids: Vec<&str> = Vec::new();
        for driver in 0..8 {
            if !self.shown[driver] {
                if let Some(o) = scene.objects.get(self.models[driver]) {
                    ids.push(o.id.as_str());
                }
            }
        }
        ids.extend(self.acorns.hidden_ids(scene));
        ids.extend(self.planks.hidden_ids(scene));
        ids.extend(self.box_ids.iter().enumerate().filter(|(i, _)| *i < 32 && self.boxes_ready & (1 << i) == 0).map(|(_, id)| id.as_str()));
        ids
    }

    /// The counters of the newest frame.
    pub fn stats(&self) -> FleetStats {
        self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENE: &str = r#"{"camera":{"position":[0,30,60],"target":[0,0,0]},
        "zones":[{"id":"line","rect":[-6,-41,6,-39]},{"id":"east","rect":[39,-6,41,6]},{"id":"south","rect":[-6,39,6,41]},{"id":"west","rect":[-41,-6,-39,6]},
                 {"id":"crate_a","rect":[0,0,4,4]},{"id":"crate_b","rect":[10,0,14,4]}],
        "race":{"gates":["line","east","south","west"],"item_boxes":["crate_a","crate_b"]},
        "objects":[{"id":"floor","type":"plane","size":[100,100]},
                   {"id":"crate_a","type":"box","size":[2,1,2],"position":[2,0.5,2],"collide":false},
                   {"id":"crate_b","type":"box","size":[2,1,2],"position":[12,0.5,2],"collide":false}]}"#;

    fn scene() -> Scene {
        crate::schema::parse_scene(SCENE).expect("scene")
    }

    fn snap(driver: u8) -> KartSnap {
        KartSnap { driver, ..Default::default() }
    }

    #[test]
    fn a_scene_without_a_race_gets_no_fleet_and_one_with_gets_every_driver_hidden() {
        let mut plain = crate::schema::parse_scene(r#"{"camera":{"position":[0,1,5],"target":[0,0,0]},"objects":[]}"#).unwrap();
        assert!(KartFleet::build(&mut plain).is_none());
        let mut s = scene();
        let before = s.objects.len();
        let fleet = KartFleet::build(&mut s).expect("a race scene has a fleet");
        assert_eq!(s.objects.len(), before + 8 + 2 * MAX_HAZARDS_PER_SNAPSHOT, "eight fallback karts and two pools");
        for d in Driver::ALL {
            let o = s.objects.iter().find(|o| o.id == model_id(d)).expect("a model for every driver");
            assert!(!o.collide);
        }
        let hidden = fleet.hidden_ids(&s);
        assert!(Driver::ALL.iter().all(|d| hidden.contains(&model_id(*d).as_str())), "no kart is drawn until it is placed");
        assert_eq!(hidden.iter().filter(|id| id.starts_with("acorn_") || id.starts_with("plank_")).count(), 2 * MAX_HAZARDS_PER_SNAPSHOT);
    }

    #[test]
    fn a_maps_own_kart_object_is_used_instead_of_the_fallback() {
        let text = SCENE.replace(r#""objects":["#, r#""objects":[{"id":"kart_duck","type":"box","size":[1,1,2],"position":[0,-100,0],"collide":false},"#);
        let mut s = crate::schema::parse_scene(&text).unwrap();
        let before = s.objects.len();
        let mut fleet = KartFleet::build(&mut s).unwrap();
        assert_eq!(s.objects.len(), before + 7 + 2 * MAX_HAZARDS_PER_SNAPSHOT, "the Duck brought its own model, so only seven fallbacks were added");
        fleet.begin_frame(&mut s);
        assert!(fleet.place_kart(&mut s, Driver::Duck.wire(), Vec3::new(5.0, 0.0, -3.0), 0.0, &snap(0)));
        let duck = s.objects.iter().find(|o| o.id == "kart_duck").unwrap();
        assert_eq!(duck.position.sample(0.0), Vec3::new(5.0, 0.0, -3.0));
        assert!(matches!(duck.kind, crate::schema::ObjectKind::Prim(..)), "and it is the map's box, not a fallback group");
    }

    #[test]
    fn a_kart_is_placed_facing_its_heading_and_only_shown_drivers_are_drawn() {
        let mut s = scene();
        let mut fleet = KartFleet::build(&mut s).unwrap();
        fleet.begin_frame(&mut s);
        assert!(fleet.place_kart(&mut s, Driver::Wolf.wire(), Vec3::new(1.0, 0.5, 2.0), std::f32::consts::FRAC_PI_2, &snap(6)));
        assert!(!fleet.place_kart(&mut s, 9, Vec3::ZERO, 0.0, &snap(9)), "no such driver");
        let wolf = s.objects.iter().find(|o| o.id == "kart_wolf").unwrap();
        assert_eq!(wolf.position.sample(0.0), Vec3::new(1.0, 0.5, 2.0));
        assert!((wolf.rotation.sample(0.0).y + 90.0).abs() < 1e-3, "heading east (+90 degrees clockwise) is -90 about +Y: {:?}", wolf.rotation.sample(0.0));
        assert_eq!(wolf.scale.sample(0.0), Vec3::ONE);
        let hidden = fleet.hidden_ids(&s);
        assert!(!hidden.contains(&"kart_wolf") && hidden.contains(&"kart_duck"));
        assert_eq!(fleet.stats().karts, 1);
        // The next frame starts empty again.
        fleet.begin_frame(&mut s);
        assert!(fleet.hidden_ids(&s).contains(&"kart_wolf"));
        let wolf = s.objects.iter().find(|o| o.id == "kart_wolf").unwrap();
        assert!(wolf.scale.sample(0.0).x < 0.01, "and the model is scaled to nothing for anything that ignores the hidden list");
    }

    #[test]
    fn acorns_and_planks_are_drawn_from_their_pools_and_an_overflow_is_counted() {
        let mut s = scene();
        let mut fleet = KartFleet::build(&mut s).unwrap();
        fleet.begin_frame(&mut s);
        let hazards = [HazardSnap { kind: 0, owner: 1, pos: [3.0, -4.0] }, HazardSnap { kind: 1, owner: 2, pos: [7.0, 8.0] }];
        fleet.place_hazards(&mut s, &hazards, 0.25);
        assert_eq!(fleet.stats().hazards, 2);
        let acorn = s.objects.iter().find(|o| o.id == "acorn_0").unwrap();
        assert_eq!((acorn.position.sample(0.0).x, acorn.position.sample(0.0).z), (3.0, -4.0));
        assert!((acorn.rotation.sample(0.0).y - 90.0).abs() < 1e-3, "an acorn tumbles with the clock");
        let plank = s.objects.iter().find(|o| o.id == "plank_0").unwrap();
        assert_eq!((plank.position.sample(0.0).x, plank.position.sample(0.0).z), (7.0, 8.0));
        let hidden = fleet.hidden_ids(&s);
        assert!(!hidden.contains(&"acorn_0") && hidden.contains(&"acorn_1") && !hidden.contains(&"plank_0"));
        // More acorns than the pool holds: the extras are counted, not silently dropped.
        fleet.begin_frame(&mut s);
        let many: Vec<HazardSnap> = (0..MAX_HAZARDS_PER_SNAPSHOT + 3).map(|i| HazardSnap { kind: 0, owner: 0, pos: [i as f32, 0.0] }).collect();
        fleet.place_hazards(&mut s, &many, 0.0);
        assert_eq!((fleet.stats().hazards, fleet.stats().undrawn_hazards), (MAX_HAZARDS_PER_SNAPSHOT, 3));
    }

    #[test]
    fn a_taken_item_box_is_hidden_until_it_is_ready_again() {
        let mut s = scene();
        let mut fleet = KartFleet::build(&mut s).unwrap();
        fleet.begin_frame(&mut s);
        assert!(!fleet.hidden_ids(&s).contains(&"crate_a"), "before any snapshot every box is shown");
        fleet.set_boxes_ready(0b10);
        let hidden = fleet.hidden_ids(&s);
        assert!(hidden.contains(&"crate_a") && !hidden.contains(&"crate_b"), "box 0 was taken, box 1 is ready: {hidden:?}");
        fleet.set_boxes_ready(0b11);
        assert!(!fleet.hidden_ids(&s).contains(&"crate_a"));
    }
}
