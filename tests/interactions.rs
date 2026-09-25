//! Authoritative interactions on the real simulation, no window and no socket: pick-up contention, the bat, the revolver
//! (cooldown, ammo, reload), damage, death and respawn, engine events reaching scene rules, and a recorded fight replaying
//! bit for bit. The Test Lab's spawn groups were laid out for exactly this: `props` has two players aiming at the same
//! barrel, `duel` has two players facing each other across a hall.

use red_engine2::player::Character;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::player::PlayerInput;
use red_engine2::sim::replay::replay;
use red_engine2::sim::spawns::{parse_spawns, Spawn};
use red_engine2::sim::trace::Header;
use red_engine2::weapons::{Ammo, Weapon, BAT_DAMAGE, RESPAWN_TICKS, REVOLVER_DAMAGE, SWITCH_TICKS};
use serde_json::{json, Value};
use std::path::PathBuf;

fn lab_text() -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")).unwrap()
}

/// The Test Lab with extra top-level keys merged in (`weapons`, `vars`, `rules`).
fn lab_with(extra: Value) -> (red_engine2::schema::Scene, Vec<Spawn>) {
    let mut v: Value = serde_json::from_str(&lab_text()).unwrap();
    for (k, x) in extra.as_object().cloned().unwrap_or_default() {
        v[k] = x;
    }
    let text = v.to_string();
    (red_engine2::schema::parse_scene(&text).unwrap_or_else(|e| panic!("{e:?}")), parse_spawns(&text).unwrap())
}

struct Rig {
    sim: MatchSim,
    seq: [u32; 8],
    yaw: [f32; 8],
    pitch: [f32; 8],
}

impl Rig {
    fn new(group: &str, extra: Value) -> Rig {
        let (scene, mut spawns) = lab_with(extra);
        spawns.retain(|s| s.group == group);
        Rig { sim: MatchSim::new(&scene, spawns), seq: [0; 8], yaw: [0.0; 8], pitch: [0.0; 8] }
    }

    fn join(&mut self, who: Character) -> usize {
        let slot = self.sim.add_player(who).expect("room for a player");
        self.yaw[slot] = self.sim.player(slot).unwrap().state.yaw;
        slot
    }

    /// Joins a human standing at `(x, z)` looking `yaw_deg` / `pitch_deg` (0 yaw = -Z, 90 = +X; negative pitch = down).
    fn join_at(&mut self, x: f32, z: f32, yaw_deg: f32, pitch_deg: f32) -> usize {
        let state = red_engine2::sim::player::PlayerState::spawn(x, z, 0.0, yaw_deg, Character::Human);
        let slot = self.sim.add_player_with(state).expect("room for a player");
        (self.yaw[slot], self.pitch[slot]) = (yaw_deg.to_radians(), pitch_deg.to_radians());
        slot
    }

    /// Queues one input for `slot` keeping its aim, with `edit` applied.
    fn push(&mut self, slot: usize, edit: impl FnOnce(&mut PlayerInput)) {
        self.seq[slot] += 1;
        let mut i = PlayerInput { seq: self.seq[slot], yaw: self.yaw[slot], pitch: self.pitch[slot], ..Default::default() };
        edit(&mut i);
        self.yaw[slot] = i.yaw;
        self.pitch[slot] = i.pitch;
        assert!(self.sim.push_input(slot, i));
    }

    /// Everyone stands still (buttons released) for `n` ticks.
    fn idle(&mut self, n: u32) {
        for _ in 0..n {
            for slot in 0..8 {
                if self.sim.player(slot).is_some() {
                    self.push(slot, |_| {});
                }
            }
            self.sim.tick_once();
        }
    }

    /// One tick in which `slot` presses a button (everybody else idles), then a released tick.
    fn press(&mut self, slot: usize, edit: impl Fn(&mut PlayerInput)) {
        for pressed in [true, false] {
            for s in 0..8 {
                if self.sim.player(s).is_some() {
                    self.push(s, |i| {
                        if pressed && s == slot {
                            edit(i)
                        }
                    });
                }
            }
            self.sim.tick_once();
        }
    }

    fn count(&self, name: &str) -> usize {
        self.sim.rules().history().iter().filter(|e| e.name == name).count()
    }

    fn hp(&self, slot: usize) -> u32 {
        self.sim.player(slot).unwrap().combat.hp
    }
}

#[test]
fn two_players_aiming_at_one_barrel_only_one_gets_it_and_the_other_can_take_it_after_the_drop() {
    let mut r = Rig::new("props", json!({}));
    // Two people standing about 1.3 m from the first barrel of the row (domino_0 at x 1.0, z 4.0), looking down at it.
    let a = r.join_at(1.0, 5.4, 0.0, -40.0);
    let b = r.join_at(1.9, 4.9, 315.0, -42.0);
    r.idle(5); // let the kinematic bodies settle
               // Both press E in the same tick.
    for s in [a, b] {
        r.push(s, |i| i.interact = true);
    }
    r.sim.tick_once();
    let (ha, hb) = (r.sim.props().held_by(a), r.sim.props().held_by(b));
    assert!(ha.is_some() ^ hb.is_some(), "exactly one of them holds the barrel (a: {ha:?}, b: {hb:?})");
    assert!(ha.is_some(), "slot order decides: the first slot wins a same-tick tie");
    assert_eq!(r.count("pickup"), 1, "one pickup event");
    let held = ha.or(hb).unwrap();
    let (winner, loser) = if ha.is_some() { (a, b) } else { (b, a) };
    // The loser pressing again gets nothing while it is held.
    r.idle(2);
    r.press(loser, |i| i.interact = true);
    assert_eq!(r.sim.props().held_by(loser), None);
    assert_eq!(r.sim.props().holder_of(held), Some(winner));
    // The carried prop stays in front of its holder: turning 90 degrees carries it along.
    r.idle(3);
    // What a snapshot would send: the published transform of the prop's entity.
    let at = |r: &Rig| {
        let e = r.sim.props().entity_of(held).expect("a carried prop is a dynamic entity");
        r.sim.props().entities().transforms.get(e.slot()).position
    };
    let eye = |r: &Rig, s: usize| {
        let p = r.sim.player(s).unwrap();
        glam::Vec3::new(p.state.pos.x, p.state.foot_y, p.state.pos.y)
    };
    let before = at(&r);
    assert!((before - eye(&r, winner)).length() < 2.5, "held in front of the player, not left behind: {}", (before - eye(&r, winner)).length());
    let yaw = r.yaw[winner] + std::f32::consts::FRAC_PI_2;
    r.push(winner, |i| i.yaw = yaw);
    r.idle(3);
    assert!((at(&r) - before).length() > 0.3, "the prop followed the turn");
    // Drop it; now the other player can pick it up if they aim at it.
    r.press(winner, |i| i.interact = true);
    assert_eq!(r.sim.props().held_by(winner), None);
    assert_eq!(r.count("drop"), 1);
    assert!(r.sim.props().holder_of(held).is_none());
}

#[test]
fn a_leaving_player_lets_go_of_what_they_carry() {
    let mut r = Rig::new("props", json!({}));
    let a = r.join_at(1.0, 5.4, 0.0, -40.0);
    r.idle(5);
    r.press(a, |i| i.interact = true);
    let held = r.sim.props().held_by(a).expect("picked up");
    r.sim.remove_player(a);
    assert!(!r.sim.props().is_held(held), "the barrel is free again");
}

#[test]
fn the_bat_hurts_a_player_in_reach_and_only_in_reach() {
    let mut r = Rig::new("duel", json!({}));
    let (a, b) = (r.join(Character::Human), r.join(Character::Human)); // 4 m apart, facing each other
    r.idle(5);
    r.press(a, |i| i.attack = true);
    r.idle(30);
    assert_eq!(r.hp(b), 100, "4 m is out of the bat's reach");
    for _ in 0..40 {
        r.push(a, |i| i.forward = 1);
        r.push(b, |_| {});
        r.sim.tick_once();
    }
    r.press(a, |i| i.attack = true);
    r.idle(30);
    assert_eq!(r.hp(b), 100 - BAT_DAMAGE, "a swing at close range lands once");
    assert_eq!(r.count("hit"), 1);
    // The rat has no weapon.
    let mut r2 = Rig::new("duel", json!({}));
    let (h, rat) = (r2.join(Character::Human), r2.join(Character::Rat));
    r2.idle(5);
    r2.press(rat, |i| i.attack = true);
    r2.idle(30);
    assert_eq!(r2.hp(h), 100);
    assert_eq!(r2.count("hit"), 0);
}

#[test]
fn the_revolver_has_a_cooldown_damages_kills_and_the_victim_respawns() {
    let mut r = Rig::new("duel", json!({}));
    let (a, b) = (r.join(Character::Human), r.join(Character::Human));
    r.idle(5);
    r.press(a, |i| i.switch_weapon = true);
    assert_eq!(r.sim.player(a).unwrap().combat.weapon, Weapon::Revolver);
    r.press(a, |i| i.attack = true);
    assert_eq!(r.count("shot"), 0, "cannot fire while the weapon is being raised");
    r.idle(SWITCH_TICKS);
    r.press(a, |i| i.attack = true);
    assert_eq!((r.count("shot"), r.hp(b)), (1, 100 - REVOLVER_DAMAGE));
    r.press(a, |i| i.attack = true);
    assert_eq!(r.count("shot"), 1, "the cooldown blocks a second shot two ticks later");
    r.idle(30);
    for _ in 0..2 {
        r.press(a, |i| i.attack = true);
        r.idle(30);
    }
    assert_eq!(r.hp(b), 100 - 3 * REVOLVER_DAMAGE);
    r.press(a, |i| i.attack = true);
    assert!(r.sim.player(b).unwrap().combat.is_dead(), "hp {}", r.hp(b));
    assert_eq!((r.count("kill"), r.sim.player(a).unwrap().combat.kills, r.sim.player(b).unwrap().combat.deaths), (1, 1, 1));
    // The dead cannot move; their inputs are consumed but ignored.
    let dead_at = r.sim.player(b).unwrap().state.pos;
    for _ in 0..20 {
        r.push(a, |_| {});
        r.push(b, |i| i.forward = 1);
        r.sim.tick_once();
    }
    assert_eq!(r.sim.player(b).unwrap().state.pos, dead_at, "a dead player stays put");
    // After the respawn delay they are back at a spawn point with full health and the bat.
    r.idle(RESPAWN_TICKS as u32);
    let pb = &r.sim.player(b).unwrap();
    assert!(!pb.combat.is_dead() && pb.combat.hp == 100 && pb.combat.weapon == Weapon::Bat, "respawned: {:?}", pb.combat);
    assert_eq!(r.count("respawn"), 1);
    assert_eq!(pb.combat.deaths, 1, "the score survives the respawn");
}

#[test]
fn limited_ammo_from_the_scene_runs_dry_clicks_and_reloads_from_the_reserve() {
    let mut r = Rig::new("duel", json!({"weapons": {"revolver": {"ammo": {"loaded": 2, "reserve": 3}}}}));
    let (a, _b) = (r.join(Character::Human), r.join(Character::Human));
    r.idle(5);
    r.press(a, |i| i.switch_weapon = true);
    r.idle(SWITCH_TICKS);
    for _ in 0..3 {
        r.press(a, |i| i.attack = true);
        r.idle(30);
    }
    assert_eq!(r.count("shot"), 2, "two rounds in the cylinder: the third click is dry");
    assert_eq!(r.sim.player(a).unwrap().combat.ammo, Ammo::Limited { loaded: 0, capacity: 6, reserve: 3 });
    r.press(a, |i| i.reload = true);
    assert_eq!(r.sim.player(a).unwrap().combat.ammo, Ammo::Limited { loaded: 3, capacity: 6, reserve: 0 }, "the reserve went in");
    r.idle(30);
    r.press(a, |i| i.attack = true);
    assert_eq!(r.count("shot"), 3);
}

#[test]
fn engine_events_reach_scene_rules_so_a_game_can_score_kills_with_data_only() {
    let extra = json!({
        "vars": {"score": 0, "carried": 0},
        "rules": [
            {"id": "score_kill", "when": {"event": "kill"}, "do": [{"add": ["score", 10]}]},
            {"id": "note_pickup", "when": {"event": "pickup"}, "do": [{"add": ["carried", 1]}]},
            {"id": "first_to_10", "when": {"event": "kill"}, "if": "score >= 10", "do": [{"end": "round_over"}]}
        ],
        "weapons": {"bat": {"damage": 100}}
    });
    let mut r = Rig::new("duel", extra);
    let (a, b) = (r.join(Character::Human), r.join(Character::Human));
    r.idle(5);
    for _ in 0..40 {
        r.push(a, |i| i.forward = 1);
        r.push(b, |_| {});
        r.sim.tick_once();
    }
    r.press(a, |i| i.attack = true);
    r.idle(30);
    assert!(r.sim.player(b).unwrap().combat.is_dead(), "bat damage from the scene: one hit kills");
    assert_eq!(r.sim.rules().var("score"), Some(10.0), "the `kill` engine event triggered a data rule");
    assert_eq!(r.sim.rules().ended(), Some("round_over"));
}

#[test]
fn a_recorded_fight_replays_bit_for_bit() {
    let mut r = Rig::new("duel", json!({}));
    r.sim.start_recording(Header::new(0, 0, "duel", 1, 30)).unwrap();
    let (a, b) = (r.join(Character::Human), r.join(Character::Human));
    r.idle(5);
    r.press(a, |i| i.switch_weapon = true);
    r.idle(SWITCH_TICKS);
    for _ in 0..3 {
        r.press(a, |i| i.attack = true);
        r.idle(30);
    }
    for _ in 0..40 {
        r.push(b, |i| i.forward = 1);
        r.push(a, |_| {});
        r.sim.tick_once();
    }
    r.press(b, |i| i.attack = true);
    r.idle(30);
    let trace = r.sim.take_trace().unwrap();
    let (scene, spawns) = lab_with(json!({}));
    let report = replay(&trace, &scene, &spawns).unwrap();
    assert!(report.is_clean(), "combat is deterministic and part of the checksum: {report:?}");
    assert!(trace.checkpoints.len() > 150);
}

#[test]
fn a_scenario_can_fight_with_the_hold_step() {
    use red_engine2::sim::scenario::{parse, run};
    let (scene, spawns) = lab_with(json!({}));
    let ids: Vec<String> = scene.objects.iter().map(|o| o.id.clone()).collect();
    let s = parse(
        &json!({
            "name": "revolver duel", "spawn_group": "duel",
            "players": [{"id": "a", "spawn": "spawn_a"}, {"id": "b", "spawn": "spawn_b"}],
            "script": [
                {"player": "a", "hold": {"switch": true, "seconds": 0.05}},
                {"player": "a", "wait": 0.5},
                {"player": "a", "hold": {"attack": true, "seconds": 0.05}},
                {"player": "a", "wait": 0.6},
                {"player": "a", "hold": {"attack": true, "seconds": 0.05}},
                {"player": "a", "wait": 0.6}
            ],
            "expect": [{"event": "shot", "count": 2}, {"event": "hit", "count": 2}, {"no_event": "kill"}]
        }),
        &scene.rules,
        &ids,
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let result = run(&s, &scene, &spawns, None).unwrap();
    assert!(result.passed, "{}", result.render());
}
