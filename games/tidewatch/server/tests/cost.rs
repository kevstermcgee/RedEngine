//! What the story costs at run time (a probe, not a gate: `cargo test -p tidewatch_server --release --test cost -- --ignored --nocapture`):
//! loading and binding the package, and the game's systems host ticking a scripted night (two watches, the brazier fed, every bark, notice and
//! director decision that brings) with the story and without it, on the same commands and the same seed.

use red_engine2::sim::defs::Id;
use red_engine2::sim::story::package::Loaded;
use red_engine2::sim::story::runtime::Story;
use red_engine2::sim::systems::{Command, Outside, System, Systems};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tidewatch_server::watch::{Rules, Watch};

/// The scene's variables a story reads: the player stands by Odile all night.
struct ByTheBrazier;

impl Outside for ByTheBrazier {
    fn query(&self, q: &Id) -> Option<f64> {
        Some(if q.as_str() == "scene:near_odile" { 1.0 } else { 0.0 })
    }
}

fn package() -> (String, red_engine2::sim::story::package::Catalog) {
    let content = Path::new(env!("CARGO_MANIFEST_DIR")).join("../content");
    let text = std::fs::read_to_string(content.join("story/tidewatch.story.json")).unwrap();
    let scene: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(content.join("tidewatch.json")).unwrap()).unwrap();
    (text, tidewatch_server::content::catalog(&Rules::default(), scene.as_object().unwrap()))
}

fn host(with_story: bool) -> Systems {
    let rules = Rules::default();
    let mut systems: Vec<Box<dyn System>> = vec![Box::new(Watch::new(rules.clone()))];
    if with_story {
        let (text, catalog) = package();
        systems.push(Box::new(Story::new(Arc::new(Loaded::from_json(&text, &catalog).unwrap()))));
    }
    Systems::new(systems, Watch::ledger(&rules).unwrap(), 11).unwrap()
}

/// A night: ring, feed a brick every 6 s through both watches. Returns each tick's time in microseconds.
fn night(s: &mut Systems) -> Vec<f64> {
    let cmd = |s: &mut Systems, n: &str| {
        let _ = s.command_with(&Command { name: Id::parse(n).unwrap(), subject: None, value: 0 }, &ByTheBrazier);
    };
    let mut out = Vec::new();
    for watch in 0..2 {
        cmd(s, "tw:ring");
        for t in 0..60 * 62 {
            if t % 360 == 0 {
                cmd(s, "tw:take");
                cmd(s, "tw:feed");
            }
            let at = Instant::now();
            s.tick_with(&ByTheBrazier).unwrap();
            out.push(at.elapsed().as_secs_f64() * 1e6);
        }
        let _ = watch;
    }
    out
}

fn pct(mut v: Vec<f64>, p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * p) as usize]
}

#[test]
#[ignore]
fn what_the_story_costs() {
    let (text, catalog) = package();
    let mut loads = Vec::new();
    for _ in 0..50 {
        let at = Instant::now();
        let _ = Loaded::from_json(&text, &catalog).unwrap();
        loads.push(at.elapsed().as_secs_f64() * 1e3);
    }
    println!("package: {} bytes; load + check + bind: p50 {:.2} ms, p95 {:.2} ms", text.len(), pct(loads.clone(), 0.5), pct(loads, 0.95));
    for with_story in [false, true] {
        let mut s = host(with_story);
        let ticks = night(&mut s);
        let decisions = s.get::<Story>().map_or(0, |st| st.tally().values().sum::<u32>());
        println!(
            "{}: {} ticks, p50 {:.2} us, p95 {:.2} us, p99 {:.2} us, max {:.1} us; story decisions {decisions}",
            if with_story { "watch + story" } else { "watch alone  " },
            ticks.len(),
            pct(ticks.clone(), 0.5),
            pct(ticks.clone(), 0.95),
            pct(ticks.clone(), 0.99),
            pct(ticks, 1.0)
        );
    }
}
