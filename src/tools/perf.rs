//! `red_engine2 perf`: performance as a **testable contract** (ADR 0030). Measures a scene with real players walking in it and judges the
//! result against a budget the scene (or a file) declares, so "is this map cheap enough?" has an answer an AI or CI can act on.
//!
//! Two passes, both in-process and window-free:
//! * **sim** — the authoritative [`MatchSim`] alone: microseconds per tick (mean, p50, p95, p99, worst) with `N` players walking;
//! * **server** — a real [`Server`] on loopback with `N` handshaking clients: microseconds per whole server tick (simulation, snapshots,
//!   status, authentication), bytes per client per second, the largest datagram, and how many props the physics has promoted.
//!
//! Time is noisy and only ever noisy *upward* (a busy machine makes a tick slower, never faster), so each figure is the **minimum over
//! several measurement windows**; byte counts are deterministic. Budgets live in the scene's `checks.perf` block, e.g.
//! `"perf": {"players": 4, "sim_tick_p95_us": 3000, "bytes_per_client_sec": 20000}`, and `verify` runs them with every other check.

use crate::net::protocol::{ClientMsg, InputPacket};
use crate::net::server::{Server, ServerConfig};
use crate::net::testkit::RawClient;
use crate::player::Character;
use crate::sim::match_sim::{MatchSim, MAX_PLAYERS};
use crate::sim::player::PlayerInput;
use crate::sim::spawns::parse_spawns;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::Path;
use std::time::{Duration, Instant};

/// Keys a `checks.perf` block may hold (the measurement setup, then the budgets).
pub const PERF_KEYS: &[&str] = &[
    "players",
    "secs",
    "windows",
    "sim_tick_p95_us",
    "server_tick_p95_us",
    "server_tick_p99_us",
    "bytes_per_client_sec",
    "largest_datagram",
    "promoted_props_max",
];

/// Microseconds per tick: the distribution over one measurement.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Percentiles {
    /// Mean.
    pub mean: f64,
    /// Median.
    pub p50: f64,
    /// 95th percentile.
    pub p95: f64,
    /// 99th percentile.
    pub p99: f64,
    /// Slowest tick.
    pub max: f64,
}

impl Percentiles {
    /// The distribution of `samples` (microseconds).
    pub fn of(samples: &mut [f64]) -> Percentiles {
        if samples.is_empty() {
            return Percentiles::default();
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let at = |q: f64| samples[((samples.len() as f64 - 1.0) * q).round() as usize];
        Percentiles { mean: samples.iter().sum::<f64>() / samples.len() as f64, p50: at(0.5), p95: at(0.95), p99: at(0.99), max: samples[samples.len() - 1] }
    }

    /// The element-wise minimum of two distributions: the best window (noise only adds time).
    pub fn best(self, o: Percentiles) -> Percentiles {
        Percentiles { mean: self.mean.min(o.mean), p50: self.p50.min(o.p50), p95: self.p95.min(o.p95), p99: self.p99.min(o.p99), max: self.max.min(o.max) }
    }
}

/// What was measured.
#[derive(Debug, Clone, PartialEq)]
pub struct Measurement {
    /// Players walking.
    pub players: usize,
    /// Seconds per window.
    pub secs: f64,
    /// Windows measured (each figure is the best of them).
    pub windows: usize,
    /// Milliseconds to parse the scene and build the world once.
    pub build_ms: f64,
    /// Loose physics props in the scene.
    pub loose_props: usize,
    /// Props the physics had promoted to full bodies at the end (walking into things wakes them).
    pub promoted_props: usize,
    /// `MatchSim::tick_once` alone.
    pub sim_tick_us: Percentiles,
    /// A whole `Server::tick` (simulation + snapshots + status + tags).
    pub server_tick_us: Percentiles,
    /// Bytes the server sent per client per second.
    pub bytes_per_client_sec: f64,
    /// The largest datagram any client received.
    pub largest_datagram: usize,
}

/// The limits a measurement is judged against. `None` = not checked.
#[derive(Debug, Clone, PartialEq)]
pub struct Budget {
    /// Sim tick p95, microseconds.
    pub sim_tick_p95_us: Option<f64>,
    /// Server tick p95, microseconds.
    pub server_tick_p95_us: Option<f64>,
    /// Server tick p99, microseconds.
    pub server_tick_p99_us: Option<f64>,
    /// Bytes per client per second.
    pub bytes_per_client_sec: Option<f64>,
    /// Largest datagram, bytes.
    pub largest_datagram: Option<usize>,
    /// Most promoted props.
    pub promoted_props_max: Option<usize>,
}

impl Default for Budget {
    /// Generous but real: a 60 Hz tick has 16 667 us in total, and a snapshot must fit one datagram with room for its tag. A game tightens
    /// these in its scene.
    fn default() -> Self {
        Budget {
            sim_tick_p95_us: Some(3_000.0),
            server_tick_p95_us: Some(4_000.0),
            server_tick_p99_us: Some(8_000.0),
            bytes_per_client_sec: Some(20_000.0),
            largest_datagram: Some(1_250),
            promoted_props_max: Some(64),
        }
    }
}

/// One judged figure.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    /// The budget key (`sim_tick_p95_us`).
    pub name: String,
    /// Whether it held.
    pub ok: bool,
    /// The measured value against the limit.
    pub detail: String,
}

/// Reads the budget keys of a `checks.perf` block over the defaults; unknown or ill-typed keys are `Err`.
pub fn parse_budget(block: &Value) -> Result<(Budget, usize, f64, usize), String> {
    let obj = block.as_object().ok_or("checks.perf must be an object")?;
    let mut errs = Vec::new();
    crate::strict::check_keys(&mut errs, "checks.perf", obj, PERF_KEYS);
    if let Some(e) = errs.into_iter().next() {
        return Err(e);
    }
    let num = |k: &str| -> Result<Option<f64>, String> {
        match obj.get(k) {
            None => Ok(None),
            Some(v) => v.as_f64().filter(|n| n.is_finite() && *n >= 0.0).map(Some).ok_or(format!("checks.perf.{k}: must be a number >= 0")),
        }
    };
    let mut b = Budget::default();
    if let Some(v) = num("sim_tick_p95_us")? {
        b.sim_tick_p95_us = Some(v);
    }
    if let Some(v) = num("server_tick_p95_us")? {
        b.server_tick_p95_us = Some(v);
    }
    if let Some(v) = num("server_tick_p99_us")? {
        b.server_tick_p99_us = Some(v);
    }
    if let Some(v) = num("bytes_per_client_sec")? {
        b.bytes_per_client_sec = Some(v);
    }
    if let Some(v) = num("largest_datagram")? {
        b.largest_datagram = Some(v as usize);
    }
    if let Some(v) = num("promoted_props_max")? {
        b.promoted_props_max = Some(v as usize);
    }
    let players = num("players")?.map_or(4, |v| v as usize);
    let secs = num("secs")?.unwrap_or(1.0);
    let windows = num("windows")?.map_or(2, |v| v as usize);
    if !(1..=MAX_PLAYERS).contains(&players) || !(0.2..=60.0).contains(&secs) || !(1..=10).contains(&windows) {
        return Err(format!("checks.perf: players must be 1..={MAX_PLAYERS}, secs 0.2..60, windows 1..10"));
    }
    Ok((b, players, secs, windows))
}

fn walk_input(k: usize, tick: u32) -> PlayerInput {
    // Each player walks in its own slow circle so they cover the map, and swings the bat and grabs things now and then, which is what
    // wakes props into full physics bodies (an idle world would measure nothing).
    let t = tick + 11 * k as u32;
    PlayerInput {
        seq: tick + 1,
        forward: 1,
        yaw: (tick as f32 * (0.006 + 0.002 * k as f32)) + k as f32,
        attack: t.is_multiple_of(40),
        interact: t.is_multiple_of(90),
        ..Default::default()
    }
}

/// Times `MatchSim::tick_once` with `players` walking for `ticks` ticks.
fn sim_window(scene: &crate::schema::Scene, spawns: &[crate::sim::spawns::Spawn], players: usize, ticks: u32) -> Result<(Percentiles, usize), String> {
    let mut sim = MatchSim::try_new(scene, spawns.to_vec())?;
    let slots: Vec<usize> = (0..players).filter_map(|k| sim.add_player(if k % 2 == 0 { Character::Human } else { Character::Rat })).collect();
    let mut samples = Vec::with_capacity(ticks as usize);
    for t in 0..ticks {
        for (k, &slot) in slots.iter().enumerate() {
            sim.push_input(slot, walk_input(k, t));
        }
        let t0 = Instant::now();
        sim.tick_once();
        samples.push(t0.elapsed().as_secs_f64() * 1e6);
    }
    Ok((Percentiles::of(&mut samples), sim.props().dynamic_count()))
}

/// Times whole `Server::tick`s with `players` real (handshaken) clients walking, on a virtual clock.
fn server_window(
    text: &str,
    scene: &crate::schema::Scene,
    spawns: &[crate::sim::spawns::Spawn],
    players: usize,
    ticks: u32,
) -> Result<(Percentiles, f64, usize), String> {
    let hash = crate::net::map_hash(text);
    let mut server = Server::bind(ServerConfig::new("127.0.0.1:0".parse().map_err(|e| format!("{e}"))?, hash), MatchSim::try_new(scene, spawns.to_vec())?)
        .map_err(|e| format!("cannot bind a loopback socket: {e}"))?;
    server.set_logger(|_| {});
    let addr = SocketAddr::new("127.0.0.1".parse().map_err(|e| format!("{e}"))?, server.local_addr().map_err(|e| e.to_string())?.port());
    let mut now = Instant::now();
    let mut clients = Vec::new();
    for k in 0..players {
        let mut c = RawClient::new(addr, hash, None, 9_000 + k as u64).map_err(|e| e.to_string())?;
        c.handshake(|| {
            std::thread::sleep(Duration::from_millis(3));
            server.pump(now);
        })?;
        clients.push(c);
    }
    let bytes_before = server.stats().bytes_out;
    let mut samples = Vec::with_capacity(ticks as usize);
    let mut largest = 0usize;
    for t in 0..ticks {
        now += Duration::from_micros(16_667);
        for (k, c) in clients.iter().enumerate() {
            c.send(&ClientMsg::Input(InputPacket { inputs: vec![walk_input(k, t)], ..Default::default() }));
        }
        server.pump(now);
        let t0 = Instant::now();
        server.tick(now);
        samples.push(t0.elapsed().as_secs_f64() * 1e6);
        if t % 6 == 5 {
            for c in clients.iter_mut() {
                c.poll();
                largest = largest.max(c.largest_datagram);
            }
        }
    }
    let bytes = (server.stats().bytes_out - bytes_before) as f64 / (ticks as f64 / 60.0) / players as f64;
    Ok((Percentiles::of(&mut samples), bytes, largest))
}

/// Measures `scene_path` with `players` walking, `secs` per window, best of `windows`.
pub fn measure(scene_path: &Path, players: usize, secs: f64, windows: usize) -> Result<Measurement, String> {
    let text = std::fs::read_to_string(scene_path).map_err(|e| format!("{}: {e}", scene_path.display()))?;
    let t0 = Instant::now();
    let scene = crate::schema::parse_scene(&text).map_err(|e| e.join("; "))?;
    let spawns = parse_spawns(&text)?;
    let probe = MatchSim::try_new(&scene, spawns.clone())?;
    let build_ms = t0.elapsed().as_secs_f64() * 1e3;
    let loose_props = probe.props().props().len();
    drop(probe);
    let ticks = ((secs * 60.0) as u32).max(10);
    let (mut sim_best, mut server_best) = (None::<Percentiles>, None::<Percentiles>);
    let (mut promoted, mut bytes, mut largest) = (0usize, 0.0f64, 0usize);
    for _ in 0..windows.max(1) {
        let (s, dynamic) = sim_window(&scene, &spawns, players, ticks)?;
        sim_best = Some(sim_best.map_or(s, |b| b.best(s)));
        promoted = promoted.max(dynamic);
        let (v, b, l) = server_window(&text, &scene, &spawns, players, ticks)?;
        server_best = Some(server_best.map_or(v, |x| x.best(v)));
        bytes = bytes.max(b);
        largest = largest.max(l);
    }
    Ok(Measurement {
        players,
        secs,
        windows,
        build_ms,
        loose_props,
        promoted_props: promoted,
        sim_tick_us: sim_best.unwrap_or_default(),
        server_tick_us: server_best.unwrap_or_default(),
        bytes_per_client_sec: bytes,
        largest_datagram: largest,
    })
}

/// Judges a measurement.
pub fn evaluate(m: &Measurement, b: &Budget) -> Vec<Verdict> {
    let mut out = Vec::new();
    let mut upper = |name: &str, value: f64, limit: Option<f64>, unit: &str| {
        if let Some(l) = limit {
            out.push(Verdict { name: name.to_string(), ok: value <= l, detail: format!("{value:.0} {unit} (budget {l:.0})") });
        }
    };
    upper("sim_tick_p95_us", m.sim_tick_us.p95, b.sim_tick_p95_us, "us");
    upper("server_tick_p95_us", m.server_tick_us.p95, b.server_tick_p95_us, "us");
    upper("server_tick_p99_us", m.server_tick_us.p99, b.server_tick_p99_us, "us");
    upper("bytes_per_client_sec", m.bytes_per_client_sec, b.bytes_per_client_sec, "B/s");
    upper("largest_datagram", m.largest_datagram as f64, b.largest_datagram.map(|v| v as f64), "B");
    upper("promoted_props", m.promoted_props as f64, b.promoted_props_max.map(|v| v as f64), "props");
    out
}

/// Advice for whichever budgets were blown (or nearly).
pub fn advice(m: &Measurement, verdicts: &[Verdict]) -> Vec<String> {
    let mut tips = Vec::new();
    let failed = |n: &str| verdicts.iter().any(|v| v.name == n && !v.ok);
    if failed("promoted_props") || m.promoted_props > 40 {
        tips.push("Many props were promoted to full physics bodies. Untouched props cost almost nothing; keep clutter that players will not touch out of walking lanes, or shrink what is `movable`.".to_string());
    }
    if failed("sim_tick_p95_us") || failed("server_tick_p95_us") {
        tips.push(format!("The tick is slow with {} players. Check `lint` for overlapping colliders (they multiply contacts) and how many loose props are near the walking lanes.", m.players));
    }
    if failed("bytes_per_client_sec") || failed("largest_datagram") {
        tips.push(
            "Snapshots are large. Enable interest management (give the map `zones` and `portals`) so clients only receive the rooms near them.".to_string(),
        );
    }
    tips
}

/// The measurement and verdicts as JSON.
pub fn to_json(m: &Measurement, verdicts: &[Verdict]) -> Value {
    let p = |x: Percentiles| json!({"mean": x.mean.round(), "p50": x.p50.round(), "p95": x.p95.round(), "p99": x.p99.round(), "max": x.max.round()});
    json!({
        "ok": verdicts.iter().all(|v| v.ok),
        "players": m.players, "secs": m.secs, "windows": m.windows,
        "build_ms": m.build_ms.round(), "loose_props": m.loose_props, "promoted_props": m.promoted_props,
        "sim_tick_us": p(m.sim_tick_us), "server_tick_us": p(m.server_tick_us),
        "bytes_per_client_sec": m.bytes_per_client_sec.round(), "largest_datagram": m.largest_datagram,
        "verdicts": verdicts.iter().map(|v| json!({"name": v.name, "ok": v.ok, "detail": v.detail})).collect::<Vec<_>>(),
        "advice": advice(m, verdicts),
    })
}

/// The measurement and verdicts as text.
pub fn render(m: &Measurement, verdicts: &[Verdict]) -> String {
    let mut s = format!(
        "perf: {} walking player(s), best of {} window(s) of {:.1} s ({} loose props, {} promoted, world built in {:.0} ms)\n",
        m.players, m.windows, m.secs, m.loose_props, m.promoted_props, m.build_ms
    );
    let line = |name: &str, p: Percentiles| {
        format!("  {name:<12} mean {:>6.0}  p50 {:>6.0}  p95 {:>6.0}  p99 {:>6.0}  max {:>7.0} us\n", p.mean, p.p50, p.p95, p.p99, p.max)
    };
    s.push_str(&line("sim tick", m.sim_tick_us));
    s.push_str(&line("server tick", m.server_tick_us));
    s.push_str(&format!("  network      {:.0} B/s per client, largest datagram {} B\n", m.bytes_per_client_sec, m.largest_datagram));
    for v in verdicts {
        s.push_str(&format!("  {} {}: {}\n", if v.ok { "ok  " } else { "FAIL" }, v.name, v.detail));
    }
    for t in advice(m, verdicts) {
        s.push_str(&format!("  tip: {t}\n"));
    }
    s
}

/// The scene's `checks.perf` as `verify` rows `(name, ok, detail)`.
pub fn verify_checks(path: &Path, block: &Value) -> Result<Vec<(String, bool, String)>, String> {
    let (budget, players, secs, windows) = parse_budget(block)?;
    let m = measure(path, players, secs, windows)?;
    let verdicts = evaluate(&m, &budget);
    Ok(verdicts.into_iter().map(|v| (format!("perf {}", v.name), v.ok, format!("{} with {} players", v.detail, m.players))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_are_ordered_and_best_takes_the_minimum() {
        let mut v: Vec<f64> = (1..=100).map(|x| x as f64).collect();
        let p = Percentiles::of(&mut v);
        assert!(p.p50 <= p.p95 && p.p95 <= p.p99 && p.p99 <= p.max);
        assert_eq!((p.max, p.mean), (100.0, 50.5));
        let noisy = Percentiles { mean: 60.0, p50: 55.0, p95: 90.0, p99: 100.0, max: 900.0 };
        let best = p.best(noisy);
        assert_eq!(best.max, 100.0);
        assert_eq!(best.p95, p.p95.min(90.0));
        assert_eq!(Percentiles::of(&mut []), Percentiles::default());
    }

    #[test]
    fn a_budget_block_overrides_defaults_and_rejects_typos_and_bad_numbers() {
        let (b, players, secs, windows) = parse_budget(&json!({"players": 6, "secs": 0.5, "sim_tick_p95_us": 1234, "promoted_props_max": 3})).unwrap();
        assert_eq!((players, secs, windows), (6, 0.5, 2));
        assert_eq!((b.sim_tick_p95_us, b.promoted_props_max), (Some(1234.0), Some(3)));
        assert_eq!(b.bytes_per_client_sec, Budget::default().bytes_per_client_sec, "unset keys keep the defaults");
        let e = parse_budget(&json!({"sim_tick_p95": 1})).unwrap_err();
        assert!(e.contains("sim_tick_p95") && e.contains("sim_tick_p95_us"), "{e}");
        assert!(parse_budget(&json!({"players": 99})).is_err());
        assert!(parse_budget(&json!({"secs": -1})).is_err());
        assert!(parse_budget(&json!({"bytes_per_client_sec": "lots"})).is_err());
        assert!(parse_budget(&json!([])).is_err());
    }

    #[test]
    fn evaluation_flags_exactly_the_blown_budgets_and_gives_advice() {
        let m = Measurement {
            players: 4,
            secs: 1.0,
            windows: 1,
            build_ms: 5.0,
            loose_props: 100,
            promoted_props: 80,
            sim_tick_us: Percentiles { mean: 900.0, p50: 800.0, p95: 5000.0, p99: 6000.0, max: 9000.0 },
            server_tick_us: Percentiles { mean: 1000.0, p50: 900.0, p95: 1500.0, p99: 2000.0, max: 3000.0 },
            bytes_per_client_sec: 30_000.0,
            largest_datagram: 900,
        };
        let v = evaluate(&m, &Budget::default());
        let failed: Vec<&str> = v.iter().filter(|x| !x.ok).map(|x| x.name.as_str()).collect();
        assert_eq!(failed, vec!["sim_tick_p95_us", "bytes_per_client_sec", "promoted_props"]);
        let tips = advice(&m, &v).join(" ");
        assert!(tips.contains("promoted") && tips.contains("interest management") && tips.contains("lint"), "{tips}");
        assert!(evaluate(&m, &Budget { sim_tick_p95_us: None, bytes_per_client_sec: None, promoted_props_max: None, ..Budget::default() })
            .iter()
            .all(|x| x.ok));
    }

    #[test]
    fn the_test_lab_is_measured_and_stays_inside_the_default_budget() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json");
        let m = measure(&path, 3, 0.6, 1).unwrap();
        assert!(m.sim_tick_us.p50 > 0.0 && m.server_tick_us.p50 > 0.0, "{m:?}");
        assert!(m.bytes_per_client_sec > 1_000.0, "snapshots really flowed: {m:?}");
        assert!(m.largest_datagram > 100 && m.largest_datagram < 1_250, "{m:?}");
        assert!(m.loose_props >= 20, "the lab has loose props: {m:?}");
        let v = evaluate(&m, &Budget::default());
        assert!(v.iter().all(|x| x.ok), "{v:?}");
    }
}
