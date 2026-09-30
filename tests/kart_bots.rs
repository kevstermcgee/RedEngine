//! Kart bots in the authoritative simulation: eight of them race a diamond track with no human anywhere. Proves the bots can finish, differ by
//! skill and animal, use their pickups, stay deterministic, and gives the balance table of lap time per driver (ADR
//! 2026-09-29-great-outdoors-karts-as-a-first-class-engine-feature). Everything is headless: the bots are ordinary players fed by `run_bots`.

use red_engine2::sim::ai::BotsConfig;
use red_engine2::sim::kart::Driver;
use red_engine2::sim::match_sim::{MatchSim, MAX_PLAYERS};
use red_engine2::sim::race::Phase;
use red_engine2::sim::spawns::parse_spawns;

/// A diamond of four gates (radius 40), a grid of eight behind the line, optionally item boxes on the gate points.
fn scene(laps: u32, boxes: bool) -> String {
    let spawns: Vec<String> =
        (0..8).map(|i| format!(r#"{{"id":"g{i}","position":[{},0,{}],"yaw_deg":90}}"#, -20.0 - 2.5 * (i / 2) as f32, -41.5 + 3.0 * (i % 2) as f32)).collect();
    let (zones_extra, race_extra) = if boxes {
        (
            r#",{"id":"b1","rect":[-3,-43,3,-37]},{"id":"b2","rect":[37,-3,43,3]},{"id":"b3","rect":[-3,37,3,43]},{"id":"b4","rect":[-43,-3,-37,3]}"#,
            r#","item_boxes":["b1","b2","b3","b4"],"item_respawn_secs":3"#,
        )
    } else {
        ("", "")
    };
    format!(
        r#"{{"camera":{{"position":[0,30,60],"target":[0,0,0]}},
            "zones":[{{"id":"line","rect":[-6,-41,6,-39]}},{{"id":"east","rect":[39,-6,41,6]}},{{"id":"south","rect":[-6,39,6,41]}},{{"id":"west","rect":[-41,-6,-39,6]}}{zones_extra}],
            "race":{{"laps":{laps},"gates":["line","east","south","west"],"countdown_secs":1,"finish_grace_secs":120{race_extra}}},
            "spawns":[{}],
            "objects":[{{"id":"floor","type":"plane","size":[300,300],"position":[0,0.01,0]}}]}}"#,
        spawns.join(",")
    )
}

/// A match with `n` bots in slots `0..n`, all at `level` (the driver of each is its slot's animal).
fn bots(text: &str, n: usize, level: f32) -> MatchSim {
    let parsed = red_engine2::schema::parse_scene(text).expect("scene");
    let mut sim = MatchSim::new(&parsed, parse_spawns(text).unwrap());
    let cfg = BotsConfig::default();
    for slot in 0..n {
        let mut spec = cfg.spec(slot);
        spec.level = level;
        assert!(sim.add_bot_in_slot(slot, &spec), "bot {slot}");
    }
    sim
}

/// Runs until the race is over or `max_ticks`; returns the ticks run.
fn run(sim: &mut MatchSim, max_ticks: u32) -> u32 {
    for t in 1..=max_ticks {
        sim.tick_once();
        if sim.race().unwrap().phase() == Phase::Finished {
            return t;
        }
    }
    max_ticks
}

#[test]
fn eight_bots_race_to_the_finish_each_as_a_different_animal() {
    let text = scene(2, false);
    let mut sim = bots(&text, 8, 0.8);
    assert_eq!((sim.bot_count(), (0..8).filter(|s| sim.is_bot(*s)).count()), (8, 8));
    let drivers: std::collections::HashSet<Driver> = (0..8).map(|s| sim.driver(s).unwrap()).collect();
    assert_eq!(drivers.len(), 8, "one bot per animal");
    assert!(sim.bot_name(3).is_some());
    let ticks = run(&mut sim, 60 * 150);
    let race = sim.race().unwrap();
    assert_eq!(race.phase(), Phase::Finished, "the race ended (took {ticks} ticks)");
    let standings = race.standings();
    assert_eq!(standings.len(), 8);
    assert!(standings.iter().all(|s| s.finished_at.is_some()), "every bot finished two laps: {standings:?}");
    let times: Vec<u32> = standings.iter().map(|s| s.finished_at.unwrap()).collect();
    assert!(times.windows(2).all(|w| w[0] <= w[1]), "the standings are in finishing order");
    assert!(*times.last().unwrap() < 60 * 140, "and nobody limped round: last home at tick {}", times.last().unwrap());
    // Removing a bot frees its slot and its mind.
    sim.remove_player(3);
    assert!(!sim.is_bot(3) && sim.bot_count() == 7);
}

#[test]
fn a_better_driver_beats_a_worse_one_in_the_same_kart() {
    let text = scene(3, false);
    let parsed = red_engine2::schema::parse_scene(&text).unwrap();
    let mut sim = MatchSim::new(&parsed, parse_spawns(&text).unwrap());
    let cfg = BotsConfig::default();
    for (slot, level) in [(0usize, 0.05f32), (1, 1.0)] {
        let mut spec = cfg.spec(slot);
        spec.level = level;
        assert!(sim.add_bot_in_slot(slot, &spec));
        sim.set_driver(slot, Driver::Duck);
    }
    run(&mut sim, 60 * 200);
    let race = sim.race().unwrap();
    let (rookie, ace) = (race.progress(0).unwrap().finished_at, race.progress(1).unwrap().finished_at);
    assert!(ace.is_some(), "the nightmare bot finished");
    assert!(rookie.is_none_or(|r| r > ace.unwrap() + 60), "and beat the rookie by a clear margin: ace {ace:?}, rookie {rookie:?}");
    assert_eq!(race.place_of(1), Some(1));
}

#[test]
fn bots_pick_up_and_use_items_and_sometimes_hit_each_other() {
    let text = scene(3, true);
    let mut sim = bots(&text, 8, 0.9);
    let (mut picked, mut hazards_seen, mut spins) = (0u32, 0u32, 0u32);
    let mut planks = 0u32;
    let mut held_before = [false; MAX_PLAYERS];
    for _ in 0..60 * 120 {
        sim.tick_once();
        for slot in 0..8 {
            let k = sim.kart(slot).unwrap();
            let holds = k.item != red_engine2::sim::kart::Item::None;
            if holds && !held_before[slot] {
                picked += 1;
            }
            held_before[slot] = holds;
            if k.spin_ticks == 59 || k.spin_ticks == 29 {
                spins += 1; // the first tick of an acorn (60) or plank (30) spin-out
            }
        }
        hazards_seen = hazards_seen.max(sim.hazards().len() as u32);
        planks += sim.hazards().iter().filter(|h| h.kind == red_engine2::sim::items::HazardKind::Plank).count() as u32;
        if sim.race().unwrap().phase() == Phase::Finished {
            break;
        }
    }
    println!("bot race with items: {picked} items picked up, up to {hazards_seen} hazards at once, {spins} hit spin-outs, {planks} plank-ticks");
    assert!(picked >= 8, "the boxes hand out items: {picked}");
    assert!(hazards_seen >= 1, "the bots threw acorns or laid planks");
    assert!(spins >= 1, "and at least one of them landed");
}

#[test]
fn a_bot_race_is_deterministic() {
    let text = scene(2, true);
    let checksums = || {
        let mut sim = bots(&text, 8, 0.7);
        (0..60 * 60)
            .map(|_| {
                sim.tick_once();
                sim.checksum()
            })
            .collect::<Vec<u64>>()
    };
    assert_eq!(checksums(), checksums(), "the same bots on the same track always race the same race");
}

#[test]
fn balance_lap_times_per_animal() {
    // Each animal alone on the diamond at top skill, three laps: the data behind "is any driver hopeless or unbeatable?". Recorded, asserted loosely.
    let text = scene(3, false);
    let mut table = Vec::new();
    for driver in Driver::ALL {
        let mut sim = bots(&text, 1, 1.0);
        sim.set_driver(0, driver);
        run(&mut sim, 60 * 200);
        let finished = sim.race().unwrap().progress(0).unwrap().finished_at;
        table.push((driver, finished));
    }
    for (driver, t) in &table {
        println!("{:<7} 3 laps: {}", driver.name(), t.map_or("did not finish".into(), |t| format!("{:.1} s", t as f32 / 60.0)));
    }
    assert!(table.iter().all(|(_, t)| t.is_some()), "every animal can finish the track: {table:?}");
    let time = |d: Driver| table.iter().find(|(x, _)| *x == d).unwrap().1.unwrap();
    let (fastest, slowest) = (table.iter().map(|(_, t)| t.unwrap()).min().unwrap(), table.iter().map(|(_, t)| t.unwrap()).max().unwrap());
    assert!(time(Driver::Deer) < time(Driver::Beaver), "the fastest kart beats the slowest on a plain track");
    assert!((slowest as f32) < fastest as f32 * 1.6, "no animal is more than 60% slower than the best: {fastest} vs {slowest} ticks");
}
