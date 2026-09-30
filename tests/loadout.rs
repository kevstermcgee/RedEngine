//! The loadout shooter on the real simulation, no window and no socket: a kit of two guns with their own ammunition, reloads, headshots,
//! teams and friendly fire, grenades and blasts (and walls that stop them), pickups and the drops of the dead.

use red_engine2::player::Character;
use red_engine2::sim::kit::Slot;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::player::{PlayerInput, PlayerState};
use red_engine2::sim::spawns::Spawn;
use red_engine2::weapons::Weapon;
use serde_json::json;

struct Rig {
    sim: MatchSim,
    seq: [u32; 12],
}

fn scene(extra_objects: serde_json::Value, shooter: serde_json::Value) -> MatchSim {
    let mut objects = vec![json!({"id":"floor","type":"plane","size":[80,80]})];
    objects.extend(extra_objects.as_array().cloned().unwrap_or_default());
    let text = json!({
        "camera": {"position":[0,1.7,0], "target":[0,1.7,-5]},
        "combat": {"respawn_secs": 4},
        "shooter": shooter,
        "objects": objects,
    })
    .to_string();
    let scene = red_engine2::schema::parse_scene(&text).unwrap_or_else(|e| panic!("{e:?}"));
    let spawn = Spawn { id: "s".into(), position: [0.0, 0.0, 0.0], yaw_deg: 0.0, group: String::new() };
    MatchSim::new(&scene, vec![spawn])
}

impl Rig {
    fn new(extra_objects: serde_json::Value, shooter: serde_json::Value) -> Rig {
        Rig { sim: scene(extra_objects, shooter), seq: [0; 12] }
    }

    /// A player at (x, z) facing `yaw_deg` (0 = -Z), on `team`.
    fn join(&mut self, slot: usize, x: f32, z: f32, yaw_deg: f32, team: u8) {
        assert!(self.sim.add_player_at(slot, PlayerState::spawn(x, z, 0.0, yaw_deg, Character::Human)));
        self.sim.set_team(slot, team);
    }

    fn push(&mut self, slot: usize, f: impl FnOnce(&mut PlayerInput)) {
        self.seq[slot] += 1;
        let mut input = PlayerInput { seq: self.seq[slot], yaw: self.sim.player(slot).unwrap().state.yaw, ..Default::default() };
        f(&mut input);
        self.sim.push_input(slot, input);
    }

    /// One tick in which `slot` does `f` and everybody else does nothing.
    fn step(&mut self, slot: usize, f: impl FnOnce(&mut PlayerInput)) {
        self.push(slot, f);
        self.sim.tick_once();
    }

    fn idle(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.sim.tick_once();
        }
    }

    fn hp(&self, slot: usize) -> u32 {
        self.sim.player(slot).unwrap().combat.hp
    }

    fn kit(&self, slot: usize) -> &red_engine2::sim::kit::Kit {
        self.sim.player(slot).unwrap().combat.kit.as_ref().expect("a loadout match gives everyone a kit")
    }
}

fn start() -> serde_json::Value {
    json!({"start": ["pistol", "knife"]})
}

/// Wait out the draw time, then fire one aimed shot at `pitch` radians.
fn fire_aimed(rig: &mut Rig, slot: usize, pitch: f32) {
    rig.idle(40);
    rig.step(slot, |i| {
        i.aim = true;
        i.pitch = pitch;
    });
    rig.step(slot, |i| {
        i.aim = true;
        i.pitch = pitch;
        i.attack = true;
    });
    rig.idle(1);
}

#[test]
fn everyone_starts_with_the_same_pistol_and_knife() {
    let mut rig = Rig::new(json!([]), start());
    rig.join(0, 0.0, 0.0, 0.0, 1);
    rig.join(1, 5.0, 0.0, 0.0, 2);
    for slot in 0..2 {
        let kit = rig.kit(slot);
        assert_eq!(kit.current(), Weapon::Pistol);
        assert_eq!(kit.melee, Weapon::Knife);
        assert_eq!(kit.guns[1], None);
        assert_eq!(kit.grenade_count(), 0);
        assert_eq!(rig.sim.player(slot).unwrap().combat.weapon, Weapon::Pistol);
    }
}

#[test]
fn each_gun_has_its_own_ammunition_and_a_reload_takes_its_time() {
    let mut rig = Rig::new(json!([]), json!({"start": ["pistol", "rifle", "knife"]}));
    rig.join(0, 0.0, 0.0, 0.0, 1);
    rig.idle(40);
    assert_eq!(rig.kit(0).gun().map(|g| (g.weapon, g.loaded, g.reserve)), Some((Weapon::Pistol, 17, 68)));
    for _ in 0..3 {
        rig.step(0, |i| i.attack = true);
        rig.step(0, |_| {});
        rig.idle(10);
    }
    assert_eq!(rig.kit(0).gun().unwrap().loaded, 14, "three shots left the pistol with fourteen");
    // The rifle in the other slot is untouched and is chosen with `2`.
    rig.step(0, |i| i.select = 2);
    assert_eq!(rig.kit(0).sel, Slot::Gun(1));
    rig.idle(30);
    assert_eq!(rig.kit(0).gun().map(|g| (g.weapon, g.loaded, g.reserve)), Some((Weapon::Rifle, 30, 90)));
    rig.step(0, |i| i.select = 1);
    rig.idle(30);
    rig.step(0, |i| i.reload = true);
    let ticks = Weapon::Pistol.kit().reload_ticks();
    rig.idle(ticks - 6);
    assert_eq!(rig.kit(0).gun().unwrap().loaded, 14, "still reloading");
    rig.idle(10);
    assert_eq!(rig.kit(0).gun().map(|g| (g.loaded, g.reserve)), Some((17, 65)), "three rounds came out of this gun's reserve");
}

#[test]
fn a_headshot_hurts_more_than_a_body_shot_and_distance_weakens_a_pistol() {
    let mut rig = Rig::new(json!([]), json!({"start": ["rifle", "knife"]}));
    rig.join(0, 0.0, 0.0, 0.0, 1);
    rig.join(1, 0.0, -10.0, 0.0, 2);
    // Chest: the victim's chest is 0.5 m below the shooter's eye, 10 m away.
    fire_aimed(&mut rig, 0, (-0.5f32 / 10.0).atan());
    let body = 100 - rig.hp(1);
    assert!(body > 0 && body <= 36, "a rifle body shot does its damage: {body}");
    let mut rig = Rig::new(json!([]), json!({"start": ["rifle", "knife"]}));
    rig.join(0, 0.0, 0.0, 0.0, 1);
    rig.join(1, 0.0, -10.0, 0.0, 2);
    fire_aimed(&mut rig, 0, 0.0); // straight ahead is eye height: the head
    assert_eq!(rig.hp(1), 0, "an AK headshot kills at once");
    assert!(rig.sim.player(1).unwrap().combat.is_dead());
    assert_eq!(rig.sim.player(0).unwrap().combat.kills, 1);
    assert_eq!(rig.sim.player(0).unwrap().combat.headshots, 1);
    assert_eq!(rig.sim.player(1).unwrap().combat.killed_by, Some((0, Weapon::Rifle, true)));
    assert_eq!(rig.sim.team_kills(), [1, 0]);
}

#[test]
fn teammates_are_safe_from_bullets_and_blasts_unless_the_scene_allows_friendly_fire() {
    for (friendly, expect_hurt) in [(false, false), (true, true)] {
        let mut rig = Rig::new(json!([]), json!({"start": ["rifle", "knife"], "friendly_fire": friendly}));
        rig.join(0, 0.0, 0.0, 0.0, 1);
        rig.join(1, 0.0, -10.0, 0.0, 1);
        fire_aimed(&mut rig, 0, (-0.5f32 / 10.0).atan());
        assert_eq!(rig.hp(1) < 100, expect_hurt, "friendly fire {friendly}");
    }
}

#[test]
fn a_frag_grenade_hurts_by_distance_and_a_wall_stops_it() {
    let wall = json!([{"id":"wall","type":"box","size":[12,6,0.4],"position":[0,3,-6]}]);
    let mut rig = Rig::new(wall, json!({"start": ["pistol", "knife", "frag"]}));
    rig.join(0, 0.0, 10.0, 0.0, 1); // the thrower, south of the wall, looking north at it
    rig.join(1, 0.0, -9.0, 0.0, 2); // behind the wall
    rig.join(2, 6.0, 9.0, 0.0, 2); // near the thrower, in the open
    rig.idle(40);
    rig.step(0, |i| i.select = 4);
    rig.idle(40);
    assert_eq!(rig.kit(0).current(), Weapon::Frag);
    // Throw it straight down, at the thrower's own feet region: a short toss.
    rig.step(0, |i| {
        i.attack = true;
        i.aim = true;
        i.pitch = -1.0;
    });
    rig.idle(20);
    assert_eq!(rig.kit(0).grenade_count(), 0, "the grenade left the hand");
    rig.idle(200);
    assert_eq!(rig.hp(1), 100, "behind a wall nothing reached");
    assert!(rig.hp(0) < 100, "the thrower stood in their own blast: {}", rig.hp(0));
    assert!(rig.sim.arena().unwrap().projectiles.is_empty());
}

#[test]
fn walking_over_a_weapon_picks_it_up_and_a_full_kit_trades_on_interact() {
    let shooter = json!({"start": ["pistol", "knife"], "pickups": [
        {"weapon": "rifle", "at": [3, 0.3, 0], "respawn_secs": 5},
        {"weapon": "smg",   "at": [6, 0.3, 0]},
        {"weapon": "frag",  "at": [-3, 0.3, 0]},
        {"ammo": true,      "at": [-6, 0.3, 0]}
    ]});
    let mut rig = Rig::new(json!([]), shooter);
    rig.join(0, 0.0, 0.0, 90.0, 1);
    // Walk +X over the rifle.
    for _ in 0..40 {
        rig.step(0, |i| {
            i.forward = 1;
            i.yaw = 90f32.to_radians();
        });
    }
    assert_eq!(rig.kit(0).guns[1].map(|g| g.weapon), Some(Weapon::Rifle), "the free gun slot took it");
    assert_eq!(rig.kit(0).sel, Slot::Gun(1), "and it is in hand");
    let spot = &rig.sim.arena().unwrap().pickups[0];
    assert!(spot.taken_until.is_some(), "the spot is empty until it respawns");
    // The smg does not fit without a trade: walking over it leaves it.
    for _ in 0..60 {
        rig.step(0, |i| {
            i.forward = 1;
            i.yaw = 90f32.to_radians();
        });
    }
    assert_eq!(rig.kit(0).guns[0].map(|g| g.weapon), Some(Weapon::Pistol));
    assert!(rig.sim.arena().unwrap().pickups[1].taken_until.is_none(), "still lying there");
    // Interact trades it for the gun in hand (the rifle), which lands on the floor with its ammunition.
    rig.idle(1);
    rig.step(0, |i| i.interact = true);
    assert_eq!(rig.kit(0).guns[1].map(|g| g.weapon), Some(Weapon::Smg));
    let dropped = &rig.sim.arena().unwrap().dropped;
    assert_eq!(dropped.len(), 1);
    assert_eq!((dropped[0].weapon, dropped[0].loaded), (Weapon::Rifle, 30));
    // The respawn timer brings the rifle spot back.
    rig.idle(400);
    assert!(rig.sim.arena().unwrap().pickups[0].taken_until.is_none());
}

#[test]
fn the_dead_drop_their_guns_with_the_ammunition_left_in_them() {
    let mut rig = Rig::new(json!([]), json!({"start": ["rifle", "pistol", "knife"]}));
    rig.join(0, 0.0, 0.0, 0.0, 1);
    rig.join(1, 0.0, -10.0, 0.0, 2);
    rig.idle(40);
    rig.step(1, |i| i.attack = true); // the victim spends a round first
    fire_aimed(&mut rig, 0, 0.0);
    assert!(rig.sim.player(1).unwrap().combat.is_dead());
    let dropped = &rig.sim.arena().unwrap().dropped;
    assert_eq!(dropped.len(), 2, "both guns fell");
    let rifle = dropped.iter().find(|d| d.weapon == Weapon::Rifle).expect("the rifle");
    assert_eq!(rifle.loaded, 29, "with the round that was spent already gone");
    assert!(rig.sim.player(1).unwrap().combat.kit.as_ref().unwrap().guns.iter().all(Option::is_none));
    // After the respawn delay the victim is back with the starting kit.
    rig.idle(4 * 60 + 5);
    assert!(!rig.sim.player(1).unwrap().combat.is_dead());
    assert_eq!(rig.kit(1).current(), Weapon::Rifle);
}
