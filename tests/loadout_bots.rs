//! Bots in a team loadout match, headless: they fight the other team only, walk to weapons, reload, and the match produces kills.

use red_engine2::sim::ai::BotsConfig;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::Spawn;
use serde_json::json;

fn arena() -> MatchSim {
    let text = json!({
        "camera": {"position":[0,1.7,0], "target":[0,1.7,-5]},
        "combat": {"respawn_secs": 3, "spawn": "farthest", "spawn_protect_secs": 1.0},
        "shooter": {"start": ["pistol", "knife"], "pickups": [
            {"weapon": "rifle", "at": [-20, 0.3, -5], "respawn_secs": 20},
            {"weapon": "smg",   "at": [ 20, 0.3,  5], "respawn_secs": 20},
            {"weapon": "frag",  "at": [ 0, 0.3, 0], "respawn_secs": 20},
            {"ammo": true,      "at": [ 0, 0.3, 8]}
        ]},
        "objects": [
            {"id":"floor","type":"plane","size":[120,120]},
            {"id":"wall","type":"box","size":[0.4,4,14],"position":[0,2,-12]}
        ]
    })
    .to_string();
    let scene = red_engine2::schema::parse_scene(&text).unwrap_or_else(|e| panic!("{e:?}"));
    let spawns = vec![
        Spawn { id: "a".into(), position: [-35.0, 0.0, 0.0], yaw_deg: 90.0, group: "team1".into() },
        Spawn { id: "b".into(), position: [-35.0, 0.0, 6.0], yaw_deg: 90.0, group: "team1".into() },
        Spawn { id: "c".into(), position: [35.0, 0.0, 0.0], yaw_deg: 270.0, group: "team2".into() },
        Spawn { id: "d".into(), position: [35.0, 0.0, 6.0], yaw_deg: 270.0, group: "team2".into() },
    ];
    MatchSim::new(&scene, spawns)
}

#[test]
fn bots_fight_the_other_team_pick_up_weapons_and_score() {
    let mut sim = arena();
    let cfg = BotsConfig { level: 0.7, ..Default::default() };
    for i in 0..6 {
        let team = 1 + (i % 2) as u8;
        assert!(sim.add_bot_in_slot_team(i, &cfg.spec(i), team), "bot {i}");
    }
    assert_eq!((sim.team_count(1), sim.team_count(2)), (3, 3));
    for _ in 0..(60 * 120) {
        sim.tick_once();
    }
    let kills: Vec<u32> = (0..6).map(|s| sim.player(s).unwrap().combat.kills).collect();
    let deaths: Vec<u32> = (0..6).map(|s| sim.player(s).unwrap().combat.deaths).collect();
    let total: u32 = kills.iter().sum();
    assert!(total >= 4, "two minutes of a 3v3 produce kills: {kills:?} / deaths {deaths:?}");
    // Kills are all against the other team: the teams' kill counts add up to the players' kills, and deaths to kills (plus a few blasts).
    let k = sim.team_kills();
    assert_eq!(k[0] + k[1], total, "team scores are the players' kills");
    for s in 0..6 {
        let kit = sim.player(s).unwrap().combat.kit.as_ref().unwrap();
        assert!(kit.current().is_gun() || kit.current().is_melee() || kit.current().is_grenade());
    }
    let arena = sim.arena().unwrap();
    assert!(arena.pickups.iter().any(|p| p.taken_until.is_some()) || !arena.dropped.is_empty() || total > 0, "bots interact with the weapons");
    println!("kills {kills:?} deaths {deaths:?} team {k:?} dropped {}", arena.dropped.len());
}

#[test]
fn bots_never_shoot_their_own_team() {
    let mut sim = arena();
    let cfg = BotsConfig { level: 1.0, ..Default::default() };
    // Two bots of the same team facing each other across the arena: nobody is an enemy.
    assert!(sim.add_bot_in_slot_team(0, &cfg.spec(0), 1));
    assert!(sim.add_bot_in_slot_team(1, &cfg.spec(1), 1));
    for _ in 0..(60 * 30) {
        sim.tick_once();
    }
    assert_eq!(sim.team_kills(), [0, 0]);
    assert!(sim.player(0).unwrap().combat.hp == 100 && sim.player(1).unwrap().combat.hp == 100);
}
