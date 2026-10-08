//! The loadout shooter's modes in a real `MatchSim`: free for all and duels (scoring and limits), capture the flag (take, carry, capture) and
//! search and destroy (freeze, plant, blast, one life, new round).

use red_engine2::player::Character;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::objective::{BombState, RoundPhase};
use red_engine2::sim::player::PlayerInput;
use red_engine2::sim::shooter::ModeKind;
use red_engine2::sim::spawns::Spawn;
use serde_json::json;

fn scene_text(shooter: serde_json::Value) -> String {
    json!({
        "camera": {"position":[0,1.7,0], "target":[0,1.7,-5]},
        "combat": {"respawn_secs": 2, "spawn": "round_robin", "spawn_protect_secs": 0.0},
        "shooter": shooter,
        "objects": [{"id":"floor","type":"plane","size":[200,200]}]
    })
    .to_string()
}

fn sim(shooter: serde_json::Value) -> MatchSim {
    let scene = red_engine2::schema::parse_scene(&scene_text(shooter)).unwrap_or_else(|e| panic!("{e:?}"));
    let spawns = vec![
        Spawn { id: "a".into(), position: [-40.0, 0.0, 10.0], yaw_deg: 90.0, group: "team1".into() },
        Spawn { id: "b".into(), position: [-40.0, 0.0, 14.0], yaw_deg: 90.0, group: "team1".into() },
        Spawn { id: "c".into(), position: [40.0, 0.0, 10.0], yaw_deg: 270.0, group: "team2".into() },
        Spawn { id: "d".into(), position: [40.0, 0.0, 14.0], yaw_deg: 270.0, group: "team2".into() },
    ];
    MatchSim::new(&scene, spawns)
}

fn place(sim: &mut MatchSim, slot: usize, team: u8, x: f32, z: f32) {
    use red_engine2::sim::player::PlayerState;
    assert!(sim.add_player_at(slot, PlayerState::spawn(x, z, 0.0, 0.0, Character::Human)));
    sim.set_team(slot, team);
}

fn walk(sim: &mut MatchSim, slot: usize, seq: &mut u32, yaw: f32, ticks: u32) {
    for _ in 0..ticks {
        *seq += 1;
        sim.push_input(slot, PlayerInput { seq: *seq, forward: 1, sprint: true, yaw, ..Default::default() });
        sim.tick_once();
    }
}

#[test]
fn free_for_all_has_no_teams_and_a_duel_holds_two() {
    let ffa = sim(json!({"mode": "ffa", "team_size": 3}));
    assert_eq!(ffa.mode(), ModeKind::Ffa);
    assert!(!ffa.teams_enabled(), "no teams in free for all");
    assert_eq!(ffa.max_players(), 6);
    assert_eq!(ffa.team_score(), [0, 0]);

    let duel = sim(json!({"mode": "tdm", "team_size": 1}));
    assert!(duel.teams_enabled());
    assert_eq!(duel.max_players(), 2, "1v1");
    assert_eq!(sim(json!({})).max_players(), 12, "the default is the full roster");
}

#[test]
fn a_kill_scores_for_the_player_in_ffa_and_for_the_team_in_tdm() {
    for (mode, expect_team) in [("ffa", [0, 0]), ("tdm", [1, 0])] {
        let mut s = sim(json!({"mode": mode, "start": ["pistol", "knife"]}));
        place(&mut s, 0, if mode == "ffa" { 0 } else { 1 }, 0.0, 0.0);
        place(&mut s, 1, if mode == "ffa" { 0 } else { 2 }, 0.0, -3.0);
        // The killer shoots the other in the back until it dies.
        let mut seq = 0;
        for _ in 0..(60 * 20) {
            seq += 1;
            s.push_input(0, PlayerInput { seq, attack: seq % 2 == 0, ..Default::default() });
            s.tick_once();
            if s.player(0).unwrap().combat.kills > 0 {
                break;
            }
        }
        assert!(s.player(0).unwrap().combat.kills > 0, "{mode}: the target was shot");
        assert_eq!(s.team_score(), expect_team, "{mode}");
    }
}

fn ctf() -> MatchSim {
    sim(json!({
        "mode": "ctf",
        "flags": [{"team": 1, "at": [-30, 0, 0]}, {"team": 2, "at": [30, 0, 0]}],
        "objective": {"capture_limit": 3, "return_secs": 5}
    }))
}

#[test]
fn a_raider_takes_the_enemy_flag_carries_it_home_and_scores() {
    let mut s = ctf();
    // Team 1's raider appears on team 2's flag and walks home to -x.
    place(&mut s, 0, 1, 30.0, 0.0);
    s.tick_once();
    let flag = s.arena().unwrap().ctf.as_ref().unwrap().flags[1];
    assert!(matches!(flag.state, red_engine2::sim::objective::FlagState::Carried { slot: 0, .. }), "{:?}", flag.state);
    let mut seq = 0;
    walk(&mut s, 0, &mut seq, -std::f32::consts::FRAC_PI_2, 60 * 12);
    assert_eq!(s.team_score(), [1, 0], "one capture for team 1");
    let flags = s.arena().unwrap().ctf.as_ref().unwrap().flags;
    assert_eq!(flags[1].state, red_engine2::sim::objective::FlagState::Home, "the captured flag is back");
    assert!(s.arena().unwrap().obj_events.iter().any(|e| e.kind == red_engine2::sim::objective::ev::FLAG_CAPTURED));
}

#[test]
fn killing_the_carrier_drops_the_flag() {
    let mut s = ctf();
    place(&mut s, 0, 1, 30.0, 0.0);
    s.tick_once();
    assert_eq!(s.arena().unwrap().ctf.as_ref().unwrap().carried_by(0), Some(2));
    // A defender (team 2) stands behind the carrier and shoots.
    place(&mut s, 1, 2, 30.0, 4.0);
    let mut seq = 0;
    for _ in 0..(60 * 20) {
        seq += 1;
        s.push_input(1, PlayerInput { seq, attack: seq % 2 == 0, yaw: 0.0, ..Default::default() });
        s.tick_once();
        if s.player(0).unwrap().combat.is_dead() {
            break;
        }
    }
    assert!(s.player(0).unwrap().combat.is_dead(), "the carrier was shot dead");
    s.tick_once();
    let st = s.arena().unwrap().ctf.as_ref().unwrap().flags[1].state;
    assert!(!matches!(st, red_engine2::sim::objective::FlagState::Carried { .. }), "dropped or returned: {st:?}");
    assert!(s.arena().unwrap().obj_events.iter().any(|e| e.kind == red_engine2::sim::objective::ev::FLAG_DROPPED));
}

fn snd() -> MatchSim {
    sim(json!({
        "mode": "snd",
        "sites": [{"name": "A", "at": [0, 0, 0], "radius": 5}],
        "objective": {"freeze_secs": 2, "round_secs": 40, "plant_secs": 2, "defuse_secs": 3, "fuse_secs": 6, "win_rounds": 2, "swap_after": 1}
    }))
}

#[test]
fn search_and_destroy_freezes_then_plants_blows_up_and_starts_a_fresh_round() {
    let mut s = snd();
    place(&mut s, 0, 1, 0.0, 0.0); // attacker on the site
    place(&mut s, 1, 2, 38.0, 0.0); // defender far away
                                    // Frozen: pushing forward goes nowhere.
    let start = s.player(0).unwrap().state.pos;
    let mut seq = 0;
    walk(&mut s, 0, &mut seq, 0.0, 30);
    assert!(s.input_locked(), "still in the freeze");
    assert!((s.player(0).unwrap().state.pos - start).length() < 0.01, "held in place");
    // Live after the freeze; the attacker holds Interact for the plant.
    for _ in 0..(60 * 2) {
        seq += 1;
        s.push_input(0, PlayerInput { seq, interact: true, ..Default::default() });
        s.tick_once();
    }
    assert!(!s.input_locked());
    for _ in 0..(60 * 3) {
        seq += 1;
        s.push_input(0, PlayerInput { seq, interact: true, ..Default::default() });
        s.tick_once();
    }
    let snd_state = s.arena().unwrap().snd.as_ref().unwrap();
    assert!(matches!(snd_state.bomb, BombState::Planted { .. }), "{:?}", snd_state.bomb);
    // The fuse burns; the planter stays on the site and dies in the blast, but the round is the attackers'.
    for _ in 0..(60 * 7) {
        s.tick_once();
    }
    assert_eq!(s.team_score(), [1, 0], "the blast scored for the attackers");
    assert!(s.player(0).unwrap().combat.is_dead(), "the planter stood in the blast");
    assert!(matches!(s.arena().unwrap().snd.as_ref().unwrap().phase, RoundPhase::Over { winner: 1, .. }));
    // A dead player stays dead until the next round: no respawn timer.
    for _ in 0..(60 * 3) {
        s.tick_once();
    }
    // After the over-pause everyone is back, sides swapped (swap_after 1).
    for _ in 0..(60 * 3) {
        s.tick_once();
    }
    assert!(!s.player(0).unwrap().combat.is_dead(), "revived for round 2");
    let st = s.arena().unwrap().snd.as_ref().unwrap();
    assert_eq!(st.round, 2);
    assert_eq!(st.attackers, 2, "sides swapped");
}

/// Skilled team-1 bots in slots 0, 2, 4 and `defenders` rookie team-2 bots in slots 3, 1, 5: the objective is then decided by what the
/// skilled side does, not by which team wins the open-field shoot-out in the middle of a map with no cover.
fn bots(sim: &mut MatchSim, attackers: usize, defenders: usize) {
    let strong = red_engine2::sim::ai::BotsConfig { level: 0.8, ..Default::default() };
    let weak = red_engine2::sim::ai::BotsConfig { level: 0.05, ..Default::default() };
    for (n, slot) in [0usize, 2, 4].into_iter().take(attackers).enumerate() {
        assert!(sim.add_bot_in_slot_team(slot, &strong.spec(n), 1), "attacker {slot}");
    }
    for (n, slot) in [3usize, 1, 5].into_iter().take(defenders).enumerate() {
        assert!(sim.add_bot_in_slot_team(slot, &weak.spec(n), 2), "defender {slot}");
    }
}

#[test]
fn bots_play_capture_the_flag_taking_carrying_and_scoring() {
    let mut s = ctf();
    bots(&mut s, 3, 1);
    let (mut taken, mut captured, mut seen) = (0, 0, std::collections::HashSet::new());
    for _ in 0..(60 * 60 * 6) {
        s.tick_once();
        for e in &s.arena().unwrap().obj_events {
            if seen.insert(e.id) {
                taken += (e.kind == red_engine2::sim::objective::ev::FLAG_TAKEN) as u32;
                captured += (e.kind == red_engine2::sim::objective::ev::FLAG_CAPTURED) as u32;
            }
        }
    }
    println!("flags taken {taken}, captured {captured}, score {:?}", s.team_score());
    assert!(taken >= 2, "bots go for the flag (taken {taken})");
    assert!(captured >= 1, "bots bring the flag home (captured {captured})");
}

#[test]
fn bots_play_search_and_destroy_through_several_rounds() {
    let mut s = snd();
    bots(&mut s, 3, 0);
    let mut planted = 0;
    let mut seen = std::collections::HashSet::new();
    let mut rounds = 0;
    for _ in 0..(60 * 60 * 8) {
        s.tick_once();
        for e in &s.arena().unwrap().obj_events {
            if seen.insert(e.id) {
                planted += (e.kind == red_engine2::sim::objective::ev::BOMB_PLANTED) as u32;
                rounds += (e.kind == red_engine2::sim::objective::ev::ROUND_WON) as u32;
            }
        }
        if s.team_score().iter().any(|p| *p >= 2) {
            break;
        }
    }
    println!("rounds {rounds}, plants {planted}, score {:?}", s.team_score());
    assert!(rounds >= 2, "rounds are decided (won {rounds})");
    assert!(planted >= 1, "the attackers plant the bomb at least once");
}

/// Debug aid, not part of the suite: `KC_MAP=path/to/map.json KC_MODE=ctf cargo test --test objective_modes real_map -- --ignored --nocapture`
/// runs bots only (3 skilled team-1 bots in slots 0, 2, 4 against 3 rookies in 1, 3, 5) on a real map and prints where they go and what happens.
#[test]
#[ignore]
fn real_map_bots() {
    let Ok(path) = std::env::var("KC_MAP") else { return };
    let mode = std::env::var("KC_MODE").ok().and_then(|m| ModeKind::parse(&m));
    let text = std::fs::read_to_string(&path).unwrap();
    let text = red_engine2::sim::shooter::with_overrides(&text, mode, None, None).unwrap();
    let scene = red_engine2::schema::parse_scene(&text).unwrap_or_else(|e| panic!("{e:?}"));
    let spawns = red_engine2::sim::spawns::parse_spawns(&text).unwrap();
    let mut s = MatchSim::try_new(&scene, spawns).unwrap();
    if std::env::var("KC_EQUAL").is_ok() {
        // Equal skill on both sides: the realistic case, and the hard one for an objective (nobody walks through five enemies).
        let cfg = red_engine2::sim::ai::BotsConfig { level: 0.7, ..Default::default() };
        for i in 0..6 {
            assert!(s.add_bot_in_slot_team(i, &cfg.spec(i), 1 + (i % 2) as u8));
        }
    } else {
        bots(&mut s, 3, 3);
    }
    let secs: u64 = std::env::var("KC_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(180);
    let mut seen = std::collections::HashSet::new();
    for t in 0..(60 * secs) {
        s.tick_once();
        for e in s.arena().unwrap().obj_events.clone() {
            if seen.insert(e.id) {
                println!("t={:>5.1}s {}", t as f32 / 60.0, red_engine2::sim::objective::describe(e.kind, e.team, e.slot));
            }
        }
        if t % 600 == 0 {
            let row: Vec<String> = [0usize, 2, 4, 1, 3, 5]
                .iter()
                .map(|&i| {
                    let p = s.player(i).unwrap();
                    format!("{i}:({:.0},{:.0}){}", p.state.pos.x, p.state.pos.y, if p.combat.is_dead() { "x" } else { "" })
                })
                .collect();
            println!("t={:>5.1}s {}  score {:?}", t as f32 / 60.0, row.join(" "), s.team_score());
        }
    }
}
