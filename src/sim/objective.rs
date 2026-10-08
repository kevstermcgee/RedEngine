//! Objective modes of the loadout shooter: capture the flag and search and destroy (ADR 2026-10-07-killchain-game-modes-free-for-all-capture-the-flag).
//!
//! Both are pure state machines. Each tick the match hands them a list of [`Actor`]s (where everybody is, who is alive, who holds Interact)
//! and they answer with [`Cmd`]s for the match to carry out (revive everyone for a new round, set off the bomb). They know nothing of sockets,
//! physics or weapons, so every rule is unit-tested below without a world.
//!
//! * **Capture the flag.** One flag per team at its base. An enemy touching it takes it; the carrier scores by touching their own flag
//!   while it is at home. A dead carrier drops the flag where they fell; a teammate touching it sends it home at once, an enemy takes it
//!   again, and it returns by itself after a while.
//! * **Search and destroy.** One life per round. The attackers carry a bomb to a site and plant it by holding Interact; the defenders stop
//!   them, or defuse it by holding Interact next to it. A round ends when the bomb goes off or is defused, a side is eliminated or time runs
//!   out. Sides swap partway through the match.

use crate::strict::check_keys;
use glam::Vec3;
use serde_json::{Map, Value};

/// How close (metres, horizontally) a player must be to touch a flag or pick up the bomb.
pub const TOUCH_RADIUS: f32 = 1.6;
/// How far above or below a flag a player may be and still touch it.
pub const TOUCH_HEIGHT: f32 = 2.6;
/// How close a defender must stay to the planted bomb to defuse it.
pub const DEFUSE_RADIUS: f32 = 2.0;
/// Radius (metres) of the planted bomb's blast.
pub const BLAST_RADIUS: f32 = 14.0;

/// Keys of the `shooter.objective` block.
pub const OBJECTIVE_KEYS: &[&str] =
    &["capture_limit", "return_secs", "win_rounds", "swap_after", "round_secs", "freeze_secs", "plant_secs", "defuse_secs", "fuse_secs"];
const FLAG_KEYS: &[&str] = &["team", "at"];
const SITE_KEYS: &[&str] = &["name", "at", "radius"];

/// Most bomb sites a map may have.
pub const MAX_SITES: usize = 3;

/// A team's flag base.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlagSpec {
    /// The team that owns the flag (`1` or `2`).
    pub team: u8,
    /// Where it stands (the ground under the pole).
    pub at: Vec3,
}

/// A place the bomb can be planted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SiteSpec {
    /// Its letter on the HUD (`A`, `B`, `C`).
    pub name: char,
    /// Its centre (on the ground).
    pub at: Vec3,
    /// How far from the centre the bomb may be planted.
    pub radius: f32,
}

/// The parsed objective tunables, with their defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectiveConfig {
    /// Capture the flag: captures that win the match.
    pub capture_limit: u32,
    /// Capture the flag: seconds a dropped flag lies before it goes home by itself.
    pub return_secs: f32,
    /// Search and destroy: rounds that win the match.
    pub win_rounds: u32,
    /// Search and destroy: rounds played before the sides swap.
    pub swap_after: u32,
    /// Search and destroy: length of a round once it is live.
    pub round_secs: f32,
    /// Search and destroy: seconds everyone is held at their spawn before a round.
    pub freeze_secs: f32,
    /// Search and destroy: seconds the planter holds Interact.
    pub plant_secs: f32,
    /// Search and destroy: seconds a defender holds Interact.
    pub defuse_secs: f32,
    /// Search and destroy: seconds from the plant to the blast.
    pub fuse_secs: f32,
    /// Flag bases (capture the flag).
    pub flags: Vec<FlagSpec>,
    /// Bomb sites (search and destroy).
    pub sites: Vec<SiteSpec>,
}

impl Default for ObjectiveConfig {
    fn default() -> Self {
        ObjectiveConfig {
            capture_limit: 3,
            return_secs: 15.0,
            win_rounds: 4,
            swap_after: 3,
            round_secs: 100.0,
            freeze_secs: 5.0,
            plant_secs: 3.0,
            defuse_secs: 5.0,
            fuse_secs: 40.0,
            flags: Vec::new(),
            sites: Vec::new(),
        }
    }
}

fn vec3(v: &Value) -> Option<Vec3> {
    let a = v.as_array().filter(|a| a.len() == 3)?;
    let n: Vec<f32> = a.iter().filter_map(Value::as_f64).map(|x| x as f32).collect();
    (n.len() == 3 && n.iter().all(|x| x.is_finite())).then(|| Vec3::new(n[0], n[1], n[2]))
}

/// Reads the objective parts of a `shooter` block (`flags`, `sites`, `objective`), pushing readable errors.
pub fn parse_objective(o: &Map<String, Value>, errs: &mut Vec<String>) -> ObjectiveConfig {
    let mut cfg = ObjectiveConfig::default();
    if let Some(v) = o.get("objective") {
        match v.as_object() {
            Some(ob) => {
                check_keys(errs, "shooter.objective", ob, OBJECTIVE_KEYS);
                let mut num = |key: &str, lo: f64, hi: f64| -> Option<f64> {
                    let v = ob.get(key)?;
                    match v.as_f64().filter(|n| (lo..=hi).contains(n)) {
                        Some(n) => Some(n),
                        None => {
                            errs.push(format!("shooter.objective.{key}: must be a number from {lo} to {hi}"));
                            None
                        }
                    }
                };
                if let Some(n) = num("capture_limit", 1.0, 50.0) {
                    cfg.capture_limit = n as u32;
                }
                if let Some(n) = num("return_secs", 1.0, 120.0) {
                    cfg.return_secs = n as f32;
                }
                if let Some(n) = num("win_rounds", 1.0, 20.0) {
                    cfg.win_rounds = n as u32;
                }
                if let Some(n) = num("swap_after", 1.0, 20.0) {
                    cfg.swap_after = n as u32;
                }
                if let Some(n) = num("round_secs", 20.0, 600.0) {
                    cfg.round_secs = n as f32;
                }
                if let Some(n) = num("freeze_secs", 0.0, 30.0) {
                    cfg.freeze_secs = n as f32;
                }
                if let Some(n) = num("plant_secs", 0.5, 15.0) {
                    cfg.plant_secs = n as f32;
                }
                if let Some(n) = num("defuse_secs", 0.5, 30.0) {
                    cfg.defuse_secs = n as f32;
                }
                if let Some(n) = num("fuse_secs", 5.0, 120.0) {
                    cfg.fuse_secs = n as f32;
                }
            }
            None => errs.push("shooter.objective: must be an object like {\"capture_limit\": 3}".to_string()),
        }
    }
    if let Some(v) = o.get("flags") {
        match v.as_array() {
            Some(list) if list.len() <= 2 => {
                for (i, f) in list.iter().enumerate() {
                    let path = format!("shooter.flags[{i}]");
                    let Some(fo) = f.as_object() else {
                        errs.push(format!("{path}: must be an object like {{\"team\": 1, \"at\": [x, y, z]}}"));
                        continue;
                    };
                    check_keys(errs, &path, fo, FLAG_KEYS);
                    let team = fo.get("team").and_then(Value::as_u64).filter(|t| (1..=2).contains(t));
                    let at = fo.get("at").and_then(vec3);
                    match (team, at) {
                        (Some(team), Some(at)) => cfg.flags.push(FlagSpec { team: team as u8, at }),
                        _ => errs.push(format!("{path}: needs \"team\" (1 or 2) and \"at\" [x, y, z]")),
                    }
                }
                if cfg.flags.len() == 2 && cfg.flags[0].team == cfg.flags[1].team {
                    errs.push("shooter.flags: the two flags must belong to different teams".to_string());
                }
            }
            _ => errs.push("shooter.flags: must be a list of at most 2 flags (one per team)".to_string()),
        }
    }
    if let Some(v) = o.get("sites") {
        match v.as_array() {
            Some(list) if list.len() <= MAX_SITES => {
                for (i, s) in list.iter().enumerate() {
                    let path = format!("shooter.sites[{i}]");
                    let Some(so) = s.as_object() else {
                        errs.push(format!("{path}: must be an object like {{\"name\": \"A\", \"at\": [x, y, z], \"radius\": 4}}"));
                        continue;
                    };
                    check_keys(errs, &path, so, SITE_KEYS);
                    let name = so
                        .get("name")
                        .and_then(Value::as_str)
                        .and_then(|n| n.chars().next())
                        .filter(char::is_ascii_uppercase)
                        .unwrap_or((b'A' + i as u8) as char);
                    let radius = so.get("radius").and_then(Value::as_f64).unwrap_or(4.0) as f32;
                    match so.get("at").and_then(vec3) {
                        Some(at) if (1.0..=12.0).contains(&radius) => cfg.sites.push(SiteSpec { name, at, radius }),
                        Some(_) => errs.push(format!("{path}.radius: must be 1 to 12 metres")),
                        None => errs.push(format!("{path}.at: must be [x, y, z]")),
                    }
                }
            }
            _ => errs.push(format!("shooter.sites: must be a list of at most {MAX_SITES} bomb sites")),
        }
    }
    cfg
}

/// What the objective modes need to know about one player this tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Actor {
    /// The player's slot.
    pub slot: u8,
    /// Their team (`1` or `2`).
    pub team: u8,
    /// Where they stand (x, ground height, z).
    pub pos: Vec3,
    /// Whether they are alive.
    pub alive: bool,
    /// Whether they hold Interact.
    pub interact: bool,
}

/// What the match must do because of an objective rule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmd {
    /// Put everyone back at a spawn with a fresh kit and clear the floor (a new search and destroy round).
    NewRound,
    /// The bomb goes off here: hurt everyone near it and show the blast.
    Explode(Vec3),
}

/// The kind of an [`ObjEvent`] (also its wire byte).
pub mod ev {
    /// A flag was picked up (`team` = the flag's team, `slot` = the taker).
    pub const FLAG_TAKEN: u8 = 1;
    /// A carrier lost the flag.
    pub const FLAG_DROPPED: u8 = 2;
    /// A flag went home (`slot` = who touched it, `255` = it timed out).
    pub const FLAG_RETURNED: u8 = 3;
    /// A flag was captured (`team` = the scoring team).
    pub const FLAG_CAPTURED: u8 = 4;
    /// Somebody picked the bomb up.
    pub const BOMB_PICKED: u8 = 5;
    /// The bomb carrier died or left.
    pub const BOMB_DROPPED: u8 = 6;
    /// The bomb was planted.
    pub const BOMB_PLANTED: u8 = 7;
    /// The bomb was defused.
    pub const BOMB_DEFUSED: u8 = 8;
    /// The bomb went off.
    pub const BOMB_EXPLODED: u8 = 9;
    /// A round went live.
    pub const ROUND_LIVE: u8 = 10;
    /// A round was won (`team` = the winner, `slot` = how: see [`super::WinHow`]).
    pub const ROUND_WON: u8 = 11;
}

/// A plain-words line for an objective event (the server log; the client words it its own way).
pub fn describe(kind: u8, team: u8, slot: u8) -> String {
    let who = if slot == 255 { String::new() } else { format!(" (player {slot})") };
    match kind {
        ev::FLAG_TAKEN => format!("team {team}'s flag taken{who}"),
        ev::FLAG_DROPPED => format!("team {team}'s flag dropped{who}"),
        ev::FLAG_RETURNED => format!("team {team}'s flag returned{who}"),
        ev::FLAG_CAPTURED => format!("team {team} captured a flag{who}"),
        ev::BOMB_PICKED => format!("bomb picked up{who}"),
        ev::BOMB_DROPPED => format!("bomb dropped{who}"),
        ev::BOMB_PLANTED => format!("bomb planted{who}"),
        ev::BOMB_DEFUSED => format!("bomb defused{who}"),
        ev::BOMB_EXPLODED => "bomb exploded".to_string(),
        ev::ROUND_LIVE => format!("round live; team {team} attacks"),
        ev::ROUND_WON => {
            format!("round won by team {team} ({})", ["bomb exploded", "bomb defused", "eliminated", "time up"].get(slot as usize).copied().unwrap_or("?"))
        }
        _ => format!("objective event {kind}"),
    }
}

/// Something that happened, kept for a few snapshots so a client that missed one still hears it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObjEvent {
    /// Unique, growing id.
    pub id: u16,
    /// What happened (see [`ev`]).
    pub kind: u8,
    /// The team it concerns.
    pub team: u8,
    /// The player it concerns (`255` = nobody).
    pub slot: u8,
}

/// How many events are kept.
pub const EVENT_LOG: usize = 4;

/// How a search and destroy round was won (carried in [`ObjEvent::slot`] of a `ROUND_WON`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinHow {
    /// The bomb went off.
    Exploded = 0,
    /// The bomb was defused.
    Defused = 1,
    /// One side was eliminated.
    Eliminated = 2,
    /// Time ran out with no bomb planted.
    TimeUp = 3,
}

/// Where a flag is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FlagState {
    /// At its base.
    Home,
    /// Carried by `slot`, who was last seen at `pos`.
    Carried {
        /// The carrier.
        slot: u8,
        /// Where the carrier was last seen.
        pos: Vec3,
    },
    /// Lying where a carrier fell.
    Dropped {
        /// Where.
        pos: Vec3,
        /// The tick it goes home by itself.
        return_at: u64,
    },
}

/// One flag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flag {
    /// The team that owns it.
    pub team: u8,
    /// Its base.
    pub home: Vec3,
    /// Where it is.
    pub state: FlagState,
}

impl Flag {
    /// Where the flag is right now (for drawing).
    pub fn pos(&self) -> Vec3 {
        match self.state {
            FlagState::Home => self.home,
            FlagState::Carried { pos, .. } | FlagState::Dropped { pos, .. } => pos,
        }
    }
}

fn near(a: Vec3, b: Vec3) -> bool {
    let d = a - b;
    d.x * d.x + d.z * d.z <= TOUCH_RADIUS * TOUCH_RADIUS && d.y.abs() <= TOUCH_HEIGHT
}

/// Capture the flag.
#[derive(Debug, Clone, PartialEq)]
pub struct CtfState {
    /// The two flags (index = team - 1).
    pub flags: [Flag; 2],
    return_ticks: u64,
}

impl CtfState {
    /// Both flags at home. `None` unless the config has one flag per team.
    pub fn new(cfg: &ObjectiveConfig, tick_hz: u32) -> Option<CtfState> {
        let find = |t: u8| cfg.flags.iter().find(|f| f.team == t).map(|f| Flag { team: t, home: f.at, state: FlagState::Home });
        Some(CtfState { flags: [find(1)?, find(2)?], return_ticks: (cfg.return_secs * tick_hz as f32).round() as u64 })
    }

    /// One tick. Returns captures as `(team, event)`; events are appended to `events`.
    pub fn step(&mut self, tick: u64, actors: &[Actor], points: &mut [u32; 2], events: &mut Vec<(u8, u8, u8)>) {
        for fi in 0..2 {
            let team = fi as u8 + 1;
            let flag = self.flags[fi];
            match flag.state {
                FlagState::Home => {
                    if let Some(a) = actors.iter().find(|a| a.alive && a.team != team && near(a.pos, flag.home)) {
                        self.flags[fi].state = FlagState::Carried { slot: a.slot, pos: a.pos };
                        events.push((ev::FLAG_TAKEN, team, a.slot));
                    }
                }
                FlagState::Dropped { pos, return_at } => {
                    if let Some(a) = actors.iter().find(|a| a.alive && a.team == team && near(a.pos, pos)) {
                        self.flags[fi].state = FlagState::Home;
                        events.push((ev::FLAG_RETURNED, team, a.slot));
                    } else if tick >= return_at {
                        self.flags[fi].state = FlagState::Home;
                        events.push((ev::FLAG_RETURNED, team, 255));
                    } else if let Some(a) = actors.iter().find(|a| a.alive && a.team != team && near(a.pos, pos)) {
                        self.flags[fi].state = FlagState::Carried { slot: a.slot, pos: a.pos };
                        events.push((ev::FLAG_TAKEN, team, a.slot));
                    }
                }
                FlagState::Carried { slot, pos } => match actors.iter().find(|a| a.slot == slot && a.alive) {
                    Some(a) => self.flags[fi].state = FlagState::Carried { slot, pos: a.pos },
                    None => {
                        self.flags[fi].state = FlagState::Dropped { pos, return_at: tick + self.return_ticks };
                        events.push((ev::FLAG_DROPPED, team, slot));
                    }
                },
            }
        }
        // A capture: carry the enemy flag onto your own base while your own flag is home.
        for a in actors.iter().filter(|a| a.alive && (1..=2).contains(&a.team)) {
            let (mine, theirs) = ((a.team - 1) as usize, (2 - a.team) as usize);
            let carrying = matches!(self.flags[theirs].state, FlagState::Carried { slot, .. } if slot == a.slot);
            if carrying && self.flags[mine].state == FlagState::Home && near(a.pos, self.flags[mine].home) {
                self.flags[theirs].state = FlagState::Home;
                points[mine] += 1;
                events.push((ev::FLAG_CAPTURED, a.team, a.slot));
            }
        }
    }

    /// The flag `slot` is carrying, if any (a team number).
    pub fn carried_by(&self, slot: u8) -> Option<u8> {
        self.flags.iter().find(|f| matches!(f.state, FlagState::Carried { slot: s, .. } if s == slot)).map(|f| f.team)
    }
}

/// Where the bomb is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BombState {
    /// Carried by an attacker, last seen at `pos`.
    Carried {
        /// The carrier.
        slot: u8,
        /// Where they were last seen.
        pos: Vec3,
    },
    /// Lying on the ground.
    Dropped(Vec3),
    /// Ticking.
    Planted {
        /// Where.
        pos: Vec3,
        /// Which site (index into the config's sites).
        site: u8,
        /// The tick it goes off.
        explode_at: u64,
    },
    /// Defused: the round is over.
    Defused(Vec3),
    /// Went off: the round is over.
    Exploded(Vec3),
}

/// Where a search and destroy round is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RoundPhase {
    /// Everyone is held at their spawn until `until`.
    Freeze {
        /// The tick it goes live.
        until: u64,
    },
    /// Being played; ends at `until` if nothing else ends it.
    Live {
        /// The tick time runs out.
        until: u64,
    },
    /// Decided; the next round starts at `until`.
    Over {
        /// The tick the next round starts.
        until: u64,
        /// The winning team.
        winner: u8,
    },
}

/// Search and destroy.
#[derive(Debug, Clone, PartialEq)]
pub struct SndState {
    /// Where the round is.
    pub phase: RoundPhase,
    /// The round number, from 1.
    pub round: u8,
    /// The team attacking this round (`1` or `2`).
    pub attackers: u8,
    /// The bomb.
    pub bomb: BombState,
    /// Who is planting and since when.
    pub planting: Option<(u8, u64)>,
    /// Who is defusing and since when.
    pub defusing: Option<(u8, u64)>,
    /// How many players each side had when the round went live (elimination only counts a side that started with someone).
    started_with: [u8; 2],
    /// Rounds finished so far.
    pub rounds_done: u32,
    freeze_ticks: u64,
    round_ticks: u64,
    plant_ticks: u64,
    defuse_ticks: u64,
    fuse_ticks: u64,
    over_ticks: u64,
    swap_after: u32,
    sites: Vec<SiteSpec>,
}

const OVER_SECS: f32 = 4.0;

impl SndState {
    /// A match about to start its first round. `None` without a bomb site.
    pub fn new(cfg: &ObjectiveConfig, tick_hz: u32) -> Option<SndState> {
        if cfg.sites.is_empty() {
            return None;
        }
        let t = |s: f32| (s * tick_hz as f32).round().max(1.0) as u64;
        Some(SndState {
            phase: RoundPhase::Freeze { until: t(cfg.freeze_secs) },
            round: 1,
            attackers: 1,
            bomb: BombState::Dropped(Vec3::ZERO),
            planting: None,
            defusing: None,
            started_with: [0; 2],
            rounds_done: 0,
            freeze_ticks: t(cfg.freeze_secs),
            round_ticks: t(cfg.round_secs),
            plant_ticks: t(cfg.plant_secs),
            defuse_ticks: t(cfg.defuse_secs),
            fuse_ticks: t(cfg.fuse_secs),
            over_ticks: t(OVER_SECS),
            swap_after: cfg.swap_after,
            sites: cfg.sites.clone(),
        })
    }

    /// The team defending this round.
    pub fn defenders(&self) -> u8 {
        3 - self.attackers
    }

    /// Whether players are held in place (the freeze before a round).
    pub fn locked(&self) -> bool {
        matches!(self.phase, RoundPhase::Freeze { .. })
    }

    /// Whether the dead come back (never during a round: one life).
    pub fn respawns_allowed(&self) -> bool {
        false
    }

    /// The bomb sites.
    pub fn sites(&self) -> &[SiteSpec] {
        &self.sites
    }

    /// Plant progress (0-255) of whoever is planting, or defuse progress, for the HUD.
    pub fn progress(&self, tick: u64) -> u8 {
        let frac = |start: u64, total: u64| ((tick.saturating_sub(start)) * 255 / total.max(1)).min(255) as u8;
        match (self.planting, self.defusing) {
            (Some((_, s)), _) => frac(s, self.plant_ticks),
            (_, Some((_, s))) => frac(s, self.defuse_ticks),
            _ => 0,
        }
    }

    fn site_at(&self, p: Vec3) -> Option<u8> {
        self.sites
            .iter()
            .position(|s| {
                let d = p - s.at;
                d.x * d.x + d.z * d.z <= s.radius * s.radius && d.y.abs() <= TOUCH_HEIGHT
            })
            .map(|i| i as u8)
    }

    fn start_round(&mut self, tick: u64, actors: &[Actor], cmds: &mut Vec<Cmd>) {
        cmds.push(Cmd::NewRound);
        self.phase = RoundPhase::Freeze { until: tick + self.freeze_ticks };
        self.planting = None;
        self.defusing = None;
        // The bomb starts with the lowest-numbered attacker (the match revives everyone first, so all are alive).
        self.bomb = match actors.iter().filter(|a| a.team == self.attackers).map(|a| a.slot).min() {
            Some(slot) => {
                let pos = actors.iter().find(|a| a.slot == slot).map_or(Vec3::ZERO, |a| a.pos);
                BombState::Carried { slot, pos }
            }
            None => BombState::Dropped(Vec3::ZERO),
        };
    }

    fn end_round(&mut self, tick: u64, winner: u8, how: WinHow, points: &mut [u32; 2], events: &mut Vec<(u8, u8, u8)>) {
        points[(winner - 1) as usize] += 1;
        self.rounds_done += 1;
        self.phase = RoundPhase::Over { until: tick + self.over_ticks, winner };
        self.planting = None;
        self.defusing = None;
        events.push((ev::ROUND_WON, winner, how as u8));
    }

    /// One tick. `points` are the teams' round wins; `won` says the match is decided (no new round then).
    pub fn step(&mut self, tick: u64, actors: &[Actor], points: &mut [u32; 2], won: bool, events: &mut Vec<(u8, u8, u8)>) -> Vec<Cmd> {
        let mut cmds = Vec::new();
        let alive = |team: u8| actors.iter().filter(|a| a.alive && a.team == team).count();
        match self.phase {
            RoundPhase::Freeze { until } => {
                if tick >= until {
                    self.phase = RoundPhase::Live { until: tick + self.round_ticks };
                    self.started_with = [alive(1) as u8, alive(2) as u8];
                    events.push((ev::ROUND_LIVE, self.attackers, 255));
                }
                // The very first freeze has no NewRound before it: the match built the world already.
                if self.rounds_done == 0 && self.round == 1 && matches!(self.bomb, BombState::Dropped(p) if p == Vec3::ZERO) {
                    if let Some(a) = actors.iter().filter(|a| a.team == self.attackers).min_by_key(|a| a.slot) {
                        self.bomb = BombState::Carried { slot: a.slot, pos: a.pos };
                    }
                }
            }
            RoundPhase::Over { until, .. } => {
                if tick >= until && !won {
                    self.round = self.round.saturating_add(1);
                    if self.rounds_done == self.swap_after {
                        self.attackers = 3 - self.attackers;
                    }
                    self.start_round(tick, actors, &mut cmds);
                }
            }
            RoundPhase::Live { until } => {
                let (att, def) = (self.attackers, self.defenders());
                // The bomb.
                match self.bomb {
                    BombState::Carried { slot, pos } => match actors.iter().find(|a| a.slot == slot && a.alive) {
                        None => {
                            self.bomb = BombState::Dropped(pos);
                            self.planting = None;
                            events.push((ev::BOMB_DROPPED, att, slot));
                        }
                        Some(a) => {
                            self.bomb = BombState::Carried { slot, pos: a.pos };
                            let on_site = self.site_at(a.pos);
                            match (a.interact, on_site, self.planting) {
                                (true, Some(_), Some((s, since))) if s == slot => {
                                    if tick >= since + self.plant_ticks {
                                        let site = on_site.unwrap_or(0);
                                        self.bomb = BombState::Planted { pos: a.pos, site, explode_at: tick + self.fuse_ticks };
                                        self.planting = None;
                                        events.push((ev::BOMB_PLANTED, att, slot));
                                    }
                                }
                                (true, Some(_), _) => self.planting = Some((slot, tick)),
                                _ => self.planting = None,
                            }
                        }
                    },
                    BombState::Dropped(pos) => {
                        if let Some(a) = actors.iter().find(|a| a.alive && a.team == att && near(a.pos, pos)) {
                            self.bomb = BombState::Carried { slot: a.slot, pos: a.pos };
                            events.push((ev::BOMB_PICKED, att, a.slot));
                        }
                    }
                    BombState::Planted { pos, explode_at, .. } => {
                        if tick >= explode_at {
                            self.bomb = BombState::Exploded(pos);
                            cmds.push(Cmd::Explode(pos));
                            events.push((ev::BOMB_EXPLODED, att, 255));
                            self.end_round(tick, att, WinHow::Exploded, points, events);
                        } else {
                            let at_bomb = |a: &&Actor| {
                                a.alive && a.team == def && {
                                    let d = a.pos - pos;
                                    d.x * d.x + d.z * d.z <= DEFUSE_RADIUS * DEFUSE_RADIUS && d.y.abs() <= TOUCH_HEIGHT
                                }
                            };
                            let holder = actors.iter().filter(at_bomb).find(|a| a.interact);
                            match (holder, self.defusing) {
                                (Some(a), Some((s, since))) if a.slot == s => {
                                    if tick >= since + self.defuse_ticks {
                                        self.bomb = BombState::Defused(pos);
                                        self.defusing = None;
                                        events.push((ev::BOMB_DEFUSED, def, a.slot));
                                        self.end_round(tick, def, WinHow::Defused, points, events);
                                    }
                                }
                                (Some(a), _) => self.defusing = Some((a.slot, tick)),
                                (None, _) => self.defusing = None,
                            }
                        }
                    }
                    BombState::Defused(_) | BombState::Exploded(_) => {}
                }
                // The round's other endings, unless the bomb just decided it.
                if matches!(self.phase, RoundPhase::Live { .. }) {
                    let planted = matches!(self.bomb, BombState::Planted { .. });
                    let (a_alive, d_alive) = (alive(att), alive(def));
                    let (a_had, d_had) = (self.started_with[(att - 1) as usize] > 0, self.started_with[(def - 1) as usize] > 0);
                    if d_had && d_alive == 0 {
                        self.end_round(tick, att, WinHow::Eliminated, points, events);
                    } else if a_had && a_alive == 0 && !planted {
                        self.end_round(tick, def, WinHow::Eliminated, points, events);
                    } else if tick >= until && !planted {
                        self.end_round(tick, def, WinHow::TimeUp, points, events);
                    }
                }
            }
        }
        cmds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HZ: u32 = 60;

    fn root(s: &str) -> Map<String, Value> {
        serde_json::from_str::<Value>(s).unwrap().as_object().unwrap().clone()
    }

    fn actor(slot: u8, team: u8, x: f32, z: f32) -> Actor {
        Actor { slot, team, pos: Vec3::new(x, 0.0, z), alive: true, interact: false }
    }

    fn ctf_cfg() -> ObjectiveConfig {
        let mut e = Vec::new();
        let c = parse_objective(&root(r#"{"flags":[{"team":1,"at":[-20,0,0]},{"team":2,"at":[20,0,0]}],"objective":{"return_secs":10}}"#), &mut e);
        assert!(e.is_empty(), "{e:?}");
        c
    }

    #[test]
    fn mistakes_are_named_with_their_path() {
        let mut e = Vec::new();
        parse_objective(&root(r#"{"flags":[{"team":3,"at":[0,0,0]}],"sites":[{"at":[0,0]}],"objective":{"capture_limt":3,"plant_secs":999}}"#), &mut e);
        let e = e.join("\n");
        assert!(e.contains("shooter.flags[0]"), "{e}");
        assert!(e.contains("shooter.sites[0].at"), "{e}");
        assert!(e.contains("shooter.objective.capture_limt: unknown field"), "{e}");
        assert!(e.contains("shooter.objective.plant_secs"), "{e}");
    }

    #[test]
    fn a_flag_is_taken_carried_and_captured() {
        let cfg = ctf_cfg();
        let mut ctf = CtfState::new(&cfg, HZ).unwrap();
        let (mut points, mut ev_) = ([0u32; 2], Vec::new());
        // Team 1's raider walks onto team 2's flag.
        let mut runner = actor(0, 1, 20.0, 0.0);
        ctf.step(1, &[runner, actor(1, 2, 60.0, 0.0)], &mut points, &mut ev_);
        assert_eq!(ev_.pop(), Some((ev::FLAG_TAKEN, 2, 0)));
        assert_eq!(ctf.carried_by(0), Some(2));
        // Home again with the flag: scores, and the flag goes back.
        runner.pos = Vec3::new(-20.0, 0.0, 0.0);
        ctf.step(2, &[runner], &mut points, &mut ev_);
        assert_eq!(points, [1, 0]);
        assert_eq!(ev_.pop(), Some((ev::FLAG_CAPTURED, 1, 0)));
        assert_eq!(ctf.flags[1].state, FlagState::Home);
    }

    #[test]
    fn you_cannot_capture_while_your_own_flag_is_away() {
        let cfg = ctf_cfg();
        let mut ctf = CtfState::new(&cfg, HZ).unwrap();
        let (mut points, mut ev_) = ([0u32; 2], Vec::new());
        // Team 2 takes team 1's flag; team 1 takes team 2's, but cannot score until its own flag is back.
        ctf.step(1, &[actor(5, 2, -20.0, 0.0), actor(0, 1, 20.0, 0.0)], &mut points, &mut ev_);
        assert!(matches!(ctf.flags[0].state, FlagState::Carried { slot: 5, .. }));
        ctf.step(2, &[actor(5, 2, -10.0, 0.0), actor(0, 1, -20.0, 0.0)], &mut points, &mut ev_);
        assert_eq!(points, [0, 0], "own flag is away");
    }

    #[test]
    fn a_dead_carrier_drops_the_flag_and_a_teammate_returns_it() {
        let cfg = ctf_cfg();
        let mut ctf = CtfState::new(&cfg, HZ).unwrap();
        let (mut points, mut ev_) = ([0u32; 2], Vec::new());
        ctf.step(1, &[actor(0, 1, 20.0, 0.0)], &mut points, &mut ev_);
        ev_.clear();
        let mut dead = actor(0, 1, 5.0, 0.0);
        dead.alive = false;
        ctf.step(2, &[dead], &mut points, &mut ev_);
        assert_eq!(ev_[0], (ev::FLAG_DROPPED, 2, 0));
        assert!(matches!(ctf.flags[1].state, FlagState::Dropped { .. }));
        // A teammate of the flag (team 2) touching it sends it home.
        let pos = ctf.flags[1].pos();
        ctf.step(3, &[Actor { pos, ..actor(7, 2, 0.0, 0.0) }], &mut points, &mut ev_);
        assert_eq!(ctf.flags[1].state, FlagState::Home);
        assert_eq!(ev_.last(), Some(&(ev::FLAG_RETURNED, 2, 7)));
    }

    #[test]
    fn a_dropped_flag_goes_home_by_itself() {
        let cfg = ctf_cfg();
        let mut ctf = CtfState::new(&cfg, HZ).unwrap();
        let (mut points, mut ev_) = ([0u32; 2], Vec::new());
        ctf.step(1, &[actor(0, 1, 20.0, 0.0)], &mut points, &mut ev_);
        ctf.step(2, &[], &mut points, &mut ev_);
        assert!(matches!(ctf.flags[1].state, FlagState::Dropped { .. }));
        ctf.step(2 + 10 * HZ as u64 + 1, &[], &mut points, &mut ev_);
        assert_eq!(ctf.flags[1].state, FlagState::Home);
        assert_eq!(ev_.last(), Some(&(ev::FLAG_RETURNED, 2, 255)));
    }

    fn snd() -> SndState {
        let mut e = Vec::new();
        let c = parse_objective(
            &root(
                r#"{"sites":[{"name":"A","at":[0,0,0],"radius":4}],"objective":{"freeze_secs":1,"round_secs":30,"plant_secs":2,"defuse_secs":3,"fuse_secs":10,"swap_after":2}}"#,
            ),
            &mut e,
        );
        assert!(e.is_empty(), "{e:?}");
        SndState::new(&c, HZ).unwrap()
    }

    fn live(s: &mut SndState, actors: &[Actor], points: &mut [u32; 2], ev_: &mut Vec<(u8, u8, u8)>) -> u64 {
        let mut t = 0;
        while s.locked() {
            t += 1;
            s.step(t, actors, points, false, ev_);
        }
        t
    }

    #[test]
    fn the_attackers_plant_and_the_bomb_goes_off() {
        let mut s = snd();
        let (mut pts, mut ev_) = ([0u32; 2], Vec::new());
        let mut att = actor(0, 1, 0.0, 0.0);
        let def = actor(1, 2, 50.0, 0.0);
        let mut t = live(&mut s, &[att, def], &mut pts, &mut ev_);
        assert!(matches!(s.bomb, BombState::Carried { slot: 0, .. }));
        att.interact = true;
        // Hold for the plant time.
        for _ in 0..(2 * HZ + 2) {
            t += 1;
            s.step(t, &[att, def], &mut pts, false, &mut ev_);
        }
        assert!(matches!(s.bomb, BombState::Planted { .. }), "{:?}", s.bomb);
        // Nobody defuses: it goes off and the attackers take the round.
        att.interact = false;
        let mut cmds = Vec::new();
        for _ in 0..(10 * HZ + 2) {
            t += 1;
            cmds.extend(s.step(t, &[att, def], &mut pts, false, &mut ev_));
        }
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Explode(_))));
        assert_eq!(pts, [1, 0]);
        assert!(matches!(s.phase, RoundPhase::Over { winner: 1, .. }));
    }

    #[test]
    fn planting_needs_an_unbroken_hold_on_a_site() {
        let mut s = snd();
        let (mut pts, mut ev_) = ([0u32; 2], Vec::new());
        let mut att = actor(0, 1, 0.0, 0.0);
        let def = actor(1, 2, 50.0, 0.0);
        let mut t = live(&mut s, &[att, def], &mut pts, &mut ev_);
        att.interact = true;
        for _ in 0..HZ {
            t += 1;
            s.step(t, &[att, def], &mut pts, false, &mut ev_);
        }
        att.interact = false; // let go halfway
        t += 1;
        s.step(t, &[att, def], &mut pts, false, &mut ev_);
        assert_eq!(s.planting, None);
        att.interact = true;
        att.pos = Vec3::new(30.0, 0.0, 0.0); // off the site
        for _ in 0..(3 * HZ) {
            t += 1;
            s.step(t, &[att, def], &mut pts, false, &mut ev_);
        }
        assert!(matches!(s.bomb, BombState::Carried { .. }), "{:?}", s.bomb);
    }

    #[test]
    fn a_defender_defuses_and_wins_the_round() {
        let mut s = snd();
        let (mut pts, mut ev_) = ([0u32; 2], Vec::new());
        let mut att = actor(0, 1, 0.0, 0.0);
        let mut def = actor(1, 2, 1.0, 0.0);
        let mut t = live(&mut s, &[att, def], &mut pts, &mut ev_);
        att.interact = true;
        for _ in 0..(2 * HZ + 2) {
            t += 1;
            s.step(t, &[att, def], &mut pts, false, &mut ev_);
        }
        assert!(matches!(s.bomb, BombState::Planted { .. }));
        att.interact = false;
        def.interact = true;
        for _ in 0..(3 * HZ + 2) {
            t += 1;
            s.step(t, &[att, def], &mut pts, false, &mut ev_);
        }
        assert!(matches!(s.bomb, BombState::Defused(_)), "{:?}", s.bomb);
        assert_eq!(pts, [0, 1]);
    }

    #[test]
    fn eliminating_the_defenders_wins_for_the_attackers_but_a_planted_bomb_outlives_its_planters() {
        let mut s = snd();
        let (mut pts, mut ev_) = ([0u32; 2], Vec::new());
        let att = actor(0, 1, 0.0, 0.0);
        let mut def = actor(1, 2, 50.0, 0.0);
        let mut t = live(&mut s, &[att, def], &mut pts, &mut ev_);
        def.alive = false;
        t += 1;
        s.step(t, &[att, def], &mut pts, false, &mut ev_);
        assert_eq!(pts, [1, 0]);

        // A second match: the attackers die after planting, and the defenders must still deal with the bomb.
        let mut s = snd();
        let (mut pts, mut ev_) = ([0u32; 2], Vec::new());
        let mut att = actor(0, 1, 0.0, 0.0);
        let def = actor(1, 2, 50.0, 0.0);
        let mut t = live(&mut s, &[att, def], &mut pts, &mut ev_);
        att.interact = true;
        for _ in 0..(2 * HZ + 2) {
            t += 1;
            s.step(t, &[att, def], &mut pts, false, &mut ev_);
        }
        att.alive = false;
        t += 1;
        s.step(t, &[att, def], &mut pts, false, &mut ev_);
        assert_eq!(pts, [0, 0], "planted: the round goes on");
    }

    #[test]
    fn time_running_out_without_a_plant_favours_the_defenders_and_the_sides_swap() {
        let mut s = snd();
        let (mut pts, mut ev_) = ([0u32; 2], Vec::new());
        let att = actor(0, 1, 40.0, 0.0);
        let def = actor(1, 2, 50.0, 0.0);
        let mut t = live(&mut s, &[att, def], &mut pts, &mut ev_);
        for _ in 0..(31 * HZ) {
            t += 1;
            s.step(t, &[att, def], &mut pts, false, &mut ev_);
        }
        assert_eq!(pts, [0, 1]);
        // Over -> next round (NewRound) -> after two rounds the sides swap.
        let mut cmds = Vec::new();
        for _ in 0..(5 * HZ) {
            t += 1;
            cmds.extend(s.step(t, &[att, def], &mut pts, false, &mut ev_));
        }
        assert!(cmds.contains(&Cmd::NewRound));
        assert_eq!(s.round, 2);
        assert_eq!(s.attackers, 1, "not swapped yet after one round");
        let _ = t;
    }

    #[test]
    fn a_side_that_started_empty_cannot_be_eliminated() {
        let mut s = snd();
        let (mut pts, mut ev_) = ([0u32; 2], Vec::new());
        let att = actor(0, 1, 40.0, 0.0);
        let mut t = live(&mut s, &[att], &mut pts, &mut ev_);
        for _ in 0..HZ {
            t += 1;
            s.step(t, &[att], &mut pts, false, &mut ev_);
        }
        assert_eq!(pts, [0, 0], "a solo tester is not an instant win");
    }
}
