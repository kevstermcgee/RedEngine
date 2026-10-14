//! A declared scale probe of the composed systems (ADR 2026-10-14-game-systems-compose-under-one-match-authority): N machine instances
//! (half furnaces, half presses, one machine behavior) on one shared storage, through the real match (`MatchSim::tick_once` with Workshop as its
//! game), at N = 10, 100, 1000. Probe sizes, not capacities.
//!
//! Budget: the probe stops escalating once a size's p95 tick exceeds 4 ms (a quarter of a 60 Hz tick) or its run takes more than 60 s.
//! `cargo test -p workshop_server --release --test scale -- --ignored --nocapture` prints the table; the plain test runs N = 10 as a check.

use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::parse_spawns;
mod common;

use common::workshop;
use std::time::{Duration, Instant};
use workshop_server::game::WorkshopGame;

const P95_BUDGET: Duration = Duration::from_millis(4);
const RUN_BUDGET: Duration = Duration::from_secs(60);

/// RSS in KB (Linux; 0 elsewhere).
fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/statm").ok().and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok()).map_or(0, |pages| pages * 4)
}

fn pct(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

struct Row {
    n: usize,
    build: Duration,
    ticks: [Duration; 3],
    max: Duration,
    save_bytes: usize,
    save: Duration,
    load: Duration,
    part_bytes: usize,
    rss_kb: u64,
    shipped: u64,
    run: Duration,
}

fn probe(n: usize, ticks: usize) -> Row {
    workshop_server::register();
    let (text, dir) = workshop(n);
    let rss0 = rss_kb();
    let t0 = Instant::now();
    let scene = red_engine2::scene_blocks::parse_scene_in(&text, Some(&dir)).unwrap();
    let mut sim = MatchSim::try_new(&scene, parse_spawns(&text).unwrap()).unwrap();
    let build = t0.elapsed();
    let run0 = Instant::now();
    for _ in 0..120 {
        sim.tick_once();
    }
    let mut times = Vec::with_capacity(ticks);
    for _ in 0..ticks {
        let t = Instant::now();
        sim.tick_once();
        times.push(t.elapsed());
    }
    let run = run0.elapsed();
    times.sort();
    let g = sim.game::<WorkshopGame>().unwrap();
    assert!(g.systems().halted().is_none());
    g.systems().ledger().audit().unwrap();
    let t = Instant::now();
    let saved = g.save().to_string();
    let save = t.elapsed();
    let value: serde_json::Value = serde_json::from_str(&saved).unwrap();
    let mut fresh = MatchSim::try_new(&scene, parse_spawns(&text).unwrap()).unwrap();
    let t = Instant::now();
    fresh.game_mut::<WorkshopGame>().unwrap().load(&value).unwrap();
    let load = t.elapsed();
    assert_eq!(fresh.game::<WorkshopGame>().unwrap().systems().hash(), g.systems().hash());
    let part_bytes = workshop_server::wire::encode(g).bytes.len();
    let shipped = g.shipped(&red_engine2::sim::defs::Id::parse("sc:ship").unwrap());
    let _ = std::fs::remove_dir_all(&dir);
    Row {
        n,
        build,
        ticks: [pct(&times, 0.5), pct(&times, 0.95), pct(&times, 0.99)],
        max: *times.last().unwrap(),
        save_bytes: saved.len(),
        save,
        load,
        part_bytes,
        rss_kb: rss_kb().saturating_sub(rss0),
        shipped,
        run,
    }
}

#[test]
fn ten_machines_run_audit_and_reload() {
    let r = probe(10, 300);
    assert!(r.shipped > 0, "the probe workshop produces and ships");
}

#[test]
#[ignore]
fn scale_probe() {
    println!("probe: N machines (half furnaces, half presses), one shared storage; 120 warm-up ticks, then 600 timed MatchSim ticks");
    println!(
        "{:>6} {:>9} {:>9} {:>9} {:>9} {:>9} {:>10} {:>8} {:>8} {:>9} {:>9} {:>8}",
        "N", "build", "p50", "p95", "p99", "max", "save B", "save", "load", "part B", "RSS KB", "shipped"
    );
    for n in [10, 100, 1000] {
        let r = probe(n, 600);
        println!(
            "{:>6} {:>9.2?} {:>9.2?} {:>9.2?} {:>9.2?} {:>9.2?} {:>10} {:>8.2?} {:>8.2?} {:>9} {:>9} {:>8}",
            r.n, r.build, r.ticks[0], r.ticks[1], r.ticks[2], r.max, r.save_bytes, r.save, r.load, r.part_bytes, r.rss_kb, r.shipped
        );
        if r.ticks[1] > P95_BUDGET || r.run > RUN_BUDGET {
            println!("stopped escalating: N = {} is over the budget (p95 {:?} > {:?} or run {:?} > {:?})", r.n, r.ticks[1], P95_BUDGET, r.run, RUN_BUDGET);
            break;
        }
    }
}
