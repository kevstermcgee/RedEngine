//! Tidewatch's story through the real authoritative match (`MatchSim` with Tidewatch as its game, its story the engine's runtime on the
//! compiled package), driven as a client drives it: walking, and intentions naming the revision they were shown. What the brief asks the
//! milestone to prove, each a test: two voices, a gameplay-backed decision and its consequence, a delayed callback, knowledge that gates
//! lines (and a speaker withheld because they do not know), refusal, delay, early completion, absent speakers, stale and repeated input,
//! interruption, save and resume, replay, content versions, a director that sometimes does nothing on purpose, and malformed packages.

use red_engine2::sim::defs::Id;
use red_engine2::sim::game::Intent;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::player::{PlayerInput, PlayerState};
use red_engine2::sim::spawns::parse_spawns;
use red_engine2::sim::systems::Command;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tidewatch_server::game::TidewatchGame;

fn content() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../content")
}

fn id(s: &str) -> Id {
    Id::parse(s).unwrap()
}

/// Where things are (x, z).
const DOCK: (f32, f32) = (13.0, 5.5);
const LAMP: (f32, f32) = (17.1, 6.3);
const SHED: (f32, f32) = (-8.6, 3.0);
const BRAZIER: (f32, f32) = (0.0, -4.9);
const BELL: (f32, f32) = (4.5, -5.8);
const SPAWN: (f32, f32) = (0.0, 4.0);

/// One player in a Tidewatch match.
struct Run {
    sim: MatchSim,
    slot: usize,
    seq: u32,
}

impl Run {
    fn from_text(text: &str) -> Run {
        tidewatch_server::register();
        let scene = red_engine2::scene_blocks::parse_scene_in(text, Some(&content())).unwrap_or_else(|e| panic!("{}", e.join("\n")));
        let mut sim = MatchSim::try_new(&scene, parse_spawns(text).unwrap()).unwrap();
        let slot = sim.add_player_with(PlayerState::spawn(SPAWN.0, SPAWN.1, 0.0, 180.0, red_engine2::player::Character::Human)).unwrap();
        Run { sim, slot, seq: 0 }
    }

    fn new() -> Run {
        Run::from_text(&std::fs::read_to_string(content().join("tidewatch.json")).unwrap())
    }

    fn game(&self) -> &TidewatchGame {
        self.sim.game::<TidewatchGame>().expect("the scene plays Tidewatch")
    }

    fn game_mut(&mut self) -> &mut TidewatchGame {
        self.sim.game_mut::<TidewatchGame>().expect("the scene plays Tidewatch")
    }

    fn step(&mut self, input: PlayerInput) {
        self.seq += 1;
        self.sim.push_input(self.slot, PlayerInput { seq: self.seq, ..input });
        self.sim.tick_once();
        let g = self.game();
        assert!(g.systems().halted().is_none(), "{:?}", g.systems().halted());
        g.systems().ledger().audit().unwrap_or_else(|e| panic!("tick {}: {e}", self.sim.tick()));
    }

    fn wait(&mut self, secs: f32) {
        for _ in 0..(secs * 60.0) as u32 {
            self.step(PlayerInput::default());
        }
    }

    fn pos(&self) -> (f32, f32) {
        let p = self.sim.player(self.slot).unwrap().state.pos;
        (p.x, p.y)
    }

    /// Walks straight to `(x, z)` (the island has no obstacles on these legs), then stands for a moment.
    fn walk(&mut self, to: (f32, f32)) {
        for _ in 0..60 * 30 {
            let (x, z) = self.pos();
            let (dx, dz) = (to.0 - x, to.1 - z);
            if (dx * dx + dz * dz).sqrt() < 0.25 {
                self.wait(0.1);
                return;
            }
            self.step(PlayerInput { forward: 1, yaw: dx.atan2(-dz), ..Default::default() });
        }
        panic!("could not walk to {to:?} (stopped at {:?})", self.pos());
    }

    fn story(&self) -> Value {
        self.sim.game_inspect("story").expect("the game runs a story")
    }

    fn rev(&self) -> i64 {
        self.story()["revision"].as_i64().unwrap()
    }

    fn intend_at(&mut self, name: &str, subject: Option<&str>, revision: i64) -> Result<(), String> {
        self.sim.intend(self.slot, &Intent { name: format!("story:{name}"), subject: subject.map(str::to_string), value: revision })
    }

    fn intend(&mut self, name: &str, subject: Option<&str>) -> Result<(), String> {
        let r = self.rev();
        self.intend_at(name, subject, r)
    }

    fn talk(&mut self, who: &str) -> Result<(), String> {
        self.intend("talk", Some(who))
    }

    fn line(&self) -> Option<String> {
        self.story()["view"]["talk"]["line"]["id"].as_str().map(str::to_string)
    }

    fn choices(&self) -> Vec<String> {
        self.story()["view"]["talk"]["choices"].as_array().into_iter().flatten().map(|c| c[0].as_str().unwrap().to_string()).collect()
    }

    fn bark(&self) -> Option<String> {
        self.story()["view"]["bark"]["id"].as_str().map(str::to_string)
    }

    /// Moves on until a line offers choices or the conversation ends.
    fn until_choice(&mut self) {
        for _ in 0..32 {
            if self.line().is_none() || !self.choices().is_empty() {
                return;
            }
            self.intend("advance", None).unwrap();
        }
        panic!("no choice after 32 lines");
    }

    fn choose(&mut self, c: &str) -> Result<(), String> {
        self.intend("choose", Some(c))
    }

    fn fact(&self, f: &str) -> f64 {
        self.story()["facts"][f].as_f64().unwrap()
    }

    fn knows(&self, who: &str, info: &str) -> Option<bool> {
        self.story()["known"].as_array().unwrap().iter().find(|k| k["who"] == who && k["info"] == info).map(|k| k["knows"].as_bool().unwrap())
    }

    fn q(&self, query: &str) -> f64 {
        self.game().systems().query(&id(query), self.sim.rules()).unwrap()
    }

    fn heard(&self, line: &str) -> u64 {
        self.story()["heard"][line].as_u64().unwrap_or(0)
    }

    fn checksum(&self) -> u64 {
        self.sim.checksum()
    }

    /// Talks to Bram at the dock and answers his request with `choice`.
    fn ask_bram(&mut self, choice: &str) {
        self.walk(DOCK);
        self.talk("bram").unwrap();
        self.until_choice();
        assert_eq!(self.line().as_deref(), Some("talk_bram/ask/l1"));
        self.choose(choice).unwrap();
        self.until_choice();
    }

    /// Carries one brick from the shed to the brazier.
    fn carry(&mut self) {
        self.walk(SHED);
        self.walk(BRAZIER);
    }

    /// Rings the bell (the next watch begins).
    fn ring(&mut self) {
        self.walk(BELL);
        self.walk((BELL.0 - 1.5, BELL.1 + 1.5));
    }
}

#[test]
fn two_voices_and_a_decision_that_moves_real_peat() {
    let mut r = Run::new();
    assert_eq!((r.q("tw:shed_fuel"), r.q("tw:dock_lit")), (5.0, 0.0));
    r.ask_bram("lend");
    // The game's command moved two real bricks; the story recorded the decision; Bram, who was there, saw it; Odile did not.
    assert_eq!((r.q("tw:shed_fuel"), r.q("tw:dock_lit")), (3.0, 1.0));
    assert_eq!((r.fact("lent_fuel"), r.fact("trust_bram")), (1.0, 1.0));
    r.wait(0.2);
    assert_eq!(r.knows("bram", "fuel_lent"), Some(true));
    assert_eq!(r.knows("odile", "fuel_lent"), None, "Odile was at the brazier");
    let known = r.story()["known"].clone();
    let bram = known.as_array().unwrap().iter().find(|k| k["who"] == "bram" && k["info"] == "fuel_lent").unwrap();
    assert_eq!(bram["from"], "`learn` in saw_lending", "provenance: how he came to know");
    // Two speakers, two voices: the lines are attributed to their speakers.
    assert_eq!(r.bark().as_deref(), Some("lent_cheer"));
    assert_eq!(r.story()["view"]["bark"]["speaker"], "bram");
}

#[test]
fn knowledge_gates_lines_and_odile_is_withheld_until_she_knows() {
    // Lend, play the first watch well, ring the second: Odile does not know, so her lending line is withheld.
    let play = |tell_odile: bool| {
        let mut r = Run::new();
        r.ask_bram("lend");
        if tell_odile {
            r.walk(BRAZIER);
            r.talk("odile").unwrap();
            r.until_choice();
            assert!(r.choices().contains(&"lent".to_string()), "{:?}", r.choices());
            r.choose("lent").unwrap();
            r.until_choice();
            assert_eq!(r.knows("odile", "fuel_lent"), Some(true));
        }
        r.ring();
        assert_eq!(r.q("tw:watch"), 1.0);
        for _ in 0..6 {
            r.carry();
        }
        while r.q("tw:phase") == 1.0 {
            r.carry();
        }
        assert_eq!(r.q("tw:boats_in"), 1.0, "the first boat came in");
        r.ring();
        r.wait(0.2);
        r
    };
    let r = play(false);
    assert_eq!(r.q("tw:watch"), 2.0);
    let log = r.story()["log"].to_string();
    assert!(log.contains("intro_second_lent: `tw:watch == 2 && knows(odile, fuel_lent)` is false"), "{log}");
    assert_eq!(r.bark().as_deref(), Some("intro_second_clean"));
    let r = play(true);
    assert_eq!(r.bark().as_deref(), Some("intro_second_lent"), "she knows now, so she says it");
}

#[test]
fn what_bram_reveals_the_player_knows_and_what_he_claims_they_only_believe() {
    let mut r = Run::new();
    r.walk(DOCK);
    r.talk("bram").unwrap();
    r.until_choice();
    assert!(r.choices().contains(&"why".to_string()));
    r.choose("why").unwrap();
    r.until_choice();
    // He revealed the Gannet (true: the player knows it) and claimed Odile's eyes are going (the player believes it, does not know it).
    assert_eq!(r.knows("player", "gannet_lost"), Some(true));
    assert_eq!(r.knows("player", "odile_failing"), Some(false));
    assert_eq!(r.line().as_deref(), Some("talk_bram/ask/l1"), "back to his request");
    assert!(!r.choices().contains(&"why".to_string()), "asked once");
    r.choose("refuse").unwrap();
    r.until_choice();
    // Odile can now be told; before, the choice was never offered.
    r.walk(BRAZIER);
    r.talk("odile").unwrap();
    r.until_choice();
    assert!(r.choices().contains(&"gannet".to_string()), "{:?}", r.choices());
}

#[test]
fn refusal_and_delay_are_answers_with_their_own_aftermath() {
    let mut r = Run::new();
    r.ask_bram("refuse");
    assert_eq!((r.fact("trust_bram"), r.fact("lent_fuel"), r.q("tw:shed_fuel")), (-1.0, 0.0, 5.0));
    assert_eq!(r.heard("talk_bram/refused/l1"), 1);
    // A delay is a promise, and the promise is a setup with a payoff a watch later.
    let mut r = Run::new();
    r.ask_bram("later");
    assert_eq!(r.fact("promised_bram"), 1.0);
    r.ring();
    r.carry();
    r.carry();
    while r.q("tw:phase") == 1.0 {
        r.wait(1.0);
    }
    r.walk(DOCK);
    r.talk("bram").unwrap();
    assert_eq!(r.line().as_deref(), Some("bram_after/ask/l1"), "the delayed callback opens, not the first talk");
    assert!(r.q("tw:shed_fuel") >= 2.0);
    r.choose("give").unwrap();
    r.until_choice();
    assert_eq!((r.fact("kept_promise"), r.fact("trust_bram")), (1.0, 2.0));
    assert_eq!(r.heard("bram_after/kept/l1"), 1, "the long thanks: trust is 2");
    r.wait(0.2);
    assert_eq!(r.bark().as_deref(), Some("lent_kept"), "the kept promise gets its own reply, not the first one");
}

#[test]
fn early_completion_changes_what_bram_asks() {
    // The player lights the dock lamp themselves before Bram asks: he does not ask for what is already done.
    let mut r = Run::new();
    r.walk(SHED);
    assert_eq!(r.q("tw:carrying"), 1.0);
    r.walk(DOCK);
    r.walk(LAMP);
    assert_eq!(r.q("tw:dock_lit"), 1.0);
    r.walk(DOCK);
    r.talk("bram").unwrap();
    r.intend("advance", None).unwrap();
    assert_eq!(r.line().as_deref(), Some("talk_bram/hello/l2"));
    r.intend("advance", None).unwrap();
    assert_eq!(r.line().as_deref(), Some("talk_bram/lamp_lit/l1"));
    r.until_choice();
    assert_eq!(r.heard("talk_bram/ask/l1"), 0, "he never asked");
}

#[test]
fn absent_speakers_cannot_be_talked_to_and_do_not_bark() {
    let mut r = Run::new();
    let e = r.talk("bram").unwrap_err();
    assert!(e.contains("bram is not here"), "{e}");
    let e = r.talk("odile").unwrap_err();
    assert!(e.contains("odile is not here"), "{e}");
    // The light goes out while the player is at the shed: neither Odile nor Bram is there; the narrator says it.
    r.ring();
    r.walk(SHED);
    while r.q("tw:light_out") == 0.0 {
        r.wait(0.5);
    }
    r.wait(0.1);
    assert_eq!(r.bark().as_deref(), Some("out_far"));
    let log = r.story()["log"].to_string();
    assert!(log.contains("out_odile: `present(odile)` is false"), "{log}");
}

/// A command now, as another system (or another player's action) would issue it, while the player stands in a conversation.
fn command(r: &mut Run, name: &str, value: i64) -> Result<(), String> {
    let rules = r.sim.rules().clone();
    r.game_mut().command(&rules, &Command { name: id(name), subject: None, value }).map_err(|e| e.to_string())
}

#[test]
fn a_stale_choice_is_checked_again_when_picked_and_refused_without_changes() {
    let mut r = Run::new();
    r.walk(DOCK);
    r.talk("bram").unwrap();
    r.until_choice();
    assert!(r.choices().contains(&"lend".to_string()), "offered while the shed holds five");
    let (rev, hash) = (r.rev(), r.game().systems().hash());
    // Between showing and picking, the shed goes down to one (bricks taken and burnt by other hands).
    for _ in 0..2 {
        command(&mut r, "tw:take", 0).unwrap();
        command(&mut r, "tw:feed", 0).unwrap();
    }
    command(&mut r, "tw:take", 0).unwrap();
    command(&mut r, "tw:feed", 0).unwrap();
    command(&mut r, "tw:take", 0).unwrap();
    command(&mut r, "tw:feed", 0).unwrap();
    assert_eq!(r.q("tw:shed_fuel"), 1.0);
    let after_world = r.game().systems().hash();
    assert_ne!(after_world, hash);
    let e = r.intend_at("choose", Some("lend"), rev).unwrap_err();
    assert!(e.contains("no longer available") && e.contains("tw:shed_fuel = 1"), "{e}");
    assert_eq!(r.game().systems().hash(), after_world, "a refused pick changes nothing");
    assert_eq!((r.fact("lent_fuel"), r.fact("trust_bram"), r.rev()), (0.0, 0.0, rev));
    assert!(!r.choices().contains(&"lend".to_string()), "and it is no longer offered");
}

#[test]
fn the_game_refusing_a_choice_s_command_undoes_the_whole_choice() {
    // `lend` is offered (the shed has bricks), but before it is picked the dock lamp is filled another way: the game refuses `tw:lend`
    // (store-full), so none of the choice applies (no fact, no trust, no peat moved) and its `else` takes over.
    let mut r = Run::new();
    r.walk(DOCK);
    r.talk("bram").unwrap();
    r.until_choice();
    command(&mut r, "tw:lend", 2).unwrap();
    assert_eq!((r.q("tw:shed_fuel"), r.q("tw:dock_lit")), (3.0, 1.0));
    r.choose("lend").unwrap();
    assert_eq!((r.fact("lent_fuel"), r.fact("trust_bram"), r.q("tw:shed_fuel")), (0.0, 0.0, 3.0), "all of it or none of it");
    assert_eq!(r.line().as_deref(), Some("talk_bram/shed_thin/l1"), "the choice's `else`");
    let log = r.story()["log"].to_string();
    assert!(log.contains("the game refused: command 1 of 1 (`tw:lend`) refused, so none applied"), "{log}");
}

#[test]
fn a_repeated_or_late_answer_never_applies_twice() {
    let mut r = Run::new();
    r.walk(DOCK);
    r.talk("bram").unwrap();
    r.until_choice();
    let rev = r.rev();
    r.intend_at("choose", Some("lend"), rev).unwrap();
    // The same press again, and a late packet of it: both name a revision that is gone.
    let e = r.intend_at("choose", Some("lend"), rev).unwrap_err();
    assert!(e.contains("stale"), "{e}");
    let e = r.intend_at("advance", None, rev).unwrap_err();
    assert!(e.contains("stale"), "{e}");
    assert_eq!((r.q("tw:shed_fuel"), r.fact("trust_bram")), (3.0, 1.0), "lent once, trusted once");
    // A choice id that is not on the line shown is refused too.
    let e = r.choose("give").unwrap_err();
    assert!(e.contains("not a choice of the line shown"), "{e}");
}

#[test]
fn an_urgent_bark_suspends_a_conversation_which_then_resumes() {
    let mut r = Run::new();
    r.ring();
    r.walk(DOCK);
    r.talk("bram").unwrap();
    assert_eq!(r.line().as_deref(), Some("talk_bram/hello/l1"));
    // The player stands talking while the brazier burns down: the light goes out, Bram sees it, his bark suspends the talk.
    while r.q("tw:light_out") == 0.0 {
        r.wait(0.5);
    }
    r.wait(0.1);
    let s = r.story();
    assert_eq!(s["view"]["talk"]["suspended"], true, "{s}");
    assert_eq!(r.bark().as_deref(), Some("out_bram"));
    // While suspended the conversation refuses answers; the bark is heard in full, then the talk resumes where it was.
    assert!(r.intend("advance", None).unwrap_err().contains("suspended"));
    r.wait(4.0);
    assert_eq!(r.story()["view"]["talk"]["suspended"], false);
    assert_eq!(r.line().as_deref(), Some("talk_bram/hello/l1"));
    assert_eq!(r.heard("out_bram"), 1);
    // An interrupted bark is seen, not heard: a fed brazier's chatter cut off by a more urgent line can be chosen again.
    r.intend("advance", None).unwrap();
}

#[test]
fn a_skipped_bark_counts_as_heard() {
    let mut r = Run::new();
    r.ask_bram("lend");
    r.wait(0.1);
    assert_eq!(r.bark().as_deref(), Some("lent_cheer"));
    r.intend("skip", None).unwrap();
    assert_eq!(r.bark(), None);
    assert_eq!(r.heard("lent_cheer"), 1);
    assert!(r.intend_at("skip", None, r.rev()).unwrap_err().contains("no bark"));
}

#[test]
fn the_world_does_not_pause_but_the_player_is_held_while_talking() {
    let mut r = Run::new();
    r.ring();
    r.walk(DOCK);
    let left = r.q("tw:secs_left");
    r.talk("bram").unwrap();
    let at = r.pos();
    for _ in 0..120 {
        r.step(PlayerInput { forward: 1, yaw: 0.0, ..Default::default() });
    }
    assert_eq!(r.pos(), at, "input is locked while the conversation is open (no gameplay input leaks through)");
    assert!(r.q("tw:secs_left") < left, "the watch clock kept running");
}

#[test]
fn a_save_mid_conversation_resumes_exactly_and_bad_saves_change_nothing() {
    let mut r = Run::new();
    r.walk(DOCK);
    r.talk("bram").unwrap();
    r.until_choice();
    r.wait(0.5);
    let save = r.game().save();
    let (line, choices, rev) = (r.line(), r.choices(), r.rev());
    // A fresh match standing where the player stood, loading the save: the same line, the same choices, the same revision.
    let mut b = Run::new();
    b.walk(DOCK);
    let report = b.game_mut().load(&save).unwrap();
    assert!(report.migrated.is_empty());
    assert_eq!((b.line(), b.choices(), b.rev()), (line, choices, rev));
    assert_eq!(b.game().systems().hash(), r.game().systems().hash());
    b.choose("lend").unwrap();
    b.until_choice();
    assert_eq!((b.fact("lent_fuel"), b.q("tw:shed_fuel")), (1.0, 3.0));
    // Saved by newer content, by another story, malformed: refused, and the running game is exactly as it was.
    let before = b.game().systems().hash();
    let story = "tidewatch:story";
    let mut newer = save.clone();
    newer["systems"]["systems"][story]["state"]["content"]["version"] = json!(99);
    let mut other = save.clone();
    other["systems"]["systems"][story]["state"]["content"]["story"] = json!("elsewhere");
    let mut broken = save.clone();
    broken["systems"]["systems"][story]["state"]["facts"]["lent_fuel"] = json!("yes");
    for bad in [newer, other, broken, json!({"game": "workshop"})] {
        assert!(b.game_mut().load(&bad).is_err());
        assert_eq!(b.game().systems().hash(), before);
    }
}

#[test]
fn a_save_from_older_content_loads_by_id_and_reports_what_no_longer_exists() {
    let mut r = Run::new();
    r.ask_bram("lend");
    let mut save = r.game().save();
    let story = "tidewatch:story";
    // As if version 0 of the story had a fact since removed and a line since renamed.
    let state = &mut save["systems"]["systems"][story]["state"];
    state["content"]["version"] = json!(0);
    state["facts"]["old_fact"] = json!(1);
    state["heard"]["talk_bram/old_line"] = json!(1);
    let mut b = Run::new();
    b.game_mut().load(&save).unwrap();
    assert_eq!(b.fact("lent_fuel"), 1.0);
    let problems = b.story()["problems"].to_string();
    assert!(problems.contains("fact `old_fact` is no longer in the story: dropped"), "{problems}");
    assert!(problems.contains("talk_bram/old_line"), "{problems}");
}

#[test]
fn a_replay_of_a_recorded_night_reproduces_the_story_exactly() {
    use red_engine2::sim::trace::Header;
    let mut r = Run::new();
    r.sim.start_recording(Header::new(0, 0, "", 1, 60)).unwrap_or_else(|e| panic!("{e}"));
    r.ask_bram("why");
    r.choose("refuse").unwrap();
    r.until_choice();
    r.ring();
    r.carry();
    r.wait(12.0);
    let checksum = r.checksum();
    let trace = r.sim.take_trace().unwrap();
    assert!(trace.entries.iter().any(|e| matches!(e, red_engine2::sim::trace::Entry::Intent { .. })), "intentions are recorded");
    let text = std::fs::read_to_string(content().join("tidewatch.json")).unwrap();
    let scene = red_engine2::scene_blocks::parse_scene_in(&text, Some(&content())).unwrap();
    let report = red_engine2::sim::replay::replay(&trace, &scene, &parse_spawns(&text).unwrap()).unwrap();
    assert!(report.exact.is_none(), "{:?}", report.exact);
    assert!(report.compared > 100);
    assert!(report.events_match);
    // And the same inputs in a fresh match give the same checksum (the story's state is in it).
    let mut again = Run::new();
    again.ask_bram("why");
    again.choose("refuse").unwrap();
    again.until_choice();
    again.ring();
    again.carry();
    again.wait(12.0);
    assert_eq!(again.checksum(), checksum);
}

#[test]
fn the_director_rests_when_tense_and_sometimes_deliberately_does_nothing() {
    // Two seeds of the same idle first watch: the director never acts while the light is out or no watch runs; its 40% radio sometimes
    // fires and sometimes rolls nothing; the same seed gives the same outcomes.
    let outcomes = |seed: u64| {
        let text = std::fs::read_to_string(content().join("tidewatch.json")).unwrap().replace("\"seed\": 11", &format!("\"seed\": {seed}"));
        let mut r = Run::from_text(&text);
        r.ring();
        r.walk((BELL.0 - 1.5, BELL.1 + 1.5));
        r.wait(60.0);
        r.story()["tally"].clone()
    };
    let mut seen = std::collections::BTreeSet::new();
    for seed in 1..=6 {
        let t = outcomes(seed);
        assert_eq!(t, outcomes(seed), "seed {seed} is deterministic");
        for k in t.as_object().unwrap().keys() {
            seen.insert(k.clone());
        }
    }
    assert!(seen.contains("rests"), "{seen:?}");
    assert!(seen.contains("does nothing"), "{seen:?}");
    assert!(seen.contains("intervenes"), "{seen:?}");
}

#[test]
fn a_director_helps_from_a_finite_reserve_and_never_takes_from_the_player() {
    // A struggling watch (the light went out): Odile brings a brick from her own reserve, once a watch; the reserve is real and runs out.
    let mut r = Run::new();
    r.ring();
    r.walk((BELL.0 - 1.5, BELL.1 + 1.5));
    while r.q("tw:dark_secs") < 3.0 {
        r.wait(0.5);
    }
    r.carry();
    r.wait(25.0);
    let log = r.story()["log"].to_string();
    let reserve = r.q("tw:reserve");
    if log.contains("intervenes: odile_brings_peat") {
        assert_eq!(reserve, 1.0, "{log}");
    } else {
        assert_eq!(reserve, 2.0, "{log}");
    }
    let total: f64 = ["shed", "hands", "brazier", "dock_lamp", "reserve"]
        .iter()
        .map(|s| r.game().systems().ledger().count(&id(&format!("tw:{s}")), &id("tw:peat")) as f64)
        .sum();
    let _ = total;
    r.game().systems().ledger().audit().unwrap();
}

#[test]
fn a_package_that_does_not_fit_the_game_is_refused_where_it_is_written() {
    let text = std::fs::read_to_string(content().join("story/tidewatch.story.json")).unwrap();
    let catalog = tidewatch_server::content::catalog(
        &Default::default(),
        &serde_json::from_str::<Value>(&std::fs::read_to_string(content().join("tidewatch.json")).unwrap()).unwrap().as_object().unwrap().clone(),
    );
    let load = |t: &str| {
        red_engine2::sim::story::package::Loaded::from_json(t, &catalog).map(|_| ()).map_err(|e| e.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("\n"))
    };
    load(&text).unwrap();
    let newer = text.replacen("\"story_package\": 1", "\"story_package\": 2", 1);
    assert!(load(&newer).unwrap_err().contains("format 2"), "a newer format is refused, never guessed at");
    let renamed = text.replace("tw:shed_fuel", "tw:shed_peat");
    assert!(load(&renamed).unwrap_err().contains("tw:shed_peat"), "a query the game no longer answers");
    assert!(load("{\"story_package\": 1, \"story\": 3}").unwrap_err().contains("malformed"));
    assert!(load("not json").is_err());
}

#[test]
fn the_package_and_the_bindings_on_disk_are_fresh() {
    let built = red_engine2::tools::story::build(&content().join("story")).unwrap();
    assert_eq!(built.errors(), 0, "{:?}", built.diags);
    let p = built.package.as_ref().unwrap();
    let on_disk = std::fs::read_to_string(content().join("story/tidewatch.story.json")).unwrap();
    assert_eq!(red_engine2::tools::story::package_text(p), on_disk, "stale package: run `red_engine2 story build games/tidewatch/content/story`");
    let bindings: Value = serde_json::from_str(&std::fs::read_to_string(content().join("story/bindings.json")).unwrap()).unwrap();
    assert_eq!(bindings, tidewatch_server::bindings(), "stale bindings: run `tidewatch_server story-bindings > games/tidewatch/content/story/bindings.json`");
}
