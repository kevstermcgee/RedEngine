//! Karts and the race inside the authoritative simulation: the same `MatchSim` the server runs, driven headless by a pursuit-steering loop.
//! Proves what the unit tests in `sim::kart` and `sim::race` cannot: that they work together through the real tick (countdown, inputs, bumps,
//! gates, standings, checksums), and that a race is deterministic. ADR 2026-09-29-great-outdoors-karts-as-a-first-class-engine-feature.

use glam::Vec2;
use red_engine2::player::Character;
use red_engine2::sim::kart::{Driver, Item};
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::player::PlayerInput;
use red_engine2::sim::race::Phase;
use red_engine2::sim::spawns::parse_spawns;

/// A diamond of four gates around the origin (radius 40); the grid is on the line's approach, two karts side by side heading east (+x).
fn scene(laps: u32, countdown: f32) -> String {
    format!(
        r#"{{"camera":{{"position":[0,30,60],"target":[0,0,0]}},"zones":[{{"id":"line","rect":[-6,-41,6,-39]}},{{"id":"east","rect":[39,-6,41,6]}},{{"id":"south","rect":[-6,39,6,41]}},{{"id":"west","rect":[-41,-6,-39,6]}}],
            "race":{{"laps":{laps},"gates":["line","east","south","west"],"countdown_secs":{countdown},"finish_grace_secs":60}},
            "spawns":[{{"id":"a","position":[-20,0,-41.5],"yaw_deg":90}},{{"id":"b","position":[-20,0,-38.5],"yaw_deg":90}}],
            "objects":[{{"id":"floor","type":"plane","size":[200,200],"position":[0,0.01,0]}}]}}"#
    )
}

fn sim_for(text: &str, drivers: [Driver; 2]) -> MatchSim {
    let scene = red_engine2::schema::parse_scene(text).expect("the race scene parses");
    let mut sim = MatchSim::new(&scene, parse_spawns(text).expect("spawns"));
    for (slot, driver) in drivers.into_iter().enumerate() {
        assert!(sim.add_player_in_slot(slot, Character::Human));
        assert!(sim.set_driver(slot, driver));
    }
    sim
}

/// Full throttle, steering towards the centre of the gate the race says is next.
fn pursue(sim: &MatchSim, slot: usize, seq: u32) -> PlayerInput {
    let race = sim.race().expect("a race");
    let p = sim.player(slot).expect("player");
    let progress = race.progress(slot).expect("progress");
    let target = race.course().gates[progress.next as usize].center();
    let (sin, cos) = p.state.yaw.sin_cos();
    let right = Vec2::new(cos, sin);
    let want = (target - p.state.pos).normalize_or_zero();
    let strafe = (right.dot(want) * 127.0 * 3.0).clamp(-127.0, 127.0) as i8;
    PlayerInput { seq, analog: true, forward: 127, strafe, ..Default::default() }
}

/// Runs until the race is over or `max_ticks`, returning the tick count.
fn run(sim: &mut MatchSim, max_ticks: u32) -> u32 {
    for t in 1..=max_ticks {
        for slot in 0..2 {
            let input = pursue(sim, slot, t);
            sim.push_input(slot, input);
        }
        sim.tick_once();
        if sim.race().unwrap().phase() == Phase::Finished {
            return t;
        }
    }
    max_ticks
}

#[test]
fn the_countdown_holds_the_karts_on_the_grid_then_they_race_a_lap() {
    let mut sim = sim_for(&scene(1, 1.0), [Driver::Duck, Driver::Duck]);
    let start = sim.player(0).unwrap().state.pos;
    assert!(!sim.race().unwrap().can_drive());
    for t in 1..=59 {
        sim.push_input(0, PlayerInput { seq: t, forward: 1, ..Default::default() });
        sim.tick_once();
    }
    assert!((sim.player(0).unwrap().state.pos - start).length() < 1e-5, "held on the grid while the countdown runs");
    sim.push_input(0, PlayerInput { seq: 60, forward: 1, ..Default::default() });
    sim.tick_once();
    assert!(sim.race().unwrap().can_drive(), "green light after one second");
    let ticks = run(&mut sim, 60 * 60);
    let race = sim.race().unwrap();
    assert_eq!(race.phase(), Phase::Finished, "two karts finish one lap of the diamond within a minute (took {ticks} ticks)");
    let standings = race.standings();
    assert_eq!(standings.len(), 2);
    assert!(standings.iter().all(|s| s.finished_at.is_some() && s.lap == 1), "{standings:?}");
    assert!(!race.can_drive(), "once it is over the karts are no longer driven");
}

#[test]
fn a_faster_driver_beats_a_slower_one_over_the_same_lap() {
    let mut sim = sim_for(&scene(2, 0.0), [Driver::Beaver, Driver::Deer]);
    run(&mut sim, 60 * 90);
    let race = sim.race().unwrap();
    assert_eq!(race.phase(), Phase::Finished);
    let order: Vec<usize> = race.standings().iter().map(|s| s.player).collect();
    assert_eq!(order, vec![1, 0], "the Deer (top speed 27) beats the Beaver (21) over two laps");
    let (deer, beaver) = (race.progress(1).unwrap().finished_at.unwrap(), race.progress(0).unwrap().finished_at.unwrap());
    assert!(beaver > deer + 60, "by a clear margin, not by a tick: Deer {deer}, Beaver {beaver}");
    assert_eq!(race.place_of(1), Some(1));
}

#[test]
fn a_race_is_deterministic_and_the_checksum_sees_it() {
    let checksums = |drivers: [Driver; 2]| {
        let mut sim = sim_for(&scene(2, 0.5), drivers);
        let mut sums = Vec::new();
        for t in 1..=900 {
            for slot in 0..2 {
                let input = pursue(&sim, slot, t);
                sim.push_input(slot, input);
            }
            sim.tick_once();
            sums.push(sim.checksum());
        }
        sums
    };
    let a = checksums([Driver::Wolf, Driver::Bear]);
    assert_eq!(a, checksums([Driver::Wolf, Driver::Bear]), "the same race twice gives the same checksum every tick");
    assert_ne!(a, checksums([Driver::Bear, Driver::Wolf]), "who drives what is part of the state");
    assert!(a.windows(2).any(|w| w[0] != w[1]), "and it changes as the race goes on");
}

#[test]
fn the_bear_bulldozes_and_a_scene_without_a_race_is_untouched() {
    // Two karts on a collision course: the Bear spins the Bunny out.
    let text = scene(1, 0.0);
    let mut sim = sim_for(&text, [Driver::Bear, Driver::Bunny]);
    for slot in 0..2 {
        let mut state = sim.player(slot).unwrap().state;
        state.pos = Vec2::new(if slot == 0 { -10.0 } else { 10.0 }, -40.0);
        state.yaw = if slot == 0 { 90f32.to_radians() } else { 270f32.to_radians() };
        sim.remove_player(slot);
        assert!(sim.add_player_at(slot, state));
    }
    sim.set_driver(0, Driver::Bear);
    sim.set_driver(1, Driver::Bunny);
    let mut spun = false;
    for t in 1..=240 {
        for slot in 0..2 {
            sim.push_input(slot, PlayerInput { seq: t, forward: 1, ..Default::default() });
        }
        sim.tick_once();
        spun |= sim.kart(1).is_some_and(|k| k.spin_ticks > 0);
        assert!(sim.kart(0).is_none_or(|k| k.spin_ticks == 0), "the Bear is never spun out");
    }
    assert!(spun, "the Bunny was spun out by the Bear");

    let plain = r#"{"camera":{"position":[0,5,10],"target":[0,0,0]},"spawns":[{"id":"a","position":[0,0,0],"yaw_deg":0}],"objects":[{"id":"floor","type":"plane","size":[40,40],"position":[0,0.01,0]}]}"#;
    let scene = red_engine2::schema::parse_scene(plain).unwrap();
    let mut sim = MatchSim::new(&scene, parse_spawns(plain).unwrap());
    let slot = sim.add_player(Character::Human).unwrap();
    assert!(sim.race().is_none());
    for t in 1..=60 {
        sim.push_input(slot, PlayerInput { seq: t, forward: 1, ..Default::default() });
        sim.tick_once();
    }
    let speed = sim.player(slot).unwrap().state.velocity.length();
    assert!((speed - 3.2).abs() < 0.5, "a plain match still walks at walking pace, not kart pace: {speed}");
}

/// Two karts in a row on the same line, both facing east, `gap` metres apart (slot 0 behind), with the countdown already over.
fn convoy(gap: f32, behind: Driver, ahead: Driver) -> MatchSim {
    let text = scene(3, 0.0);
    let mut sim = sim_for(&text, [behind, ahead]);
    for (slot, x) in [(0, -30.0), (1, -30.0 + gap)] {
        let mut state = sim.player(slot).unwrap().state;
        state.pos = Vec2::new(x, -40.0);
        state.yaw = 90f32.to_radians();
        sim.remove_player(slot);
        assert!(sim.add_player_at(slot, state));
    }
    sim.set_driver(0, behind);
    sim.set_driver(1, ahead);
    sim
}

#[test]
fn a_thrown_acorn_spins_out_the_kart_ahead_and_a_bubble_absorbs_it() {
    for shielded in [false, true] {
        let mut sim = convoy(12.0, Driver::Duck, Driver::Duck);
        sim.give_item(0, Item::Acorn);
        if shielded {
            sim.give_item(1, Item::Bubble);
        }
        let mut spun = false;
        for t in 1..=90 {
            sim.push_input(0, PlayerInput { seq: t, attack: t == 1, ..Default::default() });
            sim.push_input(1, PlayerInput { seq: t, attack: t == 1, ..Default::default() });
            sim.tick_once();
            spun |= sim.kart(1).unwrap().spin_ticks > 0;
            if t == 2 {
                assert_eq!(sim.hazards().len(), 1, "the acorn is in flight");
                assert_eq!(sim.kart(0).unwrap().item, Item::None, "and the item is used up");
            }
        }
        assert_eq!(sim.kart(0).unwrap().spin_ticks, 0, "the thrower is never hit by their own acorn");
        assert!(sim.hazards().is_empty(), "the acorn was used up");
        if shielded {
            assert!(!spun, "the bubble took the hit");
            assert_eq!(sim.kart(1).unwrap().shield_ticks, 0, "and popped");
        } else {
            assert!(spun, "the acorn spun the kart ahead out");
        }
    }
}

#[test]
fn the_beavers_plank_catches_a_kart_following_too_close() {
    let mut sim = convoy(10.0, Driver::Duck, Driver::Beaver);
    let mut spun_at = None;
    for t in 1..=180 {
        // The Beaver (slot 1) lays a plank on the first tick and drives on; the Duck (slot 0) chases at full throttle along the same line.
        sim.push_input(1, PlayerInput { seq: t, forward: 1, interact: t == 1, ..Default::default() });
        sim.push_input(0, PlayerInput { seq: t, forward: 1, ..Default::default() });
        sim.tick_once();
        if t == 2 {
            assert_eq!(sim.hazards().len(), 1, "the plank is down");
        }
        if spun_at.is_none() && sim.kart(0).unwrap().spin_ticks > 0 {
            spun_at = Some(t);
        }
    }
    assert!(spun_at.is_some(), "the chasing Duck drove into the plank");
    assert_eq!(sim.kart(1).unwrap().spin_ticks, 0, "the Beaver drove clear of his own plank");
    assert!(sim.hazards().is_empty(), "the plank broke when it was hit");
}

#[test]
fn item_boxes_hand_out_items_once_and_come_back() {
    let text = scene(3, 0.0)
        .replace(r#""race":{"#, r#""race":{"item_boxes":["crate"],"item_respawn_secs":2,"#)
        .replace(r#""zones":["#, r#""zones":[{"id":"crate","rect":[-14,-42,-10,-38]},"#);
    let mut sim = sim_for(&text, [Driver::Duck, Driver::Duck]);
    assert_eq!(sim.item_boxes().boxes().len(), 1);
    assert!(sim.item_boxes().is_ready(0));
    let mut got = None;
    for t in 1..=200 {
        sim.push_input(0, PlayerInput { seq: t, forward: 1, ..Default::default() });
        sim.tick_once();
        if got.is_none() && sim.kart(0).unwrap().item != Item::None {
            got = Some((t, sim.kart(0).unwrap().item));
        }
    }
    let (tick, item) = got.expect("driving through the box gave an item");
    assert!(tick > 10 && tick < 120, "at about the ten metres it took to reach it: tick {tick}");
    assert_ne!(item, Item::None);
    assert_eq!(sim.kart(1).unwrap().item, Item::None, "the kart that stayed on the grid got nothing");
}

#[test]
fn a_race_with_items_is_deterministic() {
    let text = scene(2, 0.5)
        .replace(r#""race":{"#, r#""race":{"item_boxes":["crate"],"item_respawn_secs":1,"#)
        .replace(r#""zones":["#, r#""zones":[{"id":"crate","rect":[-14,-42,-10,-38]},"#);
    let run = || {
        let mut sim = sim_for(&text, [Driver::Beaver, Driver::Wolf]);
        let mut sums = Vec::new();
        for t in 1..=1200 {
            for slot in 0..2 {
                let mut input = pursue(&sim, slot, t);
                input.attack = t % 90 == 0;
                input.interact = slot == 0 && t % 200 == 0;
                sim.push_input(slot, input);
            }
            sim.tick_once();
            sums.push(sim.checksum());
        }
        sums
    };
    assert_eq!(run(), run(), "items, acorns and planks are part of the deterministic state");
}
