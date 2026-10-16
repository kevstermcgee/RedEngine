//! A search for a playthrough: which keys reach a goal (the end of a platformer level, past every enemy) in the real simulation.
//!
//! Reach analysis (`Sim::can_reach`) answers only for top-down walkers: whether a platformer level can be finished depends on jumps, speed and
//! moving things. This plays it instead: a beam search over the inputs a player can hold for a tenth of a second (left or right, running or not,
//! jump held or not), each candidate a clone of the live [`Sim`] stepped with the game's own rules. States that meet `avoid` (a life lost) are
//! dropped, the furthest along survive, and the first to meet `goal` is the answer: a list of `hold`/`wait` steps that replays exactly as a
//! scenario script (holding the same keys in consecutive steps never releases them for a tick).

use crate::game::{GameDef, KeyMode, Move};
use crate::rules_expr::Expr;
use crate::sim::Sim;
use std::sync::Arc;

/// Ticks per decision: a tenth of a second.
pub const CHUNK: u32 = 6;

/// What to search for.
#[derive(Debug, Clone)]
pub struct Goal {
    /// The level to start in (a game with `levels`), else the start.
    pub level: Option<usize>,
    /// Done when this holds.
    pub goal: Expr,
    /// A state where this holds is dropped (a life lost, a power lost).
    pub avoid: Option<Expr>,
    /// How far along a state is (higher is better); default: the x of the thing the keys move.
    pub progress: Option<Expr>,
    /// Give up after this much game time.
    pub max_seconds: f32,
    /// Candidates kept per tenth of a second.
    pub beam: usize,
    /// The scenario seed the answer will be replayed with.
    pub seed: u64,
    /// Held first, before the search: (actions held, seconds) pairs, like a scenario's `hold`/`wait` steps. A route the search cannot find on
    /// its own (wait on a moving platform, then jump) is given as a prefix and searched on from where it ends.
    pub prefix: Vec<(Vec<String>, f32)>,
}

/// One decision: the actions held for [`CHUNK`] ticks.
pub type Inputs = &'static [&'static str];

/// The inputs considered at every decision.
pub const CHOICES: &[Inputs] = &[
    &[],
    &["right"],
    &["right", "secondary"],
    &["left"],
    &["action"],
    &["right", "action"],
    &["right", "secondary", "action"],
    &["left", "action"],
    &["down"],
];

/// The answer.
#[derive(Debug, Clone)]
pub struct Solution {
    /// The inputs, one per decision.
    pub inputs: Vec<Inputs>,
    /// Game ticks it takes.
    pub ticks: u64,
    /// Candidates simulated.
    pub explored: usize,
}

impl Solution {
    /// The answer as scenario steps: consecutive equal decisions merged, `wait` for nothing held.
    pub fn steps(&self) -> serde_json::Value {
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.inputs.len() {
            let mut j = i;
            while j < self.inputs.len() && self.inputs[j] == self.inputs[i] {
                j += 1;
            }
            let secs = ((j - i) as f64 * f64::from(CHUNK) / 60.0 * 10_000.0).round() / 10_000.0;
            out.push(if self.inputs[i].is_empty() { serde_json::json!({"wait": secs}) } else { serde_json::json!({"hold": self.inputs[i], "seconds": secs}) });
            i = j;
        }
        serde_json::Value::Array(out)
    }
}

#[derive(Clone)]
struct Node {
    sim: Sim,
    path: Vec<u16>,
    score: f64,
}

/// How far along a state is: the x of the thing the keys move (the player), or the best of the last known if it has none now.
fn progress(sim: &Sim) -> Option<(f32, f32, f32, bool)> {
    let def = &sim.def;
    sim.entities.iter().find_map(|e| match def.prefabs[e.prefab].mv {
        Move::Keys { mode: KeyMode::Platformer, .. } if e.alive => Some((e.x, e.y, e.vx, e.grounded)),
        _ => None,
    })
}

fn holds(sim: &Sim, e: &Expr) -> bool {
    e.eval(&sim.vars) != 0.0
}

/// Searches. `Err` says how far it got when nothing reached the goal.
pub fn solve(def: &Arc<GameDef>, g: &Goal) -> Result<Solution, String> {
    let mut start = Sim::new(def.clone(), g.seed);
    if let Some(l) = g.level {
        if !start.load_level(l) {
            return Err(format!("there is no level {l}"));
        }
    }
    for (k, (held, secs)) in g.prefix.iter().enumerate() {
        for a in ["left", "right", "up", "down", "action", "secondary"] {
            start.set_action(a, held.iter().any(|h| h == a));
        }
        for _ in 0..(secs * 60.0).round() as u32 {
            start.step();
            if g.avoid.as_ref().is_some_and(|a| holds(&start, a)) {
                return Err(format!("the prefix meets `avoid` in its step {} ({held:?} for {secs} s)", k + 1));
            }
        }
    }
    if progress(&start).is_none() {
        return Err("nothing is moved by platformer keys at the start: there is no player to search for".into());
    }
    let max_gens = (g.max_seconds * 60.0 / CHUNK as f32).ceil() as usize;
    let score_of = |sim: &Sim, before: f64| match &g.progress {
        Some(e) => e.eval(&sim.vars),
        None => progress(sim).map_or(before, |q| f64::from(q.0)),
    };
    let mut beam = vec![Node { score: score_of(&start, 0.0), sim: start, path: Vec::new() }];
    let mut explored = 0usize;
    let mut best_x = f64::MIN;
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 16);
    for _ in 0..max_gens {
        // Expand every survivor by every choice, in parallel; each child is (parent, choice, result).
        let jobs: Vec<(usize, usize)> = (0..beam.len()).flat_map(|p| (0..CHOICES.len()).map(move |c| (p, c))).collect();
        explored += jobs.len();
        let chunk = jobs.len().div_ceil(threads);
        let results: Vec<(usize, usize, Option<Node>, bool)> = std::thread::scope(|s| {
            let handles: Vec<_> = jobs
                .chunks(chunk.max(1))
                .map(|part| {
                    let beam = &beam;
                    let score_of = &score_of;
                    s.spawn(move || {
                        part.iter()
                            .map(|&(p, c)| {
                                let mut sim = beam[p].sim.clone();
                                for a in ["left", "right", "up", "down", "action", "secondary"] {
                                    sim.set_action(a, CHOICES[c].contains(&a));
                                }
                                let mut done = false;
                                let mut dead = false;
                                for _ in 0..CHUNK {
                                    sim.step();
                                    if g.avoid.as_ref().is_some_and(|a| holds(&sim, a)) {
                                        dead = true;
                                        break;
                                    }
                                    if holds(&sim, &g.goal) {
                                        done = true;
                                        break;
                                    }
                                }
                                if dead {
                                    return (p, c, None, false);
                                }
                                let score = score_of(&sim, beam[p].score);
                                let mut path = beam[p].path.clone();
                                path.push(c as u16);
                                (p, c, Some(Node { sim, path, score }), done)
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        });
        // The first (in a fixed order) to reach the goal wins: the answer does not depend on the thread count.
        if let Some((_, _, Some(n), _)) = results.iter().find(|r| r.3) {
            let ticks = n.sim.tick;
            return Ok(Solution { inputs: n.path.iter().map(|&c| CHOICES[c as usize]).collect(), ticks, explored });
        }
        let mut next: Vec<Node> = results.into_iter().filter_map(|r| r.2).collect();
        if next.is_empty() {
            return Err(format!("every candidate met `avoid` (furthest x {best_x:.0}): the level may not be passable this way, or `avoid` is too strict"));
        }
        // Furthest first; among equals the earlier parent and choice (stable sort). One per coarse state, so the beam is not many copies of one jump.
        next.sort_by(|a, b| b.score.total_cmp(&a.score));
        let mut seen = std::collections::HashSet::new();
        next.retain(|n| {
            let key = progress(&n.sim).map(|(x, y, vx, gr)| ((x / 3.0).floor() as i32, (y / 3.0).floor() as i32, (vx / 25.0).round() as i32, gr));
            seen.insert(key)
        });
        next.truncate(g.beam);
        best_x = best_x.max(next[0].score);
        beam = next;
    }
    Err(format!("no candidate reached the goal in {} s (furthest x {best_x:.0}): raise --max-seconds or --beam", g.max_seconds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::parse;

    /// A floor with a pit in the middle and a flag at the far end; `script`, when given, is its one scenario.
    fn pit_level(script: Option<&serde_json::Value>) -> Arc<GameDef> {
        let floor = "#".repeat(14) + "...." + &"#".repeat(22);
        let checks = match script {
            Some(s) => {
                format!(r#"{{"scenarios":[{{"name":"replay","max_seconds":30,"script":{s},"expect":[{{"var":"won","eq":1}},{{"var":"fell","eq":0}}]}}]}}"#)
            }
            None => r#"{"scenarios":[]}"#.to_string(),
        };
        let text = format!(
            r##"{{"game2d":1,"id":"t","title":"T","description":"d",
            "capabilities":{{"presentation":"2d","platforms":["windows","linux"],"networking":"offline","input":["keyboard"],"persistence":[]}},
            "view":{{"width":160,"height":96,"world":[640,96],"camera":{{"follow":"player","lerp":1}}}},
            "vars":{{"won":0,"fell":0}},
            "prefabs":{{"wall":{{"tag":"solid","shape":{{"rect":[16,16],"color":"#888"}},"body":{{"type":"static"}}}},
              "pit":{{"tag":"pit","shape":{{"rect":[16,16],"color":"#000"}},"hidden":true}},
              "flag":{{"tag":"flag","shape":{{"rect":[8,32],"color":"#0f0"}}}},
              "hero":{{"tag":"player","shape":{{"rect":[10,14],"color":"#fc0"}},"body":{{"type":"dynamic","gravity":1000}},"collide":["solid"],
                "move":{{"keys":{{"mode":"platformer","speed":90,"run":150,"jump":360,"accel":400}}}}}}}},
            "map":{{"tile":16,"rows":["{empty}","{empty}","{empty}","{empty}","{floor}","{pits}"],"legend":{{"#":"wall","p":"pit"}}}},
            "scene":[{{"prefab":"hero","at":[24,57],"id":"p"}},{{"prefab":"flag","at":[600,48]}}],
            "rules":[{{"when":{{"touch":["player","flag"]}},"do":[{{"set":["won",1]}}]}},{{"when":{{"touch":["player","pit"]}},"do":[{{"set":["fell",1]}}]}}],
            "checks":{checks}}}"##,
            empty = ".".repeat(40),
            pits = "p".repeat(40),
        );
        Arc::new(parse(&text).unwrap_or_else(|e| panic!("{e:?}")))
    }

    fn expr(def: &GameDef, s: &str) -> Expr {
        crate::rules_expr::parse(s, &def.var_names).unwrap()
    }

    #[test]
    fn it_jumps_the_pit_and_its_steps_replay_as_a_scenario() {
        let def = pit_level(None);
        let goal = Goal {
            level: None,
            goal: expr(&def, "won == 1"),
            avoid: Some(expr(&def, "fell == 1")),
            progress: None,
            max_seconds: 20.0,
            beam: 16,
            seed: 1,
            prefix: Vec::new(),
        };
        let s = solve(&def, &goal).expect("solvable");
        assert!(s.ticks > 60 && s.ticks < 20 * 60, "{} ticks", s.ticks);
        // The printed steps, played by the scenario runner, reach the same goal without falling.
        let steps = s.steps();
        let replay = pit_level(Some(&steps));
        let (rep, _) = crate::script::run_scenario(&replay, &replay.scenarios[0], None);
        assert!(rep.failures.is_empty(), "{:?}\n{steps}", rep.failures);
        // Doing nothing does not get there: the answer is not trivial.
        let idle = pit_level(Some(&serde_json::json!([{"wait": 5}])));
        assert!(!crate::script::run_scenario(&idle, &idle.scenarios[0], None).0.failures.is_empty());
    }

    #[test]
    fn an_impossible_goal_says_how_far_it_got() {
        let def = pit_level(None);
        let goal = Goal {
            level: None,
            goal: expr(&def, "won == 2"),
            avoid: Some(expr(&def, "fell == 1")),
            progress: None,
            max_seconds: 3.0,
            beam: 8,
            seed: 1,
            prefix: Vec::new(),
        };
        let e = solve(&def, &goal).unwrap_err();
        assert!(e.contains("furthest x"), "{e}");
    }
}
