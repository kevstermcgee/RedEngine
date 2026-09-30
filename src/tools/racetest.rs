//! `red_engine2 race-test`: race bots on a map, headless, and say whether the track can be finished (Great Outdoors, ADR
//! 2026-09-29-great-outdoors-karts-as-a-first-class-engine-feature).
//!
//! The racing counterpart of `sim` and `perf`: it builds the authoritative match for the map's `race` block, fills the grid with kart bots (one animal each, or the
//! animals you name), runs the real tick until the race is over or the time is up, and reports every bot's finish time and lap times, how many pickups were taken and
//! how many hits landed. A track a bot cannot finish (a fence in the racing line, a gate the kart cannot reach, a corner too tight) shows up here as a bot that did not
//! finish, before a person finds it with a controller.

use crate::sim::ai::BotsConfig;
use crate::sim::clock::TICK_RATE_HZ;
use crate::sim::items::HazardKind;
use crate::sim::kart::{Driver, Item};
use crate::sim::match_sim::{MatchSim, MAX_PLAYERS};
use crate::sim::race::Phase;
use crate::sim::spawns::parse_spawns;
use serde_json::{json, Value};
use std::path::Path;

/// One bot's race.
#[derive(Debug, Clone, PartialEq)]
pub struct BotResult {
    /// The animal it drove.
    pub driver: Driver,
    /// The tick it finished at, or `None` if it had not when the race ended.
    pub finished_at: Option<u32>,
    /// The tick each lap was completed at.
    pub lap_ticks: Vec<u32>,
    /// Its place at the end (from 1).
    pub place: usize,
    /// Where it was (x, z) when the race ended.
    pub at: (f32, f32),
}

/// What a bot race showed.
#[derive(Debug, Clone, PartialEq)]
pub struct RaceReport {
    /// Laps in the race.
    pub laps: u8,
    /// Ticks the race ran (from the start of the simulation, countdown included).
    pub ticks: u32,
    /// The bots, best place first.
    pub results: Vec<BotResult>,
    /// Item pickups taken from boxes.
    pub items_picked: u32,
    /// Spin-outs that hits caused.
    pub hits: u32,
    /// The most hazards on the track at once.
    pub peak_hazards: usize,
    /// Whether the race ended because the time ran out rather than because it was finished.
    pub timed_out: bool,
}

impl RaceReport {
    /// Whether every bot finished.
    pub fn all_finished(&self) -> bool {
        self.results.iter().all(|r| r.finished_at.is_some())
    }
}

fn seconds(ticks: u32) -> f32 {
    ticks as f32 / TICK_RATE_HZ as f32
}

/// Races `bots` kart bots (at skill `level`, `0..=1`) on the map at `path` for at most `max_secs` seconds of game time. `drivers` names the animal of each bot in
/// order (the rest take the slot's default animal).
pub fn run(path: &Path, bots: usize, level: f32, max_secs: f32, drivers: &[Driver]) -> Result<RaceReport, String> {
    if bots == 0 || bots > MAX_PLAYERS {
        return Err(format!("--bots must be 1..{MAX_PLAYERS}"));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut scene = crate::schema::parse_scene(&text).map_err(|e| e.join("; "))?;
    let course = scene.race.clone().ok_or("this scene has no `race` block (see `describe scene`, SPEC \"Races\")")?;
    // This command judges the *track*: a slow kart that would have finished must not be marked as not finished only because the game ends a race
    // `finish_grace_secs` after the winner. So the grace is made as long as the time limit here; the report says how far behind each bot finished.
    scene.race = Some(std::sync::Arc::new(crate::sim::race::RaceCourse { finish_grace_secs: max_secs.max(1.0), ..(*course).clone() }));
    let spawns = parse_spawns(&text).map_err(|e| e.to_string())?;
    let mut sim = MatchSim::try_new(&scene, spawns)?;
    let cfg = BotsConfig::default();
    for slot in 0..bots {
        let mut spec = cfg.spec(slot);
        spec.level = level.clamp(0.0, 1.0);
        if !sim.add_bot_in_slot(slot, &spec) {
            return Err(format!("could not put a bot in slot {slot}"));
        }
        if let Some(d) = drivers.get(slot) {
            sim.set_driver(slot, *d);
        }
    }
    let max_ticks = (max_secs.max(1.0) * TICK_RATE_HZ as f32) as u32;
    let mut lap_ticks: Vec<Vec<u32>> = vec![Vec::new(); bots];
    let mut last_lap = vec![0u8; bots];
    let mut held = vec![false; bots];
    let (mut items_picked, mut hits, mut peak) = (0u32, 0u32, 0usize);
    let mut last_spin = vec![0u16; bots];
    let mut ticks = 0;
    let mut timed_out = true;
    for t in 1..=max_ticks {
        sim.tick_once();
        ticks = t;
        let Some(race) = sim.race() else { break };
        for slot in 0..bots {
            if let Some(p) = race.progress(slot) {
                if p.lap > last_lap[slot] {
                    last_lap[slot] = p.lap;
                    lap_ticks[slot].push(t);
                }
            }
            if let Some(k) = sim.kart(slot) {
                let holds = k.item != Item::None;
                items_picked += u32::from(holds && !held[slot]);
                held[slot] = holds;
                hits += u32::from(k.spin_ticks > last_spin[slot] && k.spin_ticks >= 29);
                last_spin[slot] = k.spin_ticks;
            }
        }
        peak = peak.max(sim.hazards().iter().filter(|h| matches!(h.kind, HazardKind::Acorn | HazardKind::Plank)).count());
        if race.phase() == Phase::Finished {
            timed_out = false;
            break;
        }
    }
    let race = sim.race().ok_or("the race vanished")?;
    let mut results: Vec<BotResult> = (0..bots)
        .map(|slot| BotResult {
            driver: sim.driver(slot).unwrap_or(Driver::Duck),
            finished_at: race.progress(slot).and_then(|p| p.finished_at),
            lap_ticks: lap_ticks[slot].clone(),
            place: race.place_of(slot).unwrap_or(bots),
            at: sim.player(slot).map_or((0.0, 0.0), |p| (p.state.pos.x, p.state.pos.y)),
        })
        .collect();
    results.sort_by_key(|r| r.place);
    Ok(RaceReport { laps: course.laps, ticks, results, items_picked, hits, peak_hazards: peak, timed_out })
}

/// The report as text: one line per bot, then what happened.
pub fn render(r: &RaceReport) -> String {
    let mut s = format!(
        "race-test: {} bot(s), {} lap(s), {:.0} s of game time{}\n",
        r.results.len(),
        r.laps,
        seconds(r.ticks),
        if r.timed_out { " (TIME RAN OUT)" } else { "" }
    );
    for res in &r.results {
        let laps: Vec<String> = res.lap_ticks.windows(2).map(|w| format!("{:.1}", seconds(w[1] - w[0]))).collect();
        let first = res.lap_ticks.first().map(|t| seconds(*t));
        s.push_str(&format!(
            "  {}. {:<7} {}  laps: {}\n",
            res.place,
            res.driver.name(),
            res.finished_at.map_or(format!("DID NOT FINISH (last at x {:.0}, z {:.0})", res.at.0, res.at.1), |t| format!("{:6.1} s", seconds(t))),
            match (first, laps.is_empty()) {
                (Some(f), true) => format!("{f:.1} s to lap 1"),
                (Some(f), false) => format!("{f:.1} to lap 1, then {}", laps.join(", ")),
                _ => "none completed".into(),
            }
        ));
    }
    s.push_str(&format!("  {} item(s) picked up, {} hit(s) landed, up to {} hazard(s) on the track at once\n", r.items_picked, r.hits, r.peak_hazards));
    let dnf = r.results.iter().filter(|x| x.finished_at.is_none()).count();
    if dnf == 0 {
        s.push_str("  every bot finished: the track can be raced\n");
    } else {
        s.push_str(&format!(
            "  {dnf} bot(s) did not finish: look at their line (a fence across it, a gate they cannot reach, a corner too tight for the kart)\n"
        ));
    }
    s
}

/// The report as JSON (`--json` wraps it in the usual envelope).
pub fn to_json(r: &RaceReport) -> Value {
    json!({
        "laps": r.laps,
        "seconds": seconds(r.ticks),
        "timed_out": r.timed_out,
        "all_finished": r.all_finished(),
        "items_picked": r.items_picked,
        "hits": r.hits,
        "peak_hazards": r.peak_hazards,
        "bots": r.results.iter().map(|b| json!({
            "driver": b.driver.name(),
            "place": b.place,
            "finish_seconds": b.finished_at.map(seconds),
            "lap_seconds": b.lap_ticks.windows(2).map(|w| seconds(w[1] - w[0])).collect::<Vec<_>>(),
            "first_lap_seconds": b.lap_ticks.first().map(|t| seconds(*t)),
        })).collect::<Vec<_>>(),
    })
}

/// Parses `--drivers duck,bunny,...` into animals (any case), or says which name is wrong.
pub fn parse_drivers(list: &str) -> Result<Vec<Driver>, String> {
    list.split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|name| Driver::parse(name).ok_or_else(|| format!("no driver called '{}' (Duck, Bunny, Deer, Coyote, Hawk, Bear, Wolf, Beaver)", name.trim())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_map(name: &str, race: &str) -> std::path::PathBuf {
        let text = format!(
            r#"{{"camera":{{"position":[0,30,60],"target":[0,0,0]}},
                "zones":[{{"id":"line","rect":[-6,-41,6,-39]}},{{"id":"east","rect":[39,-6,41,6]}},{{"id":"south","rect":[-6,39,6,41]}},{{"id":"west","rect":[-41,-6,-39,6]}}],
                {race}
                "spawns":[{{"id":"a","position":[-20,0,-41.5],"yaw_deg":90}},{{"id":"b","position":[-20,0,-38.5],"yaw_deg":90}},{{"id":"c","position":[-24,0,-41.5],"yaw_deg":90}},{{"id":"d","position":[-24,0,-38.5],"yaw_deg":90}}],
                "objects":[{{"id":"floor","type":"plane","size":[200,200],"position":[0,0.01,0]}}]}}"#
        );
        let path = std::env::temp_dir().join(format!("re2_race_test_{}_{name}.json", std::process::id()));
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn a_raceable_track_is_finished_by_every_bot_and_reported() {
        let path = write_map("ok", r#""race":{"laps":2,"gates":["line","east","south","west"],"countdown_secs":1},"#);
        let report = run(&path, 4, 0.8, 200.0, &[Driver::Deer, Driver::Beaver]).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(report.all_finished() && !report.timed_out, "{}", render(&report));
        assert_eq!(report.results.len(), 4);
        assert_eq!(report.results.iter().map(|r| r.place).collect::<Vec<_>>(), vec![1, 2, 3, 4], "best place first");
        assert!(report.results.iter().all(|r| r.lap_ticks.len() == 2), "two laps each");
        let names: Vec<&str> = report.results.iter().map(|r| r.driver.name()).collect();
        assert!(names.contains(&"Deer") && names.contains(&"Beaver"), "the named animals raced: {names:?}");
        let text = render(&report);
        assert!(text.contains("every bot finished") && text.contains("Deer"), "{text}");
        assert_eq!(to_json(&report)["all_finished"], true);
        assert_eq!(to_json(&report)["bots"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn a_track_that_cannot_be_finished_says_so() {
        // Seven laps of a diamond in a few seconds of game time: the bots cannot finish, and the report says the time ran out.
        let path = write_map("short", r#""race":{"laps":7,"gates":["line","east","south","west"],"countdown_secs":1},"#);
        let report = run(&path, 2, 0.8, 8.0, &[]).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(!report.all_finished() && report.timed_out);
        let text = render(&report);
        assert!(text.contains("DID NOT FINISH") && text.contains("TIME RAN OUT") && text.contains("did not finish"), "{text}");
    }

    #[test]
    fn mistakes_are_named() {
        let plain = std::env::temp_dir().join(format!("re2_race_test_{}_plain.json", std::process::id()));
        std::fs::write(&plain, r#"{"camera":{"position":[0,1,5],"target":[0,0,0]},"spawns":[{"id":"a","position":[0,0,0],"yaw_deg":0}],"objects":[]}"#)
            .unwrap();
        assert!(run(&plain, 2, 0.5, 10.0, &[]).unwrap_err().contains("no `race` block"));
        let _ = std::fs::remove_file(&plain);
        let path = write_map("bots", r#""race":{"gates":["line","east","south","west"]},"#);
        assert!(run(&path, 0, 0.5, 10.0, &[]).unwrap_err().contains("--bots"));
        assert!(run(&path, 9, 0.5, 10.0, &[]).unwrap_err().contains("--bots"));
        let _ = std::fs::remove_file(&path);
        assert!(run(Path::new("/nonexistent/race.json"), 2, 0.5, 10.0, &[]).is_err());
        assert_eq!(parse_drivers("Duck, wolf,BEAVER").unwrap(), vec![Driver::Duck, Driver::Wolf, Driver::Beaver]);
        assert!(parse_drivers("duck,moose").unwrap_err().contains("moose"));
    }
}
