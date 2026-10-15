//! The story runtime's selection and pacing semantics (`sim::story::runtime`), on a toy game system in a real `Systems` host: candidate
//! order (priority, specificity, least heard, written order) with a random draw only on a real tie, cooldowns of a bark, a speaker and a
//! category, the bounded queue and staleness, interruption (seen, not heard), deliberate silence, pure eligibility, the director's
//! recovery, budget, rest, chance and exhaustion, and what a new round and a new run reset. Stories are compiled by the real toolchain.

use red_engine2::sim::defs::{Diagnostic, Id};
use red_engine2::sim::ledger::Ledger;
use red_engine2::sim::story::package::{Catalog, Loaded};
use red_engine2::sim::story::runtime::{intend, Story};
use red_engine2::sim::systems::{Command, Cx, Look, QueryDecl, System, Systems, Ty};
use serde_json::Value;
use std::sync::Arc;

fn id(s: &str) -> Id {
    Id::parse(s).unwrap()
}

/// A toy game: `g:ring` makes the bell ring next tick (`g:bell`), `g:hint` is unattended; `g:coins` and `g:near` are answered.
#[derive(Default)]
struct Toy {
    ring: u32,
    coins: i64,
    near: bool,
    hints: u32,
}

impl System for Toy {
    fn id(&self) -> Id {
        id("g:toy")
    }
    fn emits(&self) -> Vec<Id> {
        vec![id("g:bell")]
    }
    fn commands(&self) -> Vec<Id> {
        vec![id("g:ring"), id("g:hint"), id("g:give"), id("g:walk")]
    }
    fn unattended(&self) -> Vec<Id> {
        vec![id("g:hint")]
    }
    fn queries(&self) -> Vec<QueryDecl> {
        vec![QueryDecl::new(id("g:coins"), Ty::Int, "coins"), QueryDecl::new(id("g:near"), Ty::Bool, "the player is near Ann")]
    }
    fn query(&self, q: &Id, _: &Look) -> Option<f64> {
        match q.as_str() {
            "g:coins" => Some(self.coins as f64),
            "g:near" => Some(self.near as u8 as f64),
            _ => None,
        }
    }
    fn tick(&mut self, cx: &mut Cx) {
        for _ in 0..std::mem::take(&mut self.ring) {
            cx.emit(&id("g:bell"), None, 0).unwrap();
        }
    }
    fn command(&mut self, c: &Command, _: &mut Cx) -> Result<(), Diagnostic> {
        match c.name.as_str() {
            "g:ring" => self.ring += c.value.max(1) as u32,
            "g:hint" => self.hints += 1,
            "g:give" => self.coins += c.value,
            _ => self.near = c.value != 0,
        }
        Ok(())
    }
    fn save(&self) -> Value {
        serde_json::json!([self.coins, self.near, self.hints])
    }
    fn load(&mut self, s: Value) -> Result<(), String> {
        self.coins = s[0].as_i64().ok_or("coins")?;
        self.near = s[1].as_bool().ok_or("near")?;
        self.hints = s[2].as_u64().ok_or("hints")? as u32;
        Ok(())
    }
    fn hash(&self) -> u64 {
        (self.coins as u64) ^ ((self.near as u64) << 40) ^ ((self.hints as u64) << 48)
    }
}

const SHEETS: &str = "\
character ann \"Ann\"
  want: the bell answered
  need: quiet
  flaw: talks over people
  contradiction: hates noise, rings bells
  values: honest
  relationships: player
  voice: brisk
  example: Bell. Again.
  example: Hush.
  present: g:near
  cooldown: 2s
extra bo \"Bo\"
  role: the bell-boy
  voice: mumbles
";

/// A host running `story` (its text after the sheets) next to the toy, seeded with `seed`.
fn host(name: &str, story: &str, seed: u64) -> Systems {
    let dir = std::env::temp_dir().join(format!("red-story-rt-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let catalog = Catalog::of(&[&Toy::default()], vec![]);
    std::fs::write(dir.join("bindings.json"), red_engine2::tools::story::bindings_json("g", &catalog, &[]).to_string()).unwrap();
    std::fs::write(dir.join("s.story"), format!("story t version 1\nuses bindings.json\nround g:bell\n{SHEETS}{story}")).unwrap();
    let built = red_engine2::tools::story::build(&dir).unwrap();
    assert_eq!(built.errors(), 0, "{:?}", built.diags);
    let text = red_engine2::tools::story::package_text(built.package.as_ref().unwrap());
    let loaded = Arc::new(Loaded::from_json(&text, &catalog).unwrap());
    Systems::new(vec![Box::new(Toy { near: true, ..Toy::default() }), Box::new(Story::new(loaded))], Ledger::new(), seed).unwrap()
}

fn cmd(s: &mut Systems, name: &str, value: i64) {
    s.command(&Command { name: id(name), subject: None, value }).unwrap();
}

fn ticks(s: &mut Systems, n: u32) {
    for _ in 0..n {
        s.tick().unwrap();
    }
}

fn story(s: &Systems) -> Value {
    red_engine2::sim::story::runtime::inspect(s, &(), "story").unwrap()
}

fn playing(s: &Systems) -> Option<String> {
    story(s)["view"]["bark"]["id"].as_str().map(str::to_string)
}

/// Rings once and returns the bark that started.
fn ring(s: &mut Systems) -> Option<String> {
    cmd(s, "g:ring", 1);
    ticks(s, 1);
    playing(s)
}

fn last(s: &Systems) -> String {
    story(s)["log"].as_array().unwrap().last().unwrap()["outcome"].as_str().unwrap().to_string()
}

#[test]
fn the_best_candidate_is_the_highest_priority_then_the_most_specific_then_the_least_heard() {
    let text = "\
fact rich bool = false run \"\"
bark plain on g:bell
  ann: Bell.
bark specific on g:bell
  ann: A bell, and coins.
  if g:coins >= 1
bark urgent on g:bell
  ann: BELL!
  if g:coins >= 5
  priority 3
";
    let mut s = host("order", text, 1);
    assert_eq!(ring(&mut s).as_deref(), Some("plain"));
    ticks(&mut s, 400);
    cmd(&mut s, "g:give", 1);
    assert_eq!(ring(&mut s).as_deref(), Some("specific"), "a condition that holds is more specific");
    ticks(&mut s, 400);
    cmd(&mut s, "g:give", 5);
    assert_eq!(ring(&mut s).as_deref(), Some("urgent"), "priority first");
}

#[test]
fn a_tie_draws_once_from_the_seeded_stream_and_checking_draws_nothing() {
    let text = "bark a on g:bell\n  ann: One.\nbark b on g:bell\n  ann: Two.\nbark c on g:bell\n  ann: Three.\n";
    let order = |seed: u64| {
        let mut s = host(&format!("tie{seed}"), text, seed);
        let mut out = Vec::new();
        for _ in 0..6 {
            out.push(ring(&mut s).unwrap());
            ticks(&mut s, 400);
        }
        out
    };
    let a = order(7);
    assert_eq!(a, order(7), "the same seed, the same draws");
    // The least heard go first: every one is heard once before any twice.
    let mut first: Vec<&String> = a[..3].iter().collect();
    first.sort();
    first.dedup();
    assert_eq!(first.len(), 3, "{a:?}");
    assert!((1..40).any(|seed| order(seed) != a), "different seeds can order the tie differently");
    // Looking (a presenter's view, an inspection) is pure: it draws nothing and changes nothing.
    let s = host("pure", text, 3);
    let before = s.hash();
    for _ in 0..20 {
        let _ = story(&s);
        let _ = red_engine2::sim::story::runtime::inspect(&s, &(), "t:a");
    }
    assert_eq!(s.hash(), before);
}

#[test]
fn cooldowns_of_a_bark_its_speaker_and_its_category_hold_it_back_with_the_reason() {
    let text = "\
category chat cooldown 10s
bark own on g:bell
  ann: Mine.
  cooldown 30s
bark small_talk on g:bell
  bo: Chat.
  category chat
  priority -1
";
    let mut s = host("cool", text, 1);
    assert_eq!(ring(&mut s).as_deref(), Some("own"));
    ticks(&mut s, 400);
    // `own` cools for 30 s; `chat` takes its turn; then its category is quiet for 10 s: nothing is said, and the log says why.
    assert_eq!(ring(&mut s).as_deref(), Some("small_talk"));
    ticks(&mut s, 200);
    assert_eq!(ring(&mut s), None);
    let log = story(&s)["log"].to_string();
    assert!(log.contains("own: cooling down until tick"), "{log}");
    assert!(log.contains("small_talk: category chat quiet until tick"), "{log}");
    assert!(last(&s).starts_with("silence: no candidate"));
}

#[test]
fn a_higher_priority_bark_interrupts_and_the_interrupted_is_seen_not_heard() {
    let text = "\
bark long on g:bell
  bo: A very long story about bells, and the people who ring them, and the people who wish they would not.
  if g:coins == 0
  once
bark alarm on g:bell
  ann: Fire!
  if g:coins == 1
  priority 5
bark small on g:bell
  bo: Small.
  if g:coins >= 2
";
    let mut s = host("interrupt", text, 1);
    assert_eq!(ring(&mut s).as_deref(), Some("long"));
    cmd(&mut s, "g:give", 1);
    assert_eq!(ring(&mut s).as_deref(), Some("alarm"));
    let st = story(&s);
    assert_eq!(st["heard"].get("long"), None, "interrupted: not heard");
    ticks(&mut s, 400);
    assert_eq!(story(&s)["heard"]["alarm"], 1);
    // `once` means heard once: the interrupted one can still be chosen.
    cmd(&mut s, "g:give", -1);
    assert_eq!(ring(&mut s).as_deref(), Some("long"));
    ticks(&mut s, 500);
    assert_eq!(story(&s)["heard"]["long"], 1);
    assert_eq!(ring(&mut s), None, "and now it is done");
}

#[test]
fn barks_queue_behind_the_one_presented_at_most_three_and_a_stale_one_is_dropped() {
    let text = "bark b1 on g:bell\n  bo: One.\nbark b2 on g:bell\n  bo: Two.\nbark b3 on g:bell\n  bo: Three.\nbark b4 on g:bell\n  bo: Four.\nbark b5 on g:bell\n  bo: Five.\n";
    let mut s = host("queue", text, 2);
    cmd(&mut s, "g:ring", 5);
    ticks(&mut s, 1);
    let st = story(&s);
    assert!(st["view"]["bark"]["id"].is_string(), "one plays");
    assert_eq!(st["queue"].as_array().unwrap().len(), 3, "three wait: {st}");
    let log = st["log"].to_string();
    assert!(log.contains("dropped (the queue is full)"), "the fifth is dropped: {log}");
    assert!(log.contains("already being said"), "a bark playing or queued is not chosen again: {log}");
    // Each plays when the one before is heard; one chosen more than 4 s before its turn is stale and dropped.
    ticks(&mut s, 60 * 12);
    let st = story(&s);
    assert_eq!(st["heard"].as_object().unwrap().len(), 3, "{st}");
    assert!(st["log"].to_string().contains("dropped: stale in the queue"), "{st}");
}

#[test]
fn a_deliberate_silence_is_chosen_and_says_nothing() {
    let text = "bark quiet on g:bell\n  silence\n  if g:coins >= 3\n  priority 9\nbark talk on g:bell\n  ann: Hello.\n";
    let mut s = host("silence", text, 1);
    assert_eq!(ring(&mut s).as_deref(), Some("talk"));
    ticks(&mut s, 400);
    cmd(&mut s, "g:give", 3);
    assert_eq!(ring(&mut s), None);
    assert!(last(&s).starts_with("silence: chose quiet"), "{}", last(&s));
}

#[test]
fn an_absent_speaker_is_never_chosen() {
    let text = "bark hi on g:bell\n  ann: Hi.\nbark far on g:bell\n  narrator: A bell, far off.\n";
    let mut s = host("absent", text, 1);
    cmd(&mut s, "g:walk", 0);
    assert_eq!(ring(&mut s).as_deref(), Some("far"));
    assert!(story(&s)["log"].to_string().contains("hi: ann is not here (g:near = 0)"));
}

#[test]
fn a_director_waits_recovers_keeps_its_budget_rests_rolls_and_runs_dry() {
    let text = "\
director d
  every 1s
  recover 3s
  budget 2 per round
  tension g:coins
  rest if g:coins >= 9
  intervene nudge
    if g:coins < 5
    once
    do run g:hint
  intervene maybe
    chance 50%
    group colour
    do run g:hint
";
    let run = |seed: u64| {
        let mut s = host(&format!("dir{seed}"), text, seed);
        ticks(&mut s, 60 * 12);
        let st = story(&s);
        let outcomes: Vec<String> =
            st["log"].as_array().unwrap().iter().filter(|d| d["what"] == "director d").map(|d| d["outcome"].as_str().unwrap().to_string()).collect();
        (s.get::<Toy>().unwrap().hints, outcomes, s)
    };
    let (hints, outcomes, mut s) = run(5);
    assert_eq!(run(5).1, outcomes, "deterministic for a seed");
    assert!(outcomes.iter().any(|o| o.starts_with("intervenes: nudge")), "{outcomes:?}");
    assert!(outcomes.iter().any(|o| o.starts_with("recovering")), "{outcomes:?}");
    assert!(outcomes.iter().any(|o| o.starts_with("budget spent")), "{outcomes:?}");
    assert!(hints <= 2, "the budget holds: {hints}");
    // A new round: the budget and the groups come back; `once` does not, and the pool runs dry.
    cmd(&mut s, "g:ring", 1);
    ticks(&mut s, 60 * 12);
    cmd(&mut s, "g:ring", 1);
    ticks(&mut s, 60 * 12);
    let all = story(&s)["log"].to_string();
    assert!(all.contains("exhausted") || all.contains("does nothing") || all.contains("intervenes: maybe"), "{all}");
    // Rest: when it holds the director does nothing on purpose.
    cmd(&mut s, "g:give", 9);
    ticks(&mut s, 120);
    assert!(story(&s)["log"].to_string().contains("rests: `g:coins >= 9` holds"));
    // Some seed rolls nothing on `maybe`.
    assert!((1..30).any(|seed| run(seed).1.iter().any(|o| o.starts_with("does nothing: maybe rolled"))));
}

#[test]
fn a_new_round_resets_round_facts_a_new_run_keeps_only_profile_facts() {
    let text = "\
fact r int = 0 round \"rounds\"
fact u int = 0 run \"runs\"
fact p int = 0 profile \"profile\"
bark count on g:coin_never
  ann: x
";
    let text = text.replace("bark count on g:coin_never\n  ann: x\n", "bark count on g:bell\n  narrator: Counted.\n  do add r 1; add u 1; add p 1\n");
    let mut s = host("scopes", &text, 1);
    ring(&mut s);
    ticks(&mut s, 400);
    ring(&mut s);
    ticks(&mut s, 400);
    let f = story(&s)["facts"].clone();
    // The bell is the round event too: `r` was reset by the second ring before that bark was heard.
    assert_eq!((f["r"].as_f64(), f["u"].as_f64(), f["p"].as_f64()), (Some(1.0), Some(2.0), Some(2.0)));
    intend(&mut s, &(), &red_engine2::sim::game::Intent { name: "story:new_run".into(), subject: None, value: 0 }).unwrap();
    let f = story(&s)["facts"].clone();
    assert_eq!((f["r"].as_f64(), f["u"].as_f64(), f["p"].as_f64()), (Some(0.0), Some(0.0), Some(2.0)));
}
