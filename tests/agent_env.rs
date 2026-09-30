//! The agent interface (`sim::env`): a two-agent episode of the existing `recipes/coin_run.json` is played to its end through `reset` / `observe` / `step`
//! using only what `observe` returns; the episode is reproducible from its seed; an agent sees other players only by line of sight inside its field of view;
//! and what is privileged (every position, `_` variables, game events) is only in `debug_state`.

use red_engine2::player::Character;
use red_engine2::schema::parse_scene_in;
use red_engine2::sim::env::{AgentSpec, Env, Observation, Sight};
use red_engine2::sim::player::PlayerInput;
use red_engine2::sim::spawns::parse_spawns;
use std::path::PathBuf;

fn recipe(name: &str) -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("recipes").join(name)).expect("the recipe exists")
}

fn agent(id: &str, character: Character, spawn: Option<&str>) -> AgentSpec {
    AgentSpec { id: id.to_string(), character, spawn: spawn.map(str::to_string) }
}

/// Forward toward `to`, looking at it: the same steering the scenario runner's `walk` uses.
fn steer(obs: &Observation, to: (f32, f32)) -> PlayerInput {
    let (dx, dz) = (to.0 - obs.pos[0], to.1 - obs.pos[1]);
    PlayerInput { forward: 1, yaw: libm::atan2f(dx, -dz), ..Default::default() }
}

fn score(obs: &Observation) -> f64 {
    obs.vars.iter().find(|(n, _)| n == "score").map(|(_, v)| *v).unwrap_or(0.0)
}

/// Plays the coin run with two agents that split the coins and only go to the exit once the public `score` says all three are taken. Returns the episode's end.
fn play_coin_run(seed: u64, max_ticks: u64) -> (Env, Option<String>, u64) {
    let text = recipe("coin_run.json");
    let scene = parse_scene_in(&text, None).expect("the recipe is a valid scene");
    let spawns = parse_spawns(&text).unwrap();
    let agents = [agent("human", Character::Human, Some("spawn_a")), agent("rat", Character::Rat, Some("spawn_a"))];
    let (mut env, mut obs) = Env::reset(&scene, &spawns, &agents, seed, max_ticks).expect("reset");
    // Each agent's plan: coins first, then the exit (only once the public score is 3).
    let plans: [Vec<(f32, f32)>; 2] = [vec![(-5.0, -3.0), (5.0, -2.0)], vec![(0.0, 3.0)]];
    let exit = (8.8, 0.0);
    let mut leg = [0usize, 0usize];
    let mut ticks = 0;
    while env.terminated().is_none() && !env.truncated() {
        let mut actions: Vec<(&str, PlayerInput)> = Vec::new();
        for (i, name) in ["human", "rat"].iter().enumerate() {
            let o = obs.iter().find(|o| o.agent == *name).expect("observed");
            let target = match plans[i].get(leg[i]) {
                Some(t) => *t,
                None if score(o) >= 3.0 => exit,
                None => continue, // waiting for the other agent's coins: stand still
            };
            if (target.0 - o.pos[0]).hypot(target.1 - o.pos[1]) < 0.3 && plans[i].get(leg[i]).is_some() {
                leg[i] += 1;
            }
            actions.push((name, steer(o, target)));
        }
        let r = env.step(&actions).expect("step");
        obs = r.observations;
        ticks += 1;
    }
    let ended = env.terminated().map(str::to_string);
    (env, ended, ticks)
}

#[test]
fn two_agents_finish_the_coin_run_through_the_interface_and_the_game_ends_the_episode() {
    let (env, ended, ticks) = play_coin_run(1, 3000);
    assert_eq!(ended.as_deref(), Some("victory"), "the game, not the tick limit, ended it (after {ticks} ticks)");
    assert!(!env.truncated());
    let debug = env.debug_state();
    assert_eq!(debug.vars.iter().find(|(n, _)| n == "score").map(|(_, v)| *v), Some(3.0), "{debug:?}");
    assert_eq!(debug.events.iter().filter(|e| e.2 == "coin").count(), 3);
    assert!(env.agents() == ["human", "rat"]);
}

#[test]
fn an_episode_is_reproducible_from_its_seed_and_the_seed_chooses_unnamed_spawns() {
    let (a, _, ta) = play_coin_run(7, 3000);
    let (b, _, tb) = play_coin_run(7, 3000);
    assert_eq!((ta, a.debug_state().checksum), (tb, b.debug_state().checksum), "same seed, same game, bit for bit");
    assert_eq!(a.debug_state().seed, 7);
    // Unnamed agents are assigned by a shuffle of the scene's spawns seeded by the reset seed: stable for a seed, varying across seeds.
    let scene_json = |spawns: &str| {
        format!(r##"{{"camera":{{"position":[0,8,8]}},"spawns":{spawns},"objects":[{{"id":"floor","type":"plane","size":[40,40],"position":[0,0,0]}}]}}"##)
    };
    let json =
        scene_json(r#"[{"id":"s1","position":[-9,0,0],"yaw_deg":0},{"id":"s2","position":[0,0,0],"yaw_deg":0},{"id":"s3","position":[9,0,0],"yaw_deg":0}]"#);
    let (scene, spawns) = (parse_scene_in(&json, None).unwrap(), parse_spawns(&json).unwrap());
    let place = |seed: u64| {
        let agents = [agent("a", Character::Human, None), agent("b", Character::Human, None)];
        let (env, _) = Env::reset(&scene, &spawns, &agents, seed, 10).unwrap();
        env.debug_state().players.iter().map(|p| p.2 as i32).collect::<Vec<_>>()
    };
    assert_eq!(place(5), place(5));
    let distinct: std::collections::BTreeSet<Vec<i32>> = (1..30).map(place).collect();
    assert!(distinct.len() > 1, "different seeds give different assignments: {distinct:?}");
    assert!(place(5).iter().all(|x| [-9, 0, 9].contains(x)) && place(5)[0] != place(5)[1], "every agent on a distinct spawn point");
}

fn wall_scene(with_wall: bool) -> (red_engine2::schema::Scene, Vec<red_engine2::sim::spawns::Spawn>) {
    let wall = if with_wall { r#",{"id":"divider","type":"wall","from":[0,-6],"to":[0,6],"height":3}"# } else { "" };
    let json = format!(
        r##"{{"camera":{{"position":[0,12,12]}},
        "spawns":[{{"id":"west","position":[-4,0,0],"yaw_deg":90}},{{"id":"east","position":[4,0,0],"yaw_deg":-90}}],
        "vars":{{"score":0,"_secret":7}},
        "rules":[{{"id":"tell","when":{{"start":true}},"do":[{{"emit":"hidden_event"}}]}}],
        "objects":[{{"id":"floor","type":"plane","size":[30,30],"position":[0,0,0]}},
          {{"id":"n","type":"wall","from":[-12,-8],"to":[12,-8]}},{{"id":"s","type":"wall","from":[-12,8],"to":[12,8]}},
          {{"id":"e","type":"wall","from":[12,-8],"to":[12,8]}},{{"id":"w","type":"wall","from":[-12,-8],"to":[-12,8]}}{wall}]}}"##
    );
    (parse_scene_in(&json, None).expect("valid scene"), parse_spawns(&json).unwrap())
}

fn two_players(with_wall: bool) -> Env {
    let (scene, spawns) = wall_scene(with_wall);
    let agents = [agent("seeker", Character::Human, Some("west")), agent("hider", Character::Human, Some("east"))];
    Env::reset(&scene, &spawns, &agents, 1, 600).expect("reset").0
}

#[test]
fn an_agent_sees_another_player_only_by_line_of_sight_inside_its_field_of_view() {
    // A wall between them: the seeker sees nothing, although both are 8 m apart in the same room and the authoritative state has both.
    let walled = two_players(true);
    assert!(walled.observe("seeker").unwrap().seen.is_empty(), "a wall blocks the view");
    assert_eq!(walled.debug_state().players.len(), 2, "the privileged state still has everyone");
    // No wall: the seeker (facing +x) sees the hider, who stands 8 m ahead.
    let mut open = two_players(false);
    let seen = open.observe("seeker").unwrap().seen;
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].agent, "hider");
    assert!((seen[0].distance_m - 8.0).abs() < 0.5, "{seen:?}");
    // The hider is not omniscient either: both face each other here, so it sees the seeker; turn the seeker away and it no longer sees the hider behind it.
    let away = PlayerInput { yaw: -std::f32::consts::FRAC_PI_2, ..Default::default() };
    for _ in 0..3 {
        open.step(&[("seeker", away)]).unwrap();
    }
    assert!(open.observe("seeker").unwrap().seen.is_empty(), "outside the field of view");
    // Range limits the view too.
    let mut near = two_players(false);
    near.set_sight(Sight { range_m: 3.0, fov_deg: 110.0 });
    assert!(near.observe("seeker").unwrap().seen.is_empty(), "8 m is beyond a 3 m sight range");
}

#[test]
fn privileged_state_is_only_in_the_debug_view() {
    let mut env = two_players(false);
    env.step(&[]).unwrap(); // the `start` rule fires on the first tick
    let obs = env.observe("seeker").unwrap();
    assert!(obs.vars.iter().any(|(n, _)| n == "score"), "the HUD's variables are public");
    assert!(obs.vars.iter().all(|(n, _)| !n.starts_with('_')), "`_` variables are internal: {:?}", obs.vars);
    let debug = env.debug_state();
    assert!(debug.vars.iter().any(|(n, v)| n == "_secret" && *v == 7.0), "{debug:?}");
    assert!(debug.events.iter().any(|e| e.2 == "hidden_event"), "game events are privileged, in the debug view only");
    // There is no field on an Observation that could carry an event, a checksum or another player's position unless it is in `seen`: nothing to leak by construction.
    let other_positions: Vec<[f32; 2]> = obs.seen.iter().map(|s| s.pos).collect();
    assert!(other_positions.iter().all(|p| debug.players.iter().any(|d| [d.2, d.3] == *p)), "what is seen is a subset of the truth");
}

#[test]
fn stepping_rules_unknown_agents_and_a_finished_episode_are_errors_and_the_tick_limit_truncates() {
    let (scene, spawns) = wall_scene(false);
    let agents = [agent("seeker", Character::Human, Some("west")), agent("hider", Character::Human, Some("east"))];
    let dup = [agent("x", Character::Human, None), agent("x", Character::Human, None)];
    assert!(Env::reset(&scene, &spawns, &dup, 1, 10).err().unwrap().contains("two agents"));
    assert!(Env::reset(&scene, &spawns, &[agent("x", Character::Human, Some("nowhere"))], 1, 10).err().unwrap().contains("not a spawn point"));
    let (mut env, first) = Env::reset(&scene, &spawns, &agents, 1, 3).unwrap();
    assert_eq!(first.len(), 2);
    assert!(env.step(&[("ghost", PlayerInput::default())]).unwrap_err().contains("no agent called `ghost`"));
    let mut last = None;
    for _ in 0..3 {
        last = Some(env.step(&[]).unwrap());
    }
    let last = last.unwrap();
    assert!(last.truncated && last.terminated.is_none(), "the tick limit truncates, the game did not end");
    assert!(env.step(&[]).unwrap_err().contains("episode is over"));
}
