//! Tidewatch's scene block: how a night is played, which compiled story it runs, and which game answers the scene's HUD shows.
//!
//! ```json
//! "tidewatch": {"story": "story/tidewatch.story.json", "seed": 11, "watch_secs": 60, "burn_secs": 10, "late_secs": 6,
//!               "show": {"fuel": "tw:brazier_fuel", "trust": "tidewatch:trust_bram"}}
//! ```
//!
//! The story file is the toolchain's output (`red_engine2 story build content/story`): the game loads that package and binds it to its
//! systems here, so a package that no longer fits the game (a query renamed, a command removed, a newer format) is refused when the scene
//! loads, `file:line` first, before anything runs.

use crate::watch::{Rules, Watch};
use red_engine2::sim::defs::Id;
use red_engine2::sim::story::package::{Catalog, Loaded};
use serde_json::{Map, Value};
use std::path::Path;
use std::sync::Arc;

/// The block, read.
#[derive(Debug, Clone)]
pub struct TidewatchSpec {
    /// How a watch is played.
    pub rules: Rules,
    /// The host's random seed.
    pub seed: u64,
    /// The story, loaded and bound.
    pub story: Arc<Loaded>,
    /// The story file, as the block names it.
    pub story_file: String,
    /// `(scene variable, query)`: the game writes each answer into the variable every tick (the HUD, the objective and the scene's rules read it).
    pub show: Vec<(String, Id)>,
}

/// The catalog the story is bound against: the watch's queries, commands and events, and the scene's variables.
pub fn catalog(rules: &Rules, root: &Map<String, Value>) -> Catalog {
    let scene_vars = root.get("vars").and_then(Value::as_object).map(|m| m.keys().cloned().collect()).unwrap_or_default();
    Catalog::of(&[&Watch::new(rules.clone())], scene_vars)
}

/// Parses the `tidewatch` block (`Ok(None)` when the scene has none).
pub fn parse_block(root: &Map<String, Value>, base: Option<&Path>) -> Result<Option<TidewatchSpec>, Vec<String>> {
    let Some(v) = root.get("tidewatch") else { return Ok(None) };
    let Some(o) = v.as_object() else { return Err(vec!["tidewatch: must be an object like {\"story\": \"story/tidewatch.story.json\"}".into()]) };
    let mut errs = Vec::new();
    for k in o.keys().filter(|k| !["story", "seed", "watch_secs", "burn_secs", "late_secs", "show"].contains(&k.as_str())) {
        errs.push(format!("tidewatch.{k}: unknown key (story, seed, watch_secs, burn_secs, late_secs, show)"));
    }
    let secs = |k: &str, default: u32| o.get(k).and_then(Value::as_f64).map_or(default, |s| (s * 60.0).round() as u32);
    let d = Rules::default();
    let rules = Rules {
        watch_ticks: secs("watch_secs", d.watch_ticks / 60),
        burn_ticks: secs("burn_secs", d.burn_ticks / 60),
        late_ticks: secs("late_secs", d.late_ticks / 60),
        ..d
    };
    let story_file = o.get("story").and_then(Value::as_str).unwrap_or("").to_string();
    if story_file.is_empty() {
        errs.push("tidewatch.story: missing (the compiled story, `story/tidewatch.story.json`)".into());
    }
    let path = base.map_or_else(|| Path::new(&story_file).to_path_buf(), |b| b.join(&story_file));
    let cat = catalog(&rules, root);
    let story = match std::fs::read_to_string(&path) {
        Ok(text) => match Loaded::from_json(&text, &cat) {
            Ok(l) => Some(Arc::new(l)),
            Err(ds) => {
                errs.extend(
                    ds.into_iter().map(|d| format!("tidewatch.story ({story_file}): {}{}", d, d.fix.as_ref().map(|f| format!(" -> {f}")).unwrap_or_default())),
                );
                None
            }
        },
        Err(e) => {
            errs.push(format!("tidewatch.story: {}: {e} (build it with `red_engine2 story build`)", path.display()));
            None
        }
    };
    let mut show = Vec::new();
    for (var, q) in o.get("show").and_then(Value::as_object).into_iter().flatten() {
        match q.as_str().map(Id::parse) {
            Some(Ok(q))
                if cat.queries.iter().any(|d| d.id == q)
                    || story.as_ref().is_some_and(|s| s.package.facts.iter().any(|f| format!("{}:{}", s.package.story, f.id) == q.as_str())) =>
            {
                show.push((var.clone(), q))
            }
            Some(Ok(q)) => errs.push(format!("tidewatch.show.{var}: no query `{q}` (the watch's or the story's facts)")),
            _ => errs.push(format!("tidewatch.show.{var}: a query id like \"tw:brazier_fuel\"")),
        }
        if !cat.scene_vars.contains(var) {
            errs.push(format!("tidewatch.show.{var}: the scene declares no variable `{var}`"));
        }
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    let seed = o.get("seed").and_then(Value::as_u64).unwrap_or(11);
    Ok(story.map(|story| TidewatchSpec { rules, seed, story, story_file, show }))
}
