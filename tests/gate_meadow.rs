//! The combination game: `examples/2d/gate-meadow.game2d.json`.
//!
//! Its ground is the engine's own generated world (seed 7, the same generator Marcel and the ground-consistency tests use), pressed into a tile map: a tree blocks the player, a rise
//! is raised ground drawn in 3D. A fence with a gate cuts the meadow in two; a key opens the gate by a rule; progress is saved. These tests keep the file honest against its
//! generator, then run every feature against every other one: the terrain survives the gate opening, the analysis (`reach`) and the play-through agree about the gate, and a
//! saved game reloads into a world that is already open.

use red2d::game::GameDef;
use red2d::script::run_scenario;
use red2d::sim::Sim;
use red_engine2::procgen::world::{ChunkId, Config, World};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/2d/gate-meadow.game2d.json");
/// The window of the generated world the game uses: origin (metres), size (tiles), tile (metres).
const ORIGIN: (f64, f64) = (200.0, 200.0);
const COLS: usize = 30;
const ROWS: usize = 12;
const TILE_M: f64 = 2.0;
/// Rectangles `(col0, row0, col1, row1)` kept clear of trees so the game is winnable: the way along the middle rows, the start and the lantern. Everything else is the generator's.
const KEEP_CLEAR: &[(usize, usize, usize, usize)] = &[(0, 5, 29, 6), (1, 4, 4, 7), (24, 3, 28, 8)];
const FENCE_COL: usize = 17;
const GATE_ROWS: (usize, usize) = (5, 6);

/// The map rows the generator makes: `T` a tree (a trunk in the tile), `h` raised ground (more than half a metre up), `F` the fence, `.` open meadow.
fn generated_rows() -> Vec<String> {
    let world = World::new(Config { seed: 7, ..Config::default() });
    let mut trees = HashSet::new();
    let (a, b) = (ChunkId::at(ORIGIN.0, ORIGIN.1), ChunkId::at(ORIGIN.0 + COLS as f64 * TILE_M, ORIGIN.1 + ROWS as f64 * TILE_M));
    for cz in a.z.min(b.z)..=a.z.max(b.z) {
        for cx in a.x.min(b.x)..=a.x.max(b.x) {
            for t in world.blockers(ChunkId { x: cx, z: cz }) {
                let (c, r) = (((t.x - ORIGIN.0) / TILE_M).floor(), ((t.z - ORIGIN.1) / TILE_M).floor());
                if c >= 0.0 && r >= 0.0 && (c as usize) < COLS && (r as usize) < ROWS {
                    trees.insert((c as usize, r as usize));
                }
            }
        }
    }
    (0..ROWS)
        .map(|r| {
            (0..COLS)
                .map(|c| {
                    let kept_clear = KEEP_CLEAR.iter().any(|&(c0, r0, c1, r1)| (c0..=c1).contains(&c) && (r0..=r1).contains(&r));
                    if c == FENCE_COL && !(GATE_ROWS.0..=GATE_ROWS.1).contains(&r) {
                        'F'
                    } else if c == FENCE_COL {
                        '.' // the gate stands here (a scene thing, so a rule can remove it)
                    } else if trees.contains(&(c, r)) && !kept_clear {
                        'T'
                    } else if world.height(ORIGIN.0 + (c as f64 + 0.5) * TILE_M, ORIGIN.1 + (r as f64 + 0.5) * TILE_M) > 0.5 && !kept_clear {
                        'h'
                    } else {
                        '.'
                    }
                })
                .collect()
        })
        .collect()
}

fn game_json() -> Value {
    serde_json::from_str(&std::fs::read_to_string(PATH).unwrap_or_else(|e| panic!("{PATH}: {e}"))).unwrap()
}

fn def() -> Arc<GameDef> {
    let text = std::fs::read_to_string(PATH).unwrap();
    Arc::new(red2d::game::parse(&text).unwrap_or_else(|e| panic!("{e:?}")))
}

fn scenario<'a>(def: &'a GameDef, name: &str) -> &'a red2d::game::Scenario {
    def.scenarios.iter().find(|s| s.name.contains(name)).unwrap_or_else(|| panic!("no scenario containing `{name}`"))
}

fn tree_positions(sim: &Sim) -> Vec<(i32, i32)> {
    let mut v: Vec<(i32, i32)> = sim
        .entities
        .iter()
        .filter(|e| e.alive && sim.def.prefabs[e.prefab].tags.iter().any(|t| t == "tree"))
        .map(|e| (e.x.round() as i32, e.y.round() as i32))
        .collect();
    v.sort();
    v
}

#[test]
fn the_ground_in_the_file_is_what_the_generator_makes() {
    let rows = generated_rows();
    let in_file: Vec<String> = game_json()["map"]["rows"].as_array().unwrap().iter().map(|r| r.as_str().unwrap().to_string()).collect();
    assert_eq!(
        in_file,
        rows,
        "the map in the game file is the engine's generated world (seed 7, window {ORIGIN:?}) plus the fence. Regenerate: RED_PRINT_ROWS=1 cargo test --test gate_meadow print_the_generated_rows -- --nocapture"
    );
    let trees = rows.iter().map(|r| r.matches('T').count()).sum::<usize>();
    assert!(trees >= 10, "a meadow with trees on it: {trees}");
    assert!(rows.iter().any(|r| r.contains('h')), "and raised ground");
    // The same generator the 3D scenes use says the window has hills: heights span more than a metre.
    let world = World::new(Config { seed: 7, ..Config::default() });
    let hs: Vec<f32> = (0..ROWS)
        .flat_map(|r| (0..COLS).map(move |c| (c, r)))
        .map(|(c, r)| world.height(ORIGIN.0 + (c as f64 + 0.5) * TILE_M, ORIGIN.1 + (r as f64 + 0.5) * TILE_M))
        .collect();
    let (lo, hi) = hs.iter().fold((f32::MAX, f32::MIN), |a, &h| (a.0.min(h), a.1.max(h)));
    assert!(hi - lo > 1.0, "generated heights {lo}..{hi}");
}

#[test]
fn opening_the_gate_changes_the_gate_and_nothing_else_and_analysis_agrees_with_play() {
    let def = def();
    let before = Sim::new(def.clone(), 1);
    // What the analysis says the gate keeps out: the world with the gate assumed gone.
    let mut assumed = before.clone();
    assert_eq!(assumed.remove_for_analysis("gate"), 1);
    // What play says after the rule really fired.
    let (report, played) = run_scenario(&def, scenario(&def, "the key opens the gate"), None);
    assert!(report.ok, "{:?}", report.failures);
    // Exactly the gate is gone (and the key that opened it): same trees, same fence, same rises, same lantern.
    let names = |s: &Sim| {
        let mut v: Vec<String> = s
            .entities
            .iter()
            .filter(|e| e.alive)
            .map(|e| format!("{}@{:.0},{:.0}", def.prefabs[e.prefab].name, e.x, e.y))
            .filter(|n| !n.starts_with("player"))
            .collect();
        v.sort();
        v
    };
    let (b, a) = (names(&before), names(&played));
    let gone: Vec<&String> = b.iter().filter(|n| !a.contains(n)).collect();
    assert_eq!(gone.len(), 2, "the gate and the key: {gone:?}");
    assert!(gone.iter().any(|n| n.starts_with("gate@")) && gone.iter().any(|n| n.starts_with("key@")), "{gone:?}");
    assert_eq!(a.iter().filter(|n| !b.contains(n)).count(), 0, "nothing appeared");
    assert_eq!(tree_positions(&before), tree_positions(&played), "every generated tree is still where it was");
    // The analysis and the play-through agree about everything a player could want to reach.
    for target in ["tag:lantern", "tag:key", "tag:fence", "tag:tree"] {
        let live = played.can_reach("p", target);
        let assumed_open = assumed.can_reach("p", target);
        // The key is gone in play (and so is the thing to reach); every other target must get the same answer from both.
        if target == "tag:key" {
            assert!(live.is_err(), "no key left to reach in play: {live:?}");
            continue;
        }
        assert_eq!(live, assumed_open, "{target}: play says {live:?}, the analysis with the gate removed says {assumed_open:?}");
    }
    assert_eq!(before.can_reach("p", "tag:lantern"), Ok(false));
    assert_eq!(played.can_reach("p", "tag:lantern"), Ok(true));
    // The shut gate is solid for the analysis exactly as in play: it is what the walker is stopped by.
    assert_eq!(before.can_reach("p", "tag:gate"), Ok(true), "the walker can walk up to the gate");
}

#[test]
fn a_saved_game_reloads_into_an_open_gate_and_can_be_finished() {
    let def = def();
    // Play to the moment the gate opens, and take the save the page would have written.
    let (report, played) = run_scenario(&def, scenario(&def, "the key opens the gate"), None);
    assert!(report.ok);
    let save = played.save_json();
    assert!(save.contains("gate_open"), "{save}");
    // A fresh page: a new world (gate shut, key lying there), then the save is read.
    let mut page = Sim::new(def.clone(), 1);
    assert_eq!(page.count_tag("gate"), 1);
    assert_eq!(page.count_tag("key"), 1);
    assert_eq!(page.can_reach("p", "tag:lantern"), Ok(false));
    let status = page.load_save(&save);
    assert!(format!("{status}").contains("restored"), "{status}");
    // Before a single tick: the restored world is already open, with the generated meadow untouched.
    assert_eq!(page.var("gate_open"), Some(1.0));
    assert_eq!(page.count_tag("gate"), 0, "the gate is open again straight after the load");
    assert_eq!(page.count_tag("key"), 0, "and the key that opened it is not lying there");
    assert_eq!(tree_positions(&page), tree_positions(&played), "the same meadow");
    assert_eq!(page.can_reach("p", "tag:lantern"), Ok(true));
    // And the game can be finished from there: walk to the lantern.
    page.set_action("right", true);
    for _ in 0..60 * 8 {
        page.step();
        if page.ended.is_some() {
            break;
        }
    }
    assert_eq!(page.ended, Some(red2d::game::Outcome::Win), "walking right through the open gate reaches the lantern");
    assert_eq!(page.var("wins"), Some(1.0));
    assert_eq!(page.var("gate_open"), Some(0.0), "and the next game starts with the gate shut");
}

#[test]
fn a_save_with_nothing_in_it_changes_nothing_and_fires_no_rebuild() {
    let def = def();
    let page = Sim::new(def.clone(), 1);
    let fresh = page.save_json();
    let mut other = Sim::new(def.clone(), 1);
    other.load_save(&fresh);
    assert_eq!(other.count_tag("gate"), 1, "gate_open is 0 in the save: the gate stays");
    assert_eq!(page.count_tag("gate"), 1);
}
