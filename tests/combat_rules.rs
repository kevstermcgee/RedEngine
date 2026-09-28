//! The scene's pacing rules on the real simulation, no window and no socket: the weapon ladder (Gun Game), spawn protection, health
//! regeneration, where the dead reappear, and the feedback counters a client turns into sounds and hit markers. The Test Lab's `duel`
//! group (four spawns in one hall, players facing each other 4 m apart) is the arena.

use red_engine2::player::Character;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::player::{PlayerInput, PlayerState};
use red_engine2::sim::spawns::{parse_spawns, Spawn};
use red_engine2::weapons::{Weapon, SWITCH_TICKS};
use serde_json::{json, Value};
use std::path::PathBuf;

fn lab_text() -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")).unwrap()
}

/// The Test Lab with extra top-level keys merged in (`weapons`, `combat`, ...).
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
    fn new(extra: Value) -> Rig {
        let (scene, mut spawns) = lab_with(extra);
        spawns.retain(|s| s.group == "duel");
        Rig { sim: MatchSim::new(&scene, spawns), seq: [0; 8], yaw: [0.0; 8], pitch: [0.0; 8] }
    }

    fn join(&mut self) -> usize {
        let slot = self.sim.add_player(Character::Human).expect("room");
        self.yaw[slot] = self.sim.player(slot).unwrap().state.yaw;
        slot
    }

    fn push(&mut self, slot: usize, edit: impl FnOnce(&mut PlayerInput)) {
        self.seq[slot] += 1;
        let mut i = PlayerInput { seq: self.seq[slot], yaw: self.yaw[slot], pitch: self.pitch[slot], ..Default::default() };
        edit(&mut i);
        self.yaw[slot] = i.yaw;
        assert!(self.sim.push_input(slot, i));
    }

    fn idle(&mut self, n: u32) {
        for _ in 0..n {
            for s in 0..8 {
                if self.sim.player(s).is_some() {
                    self.push(s, |_| {});
                }
            }
            self.sim.tick_once();
        }
    }

    /// `slot` pulls the trigger for one tick, then everyone idles for `wait` ticks (long enough for the weapon to be ready again).
    fn shoot(&mut self, slot: usize, wait: u32) {
        for s in 0..8 {
            if self.sim.player(s).is_some() {
                self.push(s, |i| i.attack = s == slot);
            }
        }
        self.sim.tick_once();
        self.idle(wait);
    }

    fn hp(&self, slot: usize) -> u32 {
        self.sim.player(slot).unwrap().combat.hp
    }

    fn weapon(&self, slot: usize) -> Weapon {
        self.sim.player(slot).unwrap().combat.weapon
    }

    /// Turns `from` to look at the middle of `to` (bodies move when they respawn, so aim is re-taken before every shot).
    fn face(&mut self, from: usize, to: usize) {
        let (a, b) = (self.sim.player(from).unwrap().state, self.sim.player(to).unwrap().state);
        let d = b.pos - a.pos;
        self.yaw[from] = d.x.atan2(-d.y);
        self.pitch[from] = (1.0 - 1.7f32).atan2(d.length());
    }

    /// Shoots until `victim` is dead; returns the shots needed.
    fn kill(&mut self, killer: usize, victim: usize) -> u32 {
        let deaths = self.sim.player(victim).unwrap().combat.deaths;
        for n in 1..=40 {
            self.face(killer, victim);
            self.shoot(killer, 45);
            // Counted, not looked for: with a short respawn the victim can be back on their feet by the time the wait is over.
            if self.sim.player(victim).unwrap().combat.deaths > deaths {
                return n;
            }
        }
        let (k, v) = (self.sim.player(killer).unwrap(), self.sim.player(victim).unwrap());
        panic!(
            "{victim} would not die: killer at {:?} facing yaw {:.0} deg, victim at {:?} hp {}; killer {:?} shots {} hits {} protected_until {} tick {}",
            k.state.pos,
            self.yaw[killer].to_degrees(),
            v.state.pos,
            v.combat.hp,
            k.combat.weapon,
            k.combat.shots,
            k.combat.hits,
            k.combat.protected_until,
            self.sim.tick()
        );
    }
}

const LADDER: &[&str] = &["pistol", "smg", "bat"];

#[test]
fn a_kill_climbs_the_ladder_and_a_respawn_keeps_your_rung() {
    let mut r = Rig::new(json!({"weapons": {"ladder": LADDER}, "combat": {"respawn_secs": 1}}));
    let (a, b) = (r.join(), r.join());
    r.idle(5);
    assert_eq!((r.weapon(a), r.weapon(b)), (Weapon::Pistol, Weapon::Pistol), "everyone starts on rung 0");
    let shots = r.kill(a, b);
    assert_eq!(shots, 4, "four 26-damage pistol rounds kill 100 hp");
    assert_eq!((r.sim.player(a).unwrap().combat.kills, r.weapon(a)), (1, Weapon::Smg), "the killer climbs one rung at once");
    assert_eq!(r.weapon(b), Weapon::Pistol, "the victim keeps theirs");
    r.idle(60 + 5);
    let pb = r.sim.player(b).unwrap();
    assert!(!pb.combat.is_dead(), "respawned after 1 s");
    assert_eq!((pb.combat.weapon, pb.combat.deaths, pb.combat.kills), (Weapon::Pistol, 1, 0));
    // The other way round: the victim of the climber respawns on the climber's *own* rung, not the start.
    r.idle(SWITCH_TICKS);
    let shots = r.kill(b, a);
    assert!(shots >= 4);
    assert_eq!(r.weapon(b), Weapon::Smg, "b killed once: rung 1");
    r.idle(80);
    assert_eq!(r.weapon(a), Weapon::Smg, "a died holding the SMG and comes back with the SMG");
    assert_eq!(r.sim.player(a).unwrap().combat.kills, 1);
}

#[test]
fn on_a_ladder_the_weapon_cannot_be_switched_by_hand_and_the_top_rung_stays() {
    let mut r = Rig::new(json!({"weapons": {"ladder": ["pistol", "bat"]}}));
    let (a, _b) = (r.join(), r.join());
    r.idle(5);
    for _ in 0..3 {
        for s in 0..2 {
            r.push(s, |i| i.switch_weapon = s == a);
        }
        r.sim.tick_once();
        r.idle(2);
    }
    r.idle(SWITCH_TICKS);
    assert_eq!(r.weapon(a), Weapon::Pistol, "the mouse wheel does nothing on a ladder");
}

#[test]
fn spawn_protection_blocks_damage_and_ends_with_the_first_shot() {
    let mut r = Rig::new(json!({"combat": {"spawn_protect_secs": 1}, "weapons": {"starting": "pistol"}}));
    let (a, b) = (r.join(), r.join());
    r.idle(5);
    r.shoot(a, 20);
    assert_eq!(r.hp(b), 100, "b is protected: the pistol round does nothing");
    // a's own protection ended when it fired, so b can hurt it straight away.
    r.shoot(b, 20);
    assert_eq!(r.hp(a), 100 - 26, "a fired, so it lost its protection");
    // Protection also simply expires.
    r.idle(80);
    r.shoot(a, 20);
    assert_eq!(r.hp(b), 100 - 26, "after a second b can be hurt, too");
}

#[test]
fn health_regenerates_after_a_quiet_spell_and_stops_at_full() {
    let mut r = Rig::new(json!({"combat": {"regen_delay_secs": 2, "regen_per_sec": 30}, "weapons": {"starting": "pistol"}}));
    let (a, b) = (r.join(), r.join());
    r.idle(5);
    r.shoot(a, 0);
    assert_eq!(r.hp(b), 74);
    r.idle(100);
    assert_eq!(r.hp(b), 74, "nothing comes back inside the 2 s delay");
    r.idle(60); // 1 s after the delay ended
    let healed = r.hp(b);
    assert!((94..=100).contains(&healed), "about 30 hp per second: {healed}");
    r.idle(120);
    assert_eq!(r.hp(b), 100, "capped at full health");
    // Being hurt again restarts the wait.
    r.shoot(a, 0);
    let hurt = r.hp(b);
    r.idle(100);
    assert_eq!(r.hp(b), hurt, "the delay starts over");
}

#[test]
fn the_dead_come_back_at_the_spawn_farthest_from_the_living() {
    let mut r = Rig::new(json!({"combat": {"spawn": "farthest", "respawn_secs": 0.5}, "weapons": {"starting": "smg"}}));
    // a stands on spawn_a (-22, -3); b is added elsewhere, then dies and respawns.
    let spawns = parse_spawns(&lab_text()).unwrap();
    let at = |id: &str| spawns.iter().find(|s| s.id == id).unwrap().position;
    let sa = at("spawn_a");
    let a = r.sim.add_player_with(PlayerState::spawn(sa[0], sa[2], sa[1], 90.0, Character::Human)).unwrap();
    let b = r.join();
    r.yaw[a] = 90.0f32.to_radians();
    r.idle(5);
    assert_ne!(r.sim.player(b).unwrap().state.pos, glam::Vec2::new(sa[0], sa[2]), "a newcomer is not put on top of a living player");
    let shots = r.kill(a, b);
    assert!(shots > 0);
    r.idle(60);
    let pb = r.sim.player(b).unwrap();
    assert!(!pb.combat.is_dead());
    let far = at("spawn_d");
    assert!((pb.state.pos - glam::Vec2::new(far[0], far[2])).length() < 0.01, "spawn_d is the farthest from a: b is at {:?}", pb.state.pos);
}

#[test]
fn feedback_counters_count_shots_landed_hits_and_times_hurt() {
    let mut r = Rig::new(json!({"weapons": {"starting": "pistol"}}));
    let (a, b) = (r.join(), r.join());
    r.idle(5);
    r.shoot(a, 20);
    r.shoot(a, 20);
    let (ca, cb) = (&r.sim.player(a).unwrap().combat, &r.sim.player(b).unwrap().combat);
    assert_eq!((ca.shots, ca.hits, ca.hurt), (2, 2, 0));
    assert_eq!((cb.shots, cb.hits, cb.hurt), (0, 0, 2));
    // a is at spawn_a facing +X and b 4 m along it: b was hit from -X, i.e. the bearing back at the shooter is 270 degrees (yaw convention).
    let bearing = cb.hurt_bearing.to_degrees().rem_euclid(360.0);
    assert!((bearing - 270.0).abs() < 1.0, "bearing {bearing}");
    // A shot into empty space counts as a shot but not a hit.
    let mut r2 = Rig::new(json!({"weapons": {"starting": "pistol"}}));
    let a2 = r2.join();
    r2.idle(5);
    r2.shoot(a2, 20);
    let c = &r2.sim.player(a2).unwrap().combat;
    assert_eq!((c.shots, c.hits), (1, 0));
}

#[test]
fn ladder_and_combat_rules_replay_bit_for_bit() {
    use red_engine2::sim::replay::replay;
    use red_engine2::sim::trace::Header;
    let extra = json!({"weapons": {"ladder": LADDER}, "combat": {"respawn_secs": 1, "spawn": "farthest", "spawn_protect_secs": 0.5, "regen_delay_secs": 2, "regen_per_sec": 20}});
    let mut r = Rig::new(extra.clone());
    r.sim.start_recording(Header::new(0, 0, "duel", 1, 30)).unwrap();
    let (a, b) = (r.join(), r.join());
    r.idle(40);
    r.kill(a, b);
    r.idle(100);
    r.kill(b, a);
    r.idle(30);
    let trace = r.sim.take_trace().unwrap();
    let (scene, mut spawns) = lab_with(extra);
    spawns.retain(|s| s.group == "duel");
    assert!(replay(&trace, &scene, &spawns).unwrap().is_clean());
}
