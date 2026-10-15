//! Workshop through the real authoritative match (`MatchSim` with Workshop as its game, as on a server, in a local session and in `sim`):
//! isolated machine state, conserved resources, commands that cannot duplicate or lose anything, content errors with their places, and
//! saves that reload exactly or migrate from version 1.

use red_engine2::sim::defs::{code, Id};
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::parse_spawns;
use red_engine2::sim::systems::Command;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use workshop_server::game::WorkshopGame;
use workshop_server::machines::Machines;

fn content() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../content")
}

fn id(s: &str) -> Id {
    Id::parse(s).unwrap()
}

fn sim_of(text: &str) -> Result<MatchSim, String> {
    workshop_server::register();
    let scene = red_engine2::scene_blocks::parse_scene_in(text, Some(&content())).map_err(|e| e.join("\n"))?;
    MatchSim::try_new(&scene, parse_spawns(text)?)
}

fn sim(file: &str) -> MatchSim {
    sim_of(&std::fs::read_to_string(content().join(file)).unwrap()).unwrap()
}

fn game(sim: &MatchSim) -> &WorkshopGame {
    sim.game::<WorkshopGame>().expect("the scene plays Workshop")
}

fn game_mut(sim: &mut MatchSim) -> &mut WorkshopGame {
    sim.game_mut::<WorkshopGame>().expect("the scene plays Workshop")
}

fn run(sim: &mut MatchSim, ticks: u32) {
    for _ in 0..ticks {
        sim.tick_once();
        let g = game(sim);
        assert!(g.systems().halted().is_none(), "{:?}", g.systems().halted());
        g.systems().ledger().audit().unwrap_or_else(|e| panic!("tick {}: {e}", sim.tick()));
    }
}

/// Every unit of every resource, all stores together.
fn totals(g: &WorkshopGame) -> std::collections::BTreeMap<Id, u64> {
    let mut t = std::collections::BTreeMap::new();
    for (_, s) in g.systems().ledger().stores() {
        for (r, n) in s.items() {
            *t.entry(r).or_insert(0) += n as u64;
        }
    }
    t
}

#[test]
fn one_machine_behavior_two_kinds_with_their_own_parameters_and_state() {
    let mut s = sim("workshop.json");
    run(&mut s, 200);
    let m = game(&s).systems().get::<Machines>().unwrap();
    let (furnace, press) = (m.unit(&id("ws:furnace_a")).unwrap().clone(), m.unit(&id("ws:press_a")).unwrap().clone());
    assert_eq!((furnace.kind.as_str(), press.kind.as_str()), ("ws:furnace", "ws:press"));
    assert!(furnace.made > 0 && press.made > 0, "{furnace:?} {press:?}");
    // Each owns its stores; a command to one leaves the other's timer, counters and stores exactly as they were.
    let ledger = game(&s).systems().ledger();
    assert_eq!(ledger.store(&id("ws:furnace_a/in")).unwrap().owner(), id("ws:furnace_a"));
    assert_eq!(ledger.store(&id("ws:storage")).unwrap().owner(), id("workshop:floor"), "shared storage has an explicit owner");
    let press_store = ledger.store(&id("ws:press_a/in")).map(|s| s.to_store());
    game_mut(&mut s).command(&Command { name: id("ws:stop"), subject: Some(id("ws:furnace_a")), value: 0 }).unwrap();
    let m = game(&s).systems().get::<Machines>().unwrap();
    assert_eq!(m.unit(&id("ws:press_a")), Some(&press));
    assert_eq!(game(&s).systems().ledger().store(&id("ws:press_a/in")).map(|s| s.to_store()), press_store);
    assert_eq!(m.unit(&id("ws:furnace_a")).unwrap().left, None);
}

#[test]
fn resources_are_conserved_through_production_stops_and_reroutes() {
    let mut s = sim("workshop.json");
    let mine = id("ws:mine");
    let start: u64 = 6; // ws:storage starts with 6 ore
    let mut pick = 0u64;
    for step in 0..60 {
        run(&mut s, 40);
        // Stop a machine mid-cycle, flip the route, stop again: nothing appears or disappears.
        pick = pick.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let target = if pick >> 62 == 0 { "ws:furnace_a" } else { "ws:press_a" };
        let g = game_mut(&mut s);
        g.command(&Command { name: id("ws:stop"), subject: Some(id(target)), value: 0 }).unwrap();
        g.command(&Command { name: id("ws:route"), subject: Some(id("ws:press_a")), value: (step % 2) as i64 }).unwrap();
        let g = game(&s);
        let delivered = g.systems().get::<workshop_server::flow::Supply>().unwrap().delivered[&mine].0;
        let shipped = g.shipped(&id("ws:ship")) + g.shipped(&id("ws:scrap"));
        let made: Vec<u64> = ["ws:furnace_a", "ws:press_a"].iter().map(|i| g.systems().get::<Machines>().unwrap().unit(&id(i)).unwrap().made).collect();
        let t = totals(g);
        // ore: start + mined - 2 per smelt; ingots: smelted - pressed; gears: 2 per press - shipped - scrapped.
        assert_eq!(t.get(&id("ws:ore")).copied().unwrap_or(0), start + delivered - 2 * made[0]);
        assert_eq!(t.get(&id("ws:ingot")).copied().unwrap_or(0), made[0] - made[1]);
        assert_eq!(t.get(&id("ws:gear")).copied().unwrap_or(0), 2 * made[1] - shipped);
    }
}

#[test]
fn repeated_and_impossible_commands_change_nothing() {
    let mut s = sim("workshop.json");
    run(&mut s, 300);
    let route = |v: i64| Command { name: id("ws:route"), subject: Some(id("ws:press_a")), value: v };
    game_mut(&mut s).command(&route(1)).unwrap();
    let once = game(&s).systems().hash();
    for _ in 0..5 {
        game_mut(&mut s).command(&route(1)).unwrap();
    }
    assert_eq!(game(&s).systems().hash(), once, "setting the same route again is not a second change");
    let e = game_mut(&mut s).command(&route(7)).unwrap_err();
    assert_eq!(e.code, "out-of-range");
    let e = game_mut(&mut s).command(&Command { name: id("ws:route"), subject: Some(id("ws:nothing")), value: 0 }).unwrap_err();
    assert_eq!(e.code, code::UNKNOWN);
    let e = game_mut(&mut s).command(&Command { name: id("ws:teleport"), subject: None, value: 0 }).unwrap_err();
    assert_eq!(e.code, code::EVENT, "a command no system declares");
    assert_eq!(game(&s).systems().hash(), once, "refused commands change nothing");
}

#[test]
fn dismantling_hands_everything_back_or_is_refused_whole() {
    let dismantle = Command { name: id("ws:dismantle"), subject: Some(id("ws:furnace_a")), value: 0 };
    // With nothing shipped, storage fills up: at a moment it is full, the furnace's contents have nowhere to go, so it stays, untouched.
    let mut full = sim("workshop.json");
    for _ in 0..6000 {
        if game(&full).systems().ledger().free(&id("ws:storage")) == 0 && game(&full).systems().ledger().store(&id("ws:furnace_a/in")).unwrap().used() > 0 {
            break;
        }
        run(&mut full, 1);
    }
    assert_eq!(game(&full).systems().ledger().free(&id("ws:storage")), 0);
    let hash = game(&full).systems().hash();
    let e = game_mut(&mut full).command(&dismantle).unwrap_err();
    assert_eq!(e.code, code::FULL, "{e}");
    assert_eq!(game(&full).systems().hash(), hash);
    // Storage has room for what the input holds but not for the output's too: still refused whole, the input's ore not moved.
    let mut part = sim("workshop.json");
    let used = |s: &MatchSim, st: &str| game(s).systems().ledger().store(&id(st)).map_or(0, |x| x.used());
    for _ in 0..6000 {
        let free = game(&part).systems().ledger().free(&id("ws:storage"));
        let (inp, out) = (used(&part, "ws:furnace_a/in"), used(&part, "ws:furnace_a/out"));
        if inp > 0 && out > 0 && inp <= free && free < inp + out {
            break;
        }
        run(&mut part, 1);
    }
    let free = game(&part).systems().ledger().free(&id("ws:storage"));
    let (inp, out) = (used(&part, "ws:furnace_a/in"), used(&part, "ws:furnace_a/out"));
    assert!(inp > 0 && inp <= free && free < inp + out, "storage free {free}, input {inp}, output {out}");
    let hash = game(&part).systems().hash();
    let e = game_mut(&mut part).command(&dismantle).unwrap_err();
    assert_eq!(e.code, code::FULL, "{e}");
    assert_eq!(game(&part).systems().hash(), hash, "a refused dismantle changes nothing");
    assert_eq!((used(&part, "ws:furnace_a/in"), used(&part, "ws:furnace_a/out")), (inp, out));
    run(&mut part, 120);
    // Early on there is room: the furnace comes apart and everything it held lands in storage.
    let mut s = sim("workshop.json");
    run(&mut s, 100);
    let held = |s: &MatchSim| {
        let l = game(s).systems().ledger();
        ["ws:furnace_a/in", "ws:furnace_a/out"].iter().map(|st| l.store(&id(st)).map_or(0, |x| x.used())).sum::<u32>()
    };
    let (in_furnace, before) = (held(&s), totals(game(&s)));
    assert!(in_furnace > 0, "the furnace holds ore mid-cycle");
    let storage_before = game(&s).systems().ledger().store(&id("ws:storage")).unwrap().used();
    game_mut(&mut s).command(&dismantle).unwrap();
    let l = game(&s).systems().ledger();
    assert!(l.store(&id("ws:furnace_a/in")).is_none() && l.store(&id("ws:furnace_a/out")).is_none());
    assert_eq!(l.store(&id("ws:storage")).unwrap().used(), storage_before + in_furnace);
    assert_eq!(totals(game(&s)), before, "nothing appeared or disappeared");
    l.audit().unwrap();
    let e = game_mut(&mut s).command(&dismantle).unwrap_err();
    assert_eq!(e.code, code::UNKNOWN, "dismantling it twice: there is no second furnace to give back");
    run(&mut s, 120);
}

#[test]
fn content_problems_name_the_file_and_pointer_with_a_fix() {
    let dir = std::env::temp_dir().join(format!("workshop-content-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("defs")).unwrap();
    std::fs::copy(content().join("defs/base.json"), dir.join("defs/base.json")).unwrap();
    let scene = |defs: &str, extra: Value| {
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(content().join("workshop.json")).unwrap()).unwrap();
        v["workshop"]["defs"] = serde_json::from_str(defs).unwrap();
        v.as_object_mut().unwrap().remove("checks");
        if let Value::Object(o) = extra {
            for (k, x) in o {
                v["workshop"][k] = x;
            }
        }
        workshop_server::register();
        red_engine2::scene_blocks::parse_scene_in(&v.to_string(), Some(&dir)).err().unwrap_or_default().join("\n")
    };
    let write = |name: &str, v: Value| std::fs::write(dir.join("defs").join(name), v.to_string()).unwrap();
    // A second file defining a machine that is already defined: refused, naming both places.
    write("dup.json", json!({"namespace": "ws", "machines": [{"id": "furnace", "recipes": ["smelt"], "speed": 2, "input": 6, "output": 4}]}));
    let e = scene(r#"["defs/base.json", "defs/dup.json"]"#, json!({}));
    assert!(e.contains("defs/dup.json#/machines/0: [duplicate-id] `ws:furnace` is already defined at defs/base.json#/machines/0"), "{e}");
    // The same with `"replace": true`: accepted, and the later file's furnace is the one that runs.
    write(
        "faster.json",
        json!({"namespace": "ws", "machines": [{"id": "furnace", "replace": true, "recipes": ["smelt"], "speed": 2, "input": 6, "output": 4}]}),
    );
    assert_eq!(scene(r#"["defs/base.json", "defs/faster.json"]"#, json!({})), "");
    // Replacing before the definition exists: the dependency is named.
    let e = scene(r#"["defs/faster.json", "defs/base.json"]"#, json!({}));
    assert!(e.contains("[undeclared-dependency] replaces `ws:furnace`, which no earlier file defines"), "{e}");
    // A misspelt reference across namespaces, an unknown key, a scene variable that does not exist.
    write("typo.json", json!({"namespace": "wx", "machines": [{"id": "m", "recipes": ["ws:smelts"], "speed": 1, "input": 6, "output": 4}], "instancse": []}));
    let e = scene(r#"["defs/base.json", "defs/typo.json"]"#, json!({"show": {"gear_shipped": "ws:ship"}}));
    assert!(e.contains("defs/typo.json#/machines/0/recipes/0: [unknown-name] no recipe `ws:smelts` (fix: did you mean `ws:smelt`?)"), "{e}");
    assert!(e.contains("defs/typo.json#/instancse: [unknown-field]") && e.contains("did you mean `instances`?"), "{e}");
    assert!(
        e.contains("scene#/workshop/show/gear_shipped: [unknown-name] no scene variable `gear_shipped`") && e.contains("did you mean `gears_shipped`?"),
        "{e}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_same_seed_plays_the_same_match_and_another_seed_does_not() {
    let text = std::fs::read_to_string(content().join("workshop.json")).unwrap();
    let hash_after = |seed: u64| {
        let mut v: Value = serde_json::from_str(&text).unwrap();
        v["workshop"]["seed"] = json!(seed);
        let mut s = sim_of(&v.to_string()).unwrap();
        run(&mut s, 600);
        (game(&s).systems().hash(), s.checksum())
    };
    assert_eq!(hash_after(7), hash_after(7));
    assert_ne!(hash_after(7).0, hash_after(8).0, "the mine's amounts come from the seeded stream");
}

#[test]
fn a_save_mid_production_reloads_into_the_same_future() {
    let mut a = sim("workshop.json");
    a.set_var("route", 1.0); // the lever: the switch's variable routes the press to the dock
    run(&mut a, 333); // mid-cycle: the furnace and the press each have a cycle under way
    let m = game(&a).systems().get::<Machines>().unwrap();
    assert!(m.units().any(|(_, u)| u.left.is_some()), "a cycle is under way at the save");
    let saved = game(&a).save();
    // A fresh match (with its own new state), loaded from the save, plays on exactly as the first one does: timers, stores, counters and the
    // random stream included.
    let mut b = sim("workshop.json");
    run(&mut b, 17);
    let report = game_mut(&mut b).load(&saved).unwrap();
    assert!(report.migrated.is_empty() && report.fresh.is_empty(), "{report:?}");
    assert_eq!(game(&b).systems().hash(), game(&a).systems().hash());
    // The scene's own variables are the scene's: a client restores them beside the game's save (`persist`); here, the route.
    b.set_var("route", 1.0);
    run(&mut a, 500);
    run(&mut b, 500);
    assert_eq!(game(&b).systems().hash(), game(&a).systems().hash());
    assert_eq!(game(&b).save(), game(&a).save());
}

#[test]
fn a_version_1_save_migrates_keeping_progress_and_a_bad_one_changes_nothing() {
    let v1_path = content().join("saves/v1.json");
    let v1_text = std::fs::read_to_string(&v1_path).unwrap();
    let v1: Value = serde_json::from_str(&v1_text).unwrap();
    assert_eq!(v1["systems"]["systems"]["ws:machines"]["version"], 1, "the fixture is a version 1 save");
    let mut s = sim("workshop.json");
    let report = game_mut(&mut s).load(&v1).unwrap();
    assert_eq!(report.migrated, vec![(id("ws:machines"), 1, 2)]);
    // Progress kept: the stores and counters as saved; a cycle 75% done has a quarter of its ticks left.
    let g = game(&s);
    assert_eq!(g.systems().tick_count(), v1["systems"]["tick"].as_u64().unwrap());
    assert_eq!(g.shipped(&id("ws:ship")), v1["systems"]["systems"]["ws:shipping"]["state"]["ws:ship"].as_u64().unwrap());
    let furnace = g.systems().get::<Machines>().unwrap().unit(&id("ws:furnace_a")).unwrap().clone();
    assert_eq!(furnace.left, Some(30), "smelt is 120 ticks at speed 1; 0.75 done leaves 30");
    assert_eq!(furnace.made, 0, "version 1 did not count cycles");
    run(&mut s, 200);
    // Newer than this program, or not a Workshop save: refused clearly, nothing changed.
    let before = game(&s).systems().hash();
    let mut newer = v1.clone();
    newer["systems"]["systems"]["ws:machines"]["version"] = json!(3);
    let e = game_mut(&mut s).load(&newer).unwrap_err();
    assert_eq!(e.code, code::STATE_VERSION);
    assert!(e.message.contains("saved by a newer version (3); this program reads up to 2"), "{e}");
    let mut broken = v1.clone();
    broken["systems"]["systems"]["ws:machines"]["state"]["ws:furnace_a"]["progress"] = json!(4.0);
    let e = game_mut(&mut s).load(&broken).unwrap_err();
    assert!(e.message.contains("cannot migrate version 1 to 2") && e.message.contains("not between 0 and 1"), "{e}");
    let e = game_mut(&mut s).load(&json!({"game": "frameless"})).unwrap_err();
    assert!(e.message.contains("not a Workshop save"), "{e}");
    assert_eq!(game(&s).systems().hash(), before);
    // The file itself is only read: loading (even failing to) never touches it; the next save is written atomically beside a backup.
    assert_eq!(std::fs::read_to_string(&v1_path).unwrap(), v1_text);
    let dir = std::env::temp_dir().join(format!("workshop-save-{}", std::process::id()));
    let path = dir.join("workshop.json");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&path, &v1_text).unwrap();
    red_engine2::sim::systems::write_atomic(&path, &game(&s).save().to_string()).unwrap();
    assert_eq!(std::fs::read_to_string(path.with_extension("bak")).unwrap(), v1_text, "the version 1 file is kept as the backup");
    let reread: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(reread["systems"]["systems"]["ws:machines"]["version"], 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Writes a version-2 save mid-production to stdout (how `content/saves/v1.json` was made: this, rewritten by hand into version 1's shape).
#[test]
#[ignore]
fn print_a_save() {
    let mut s = sim("workshop.json");
    run(&mut s, 333);
    println!("{}", serde_json::to_string_pretty(&game(&s).save()).unwrap());
}
