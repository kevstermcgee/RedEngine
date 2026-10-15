use red_engine2::sim::defs::Id;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::parse_spawns;
use red_engine2::sim::systems::Command;
use std::path::{Path, PathBuf};
use workshop_server::game::WorkshopGame;

fn content() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../content")
}
fn id(s: &str) -> Id {
    Id::parse(s).unwrap()
}
fn sim(file: &str) -> MatchSim {
    workshop_server::register();
    let text = std::fs::read_to_string(content().join(file)).unwrap();
    let scene = red_engine2::scene_blocks::parse_scene_in(&text, Some(&content())).unwrap();
    MatchSim::try_new(&scene, parse_spawns(&text).unwrap()).unwrap()
}
fn game(sim: &MatchSim) -> &WorkshopGame {
    sim.game::<WorkshopGame>().unwrap()
}

#[test]
fn a_refused_dismantle_when_only_the_input_fits_changes_nothing() {
    let mut s = sim("workshop.json");
    let used = |s: &MatchSim, st: &str| game(s).systems().ledger().store(&id(st)).map_or(0, |x| x.used());
    let mut found = None;
    for t in 0..20_000u32 {
        let free = game(&s).systems().ledger().free(&id("ws:storage"));
        let (inp, out) = (used(&s, "ws:furnace_a/in"), used(&s, "ws:furnace_a/out"));
        if out > 0 && inp <= free && free < inp + out {
            found = Some((t, free, inp, out));
            break;
        }
        s.tick_once();
    }
    let (t, free, inp, out) = found.expect("a tick where storage has room for the input but not for input + output");
    eprintln!("tick {t}: storage free {free}, furnace_a/in holds {inp}, furnace_a/out holds {out}");
    let before = game(&s).systems().hash();
    let dismantle = Command { name: id("ws:dismantle"), subject: Some(id("ws:furnace_a")), value: 0 };
    let result = s.game_mut::<WorkshopGame>().unwrap().command(&dismantle);
    game(&s).systems().ledger().audit().unwrap();
    assert!(result.is_err(), "the dismantle should be refused: the output has nowhere to go");
    assert_eq!(game(&s).systems().hash(), before, "a refused command must change nothing");
}
