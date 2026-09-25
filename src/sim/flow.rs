//! The match flow: **lobby → countdown → playing → results → (rematch)**, as a pure state machine (ADR 0029).
//!
//! It knows nothing of sockets, players or worlds. Each server tick it is handed a [`FlowInput`] (how many players are connected and ready,
//! whether the rules ended the round, the best score) and answers with at most one [`FlowEvent`] the server acts on (build the world,
//! start ticking it, record the result, go back to the lobby). That keeps every rule of the flow unit-testable without a network:
//!
//! ```text
//!  Waiting ──all ready, ≥ min_players──▶ Countdown ──0──▶ Playing ──rules end / time / score / everyone left──▶ Results
//!     ▲                                    │ someone un-readies or leaves                                         │
//!     └────────────────────────────────────┘◀──────── results_secs pass ─────────────────────────────────────────┤
//!                                          ◀─────────── everyone presses Ready again (rematch) ─────────────────┘
//! ```
//!
//! A scene opts in with a top-level `"match"` block (see [`MatchSettings`]); without one the server stays in open play (join = play, no
//! rounds), exactly as before. All timing is in simulation ticks, so the flow is as deterministic as the sim it drives.

use super::clock::{secs_to_ticks, TICK_RATE_HZ};
use crate::strict::check_keys;
use serde_json::Value;

/// Where a match is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The lobby: choosing a character and readying up. No world is running.
    Waiting,
    /// Everyone is ready: the world is built and players stand at their spawns, frozen, for the last seconds before the round.
    Countdown,
    /// The round is being played.
    Playing,
    /// The round is over: scores and the winner are shown; the world is frozen.
    Results,
}

impl Phase {
    /// How a phase travels on the wire.
    pub fn to_wire(self) -> u8 {
        match self {
            Phase::Waiting => 0,
            Phase::Countdown => 1,
            Phase::Playing => 2,
            Phase::Results => 3,
        }
    }

    /// The inverse of [`Phase::to_wire`]; `None` for a value outside the protocol.
    pub fn from_wire(v: u8) -> Option<Phase> {
        match v {
            0 => Some(Phase::Waiting),
            1 => Some(Phase::Countdown),
            2 => Some(Phase::Playing),
            3 => Some(Phase::Results),
            _ => None,
        }
    }

    /// A short lower-case name (`waiting`, `countdown`, `playing`, `results`).
    pub fn name(self) -> &'static str {
        match self {
            Phase::Waiting => "waiting",
            Phase::Countdown => "countdown",
            Phase::Playing => "playing",
            Phase::Results => "results",
        }
    }
}

/// Why a round ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndReason {
    /// A rule ran `{"end": "<outcome>"}`.
    Rules(String),
    /// `round_secs` ran out.
    TimeUp,
    /// Someone reached `score_to_win`.
    ScoreReached,
    /// Nobody was left in the round.
    Abandoned,
}

impl EndReason {
    /// `(code, text)` for the wire: `0` rules (text = the outcome), `1` time up, `2` score reached, `3` abandoned.
    pub fn to_wire(&self) -> (u8, String) {
        match self {
            EndReason::Rules(s) => (0, s.clone()),
            EndReason::TimeUp => (1, String::new()),
            EndReason::ScoreReached => (2, String::new()),
            EndReason::Abandoned => (3, String::new()),
        }
    }

    /// A player-facing line: `time up`, `score reached`, `abandoned`, or the rule's own outcome word.
    pub fn text(&self) -> String {
        match self {
            EndReason::Rules(s) => s.clone(),
            EndReason::TimeUp => "time up".to_string(),
            EndReason::ScoreReached => "score reached".to_string(),
            EndReason::Abandoned => "abandoned".to_string(),
        }
    }
}

/// The scene's `"match"` block. Every field is optional; the defaults are a forgiving small-lobby match.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchSettings {
    /// Connected players needed before a countdown can begin (default 1, so one developer can test alone).
    pub min_players: u8,
    /// Seconds of frozen countdown once everyone is ready (default 5).
    pub countdown_secs: f32,
    /// Round length in seconds; `0` = no time limit (default 300).
    pub round_secs: f32,
    /// Seconds the results stay up before returning to the lobby (default 12).
    pub results_secs: f32,
    /// A player reaching this many kills ends the round; `0` = off (default 0).
    pub score_to_win: u32,
    /// Whether someone joining mid-round plays at once (`true`, default) or watches until the next round.
    pub join_in_progress: bool,
    /// Whether the lobby waits for everyone to press Ready (`true`, default) or starts the countdown as soon as `min_players` are in.
    pub ready_check: bool,
}

impl Default for MatchSettings {
    fn default() -> Self {
        MatchSettings { min_players: 1, countdown_secs: 5.0, round_secs: 300.0, results_secs: 12.0, score_to_win: 0, join_in_progress: true, ready_check: true }
    }
}

/// Keys of the `match` block.
pub const MATCH_KEYS: &[&str] = &["min_players", "countdown_secs", "round_secs", "results_secs", "score_to_win", "join_in_progress", "ready_check"];

impl MatchSettings {
    /// Reads the `match` block of a scene's JSON text: `Ok(None)` when the scene has none (open play), `Err` naming the field when it is wrong.
    pub fn from_scene_text(text: &str) -> Result<Option<MatchSettings>, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("scene is not JSON: {e}"))?;
        match v.get("match") {
            None => Ok(None),
            Some(b) => Self::from_json(b).map(Some),
        }
    }

    /// Parses one `match` object.
    pub fn from_json(b: &Value) -> Result<MatchSettings, String> {
        let obj = b.as_object().ok_or("match: must be an object")?;
        let mut errs = Vec::new();
        check_keys(&mut errs, "match", obj, MATCH_KEYS);
        if let Some(e) = errs.into_iter().next() {
            return Err(e);
        }
        let mut s = MatchSettings::default();
        let num = |k: &str| -> Result<Option<f64>, String> {
            match obj.get(k) {
                None => Ok(None),
                Some(v) => v.as_f64().filter(|n| n.is_finite() && *n >= 0.0).map(Some).ok_or(format!("match.{k}: must be a number >= 0")),
            }
        };
        let flag = |k: &str| -> Result<Option<bool>, String> {
            match obj.get(k) {
                None => Ok(None),
                Some(v) => v.as_bool().map(Some).ok_or(format!("match.{k}: must be true or false")),
            }
        };
        if let Some(n) = num("min_players")? {
            if !(1.0..=8.0).contains(&n) {
                return Err("match.min_players: must be 1..8 (a match holds at most 8 players)".to_string());
            }
            s.min_players = n as u8;
        }
        if let Some(n) = num("countdown_secs")? {
            s.countdown_secs = n as f32;
        }
        if let Some(n) = num("round_secs")? {
            s.round_secs = n as f32;
        }
        if let Some(n) = num("results_secs")? {
            s.results_secs = n as f32;
        }
        if let Some(n) = num("score_to_win")? {
            s.score_to_win = n as u32;
        }
        if let Some(f) = flag("join_in_progress")? {
            s.join_in_progress = f;
        }
        if let Some(f) = flag("ready_check")? {
            s.ready_check = f;
        }
        Ok(s)
    }
}

/// What the server tells the flow each tick.
#[derive(Debug, Clone, Default)]
pub struct FlowInput {
    /// Players connected right now.
    pub connected: usize,
    /// Of those, how many have pressed Ready.
    pub ready: usize,
    /// Players who are part of the running round (spawned in the world).
    pub in_round: usize,
    /// The outcome word if a rule ended the round.
    pub rules_outcome: Option<String>,
    /// The highest score in the round.
    pub best_score: u32,
}

/// Something the server must act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowEvent {
    /// Everyone is ready: build the world and freeze the players at their spawns.
    CountdownStarted,
    /// The countdown broke off (someone un-readied or left): throw the world away.
    CountdownAborted,
    /// The countdown reached zero: start ticking the world.
    RoundStarted,
    /// The round ended: freeze the world, record the result.
    RoundEnded(EndReason),
    /// The results are done: back to the lobby.
    ReturnedToLobby,
}

/// The state machine. See the module docs.
#[derive(Debug, Clone)]
pub struct Flow {
    settings: MatchSettings,
    phase: Phase,
    ticks_left: u32,
    round: u16,
    last_end: Option<EndReason>,
}

/// `ticks_left` for a phase with no time limit.
pub const NO_LIMIT: u32 = u32::MAX;

impl Flow {
    /// A flow in the lobby, round 0.
    pub fn new(settings: MatchSettings) -> Flow {
        Flow { settings, phase: Phase::Waiting, ticks_left: NO_LIMIT, round: 0, last_end: None }
    }

    /// The settings it runs.
    pub fn settings(&self) -> &MatchSettings {
        &self.settings
    }

    /// The current phase.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Ticks until the phase changes by itself ([`NO_LIMIT`] when it will not).
    pub fn ticks_left(&self) -> u32 {
        self.ticks_left
    }

    /// The current round number (`0` before the first countdown; the countdown for round 1 already reads 1).
    pub fn round(&self) -> u16 {
        self.round
    }

    /// Why the last round ended (`None` before the first end).
    pub fn last_end(&self) -> Option<&EndReason> {
        self.last_end.as_ref()
    }

    /// Whether the lobby's conditions to begin a countdown hold for `i`.
    fn can_start(&self, i: &FlowInput) -> bool {
        let enough = i.connected >= self.settings.min_players as usize && i.connected > 0;
        enough && (!self.settings.ready_check || i.ready == i.connected)
    }

    fn secs(&self, s: f32) -> u32 {
        secs_to_ticks(s)
    }

    /// Advances one tick.
    pub fn step(&mut self, i: &FlowInput) -> Option<FlowEvent> {
        match self.phase {
            Phase::Waiting => {
                if self.can_start(i) {
                    self.begin_countdown();
                    return Some(FlowEvent::CountdownStarted);
                }
                None
            }
            Phase::Countdown => {
                if !self.can_start(i) {
                    self.phase = Phase::Waiting;
                    self.ticks_left = NO_LIMIT;
                    self.round = self.round.saturating_sub(1);
                    return Some(FlowEvent::CountdownAborted);
                }
                self.ticks_left = self.ticks_left.saturating_sub(1);
                if self.ticks_left == 0 {
                    self.phase = Phase::Playing;
                    self.ticks_left = if self.settings.round_secs > 0.0 { self.secs(self.settings.round_secs) } else { NO_LIMIT };
                    return Some(FlowEvent::RoundStarted);
                }
                None
            }
            Phase::Playing => {
                let reason = if let Some(o) = &i.rules_outcome {
                    Some(EndReason::Rules(o.clone()))
                } else if self.settings.score_to_win > 0 && i.best_score >= self.settings.score_to_win {
                    Some(EndReason::ScoreReached)
                } else if i.in_round == 0 {
                    Some(EndReason::Abandoned)
                } else if self.ticks_left != NO_LIMIT {
                    self.ticks_left = self.ticks_left.saturating_sub(1);
                    (self.ticks_left == 0).then_some(EndReason::TimeUp)
                } else {
                    None
                };
                let reason = reason?;
                self.phase = Phase::Results;
                self.ticks_left = if self.settings.results_secs > 0.0 { self.secs(self.settings.results_secs) } else { 1 };
                self.last_end = Some(reason.clone());
                Some(FlowEvent::RoundEnded(reason))
            }
            Phase::Results => {
                // A rematch: everyone pressed Ready again while the results were up, so skip the rest of them (the flags are kept, and
                // the next tick's Waiting arm starts the countdown). Without a ready check the results always run their course.
                let rematch = self.settings.ready_check && self.can_start(i);
                if i.connected == 0 || rematch {
                    self.phase = Phase::Waiting;
                    self.ticks_left = NO_LIMIT;
                    return Some(FlowEvent::ReturnedToLobby);
                }
                self.ticks_left = self.ticks_left.saturating_sub(1);
                if self.ticks_left == 0 {
                    self.phase = Phase::Waiting;
                    self.ticks_left = NO_LIMIT;
                    return Some(FlowEvent::ReturnedToLobby);
                }
                None
            }
        }
    }

    fn begin_countdown(&mut self) {
        self.phase = Phase::Countdown;
        self.round = self.round.wrapping_add(1).max(1);
        self.ticks_left = self.secs(self.settings.countdown_secs).max(1);
    }

    /// Seconds (rounded up) until the phase changes, `None` when it has no limit: what a HUD prints.
    pub fn secs_left(&self) -> Option<u32> {
        (self.ticks_left != NO_LIMIT).then(|| self.ticks_left.div_ceil(TICK_RATE_HZ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quick() -> MatchSettings {
        MatchSettings { min_players: 2, countdown_secs: 1.0, round_secs: 2.0, results_secs: 1.0, score_to_win: 3, ..Default::default() }
    }

    fn input(connected: usize, ready: usize) -> FlowInput {
        FlowInput { connected, ready, in_round: connected, ..Default::default() }
    }

    #[test]
    fn the_lobby_waits_for_enough_players_and_for_everyone_to_be_ready() {
        let mut f = Flow::new(quick());
        assert_eq!(f.step(&input(1, 1)), None, "one ready player is below min_players 2");
        assert_eq!(f.step(&input(2, 1)), None, "one of two is not everyone");
        assert_eq!(f.step(&input(2, 2)), Some(FlowEvent::CountdownStarted));
        assert_eq!((f.phase(), f.round()), (Phase::Countdown, 1));
    }

    #[test]
    fn a_countdown_aborts_when_a_player_un_readies_or_leaves_and_the_round_number_is_not_burned() {
        let mut f = Flow::new(quick());
        f.step(&input(2, 2));
        for _ in 0..10 {
            assert_eq!(f.step(&input(2, 2)), None);
        }
        assert_eq!(f.step(&input(2, 1)), Some(FlowEvent::CountdownAborted));
        assert_eq!((f.phase(), f.round()), (Phase::Waiting, 0));
        assert_eq!(f.step(&input(2, 2)), Some(FlowEvent::CountdownStarted));
        assert_eq!(f.round(), 1, "the retry is still round 1");
        assert_eq!(f.step(&input(1, 1)), Some(FlowEvent::CountdownAborted), "a player leaving drops below min_players");
    }

    #[test]
    fn the_countdown_lasts_exactly_countdown_secs_then_the_round_starts() {
        let mut f = Flow::new(quick());
        f.step(&input(2, 2));
        let mut ticks = 0;
        loop {
            ticks += 1;
            if f.step(&input(2, 2)) == Some(FlowEvent::RoundStarted) {
                break;
            }
            assert!(ticks < 1000);
        }
        assert_eq!(ticks, TICK_RATE_HZ, "one second of countdown is one second of ticks");
        assert_eq!(f.phase(), Phase::Playing);
        assert_eq!(f.secs_left(), Some(2));
    }

    fn playing() -> Flow {
        let mut f = Flow::new(quick());
        f.step(&input(2, 2));
        while f.step(&input(2, 2)) != Some(FlowEvent::RoundStarted) {}
        f
    }

    #[test]
    fn a_round_ends_on_time_on_score_on_rules_and_when_everyone_leaves() {
        let mut f = playing();
        let mut ticks = 0;
        let end = loop {
            ticks += 1;
            if let Some(e) = f.step(&input(2, 0)) {
                break e;
            }
        };
        assert_eq!((end, ticks), (FlowEvent::RoundEnded(EndReason::TimeUp), 2 * TICK_RATE_HZ));

        let mut f = playing();
        assert_eq!(f.step(&FlowInput { best_score: 3, ..input(2, 0) }), Some(FlowEvent::RoundEnded(EndReason::ScoreReached)));
        let mut f = playing();
        assert_eq!(f.step(&input(2, 0)), None);
        assert_eq!(
            f.step(&FlowInput { rules_outcome: Some("victory".into()), ..input(2, 0) }),
            Some(FlowEvent::RoundEnded(EndReason::Rules("victory".into())))
        );
        let mut f = playing();
        assert_eq!(f.step(&FlowInput { in_round: 0, ..input(0, 0) }), Some(FlowEvent::RoundEnded(EndReason::Abandoned)));
    }

    #[test]
    fn results_return_to_the_lobby_after_results_secs_or_earlier_when_everyone_readies_for_a_rematch() {
        let mut f = playing();
        f.step(&FlowInput { best_score: 9, ..input(2, 0) });
        assert_eq!(f.phase(), Phase::Results);
        let mut ticks = 0;
        while f.step(&input(2, 0)) != Some(FlowEvent::ReturnedToLobby) {
            ticks += 1;
            assert!(ticks < 1000);
        }
        assert_eq!(ticks + 1, TICK_RATE_HZ, "one second of results");
        assert_eq!(f.phase(), Phase::Waiting);
        assert_eq!(f.last_end(), Some(&EndReason::ScoreReached), "the result stays readable in the lobby");

        // Rematch: both press Ready while the results are up.
        let mut f = playing();
        f.step(&FlowInput { best_score: 9, ..input(2, 0) });
        assert_eq!(f.step(&input(2, 1)), None);
        assert_eq!(f.step(&input(2, 2)), Some(FlowEvent::ReturnedToLobby));
        assert_eq!(f.step(&input(2, 2)), Some(FlowEvent::CountdownStarted));
        assert_eq!(f.round(), 2);
    }

    #[test]
    fn without_a_ready_check_the_countdown_starts_as_soon_as_enough_players_are_in() {
        let mut f = Flow::new(MatchSettings { ready_check: false, min_players: 2, ..quick() });
        assert_eq!(f.step(&input(1, 0)), None);
        assert_eq!(f.step(&input(2, 0)), Some(FlowEvent::CountdownStarted));
    }

    #[test]
    fn an_empty_lobby_never_starts_and_a_round_without_a_time_limit_runs_until_something_ends_it() {
        let mut f = Flow::new(MatchSettings { min_players: 1, ready_check: false, ..Default::default() });
        assert_eq!(f.step(&input(0, 0)), None, "nobody connected: no countdown even without a ready check");
        let mut f = Flow::new(MatchSettings { round_secs: 0.0, countdown_secs: 0.1, min_players: 1, ..Default::default() });
        f.step(&input(1, 1));
        while f.step(&input(1, 1)) != Some(FlowEvent::RoundStarted) {}
        assert_eq!(f.secs_left(), None);
        for _ in 0..10_000 {
            assert_eq!(f.step(&input(1, 0)), None);
        }
    }

    #[test]
    fn the_match_block_parses_validates_and_rejects_typos() {
        assert_eq!(MatchSettings::from_scene_text(r#"{"objects":[]}"#), Ok(None));
        let s = MatchSettings::from_scene_text(r#"{"match":{"min_players":2,"round_secs":90,"score_to_win":5}}"#).unwrap().unwrap();
        assert_eq!((s.min_players, s.round_secs, s.score_to_win, s.countdown_secs), (2, 90.0, 5, 5.0));
        let e = MatchSettings::from_scene_text(r#"{"match":{"min_player":2}}"#).unwrap_err();
        assert!(e.contains("min_player") && e.contains("min_players"), "{e}");
        assert!(MatchSettings::from_scene_text(r#"{"match":{"min_players":9}}"#).is_err());
        assert!(MatchSettings::from_scene_text(r#"{"match":{"round_secs":-1}}"#).is_err());
        assert!(MatchSettings::from_scene_text(r#"{"match":{"ready_check":"yes"}}"#).is_err());
        assert!(MatchSettings::from_scene_text(r#"{"match":[]}"#).is_err());
    }

    #[test]
    fn phases_and_end_reasons_round_trip_through_their_wire_forms() {
        for p in [Phase::Waiting, Phase::Countdown, Phase::Playing, Phase::Results] {
            assert_eq!(Phase::from_wire(p.to_wire()), Some(p));
        }
        assert_eq!(Phase::from_wire(9), None);
        assert_eq!(EndReason::Rules("victory".into()).to_wire(), (0, "victory".to_string()));
        assert_eq!(EndReason::TimeUp.text(), "time up");
    }
}
