//! A recording of a match: everything needed to reproduce it without a renderer or a socket.
//!
//! A [`Trace`] holds the header (engine version, tick rate, map hash, seed, spawn group, platform), the ordered
//! **inputs** to the simulation — joins, leaves, player inputs, server-applied impulses, each stamped with the tick it
//! was applied before — a **checkpoint** of state checksums after every `checkpoint_every` ticks, the authoritative
//! **events** the rules emitted, and periodic full state **dumps** for diagnosing a divergence. Replaying the inputs
//! into a fresh [`MatchSim`](super::match_sim::MatchSim) and comparing checkpoints ([`super::replay`]) finds the first
//! tick where two runs disagree. The file is JSON, with compact arrays for the bulky parts and floats stored as bits
//! so they round-trip exactly.

use super::player::PlayerInput;
use serde_json::{json, Value};

/// The trace file format version.
pub const TRACE_VERSION: u32 = 1;

/// Describes the run a trace came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    /// Format version ([`TRACE_VERSION`]).
    pub version: u32,
    /// The engine build that recorded it (crate version).
    pub engine: String,
    /// Simulation ticks per second.
    pub tick_rate: u32,
    /// The map file the run used, as given on the command line (informational: `replay` falls back to it).
    pub scene: String,
    /// Hash of the map file text (`net::map_hash`); `0` when unknown.
    pub map_hash: u32,
    /// The match seed (reserved: the simulation has no randomness yet).
    pub seed: u64,
    /// Spawn group filter the match ran with (`""` = all).
    pub spawn_group: String,
    /// `os/arch` of the recording machine: exact float checksums are only comparable on the same platform.
    pub platform: String,
    /// A checkpoint is taken every this many ticks.
    pub checkpoint_every: u32,
    /// A full state dump is taken every this many ticks (`0` = only at the end).
    pub dump_every: u32,
}

/// One input to the simulation, applied before the tick number it carries runs.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    /// A player joined (or resumed) in exactly this state.
    Join {
        /// Tick it was applied before.
        tick: u64,
        /// Slot it got.
        slot: usize,
        /// `0` human, `1` rat.
        character: u8,
        /// `pos.x, pos.y, foot_y, vy, yaw, pitch` as `f32` bits.
        state: [u32; 6],
    },
    /// A player left.
    Leave {
        /// Tick.
        tick: u64,
        /// Slot.
        slot: usize,
    },
    /// An input was queued for a player.
    Input {
        /// Tick.
        tick: u64,
        /// Slot.
        slot: usize,
        /// The (sanitised) input.
        input: PlayerInput,
    },
    /// The server shoved a prop.
    Impulse {
        /// Tick.
        tick: u64,
        /// Prop index.
        prop: usize,
        /// Direction, `f32` bits.
        dir: [u32; 3],
        /// Point of application, `f32` bits.
        at: [u32; 3],
        /// Magnitude, `f32` bits.
        impulse: u32,
    },
}

impl Entry {
    /// The tick this entry was applied before.
    pub fn tick(&self) -> u64 {
        match self {
            Entry::Join { tick, .. } | Entry::Leave { tick, .. } | Entry::Input { tick, .. } | Entry::Impulse { tick, .. } => *tick,
        }
    }
}

/// State checksums after a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checkpoint {
    /// Ticks completed.
    pub tick: u64,
    /// Bit-exact hash of every player.
    pub players: u64,
    /// Bit-exact hash of every dynamic prop.
    pub props: u64,
    /// Hash of the rules state.
    pub rules: u64,
    /// The same state with floats quantised to millimetres: equal across platforms unless the game really diverged.
    pub coarse: u64,
}

impl Checkpoint {
    /// The three exact hashes folded into one.
    pub fn combined(&self) -> u64 {
        (self.players.rotate_left(1) ^ self.props.rotate_left(21) ^ self.rules.rotate_left(42)).wrapping_mul(0x0000_0100_0000_01b3)
    }
}

/// An authoritative game event (`emit`, `end:<outcome>`).
#[derive(Debug, Clone, PartialEq)]
pub struct TraceEvent {
    /// Tick.
    pub tick: u64,
    /// The rule that produced it.
    pub rule: String,
    /// The event name.
    pub name: String,
    /// The player that triggered it.
    pub slot: Option<usize>,
}

/// A full readable snapshot of the state at a tick (for the diff a divergence report prints).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dump {
    /// Ticks completed.
    pub tick: u64,
    /// `[slot, x, foot_y, z, yaw, pitch, vy]` per player.
    pub players: Vec<[f64; 7]>,
    /// `[prop, x, y, z, qx, qy, qz, qw]` per dynamic prop.
    pub props: Vec<[f64; 8]>,
    /// The scene's rule variables.
    pub vars: Vec<(String, f64)>,
    /// Hidden objects.
    pub hidden: Vec<String>,
    /// Top-level objects whose authored collision is disabled.
    pub collision_disabled: Vec<String>,
    /// The match outcome once ended.
    pub ended: Option<String>,
}

/// A whole recording.
#[derive(Debug, Clone, PartialEq)]
pub struct Trace {
    /// What it is.
    pub header: Header,
    /// Inputs to the simulation, in the order they were applied.
    pub entries: Vec<Entry>,
    /// Checksums, in tick order.
    pub checkpoints: Vec<Checkpoint>,
    /// Game events, in order.
    pub events: Vec<TraceEvent>,
    /// State dumps, in tick order.
    pub dumps: Vec<Dump>,
    /// Ticks the recorded match ran.
    pub final_tick: u64,
}

impl Header {
    /// A header for a match on this machine with the given map/seed/spawn group.
    pub fn new(map_hash: u32, seed: u64, spawn_group: &str, checkpoint_every: u32, dump_every: u32) -> Header {
        Header {
            version: TRACE_VERSION,
            engine: env!("CARGO_PKG_VERSION").to_string(),
            tick_rate: super::clock::TICK_RATE_HZ,
            scene: String::new(),
            map_hash,
            seed,
            spawn_group: spawn_group.to_string(),
            platform: format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
            checkpoint_every: checkpoint_every.max(1),
            dump_every,
        }
    }
}

impl Trace {
    /// An empty trace for a match that is about to start.
    pub fn new(header: Header) -> Trace {
        Trace { header, entries: Vec::new(), checkpoints: Vec::new(), events: Vec::new(), dumps: Vec::new(), final_tick: 0 }
    }

    /// The trace as JSON (see the module docs for the layout).
    pub fn to_json(&self) -> Value {
        let h = &self.header;
        let entries: Vec<Value> = self
            .entries
            .iter()
            .map(|e| match e {
                Entry::Join { tick, slot, character, state } => json!(["j", tick, slot, character, state]),
                Entry::Leave { tick, slot } => json!(["l", tick, slot]),
                Entry::Input { tick, slot, input: i } => {
                    json!(["i", tick, slot, i.seq, i.forward, i.strafe, i.flags(), i.yaw.to_bits(), i.pitch.to_bits()])
                }
                Entry::Impulse { tick, prop, dir, at, impulse } => json!(["p", tick, prop, dir, at, impulse]),
            })
            .collect();
        json!({
            "trace_version": h.version,
            "engine": h.engine,
            "tick_rate": h.tick_rate,
            "scene": h.scene,
            "map_hash": h.map_hash,
            "seed": h.seed.to_string(),
            "spawn_group": h.spawn_group,
            "platform": h.platform,
            "checkpoint_every": h.checkpoint_every,
            "dump_every": h.dump_every,
            "final_tick": self.final_tick,
            "entries": entries,
            "checkpoints": self.checkpoints.iter().map(|c| json!([c.tick, hex(c.players), hex(c.props), hex(c.rules), hex(c.coarse)])).collect::<Vec<_>>(),
            "events": self.events.iter().map(|e| json!([e.tick, e.rule, e.name, e.slot])).collect::<Vec<_>>(),
            "dumps": self.dumps.iter().map(|d| json!({
                "tick": d.tick, "players": d.players, "props": d.props, "vars": d.vars, "hidden": d.hidden,
                "collision_disabled": d.collision_disabled, "ended": d.ended,
            })).collect::<Vec<_>>(),
        })
    }

    /// Parses a trace written by [`Trace::to_json`]; the error says what is wrong and where.
    pub fn from_json(v: &Value) -> Result<Trace, String> {
        let num = |k: &str| v.get(k).and_then(Value::as_u64).ok_or_else(|| format!("trace.{k}: missing or not a whole number"));
        let version = num("trace_version")? as u32;
        if version != TRACE_VERSION {
            return Err(format!("trace.trace_version: {version} is not supported (this engine reads version {TRACE_VERSION})"));
        }
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("trace.{k}: missing or not a string"));
        let header = Header {
            version,
            engine: text("engine")?,
            tick_rate: num("tick_rate")? as u32,
            scene: v.get("scene").and_then(Value::as_str).unwrap_or("").to_string(),
            map_hash: num("map_hash")? as u32,
            seed: text("seed")?.parse().map_err(|_| "trace.seed: not a number".to_string())?,
            spawn_group: text("spawn_group")?,
            platform: text("platform")?,
            checkpoint_every: num("checkpoint_every")? as u32,
            dump_every: num("dump_every")? as u32,
        };
        let arr = |k: &str| v.get(k).and_then(Value::as_array).ok_or_else(|| format!("trace.{k}: missing or not an array"));
        let mut entries = Vec::new();
        for (n, e) in arr("entries")?.iter().enumerate() {
            entries.push(parse_entry(e).ok_or_else(|| format!("trace.entries[{n}]: malformed entry {e}"))?);
        }
        let mut checkpoints = Vec::new();
        for (n, c) in arr("checkpoints")?.iter().enumerate() {
            let cp = (|| {
                let a = c.as_array().filter(|a| a.len() == 5)?;
                Some(Checkpoint { tick: a[0].as_u64()?, players: unhex(&a[1])?, props: unhex(&a[2])?, rules: unhex(&a[3])?, coarse: unhex(&a[4])? })
            })();
            checkpoints.push(cp.ok_or_else(|| format!("trace.checkpoints[{n}]: malformed checkpoint {c}"))?);
        }
        let mut events = Vec::new();
        for (n, e) in arr("events")?.iter().enumerate() {
            let ev = (|| {
                let a = e.as_array().filter(|a| a.len() == 4)?;
                Some(TraceEvent {
                    tick: a[0].as_u64()?,
                    rule: a[1].as_str()?.to_string(),
                    name: a[2].as_str()?.to_string(),
                    slot: a[3].as_u64().map(|s| s as usize),
                })
            })();
            events.push(ev.ok_or_else(|| format!("trace.events[{n}]: malformed event {e}"))?);
        }
        let mut dumps = Vec::new();
        for (n, d) in arr("dumps")?.iter().enumerate() {
            dumps.push(parse_dump(d).ok_or_else(|| format!("trace.dumps[{n}]: malformed dump"))?);
        }
        Ok(Trace { header, entries, checkpoints, events, dumps, final_tick: num("final_tick")? })
    }
}

fn hex(v: u64) -> String {
    format!("{v:016x}")
}

fn unhex(v: &Value) -> Option<u64> {
    u64::from_str_radix(v.as_str()?, 16).ok()
}

fn bits3(v: &Value) -> Option<[u32; 3]> {
    let a = v.as_array().filter(|a| a.len() == 3)?;
    Some([a[0].as_u64()? as u32, a[1].as_u64()? as u32, a[2].as_u64()? as u32])
}

fn parse_entry(e: &Value) -> Option<Entry> {
    let a = e.as_array()?;
    let u = |i: usize| a.get(i).and_then(Value::as_u64);
    let i = |i: usize| a.get(i).and_then(Value::as_i64);
    match a.first()?.as_str()? {
        "j" => {
            let s = a.get(4)?.as_array().filter(|s| s.len() == 6)?;
            let mut state = [0u32; 6];
            for (k, x) in s.iter().enumerate() {
                state[k] = x.as_u64()? as u32;
            }
            Some(Entry::Join { tick: u(1)?, slot: u(2)? as usize, character: u(3)? as u8, state })
        }
        "l" => Some(Entry::Leave { tick: u(1)?, slot: u(2)? as usize }),
        "i" => {
            let flags = u(6)?;
            Some(Entry::Input {
                tick: u(1)?,
                slot: u(2)? as usize,
                input: PlayerInput {
                    seq: u(3)? as u32,
                    forward: i(4)? as i8,
                    strafe: i(5)? as i8,
                    yaw: f32::from_bits(u(7)? as u32),
                    pitch: f32::from_bits(u(8)? as u32),
                    ..Default::default()
                }
                .with_flags(flags as u8),
            })
        }
        "p" => Some(Entry::Impulse { tick: u(1)?, prop: u(2)? as usize, dir: bits3(a.get(3)?)?, at: bits3(a.get(4)?)?, impulse: u(5)? as u32 }),
        _ => None,
    }
}

fn parse_dump(d: &Value) -> Option<Dump> {
    let rows = |k: &str, n: usize| -> Option<Vec<Vec<f64>>> {
        d.get(k)?.as_array()?.iter().map(|r| r.as_array().filter(|r| r.len() == n)?.iter().map(Value::as_f64).collect()).collect()
    };
    let players = rows("players", 7)?.into_iter().map(|r| [r[0], r[1], r[2], r[3], r[4], r[5], r[6]]).collect();
    let props = rows("props", 8)?.into_iter().map(|r| [r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7]]).collect();
    let vars = d.get("vars")?.as_array()?.iter().map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.as_f64()?))).collect::<Option<Vec<_>>>()?;
    let hidden = d.get("hidden")?.as_array()?.iter().map(|s| s.as_str().map(str::to_string)).collect::<Option<Vec<_>>>()?;
    let collision_disabled = match d.get("collision_disabled") {
        None => Vec::new(),
        Some(value) => value.as_array()?.iter().map(|s| s.as_str().map(str::to_string)).collect::<Option<Vec<_>>>()?,
    };
    Some(Dump {
        tick: d.get("tick")?.as_u64()?,
        players,
        props,
        vars,
        hidden,
        collision_disabled,
        ended: d.get("ended").and_then(Value::as_str).map(str::to_string),
    })
}

/// A compact human-readable difference between two dumps (`a` = what was recorded, `b` = what a replay computed).
/// Empty when they are the same.
pub fn diff_dumps(a: &Dump, b: &Dump) -> Vec<String> {
    let mut out = Vec::new();
    let close = |x: f64, y: f64| x.to_bits() == y.to_bits();
    for p in &a.players {
        match b.players.iter().find(|q| q[0] == p[0]) {
            None => out.push(format!("player {} exists in the recording but not in the replay", p[0])),
            Some(q) => {
                if !p.iter().zip(q).all(|(x, y)| close(*x, *y)) {
                    out.push(format!(
                        "player {}: pos ({:.4}, {:.4}, {:.4}) vs ({:.4}, {:.4}, {:.4}), yaw {:.4} vs {:.4}, vy {:.4} vs {:.4}",
                        p[0], p[1], p[2], p[3], q[1], q[2], q[3], p[4], q[4], p[6], q[6]
                    ));
                }
            }
        }
    }
    for q in b.players.iter().filter(|q| !a.players.iter().any(|p| p[0] == q[0])) {
        out.push(format!("player {} exists in the replay but not in the recording", q[0]));
    }
    for p in &a.props {
        match b.props.iter().find(|q| q[0] == p[0]) {
            None => out.push(format!("prop {} is moving in the recording but was never disturbed in the replay", p[0])),
            Some(q) => {
                if !p.iter().zip(q).all(|(x, y)| close(*x, *y)) {
                    let d = ((p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2) + (p[3] - q[3]).powi(2)).sqrt();
                    out.push(format!(
                        "prop {}: pos ({:.4}, {:.4}, {:.4}) vs ({:.4}, {:.4}, {:.4}) — {:.4} m apart",
                        p[0], p[1], p[2], p[3], q[1], q[2], q[3], d
                    ));
                }
            }
        }
    }
    for q in b.props.iter().filter(|q| !a.props.iter().any(|p| p[0] == q[0])) {
        out.push(format!("prop {} is moving in the replay but was never disturbed in the recording", q[0]));
    }
    for (name, x) in &a.vars {
        match b.vars.iter().find(|(n, _)| n == name) {
            Some((_, y)) if close(*x, *y) => {}
            Some((_, y)) => out.push(format!("var {name}: {x} vs {y}")),
            None => out.push(format!("var {name} is missing in the replay")),
        }
    }
    if a.hidden != b.hidden {
        out.push(format!("hidden objects: {:?} vs {:?}", a.hidden, b.hidden));
    }
    if a.collision_disabled != b.collision_disabled {
        out.push(format!("collision-disabled objects: {:?} vs {:?}", a.collision_disabled, b.collision_disabled));
    }
    if a.ended != b.ended {
        out.push(format!("outcome: {:?} vs {:?}", a.ended, b.ended));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Trace {
        let mut t = Trace::new(Header::new(0xdead_beef, u64::MAX, "duel", 1, 60));
        t.entries.push(Entry::Join { tick: 0, slot: 0, character: 1, state: [1, 2, 3, 4, 5, 6] });
        t.entries.push(Entry::Input {
            tick: 3,
            slot: 0,
            input: PlayerInput {
                seq: 7,
                forward: 1,
                strafe: -1,
                jump: true,
                sprint: false,
                crouch: true,
                yaw: 1.234_567_9,
                pitch: -0.25,
                interact: true,
                attack: false,
                reload: true,
                switch_weapon: true,
            },
        });
        t.entries.push(Entry::Impulse { tick: 4, prop: 2, dir: [1, 2, 3], at: [4, 5, 6], impulse: 7 });
        t.entries.push(Entry::Leave { tick: 9, slot: 0 });
        t.checkpoints.push(Checkpoint { tick: 1, players: u64::MAX, props: 0, rules: 0x1234, coarse: 99 });
        t.events.push(TraceEvent { tick: 5, rule: "take".into(), name: "coin".into(), slot: Some(0) });
        t.events.push(TraceEvent { tick: 6, rule: "t".into(), name: "end:x".into(), slot: None });
        t.dumps.push(Dump {
            tick: 60,
            players: vec![[0.0, 1.5, 0.0, -2.0, 0.3, 0.0, 0.0]],
            props: vec![[3.0, 1.0, 0.5, 2.0, 0.0, 0.0, 0.0, 1.0]],
            vars: vec![("score".into(), 2.0)],
            hidden: vec!["coin".into()],
            collision_disabled: vec!["gate".into()],
            ended: Some("victory".into()),
        });
        t.final_tick = 120;
        t
    }

    #[test]
    fn a_trace_survives_a_json_round_trip_bit_for_bit() {
        let t = sample();
        let text = serde_json::to_string(&t.to_json()).unwrap_or_default();
        let back = Trace::from_json(&serde_json::from_str(&text).unwrap_or(Value::Null)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, t);
    }

    #[test]
    fn a_damaged_trace_says_what_is_wrong_and_where() {
        let mut v = sample().to_json();
        v["entries"][1] = json!(["i", 1]);
        assert!(Trace::from_json(&v).unwrap_err().starts_with("trace.entries[1]: malformed entry"));
        let mut v = sample().to_json();
        v["trace_version"] = json!(99);
        assert!(Trace::from_json(&v).unwrap_err().contains("99 is not supported"));
        assert!(Trace::from_json(&json!({})).unwrap_err().contains("trace.trace_version"));
    }

    #[test]
    fn dump_diffs_name_the_entity_and_the_field() {
        let a = sample().dumps[0].clone();
        let mut b = a.clone();
        assert!(diff_dumps(&a, &b).is_empty());
        b.players[0][1] += 0.5;
        b.props[0][2] += 0.25;
        b.vars[0].1 = 3.0;
        b.hidden.clear();
        let d = diff_dumps(&a, &b).join("\n");
        assert!(d.contains("player 0: pos (1.5000") && d.contains("prop 3: pos") && d.contains("var score: 2 vs 3") && d.contains("hidden objects"), "{d}");
    }
}
