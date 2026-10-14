//! The loadout shooter's parity harness (ADR 2026-10-14-a-system-stays-in-the-engine-when-two-games-use-it-or): bots play a loadout match in each mode
//! (team deathmatch and free for all over pickups, grenades, smoke, fire and rockets; capture the flag and search and destroy over their objective),
//! and every tick is folded into
//! one fingerprint: the match checksum (players with their kits, props, rules), the arena's own state hash, and what a snapshot carries to each player
//! (the player records and the encoded game part). The shooter is moving out of the engine to Killchain in steps; each step must leave these four
//! numbers unchanged. A change that is *meant* to change play updates them, and says why in its pull request.
//!
//! The simulation and its bots are deterministic (a bot's brain is seeded by its slot), so the same build gives the same numbers on every run; the test
//! also runs each match twice to show that.

use red_engine2::net::protocol::{ServerMsg, Snapshot};
use red_engine2::sim::ai::BotsConfig;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::Spawn;
use serde_json::{json, Value};

/// Ticks a match plays: two minutes at 60 Hz, six for the objective modes (a flag is taken and a bomb planted in that time).
fn ticks(mode: &str) -> u32 {
    if matches!(mode, "ctf" | "snd") {
        60 * 360
    } else {
        60 * 120
    }
}

/// The fingerprints of this build on Linux: (mode, value). Update them only for a change meant to change play, with the reason in the pull request.
/// Exact checksums are only guaranteed on one platform (ADR 0021-deterministic-traces-replay-and-checksums), so elsewhere the harness checks that two
/// runs agree and that the bots really play, not these numbers.
const GOLDEN: &[(&str, u64)] =
    &[("tdm", 0xef20_811b_5d5b_3e07), ("ffa", 0x8803_66bc_c801_cd9d), ("ctf", 0x4bcc_d56d_0cee_b221), ("snd", 0x8e87_8598_5318_e6ad)];

fn pickups() -> Value {
    json!([
        {"weapon": "rifle", "at": [-20, 0.3, 6]},
        {"weapon": "shotgun", "at": [20, 0.3, -6]},
        {"weapon": "rocket-launcher", "at": [0, 0.3, 18], "respawn_secs": 20},
        {"weapon": "frag", "at": [-6, 0.3, -14], "respawn_secs": 10},
        {"weapon": "smoke", "at": [6, 0.3, 14], "respawn_secs": 10},
        {"weapon": "incendiary", "at": [0, 0.3, -18], "respawn_secs": 12},
        {"weapon": "grenade-launcher", "at": [-28, 0.3, -10], "respawn_secs": 15}
    ])
}

fn shooter(mode: &str) -> Value {
    // The deathmatch modes play over pickups, launchers and every kind of grenade; in the objective modes bots fight over pickups instead of
    // flags, so there they play on their start kit (a pistol, a knife, a frag) and go for the objective.
    let mut s = json!({"start": ["pistol", "knife", "frag"], "mode": mode, "team_size": 3});
    if matches!(mode, "tdm" | "ffa") {
        s["pickups"] = pickups();
    }
    match mode {
        "ctf" => {
            s["flags"] = json!([{"team": 1, "at": [-34, 0, 0]}, {"team": 2, "at": [34, 0, 0]}]);
            s["objective"] = json!({"capture_limit": 5, "return_secs": 6});
        }
        "snd" => {
            s["sites"] = json!([{"name": "A", "at": [0, 0, 4], "radius": 5}, {"name": "B", "at": [0, 0, -16], "radius": 5}]);
            s["objective"] = json!({"freeze_secs": 2, "round_secs": 40, "plant_secs": 2, "defuse_secs": 3, "fuse_secs": 8, "win_rounds": 9, "swap_after": 2});
        }
        _ => {}
    }
    s
}

fn spawns() -> Vec<Spawn> {
    let mut v = Vec::new();
    for (i, z) in [-8.0f32, 0.0, 8.0].into_iter().enumerate() {
        v.push(Spawn { id: format!("a{i}"), position: [-38.0, 0.0, z], yaw_deg: 90.0, group: "team1".into() });
        v.push(Spawn { id: format!("b{i}"), position: [38.0, 0.0, z], yaw_deg: 270.0, group: "team2".into() });
    }
    v
}

fn match_sim(mode: &str) -> MatchSim {
    let text = json!({
        "camera": {"position": [0, 1.7, 0], "target": [0, 1.7, -5]},
        "combat": {"respawn_secs": 3, "spawn": "round_robin", "spawn_protect_secs": 1.0},
        "shooter": shooter(mode),
        "objects": [
            {"id": "floor", "type": "plane", "size": [120, 80]},
            {"id": "wall_n", "type": "box", "position": [0, 1.5, 24], "size": [10, 3, 2]},
            {"id": "crate_w", "type": "box", "position": [-14, 0.6, -4], "size": [2, 1.2, 2]},
            {"id": "crate_e", "type": "box", "position": [14, 0.6, 4], "size": [2, 1.2, 2]}
        ]
    })
    .to_string();
    let scene = red_engine2::scene_blocks::parse_scene(&text).unwrap_or_else(|e| panic!("{mode}: {e:?}"));
    let mut sim = MatchSim::new(&scene, spawns());
    // Evenly matched bots fight in the middle; in the objective modes team 1 is strong and team 2 weak, so flags are taken and bombs planted.
    let even = BotsConfig { level: 0.7, ..Default::default() };
    let (strong, weak) = (BotsConfig { level: 0.8, ..Default::default() }, BotsConfig { level: 0.05, ..Default::default() });
    for slot in 0..6 {
        let team = if mode == "ffa" { 0 } else { 1 + (slot % 2) as u8 };
        let cfg = match (mode, team) {
            ("ctf" | "snd", 1) => &strong,
            ("ctf" | "snd", _) => &weak,
            _ => &even,
        };
        assert!(sim.add_bot_in_slot_team(slot, &cfg.spec(slot), team), "{mode}: bot {slot}");
    }
    sim
}

struct Fold(u64);

impl Fold {
    fn mix(&mut self, v: u64) {
        self.0 ^= v;
        self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
    }
    fn bytes(&mut self, b: &[u8]) {
        for c in b.chunks(8) {
            let mut w = [0u8; 8];
            w[..c.len()].copy_from_slice(c);
            self.mix(u64::from_le_bytes(w));
        }
        self.mix(b.len() as u64);
    }
}

/// The fingerprint of one match in `mode`, and what it saw (kills, objective events, projectiles), so a match that stopped playing would show.
fn fingerprint(mode: &str) -> (u64, [u32; 3]) {
    let mut sim = match_sim(mode);
    let mut f = Fold(0xcbf2_9ce4_8422_2325);
    let (mut kills, mut thrown) = (0u32, 0u32);
    let mut events = std::collections::HashSet::new();
    let mut buf = Vec::new();
    for _ in 0..ticks(mode) {
        sim.tick_once();
        f.mix(sim.checksum());
        let arena = sim.arena().expect("a loadout match");
        f.mix(arena.state_hash());
        kills = kills.max(arena.team_kills.iter().sum::<u32>() + arena.kills.len() as u32);
        events.extend(arena.obj_events.iter().map(|e| (e.id, e.kind)));
        thrown += arena.projectiles.len() as u32;
        for viewer in 0..6 {
            let (players, game) = red_engine2::net::snapshot_view(&sim, viewer);
            buf.clear();
            ServerMsg::Snapshot(Snapshot { server_tick: sim.tick() as u32, players, game, ..Default::default() }).encode(&mut buf);
            f.bytes(&buf);
        }
    }
    (f.0, [kills, events.len() as u32, thrown])
}

#[test]
fn bots_play_each_loadout_mode_exactly_as_this_build_always_has() {
    let mut got = Vec::new();
    for (mode, golden) in GOLDEN {
        let (a, seen) = fingerprint(mode);
        let (b, _) = fingerprint(mode);
        assert_eq!(a, b, "{mode}: two runs of the same build differ: the harness is not deterministic");
        println!("{mode}: fingerprint {a:#018x}  (kills {}, objective events {}, projectile-ticks {})", seen[0], seen[1], seen[2]);
        assert!(seen[0] > 0, "{mode}: nobody was killed: the bots are not fighting");
        assert!(seen[2] > 0, "{mode}: nothing was thrown or fired from a launcher");
        if matches!(*mode, "ctf" | "snd") {
            assert!(seen[1] > 0, "{mode}: no objective event in six minutes");
        }
        got.push((mode, a, *golden));
    }
    if !cfg!(target_os = "linux") {
        return;
    }
    let wrong: Vec<String> = got.iter().filter(|(_, a, g)| a != g).map(|(m, a, g)| format!("{m}: {a:#018x} (was {g:#018x})")).collect();
    assert!(
        wrong.is_empty(),
        "the loadout shooter plays differently from the golden fingerprints:\n  {}\nIf the change is meant to change play, update GOLDEN in tests/loadout_parity.rs and say why.",
        wrong.join("\n  ")
    );
}
