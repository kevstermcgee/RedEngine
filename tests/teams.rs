//! A non-shooter scene can opt into team assignment (`"teams": true`) and use `who: team1`/`who: team2` in its
//! rules, without a `shooter` block — and, unlike a loadout match, a teamed bot keeps the character its roster
//! asked for rather than being reskinned into a team uniform.

use red_engine2::player::Character;
use red_engine2::sim::ai::BotSpec;
use red_engine2::sim::ai::BotsConfig;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::Spawn;
use serde_json::json;

fn scene_with_teams() -> MatchSim {
    let text = json!({
        "camera": {"position": [0, 1.7, 0], "target": [0, 1.7, -5]},
        "teams": true,
        "objects": [{"id": "floor", "type": "plane", "size": [40, 40]}],
        "vars": {"team1_here": 0, "team2_here": 0},
        "rules": [
            {"id": "a", "when": {"start": true}, "who": "team1", "do": [{"add": ["team1_here", 1]}]},
            {"id": "b", "when": {"start": true}, "who": "team2", "do": [{"add": ["team2_here", 1]}]}
        ]
    })
    .to_string();
    let scene = red_engine2::schema::parse_scene(&text).unwrap_or_else(|e| panic!("{e:?}"));
    let spawns = vec![Spawn { id: "a".into(), position: [0.0, 0.0, 0.0], yaw_deg: 0.0, group: String::new() }];
    MatchSim::new(&scene, spawns)
}

fn spec(character: Character) -> BotSpec {
    BotSpec { character, ..BotsConfig::default().spec(0) }
}

#[test]
fn teams_true_lets_a_non_shooter_scene_assign_teams_and_rules_filter_by_them() {
    let mut sim = scene_with_teams();
    assert!(sim.add_bot_in_slot_team(0, &spec(Character::Wizard), 1), "slot 0");
    assert!(sim.add_bot_in_slot_team(1, &spec(Character::Cowboy), 2), "slot 1");
    assert_eq!((sim.team_of(0), sim.team_of(1)), (1, 2));
    // Not a loadout match: the roster's own character survives, it is not reskinned into Ridgeback/Nightfall.
    assert_eq!(sim.player(0).unwrap().state.character, Character::Wizard);
    assert_eq!(sim.player(1).unwrap().state.character, Character::Cowboy);
    sim.tick_once();
    assert_eq!((sim.rules().var("team1_here"), sim.rules().var("team2_here")), (Some(1.0), Some(1.0)), "each team's own start rule fired exactly once");
}
