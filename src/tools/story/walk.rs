//! The bounded headless story-walk (`story walk SCENE WALKS`): the real story runtime under the game's real match, from each walk's
//! script, branching at every `"choice": "*"` step. It is a scenario run (`sim::scenario::run_with`) per path: the match is not cloned, so
//! a path is replayed from the start with the choices before its branch point forced, and the cost is the number of runs, reported.
//!
//! Exploration is depth-first over the choices offered at the `*` steps, within `max_paths` runs and `max_depth` branch points per run.
//! It is **exhaustive only over those choices**, from that start and that script, when no bound was hit: timings, positions, other
//! inputs and seeds are not varied, and states are not merged (two paths that reach the same state are both run). A walk's optional
//! `branches` (`{"lend": [expect, ...]}`) are expectations checked on every path that took that choice: the consequences tested.

use crate::sim::scenario::{
    self,
    story::{Offer, Policy},
    Scenario, ScenarioResult,
};
use crate::tools::simrun::{self, Loaded};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

/// How far a walk may go.
#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    /// Most runs per walk.
    pub max_paths: usize,
    /// Most branch points along one run (beyond them the first offered choice is taken, and the walk is not exhaustive).
    pub max_depth: usize,
}

/// One run of a walk.
#[derive(Debug, Clone)]
pub struct PathRun {
    /// `(line, choice)` taken at each branch point.
    pub taken: Vec<(String, String)>,
    /// Every expectation (the walk's, and its branches' for the choices taken) held.
    pub passed: bool,
    /// The ones that did not, with their evidence.
    pub failures: Vec<String>,
    /// The branch expectations checked on this path.
    pub consequences: Vec<String>,
    /// Ticks run.
    pub ticks: u64,
    /// The match checksum at the end.
    pub checksum: u64,
}

/// One walk's result.
#[derive(Debug, Clone, Default)]
pub struct WalkResult {
    /// The walk's name.
    pub name: String,
    /// Each run.
    pub paths: Vec<PathRun>,
    /// Whether every choice at every branch point was explored (no bound hit).
    pub exhaustive: bool,
    /// The bounds that were hit.
    pub limits: Vec<String>,
    /// Lines heard on some path.
    pub heard: BTreeSet<String>,
    /// Barks chosen on some path.
    pub barks: BTreeSet<String>,
    /// Choices taken on some path (`conversation/choice`).
    pub taken: BTreeSet<String>,
    /// Choices held back somewhere, with why (the first reason seen).
    pub held_back: BTreeMap<String, String>,
    /// Every line, bark and choice the story has (for what was never reached).
    pub content: (Vec<String>, Vec<String>, Vec<String>),
    /// How decisions ended across every path: `outcome kind -> count` (played, silence, cooling down, queued, dropped, ...).
    pub outcomes: BTreeMap<String, usize>,
    /// Wall time.
    pub ms: u128,
    /// Total ticks simulated.
    pub ticks: u64,
}

/// Every walk of a file.
#[derive(Debug, Clone, Default)]
pub struct WalkReport {
    /// The walks.
    pub walks: Vec<WalkResult>,
}

impl WalkReport {
    /// Runs that failed an expectation.
    pub fn failed(&self) -> usize {
        self.walks.iter().flat_map(|w| &w.paths).filter(|p| !p.passed).count()
    }

    /// As JSON.
    pub fn to_json(&self) -> Value {
        json!({"ok": self.failed() == 0, "walks": self.walks.iter().map(|w| {
            let (lines, barks, choices) = &w.content;
            json!({
                "name": w.name,
                "runs": w.paths.len(),
                "exhaustive": w.exhaustive,
                "scope": "the choices at the walk's `*` steps, from its start and script; timings, positions, other inputs and seeds are not varied",
                "limits": w.limits,
                "paths": w.paths.iter().map(|p| json!({"taken": p.taken, "passed": p.passed, "failures": p.failures, "consequences_tested": p.consequences, "ticks": p.ticks, "checksum": format!("{:016x}", p.checksum)})).collect::<Vec<_>>(),
                "reached": {"lines": w.heard.len(), "of_lines": lines.len(), "barks": w.barks.len(), "of_barks": barks.len(), "choices": w.taken.len(), "of_choices": choices.len()},
                "never_reached": {
                    "lines": lines.iter().filter(|l| !w.heard.contains(*l)).collect::<Vec<_>>(),
                    "barks": barks.iter().filter(|b| !w.barks.contains(*b)).collect::<Vec<_>>(),
                    "choices": choices.iter().filter(|c| !w.taken.contains(*c)).collect::<Vec<_>>(),
                },
                "held_back": w.held_back,
                "decisions": w.outcomes,
                "cost": {"ms": w.ms, "ticks": w.ticks},
            })
        }).collect::<Vec<_>>()})
    }

    /// As text.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for w in &self.walks {
            let (lines, barks, choices) = &w.content;
            let ok = w.paths.iter().filter(|p| p.passed).count();
            out.push_str(&format!(
                "WALK {}: {} run(s), {ok} passed; {}\n",
                w.name,
                w.paths.len(),
                if w.exhaustive {
                    "exhaustive over the choices at its `*` steps (from this start and script only)".to_string()
                } else {
                    format!("NOT exhaustive: {}", w.limits.join("; "))
                }
            ));
            for p in &w.paths {
                let path: Vec<String> = p.taken.iter().map(|(_, c)| c.clone()).collect();
                out.push_str(&format!(
                    "  {} [{}] {} ticks{}\n",
                    if p.passed { "PASS" } else { "FAIL" },
                    path.join(" > "),
                    p.ticks,
                    if p.consequences.is_empty() { String::new() } else { format!(", {} consequence check(s)", p.consequences.len()) }
                ));
                for f in &p.failures {
                    out.push_str(&format!("      {f}\n"));
                }
            }
            out.push_str(&format!(
                "  reached: {}/{} lines, {}/{} barks, {}/{} choices; cost {} ms, {} ticks\n",
                w.heard.len(),
                lines.len(),
                w.barks.len(),
                barks.len(),
                w.taken.len(),
                choices.len(),
                w.ms,
                w.ticks
            ));
            for (c, why) in &w.held_back {
                out.push_str(&format!("  held back: {c}: {why}\n"));
            }
            let never: Vec<&String> = lines.iter().filter(|l| !w.heard.contains(*l)).chain(barks.iter().filter(|b| !w.barks.contains(*b))).collect();
            if !never.is_empty() {
                out.push_str(&format!("  never reached on these paths: {}\n", never.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
            }
            let d: Vec<String> = w.outcomes.iter().map(|(k, n)| format!("{k} {n}")).collect();
            out.push_str(&format!("  decisions: {}\n", d.join(", ")));
        }
        out
    }
}

/// The policy of one run: the forced choices first, then the first one offered; every offer recorded.
struct Explore<'a> {
    forced: &'a [String],
    offers: Vec<Offer>,
    taken: Vec<String>,
    max_depth: usize,
    beyond: bool,
}

impl Policy for Explore<'_> {
    fn choose(&mut self, offer: &Offer) -> Result<String, String> {
        let k = self.taken.len();
        let pick = match self.forced.get(k) {
            Some(c) if offer.offered.contains(c) => c.clone(),
            Some(c) => {
                return Err(format!(
                    "the walk forced `{c}` at `{}`, which is no longer offered ({:?}): the run is not deterministic",
                    offer.line, offer.offered
                ))
            }
            None => offer.offered.first().cloned().ok_or_else(|| format!("nothing is offered at `{}`", offer.line))?,
        };
        if k >= self.max_depth {
            self.beyond = true;
        }
        self.offers.push(offer.clone());
        self.taken.push(pick.clone());
        Ok(pick)
    }
}

fn conv_of(line: &str) -> &str {
    line.split('/').next().unwrap_or("")
}

/// Walks every walk of `walks` (a JSON array of scenarios) against `scene`.
pub fn walk_file(scene: &Path, walks: &Path, bounds: Bounds, only: Option<&str>) -> Result<WalkReport, String> {
    let loaded = simrun::load(scene)?;
    let text = std::fs::read_to_string(walks).map_err(|e| format!("{}: {e}", walks.display()))?;
    let list: Value = serde_json::from_str(&text).map_err(|e| format!("{}: json: {e}", walks.display()))?;
    let list = match list {
        Value::Array(a) => a,
        one => vec![one],
    };
    let mut report = WalkReport::default();
    for (i, w) in list.iter().enumerate() {
        let name = w.get("name").and_then(Value::as_str).unwrap_or("walk").to_string();
        if only.is_some_and(|o| !name.contains(o)) {
            continue;
        }
        report.walks.push(walk_one(&loaded, w, &format!("{}[{i}]", walks.display()), bounds)?);
    }
    if report.walks.is_empty() {
        return Err("no walk to run (an empty file, or `--only` matched nothing)".into());
    }
    Ok(report)
}

/// The scenario of walk `w`, with the expectations of `branches` for the choices in `taken` added, and the story inspected at the end.
fn scenario_for(loaded: &Loaded, w: &Value, source: &str, taken: &[String]) -> Result<Scenario, String> {
    let mut v = w.clone();
    let branches = v.as_object_mut().and_then(|o| o.remove("branches"));
    let mut expect = v.get("expect").and_then(Value::as_array).cloned().unwrap_or_default();
    for (choice, list) in branches.as_ref().and_then(Value::as_object).into_iter().flatten() {
        if taken.contains(choice) {
            expect.extend(list.as_array().cloned().unwrap_or_default());
        }
    }
    v["expect"] = json!(expect);
    let mut inspect = v.get("inspect").and_then(Value::as_array).cloned().unwrap_or_default();
    inspect.push(json!("story"));
    v["inspect"] = json!(inspect);
    let mut s = simrun::parse_scenarios(loaded, &[v], source).map_err(|e| e.join("\n"))?;
    Ok(s.remove(0))
}

fn walk_one(loaded: &Loaded, w: &Value, source: &str, bounds: Bounds) -> Result<WalkResult, String> {
    let t = Instant::now();
    let branch_keys: BTreeSet<String> = w.get("branches").and_then(Value::as_object).map(|o| o.keys().cloned().collect()).unwrap_or_default();
    let mut out = WalkResult { name: w.get("name").and_then(Value::as_str).unwrap_or("walk").to_string(), exhaustive: true, ..WalkResult::default() };
    let mut stack: Vec<Vec<String>> = vec![Vec::new()];
    let mut seen_paths: BTreeSet<Vec<String>> = BTreeSet::new();
    while let Some(prefix) = stack.pop() {
        if out.paths.len() >= bounds.max_paths {
            out.exhaustive = false;
            out.limits.push(format!("stopped at {} runs (--max-paths) with {} branch(es) unexplored", bounds.max_paths, stack.len() + 1));
            break;
        }
        let base = scenario_for(loaded, w, source, &[])?;
        let mut policy = Explore { forced: &prefix, offers: Vec::new(), taken: Vec::new(), max_depth: bounds.max_depth, beyond: false };
        let mut result = scenario::run_with(&base, &loaded.scene, &loaded.spawns, None, &mut policy)?;
        out.ticks += result.ticks;
        let base_labels: BTreeSet<String> = result.outcomes.iter().map(|o| o.label.clone()).collect();
        let (offers, taken, beyond) = (policy.offers, policy.taken, policy.beyond);
        // A path that took a choice with branch expectations runs again with them (the path is known only once it has run).
        let mut consequences = Vec::new();
        if taken.iter().any(|c| branch_keys.contains(c)) {
            let s = scenario_for(loaded, w, source, &taken)?;
            let mut again = Explore { forced: &taken, offers: Vec::new(), taken: Vec::new(), max_depth: usize::MAX, beyond: false };
            result = scenario::run_with(&s, &loaded.scene, &loaded.spawns, None, &mut again)?;
            out.ticks += result.ticks;
            consequences = result.outcomes.iter().filter(|o| !base_labels.contains(&o.label)).map(|o| o.label.clone()).collect();
        }
        if beyond && out.exhaustive {
            out.exhaustive = false;
            out.limits.push(format!("a run went past {} branch points (--max-depth): later choices took the first offered", bounds.max_depth));
        }
        record(&mut out, &result, &offers, &taken);
        out.paths.push(PathRun {
            taken: offers.iter().zip(&taken).map(|(o, c)| (o.line.clone(), c.clone())).collect(),
            passed: result.passed,
            failures: result.outcomes.iter().filter(|o| !o.ok).map(|o| format!("{}: {}", o.label, o.detail)).collect(),
            consequences,
            ticks: result.ticks,
            checksum: result.checksum,
        });
        seen_paths.insert(taken.clone());
        // Every other choice at every branch point past the forced prefix is a new prefix.
        for d in (prefix.len()..taken.len().min(bounds.max_depth)).rev() {
            for alt in offers[d].offered.iter().filter(|c| **c != taken[d]).rev() {
                let mut next: Vec<String> = taken[..d].to_vec();
                next.push(alt.clone());
                if !seen_paths.contains(&next) {
                    stack.push(next);
                }
            }
        }
    }
    out.ms = t.elapsed().as_millis();
    Ok(out)
}

fn record(out: &mut WalkResult, r: &ScenarioResult, offers: &[Offer], taken: &[String]) {
    for (o, c) in offers.iter().zip(taken) {
        out.taken.insert(format!("{}/{c}", conv_of(&o.line)));
        for (h, why) in &o.held_back {
            out.held_back.entry(format!("{}/{h}", conv_of(&o.line))).or_insert_with(|| why.clone());
        }
    }
    let Some(story) = r.inspected.iter().find(|(t, _)| t == "story").and_then(|(_, v)| v.clone()) else { return };
    let keys = |k: &str| story.get(k).and_then(Value::as_object).map(|m| m.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
    out.heard.extend(keys("heard"));
    out.barks.extend(keys("chosen"));
    let list = |k: &str| {
        story.pointer(k).and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect::<Vec<_>>()).unwrap_or_default()
    };
    if out.content.0.is_empty() {
        out.content = (list("/content/lines"), list("/content/barks"), list("/content/choices"));
    }
    for (kind, n) in story.get("tally").and_then(Value::as_object).into_iter().flatten() {
        *out.outcomes.entry(kind.clone()).or_default() += n.as_u64().unwrap_or(0) as usize;
    }
}
