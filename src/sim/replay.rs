//! Replaying a [`Trace`] into a fresh [`MatchSim`] and finding where it stops agreeing with the recording.
//!
//! No renderer, no sockets: the recorded joins, inputs and impulses are applied at the ticks they were recorded at, and
//! after every checkpointed tick the replay's checksums are compared with the recorded ones. The report names the
//! **first divergent tick**, which of players / props / rules differ, and a compact state diff (from the nearest
//! recorded dump). Comparing two traces directly ([`compare_traces`]) does the same for a desync between two machines.

use super::match_sim::MatchSim;
use super::player::PlayerState;
use super::trace::{diff_dumps, Checkpoint, Dump, Entry, Header, Trace};
use crate::sim::spawns::Spawn;
use glam::{Vec2, Vec3};

/// Where a replay first disagreed with a recording.
#[derive(Debug, Clone, PartialEq)]
pub struct Divergence {
    /// The tick (ticks completed) of the first checkpoint that differs.
    pub tick: u64,
    /// Which parts of the state differ: any of `players`, `props`, `rules`.
    pub components: Vec<&'static str>,
    /// The recorded checkpoint.
    pub recorded: Checkpoint,
    /// The replayed checkpoint.
    pub replayed: Checkpoint,
    /// Compact lines describing the difference in state (may be empty when no dump is close enough).
    pub detail: Vec<String>,
}

/// What a replay found.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayReport {
    /// Ticks replayed.
    pub ticks: u64,
    /// Checkpoints compared.
    pub compared: usize,
    /// The first bit-exact divergence, if any.
    pub exact: Option<Divergence>,
    /// The first divergence of the coarse (millimetre) checksums: a *game* divergence rather than float noise.
    pub coarse: Option<Divergence>,
    /// Whether the recorded and replayed game events are identical.
    pub events_match: bool,
    /// Set when the recording came from a different platform, so an exact-only mismatch is expected float noise.
    pub note: Option<String>,
}

impl ReplayReport {
    /// True when nothing diverged, exactly, and the events match.
    pub fn is_clean(&self) -> bool {
        self.exact.is_none() && self.coarse.is_none() && self.events_match
    }

    /// True when the game itself agrees (coarse checksums and events) even if last-bit float noise differs.
    pub fn game_agrees(&self) -> bool {
        self.coarse.is_none() && self.events_match
    }
}

fn components(a: &Checkpoint, b: &Checkpoint) -> Vec<&'static str> {
    let mut v = Vec::new();
    if a.players != b.players {
        v.push("players");
    }
    if a.props != b.props {
        v.push("props");
    }
    if a.rules != b.rules {
        v.push("rules");
    }
    v
}

fn detail_for(tick: u64, recorded: &[Dump], replay_at: &dyn Fn(u64) -> Option<Dump>) -> Vec<String> {
    let Some(rec) = recorded.iter().rfind(|d| d.tick <= tick) else { return Vec::new() };
    let Some(rep) = replay_at(rec.tick) else { return Vec::new() };
    let diff = diff_dumps(rec, &rep);
    if diff.is_empty() && rec.tick < tick {
        // The states agreed at the last recorded dump: say what the replay had at the divergent tick instead.
        return match replay_at(tick) {
            Some(now) => vec![
                format!("recorded state agrees with the replay at tick {} (the nearest dump); it diverged after that — re-record with `--dump-every 1` for the exact state diff", rec.tick),
                format!("replay state at tick {tick}: {} player(s), {} moving prop(s), vars {:?}, hidden {:?}", now.players.len(), now.props.len(), now.vars, now.hidden),
            ],
            None => Vec::new(),
        };
    }
    let mut out = vec![format!("state at recorded dump, tick {}:", rec.tick)];
    out.extend(diff);
    out
}

/// Builds a fresh sim for `header` (same spawns and scene as the recording).
fn fresh(scene: &crate::schema::Scene, spawns: &[Spawn], header: &Header) -> Result<MatchSim, String> {
    let mut spawns: Vec<Spawn> = spawns.to_vec();
    if !header.spawn_group.is_empty() {
        spawns.retain(|s| s.group == header.spawn_group);
    }
    MatchSim::try_new(scene, spawns)
}

fn apply(sim: &mut MatchSim, e: &Entry) {
    match e {
        Entry::Join { state, character, .. } => {
            let [px, py, fy, vy, yaw, pitch] = state.map(f32::from_bits);
            let character = crate::net::protocol::character_from_wire(*character);
            let _: Option<usize> = sim.add_player_with(PlayerState { pos: Vec2::new(px, py), foot_y: fy, vy, yaw, pitch, character });
        }
        Entry::Leave { slot, .. } => {
            let _ = sim.remove_player(*slot);
        }
        Entry::Input { slot, input, .. } => {
            let _: bool = sim.push_input(*slot, *input);
        }
        Entry::Impulse { prop, dir, at, impulse, .. } => {
            sim.apply_impulse(*prop, Vec3::from_array(dir.map(f32::from_bits)), Vec3::from_array(at.map(f32::from_bits)), f32::from_bits(*impulse));
        }
    }
}

/// Replays `trace` against `scene`/`spawns` and reports where it diverges from the recording.
pub fn replay(trace: &Trace, scene: &crate::schema::Scene, spawns: &[Spawn]) -> Result<ReplayReport, String> {
    let h = &trace.header;
    if h.tick_rate != super::clock::TICK_RATE_HZ {
        return Err(format!("the trace ran at {} ticks/s but this engine runs at {}: it cannot be replayed", h.tick_rate, super::clock::TICK_RATE_HZ));
    }
    let mut sim = fresh(scene, spawns, h)?;
    let mut next = 0usize;
    let mut cps = trace.checkpoints.iter().peekable();
    let (mut exact, mut coarse) = (None, None);
    let mut compared = 0usize;
    let mut replay_dumps: Vec<Dump> = Vec::new();
    let mut events = Vec::new();
    for tick in 0..trace.final_tick {
        while next < trace.entries.len() && trace.entries[next].tick() <= tick {
            apply(&mut sim, &trace.entries[next]);
            next += 1;
        }
        sim.tick_once();
        events.extend(sim.take_events());
        if trace.dumps.iter().any(|d| d.tick == sim.tick()) {
            replay_dumps.push(sim.dump());
        }
        while let Some(cp) = cps.peek().filter(|c| c.tick <= sim.tick()) {
            if cp.tick == sim.tick() {
                compared += 1;
                let mine = sim.checkpoint();
                if exact.is_none() && (cp.players, cp.props, cp.rules) != (mine.players, mine.props, mine.rules) {
                    exact = Some(Divergence { tick: cp.tick, components: components(cp, &mine), recorded: **cp, replayed: mine, detail: Vec::new() });
                }
                if coarse.is_none() && cp.coarse != mine.coarse {
                    coarse = Some(Divergence { tick: cp.tick, components: components(cp, &mine), recorded: **cp, replayed: mine, detail: Vec::new() });
                }
            }
            cps.next();
        }
        if exact.is_some() && coarse.is_some() {
            break; // both first divergences are known
        }
    }
    let ticks = sim.tick();
    // A state diff for the first divergence: re-run to the dump ticks we need.
    for d in [exact.as_mut(), coarse.as_mut()].into_iter().flatten() {
        d.detail = detail_for(d.tick, &trace.dumps, &|want| dump_at(trace, scene, spawns, &replay_dumps, want));
    }
    let recorded_events: Vec<(u64, &str, &str, Option<usize>)> = trace.events.iter().map(|e| (e.tick, e.rule.as_str(), e.name.as_str(), e.slot)).collect();
    let replayed_events: Vec<(u64, &str, &str, Option<usize>)> = events.iter().map(|e| (e.tick, e.rule.as_str(), e.name.as_str(), e.slot)).collect();
    let events_match = exact.is_some() || recorded_events == replayed_events; // after a divergence the event lists are expected to differ
    let here = format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH);
    let note = (h.platform != here)
        .then(|| format!("recorded on {} but replayed on {here}: exact float checksums may differ in the last bit; trust `coarse`", h.platform));
    Ok(ReplayReport { ticks, compared, exact, coarse, events_match, note })
}

/// The replay's own dump at `tick` (from the ones collected while replaying, or by replaying again up to it).
fn dump_at(trace: &Trace, scene: &crate::schema::Scene, spawns: &[Spawn], have: &[Dump], tick: u64) -> Option<Dump> {
    if let Some(d) = have.iter().find(|d| d.tick == tick) {
        return Some(d.clone());
    }
    let mut sim = fresh(scene, spawns, &trace.header).ok()?;
    let mut next = 0usize;
    for t in 0..tick {
        while next < trace.entries.len() && trace.entries[next].tick() <= t {
            apply(&mut sim, &trace.entries[next]);
            next += 1;
        }
        sim.tick_once();
    }
    Some(sim.dump())
}

/// Compares two traces of the same match (e.g. from a server and from another machine) checkpoint by checkpoint.
pub fn compare_traces(a: &Trace, b: &Trace) -> Option<Divergence> {
    for ca in &a.checkpoints {
        let Some(cb) = b.checkpoints.iter().find(|c| c.tick == ca.tick) else { continue };
        if (ca.players, ca.props, ca.rules) != (cb.players, cb.props, cb.rules) {
            let detail = detail_for(ca.tick, &a.dumps, &|want| b.dumps.iter().find(|d| d.tick == want).cloned());
            return Some(Divergence { tick: ca.tick, components: components(ca, cb), recorded: *ca, replayed: *cb, detail });
        }
    }
    None
}

/// Builds a scene-less human-readable summary of a divergence (the `replay` command prints this).
pub fn describe(d: &Divergence) -> String {
    let mut out = format!("first divergence at tick {}: {} differ", d.tick, d.components.join(" + "));
    for line in &d.detail {
        out.push_str("\n    ");
        out.push_str(line);
    }
    out
}
