//! Headless playthroughs and the whole-game verification report.
//!
//! A scenario is a scripted player (hold, press, click, approach a tag, wait until something is true) run against the real simulation at full speed with no window, followed by
//! assertions. [`verify`] runs every scenario and measures every sound and the first frame, and returns rows of `PASS`/`FAIL name detail`. What a green report proves is stated in
//! each row's *claim*: SIMULATION (the rules do what the scenario asserts), RENDER (the frame is not blank), AUDIO-WAVEFORM (the samples are finite, not clipped, not silent). It proves
//! nothing about a browser, a human's ears or whether the game is fun.

use crate::game::*;
use crate::render;
use crate::sim::{Sim, TPS};
use std::collections::BTreeMap;
use std::sync::Arc;

/// What a scenario found.
#[derive(Debug, Clone)]
pub struct ScenarioReport {
    /// Its name.
    pub name: String,
    /// Every assertion held and no step failed.
    pub ok: bool,
    /// What went wrong, one sentence each.
    pub failures: Vec<String>,
    /// Ticks run.
    pub ticks: u64,
    /// The final state hash.
    pub hash: String,
    /// Final variable values (declared variables only).
    pub vars: Vec<(String, f64)>,
    /// How the game ended.
    pub ended: Option<&'static str>,
    /// Sounds played (name, count).
    pub sounds: Vec<(String, u32)>,
    /// Events emitted (name, count).
    pub events: BTreeMap<String, u32>,
    /// Authoring notes from the simulation (rule loops, runaway spawns).
    pub notes: Vec<String>,
}

fn describe_expect(def: &GameDef, e: &Expect) -> String {
    match e {
        Expect::Var(i, c, n) => format!("{} {} {n}", def.var_names[*i], c.name()),
        Expect::Ended(o) => format!("game ended in a {}", o.name()),
        Expect::NotEnded => "game not ended".into(),
        Expect::Count(t, c, n) => format!("count of `{t}` {} {n}", c.name()),
        Expect::Near(id, p, tol) => format!("`{id}` within {tol} px of ({}, {})", p[0], p[1]),
        Expect::Event(n, min, max) => format!("event `{n}` emitted {min}..{}", max.map_or("any".to_string(), |m| m.to_string())),
        Expect::Sound(i, n) => format!("sound `{}` played at least {n} time(s)", def.sounds[*i].name),
        Expect::Hash(h) => format!("state hash {h}"),
    }
}

/// `Ok` when the expectation holds now, else what was actually found.
pub fn check(sim: &Sim, e: &Expect) -> Result<(), String> {
    let def = &sim.def;
    let fail = |actual: String| Err(format!("expected {}, found {actual}", describe_expect(def, e)));
    match e {
        Expect::Var(i, c, n) => {
            let v = sim.vars[*i];
            if c.holds(v, *n) {
                Ok(())
            } else {
                fail(format!("{v}"))
            }
        }
        Expect::Ended(o) => {
            if sim.ended == Some(*o) {
                Ok(())
            } else {
                fail(sim.ended.map_or("it had not ended".to_string(), |x| format!("a {}", x.name())))
            }
        }
        Expect::NotEnded => match sim.ended {
            None => Ok(()),
            Some(o) => fail(format!("a {}", o.name())),
        },
        Expect::Count(t, c, n) => {
            let k = sim.count_tag(t) as f64;
            if c.holds(k, *n) {
                Ok(())
            } else {
                fail(format!("{k}"))
            }
        }
        Expect::Near(id, p, tol) => match sim.by_id(id) {
            Some(en) => {
                let d = ((en.x - p[0]).powi(2) + (en.y - p[1]).powi(2)).sqrt();
                if d <= *tol {
                    Ok(())
                } else {
                    fail(format!("it at ({:.1}, {:.1}), {d:.1} px away", en.x, en.y))
                }
            }
            None => fail("it does not exist (destroyed?)".into()),
        },
        Expect::Event(name, min, max) => {
            let k = sim.event_counts.get(name).copied().unwrap_or(0);
            if k >= *min && max.is_none_or(|m| k <= m) {
                Ok(())
            } else {
                fail(format!("{k} time(s) (events seen: {})", if sim.event_counts.is_empty() { "none".into() } else { sim.event_counts.keys().cloned().collect::<Vec<_>>().join(", ") }))
            }
        }
        Expect::Sound(i, n) => {
            if sim.sound_counts[*i] >= *n {
                Ok(())
            } else {
                fail(format!("{} time(s)", sim.sound_counts[*i]))
            }
        }
        Expect::Hash(h) => {
            if sim.hash_hex() == *h {
                Ok(())
            } else {
                fail(sim.hash_hex())
            }
        }
    }
}

struct Runner<'a> {
    sim: Sim,
    budget: u64,
    stop: Option<u64>,
    failures: Vec<String>,
    sc: &'a Scenario,
}

impl Runner<'_> {
    fn done(&self) -> bool {
        self.stop.is_some_and(|s| self.sim.tick >= s) || !self.failures.is_empty()
    }

    /// One tick, if the budget and the stop allow.
    fn tick(&mut self) -> bool {
        if self.done() {
            return false;
        }
        if self.sim.tick >= self.budget {
            self.failures.push(format!("the scenario ran past max_seconds ({}): raise `max_seconds`, or a step is waiting for something that never happens", self.sc.max_seconds));
            return false;
        }
        self.sim.step();
        true
    }

    fn run(&mut self, secs: f32) {
        let n = (secs * TPS as f32).round() as u64;
        for _ in 0..n {
            if !self.tick() {
                break;
            }
        }
    }

    fn player(&self) -> Option<(f32, f32, bool, KeyMode)> {
        let def = &self.sim.def;
        self.sim.entities.iter().find_map(|e| match def.prefabs[e.prefab].mv {
            Move::Keys { mode, .. } if e.alive => Some((e.x, e.y, e.grounded, mode)),
            _ => None,
        })
    }

    fn approach(&mut self, tag: &str, secs: f32) {
        let n = (secs * TPS as f32).round() as u64;
        for _ in 0..n {
            let Some((px, py, grounded, mode)) = self.player() else {
                self.failures.push("`approach` needs a player: no thing with a `keys` mover exists".into());
                return;
            };
            let def = self.sim.def.clone();
            let mut best: Option<(f32, f32, f32)> = None;
            for e in self.sim.entities.iter().filter(|e| e.alive && def.prefabs[e.prefab].tags.iter().any(|t| t == tag)) {
                let d = (e.x - px).powi(2) + (e.y - py).powi(2);
                if best.is_none_or(|(b, _, _)| d < b) {
                    best = Some((d, e.x, e.y));
                }
            }
            let (l, r, u, d) = match best {
                Some((_, tx, ty)) => {
                    let (dx, dy) = (tx - px, ty - py);
                    match mode {
                        KeyMode::TopDown => (dx < -2.0, dx > 2.0, dy < -2.0, dy > 2.0),
                        KeyMode::Platformer => (dx < -2.0, dx > 2.0, dy < -6.0 && grounded && self.sim.tick % 2 == 0, false),
                    }
                }
                None => (false, false, false, false),
            };
            for (name, on) in [("left", l), ("right", r), ("up", u), ("down", d)] {
                self.sim.set_action(name, on);
            }
            if !self.tick() {
                break;
            }
        }
        for name in ["left", "right", "up", "down"] {
            self.sim.set_action(name, false);
        }
    }

    fn step_script(&mut self, st: &Step) {
        match st {
            Step::Wait(s) => self.run(*s),
            Step::Hold(actions, s) => {
                for a in actions {
                    self.sim.set_action(a, true);
                }
                self.run(*s);
                for a in actions {
                    self.sim.set_action(a, false);
                }
            }
            Step::Press(a) => {
                self.sim.set_action(a, true);
                self.tick();
                self.sim.set_action(a, false);
                self.tick();
            }
            Step::Click(p) => {
                self.sim.click(p[0], p[1]);
                self.tick();
            }
            Step::Point(p) => {
                self.sim.set_pointer(p[0], p[1]);
                self.tick();
            }
            Step::Button(id) => match self.sim.press_button(id) {
                Ok(()) => {
                    self.tick();
                }
                Err(e) => self.failures.push(e),
            },
            Step::Approach(tag, s) => self.approach(tag, *s),
            Step::WaitUntil(e, timeout) => {
                let limit = self.sim.tick + (timeout * TPS as f32).round() as u64;
                loop {
                    if check(&self.sim, e).is_ok() {
                        break;
                    }
                    if self.sim.tick >= limit {
                        let why = check(&self.sim, e).err().unwrap_or_default();
                        self.failures.push(format!("waited {timeout} s and it never held: {why}"));
                        break;
                    }
                    if !self.tick() {
                        break;
                    }
                }
            }
        }
    }
}

/// Runs a scenario. `stop_at` ends the run early at that tick (to take a picture mid-game); expectations are only checked when the whole script ran.
pub fn run_scenario(def: &Arc<GameDef>, sc: &Scenario, stop_at: Option<u64>) -> (ScenarioReport, Sim) {
    let mut r = Runner { sim: Sim::new(def.clone(), sc.seed), budget: (sc.max_seconds * TPS as f32).round() as u64, stop: stop_at, failures: Vec::new(), sc };
    for st in &sc.script {
        if r.done() {
            break;
        }
        r.step_script(st);
    }
    let mut failures = std::mem::take(&mut r.failures);
    if stop_at.is_none() && failures.is_empty() {
        for e in &sc.expect {
            if let Err(why) = check(&r.sim, e) {
                failures.push(why);
            }
        }
    }
    let sim = r.sim;
    let rep = ScenarioReport {
        name: sc.name.clone(),
        ok: failures.is_empty(),
        failures,
        ticks: sim.tick,
        hash: sim.hash_hex(),
        vars: def.vars.iter().enumerate().map(|(i, (n, _))| (n.clone(), sim.vars[i])).collect(),
        ended: sim.ended.map(Outcome::name),
        sounds: def.sounds.iter().enumerate().filter(|(i, _)| sim.sound_counts[*i] > 0).map(|(i, s)| (s.name.clone(), sim.sound_counts[i])).collect(),
        events: sim.event_counts.clone(),
        notes: sim.notes.clone(),
    };
    (rep, sim)
}

/// One line of a verification report.
#[derive(Debug, Clone)]
pub struct Row {
    /// Passed.
    pub ok: bool,
    /// What kind of claim this row makes: `simulation`, `render`, `audio-waveform`, `authoring`.
    pub claim: &'static str,
    /// What was checked.
    pub name: String,
    /// The result in a sentence.
    pub detail: String,
}

impl Row {
    fn new(ok: bool, claim: &'static str, name: impl Into<String>, detail: impl Into<String>) -> Row {
        Row { ok, claim, name: name.into(), detail: detail.into() }
    }
}

/// Everything that can be verified about a game without a browser.
pub fn verify(def: &Arc<GameDef>) -> Vec<Row> {
    let mut rows = Vec::new();
    // The first frame is a picture of something.
    let sim = Sim::new(def.clone(), 1);
    let st = render::render(&sim).stats(def.view.background);
    rows.push(Row::new(
        st.distinct_colors >= 2 && st.covered >= 0.002,
        "render",
        "first frame",
        format!("{}x{}, {} colours, {:.1}% of pixels differ from the background{}", st.width, st.height, st.distinct_colors, st.covered * 100.0, if st.distinct_colors < 2 || st.covered < 0.002 { " — the first frame is blank: nothing is on screen" } else { "" }),
    ));
    // Scenarios.
    if def.scenarios.is_empty() {
        rows.push(Row::new(false, "simulation", "scenarios", "the game has no `checks.scenarios`: nothing proves it can be played (add a scripted playthrough with an `expect`)"));
    }
    for sc in &def.scenarios {
        let (rep, end) = run_scenario(def, sc, None);
        let frame = render::render(&end).stats(def.view.background);
        let detail = if rep.ok {
            format!("{:.1} s of play, ended {}, hash {}", rep.ticks as f64 / TPS as f64, rep.ended.unwrap_or("not"), rep.hash)
        } else {
            rep.failures.join("; ")
        };
        rows.push(Row::new(rep.ok, "simulation", format!("scenario `{}`", sc.name), detail));
        for n in &rep.notes {
            rows.push(Row::new(false, "authoring", format!("scenario `{}` note", sc.name), n.clone()));
        }
        rows.push(Row::new(frame.covered >= 0.002, "render", format!("final frame of `{}`", sc.name), format!("{} colours, {:.1}% covered", frame.distinct_colors, frame.covered * 100.0)));
        // The same scenario twice must end in the same state.
        let (again, _) = run_scenario(def, sc, None);
        rows.push(Row::new(again.hash == rep.hash, "simulation", format!("determinism of `{}`", sc.name), if again.hash == rep.hash { "two runs, one hash".to_string() } else { format!("two runs disagree: {} vs {}", rep.hash, again.hash) }));
    }
    // Sounds named by rules exist (the parser guarantees it); every sound and track is measured.
    for c in crate::sound::check_all(def) {
        rows.push(Row::new(c.problems.is_empty(), "audio-waveform", format!("{} `{}`", c.kind, c.name), if c.problems.is_empty() { format!("{:.2} s, peak {:.1} dBFS, {:.1} LUFS", c.report.secs, c.report.peak_dbfs, c.report.lufs) } else { c.problems.join("; ") }));
    }
    // Saved progress survives a round trip.
    if !def.persist.is_empty() || def.caps.persistence.contains(&crate::caps::Persistence::Settings) {
        let mut a = Sim::new(def.clone(), 1);
        for &i in &def.persist {
            a.vars[i] = def.vars[i].1 + 7.0;
        }
        a.music_on = false;
        let text = a.save_json();
        let mut b = Sim::new(def.clone(), 1);
        let status = b.load_save(&text);
        let same = def.persist.iter().all(|&i| (b.vars[i] - a.vars[i]).abs() < 1e-9) && (b.music_on == a.music_on || !def.caps.persistence.contains(&crate::caps::Persistence::Settings));
        rows.push(Row::new(same, "simulation", "save round trip", format!("{status}")));
    }
    // Declared browser input checks refer to things that exist (the parser checked); say they are not run here.
    for b in &def.browser {
        rows.push(Row::new(true, "authoring", format!("browser check `{}`", b.name), "declared; it runs only in `web verify` (a real browser), not here"));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arcade() -> Arc<GameDef> {
        let text = r##"{"game2d":1,"id":"t","title":"T","description":"d",
          "capabilities":{"presentation":"2d","platforms":["web"],"networking":"offline","input":["keyboard"],"persistence":["progress"]},
          "view":{"width":160,"height":90},
          "sounds":{"pick":{"seconds":0.1,"layers":[{"sine":660,"decay":20}]}},
          "vars":{"score":0,"best":0},"persist":["best"],
          "prefabs":{"player":{"tag":"player","shape":{"rect":[8,8],"color":"#fc0"},"move":{"keys":{"mode":"topdown","speed":80}},"clamp":true},
                     "coin":{"tag":"coin","shape":{"circle":3,"color":"#ff0"}}},
          "scene":[{"prefab":"player","at":[20,45],"id":"p"},{"prefab":"coin","at":[100,45]},{"prefab":"coin","at":[100,20]}],
          "rules":[{"when":{"touch":["player","coin"]},"do":[{"add":["score",1]},{"destroy":"other"},{"play":"pick"}]},
                   {"when":{"every":0.1},"if":"count_coin == 0","do":[{"end":"win"}]}],
          "checks":{"scenarios":[
            {"name":"collect both","script":[{"approach":"coin","seconds":6}],"expect":[{"var":"score","eq":2},{"ended":"win"},{"sound":"pick","min":2}]},
            {"name":"idle","script":[{"wait":2}],"expect":[{"var":"score","eq":0},{"not_ended":true}]}]}}"##;
        Arc::new(parse(text).unwrap_or_else(|e| panic!("{e:#?}")))
    }

    #[test]
    fn a_bot_collects_coins_and_the_game_is_won() {
        let def = arcade();
        let (rep, sim) = run_scenario(&def, &def.scenarios[0], None);
        assert!(rep.ok, "{:?}", rep.failures);
        assert_eq!(sim.var("score"), Some(2.0));
        assert_eq!(rep.sounds, vec![("pick".to_string(), 2)]);
    }

    #[test]
    fn a_failed_expectation_says_what_was_found() {
        let def = arcade();
        let mut sc = def.scenarios[1].clone();
        sc.expect = vec![Expect::Var(0, Cmp::Eq, 5.0), Expect::Ended(Outcome::Win), Expect::Event("nothing".into(), 1, None), Expect::Count("coin".into(), Cmp::Eq, 0.0), Expect::Near("p".into(), [0.0, 0.0], 1.0)];
        let (rep, _) = run_scenario(&def, &sc, None);
        assert!(!rep.ok);
        let all = rep.failures.join("\n");
        assert!(all.contains("expected score eq 5, found 0"), "{all}");
        assert!(all.contains("expected game ended in a win, found it had not ended"), "{all}");
        assert!(all.contains("event `nothing` emitted 1..any, found 0 time(s) (events seen: none)"), "{all}");
        assert!(all.contains("count of `coin` eq 0, found 2"), "{all}");
        assert!(all.contains("`p` within 1 px of (0, 0), found it at (20.0, 45.0)"), "{all}");
    }

    #[test]
    fn a_wait_that_never_holds_times_out_and_a_runaway_script_hits_max_seconds() {
        let def = arcade();
        let mut sc = def.scenarios[1].clone();
        sc.script = vec![Step::WaitUntil(Expect::Ended(Outcome::Win), 1.0)];
        let (rep, _) = run_scenario(&def, &sc, None);
        assert!(rep.failures[0].contains("waited 1 s and it never held: expected game ended in a win, found it had not ended"), "{:?}", rep.failures);
        let mut sc = def.scenarios[1].clone();
        sc.max_seconds = 1.0;
        sc.script = vec![Step::Wait(5.0)];
        let (rep, _) = run_scenario(&def, &sc, None);
        assert!(rep.failures[0].contains("ran past max_seconds"), "{:?}", rep.failures);
    }

    #[test]
    fn a_click_on_a_button_that_is_not_there_names_the_buttons() {
        let def = arcade();
        let mut sc = def.scenarios[1].clone();
        sc.script = vec![Step::Button("start".into())];
        let (rep, _) = run_scenario(&def, &sc, None);
        assert!(rep.failures[0].contains("no visible button `start`") && rep.failures[0].contains("all buttons: none"), "{:?}", rep.failures);
    }

    #[test]
    fn verify_reports_each_kind_of_claim_and_catches_a_game_with_no_scenarios() {
        let def = arcade();
        let rows = verify(&def);
        assert!(rows.iter().all(|r| r.ok), "{rows:#?}");
        for claim in ["render", "simulation", "audio-waveform"] {
            assert!(rows.iter().any(|r| r.claim == claim), "no {claim} row");
        }
        assert!(rows.iter().any(|r| r.name == "save round trip"));
        assert!(rows.iter().any(|r| r.name.starts_with("determinism")));
        let mut bare = (*def).clone();
        bare.scenarios.clear();
        let rows = verify(&Arc::new(bare));
        assert!(rows.iter().any(|r| !r.ok && r.detail.contains("no `checks.scenarios`")), "{rows:#?}");
    }

    #[test]
    fn stopping_early_gives_the_state_at_that_tick() {
        let def = arcade();
        let (rep, sim) = run_scenario(&def, &def.scenarios[0], Some(30));
        assert!(rep.ok && sim.tick == 30 && sim.var("score") == Some(0.0), "{rep:?}");
    }
}
