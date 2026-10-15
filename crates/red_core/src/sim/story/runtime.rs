//! The story system: a compiled package ([`Loaded`]) run as one of a game's [`System`]s, inside the match's one authority (ADR
//! 2026-10-15-narrative-is-compiled-content-run-by-one-story-system). Its state is the host's: in the match checksum, in versioned saves,
//! in `sim --inspect` and scenario `game` expectations. It reads gameplay only through the game's declared queries ([`Cx::query`]) and
//! changes it only through the game's commands, applied with the rest of a decision as one batch ([`Cx::issue`]).
//!
//! **Players send intentions, the story decides** (`<story>:talk`, `advance`, `choose`, `leave`, `skip`, `new_run`; [`intend`] maps a
//! match [`Intent`] to them). Every answer names the [`View::revision`] it was shown: an answer to anything older is refused, so a repeated
//! press, a retry or a late packet never applies twice. A choice is checked again when it is picked, against live state, before anything
//! changes; the line's and the choice's effects then apply together, the game's commands first (whole or not at all), the story's own after.
//!
//! **Conversations** move on through the shared core ([`super::talk`]); a line whose condition is false (or whose speaker does not know what
//! it reveals) is passed over without its effects. A conversation line is *heard* when the player moves past it. The world does not stop:
//! whether a game pauses or locks input while one is open is the game's call ([`Story::locks_input`] tells it one is).
//!
//! **Barks** answer events. Candidates are checked by pure conditions (nothing is marked, drawn or started by checking); the best is the
//! highest priority, then the most specific, then the least heard, then the first written; a tie among the best draws once from the host's
//! random stream, at the moment one is committed. Choosing starts its cooldowns (its own, its category's, and its speaker's, which holds back
//! only their lines of no higher priority: chatter never silences an alarm). A bark is presented
//! for a time from its length ([`ticks_for`]); it is *heard* when that time is up or the player skips it, and only then do its effects and
//! reveals apply. One of higher priority interrupts it (it was *seen*, not heard, and may be chosen again); others queue, at most
//! [`QUEUE_MAX`], each going stale after [`QUEUE_TTL`] and checked again before it plays. A bark with no line is a deliberate silence.
//!
//! **Directors** evaluate every so often, never while a conversation is open, within a recovery time and a per-round budget; a `rest`
//! condition makes one deliberately do nothing; an intervention may have a percent chance (one draw, only when it is otherwise eligible).
//! An intervention may run only the commands the game lets be issued unattended. Every outcome, including nothing, is in the log with why.

use super::package::{Bark, Effect, Loaded, Next, Piece, Var, Who};
use super::state::{Known, Playing, Queued, Source, State, Talk};
use super::talk::{self, Answer, Goto};
use crate::sim::defs::{code, Diagnostic, Id};
use crate::sim::game::Intent;
use crate::sim::systems::{Command, Cx, Inspection, Look, Outside, System, Systems};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::Arc;

mod explain;
mod pacing;

/// Barks that may wait behind the one presented.
pub const QUEUE_MAX: usize = 3;
/// Ticks a queued bark stays worth saying (4 s at 60 Hz).
pub const QUEUE_TTL: u64 = 240;
/// Decisions kept for inspection.
const LOG: usize = 24;

/// The kinds of decision outcome a tally counts (the log's outcome text begins with one).
pub const KINDS: &[&str] = &[
    "played from the queue",
    "played",
    "interrupted the bark before",
    "interrupted",
    "suspended the conversation",
    "queued",
    "dropped from the queue",
    "dropped",
    "silence: no candidate",
    "silence: chose",
    "does nothing",
    "intervenes",
    "rests",
    "waits",
    "recovering",
    "budget spent",
    "nothing eligible",
    "exhausted",
    "not noticed",
    "noticed",
    "opened",
    "closed at once",
    "ended",
    "taken",
    "refused",
    "the game refused",
    "passed over",
    "round facts reset",
    "left before the end",
    "no choice is offered",
    "heard",
    "nothing to say",
    "everything but profile facts reset",
];

/// Ticks a bark of `text` is presented: 1.5 s, plus a fifteenth of a second a character, at most 7 s (60 Hz).
pub fn ticks_for(text: &str) -> u64 {
    (60 + 4 * text.chars().count() as u64).clamp(90, 420)
}

/// What answers a story's queries: the host while it runs ([`Cx`]), or the host and the match from outside ([`HostAsk`]).
pub trait Ask {
    /// The answer to `query` now.
    fn ask(&self, query: &Id) -> Result<f64, Diagnostic>;
    /// The host's tick now.
    fn now(&self) -> u64;
}

impl Ask for Cx<'_> {
    fn ask(&self, query: &Id) -> Result<f64, Diagnostic> {
        self.query(query)
    }
    fn now(&self) -> u64 {
        self.tick
    }
}

/// The host and the match, asked from outside the host (a presenter's view, an inspection).
pub struct HostAsk<'a> {
    /// The game's systems.
    pub systems: &'a Systems,
    /// The match (`scene:<var>`).
    pub outside: &'a dyn Outside,
}

impl Ask for HostAsk<'_> {
    fn ask(&self, query: &Id) -> Result<f64, Diagnostic> {
        self.systems.query(query, self.outside).ok_or_else(|| Diagnostic::new(code::QUERY, query, "nothing answers it"))
    }
    fn now(&self) -> u64 {
        self.systems.tick_count()
    }
}

/// One decision, as the log keeps it (why something was said, or not).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Decision {
    /// The tick.
    pub tick: u64,
    /// What asked (`bark on tw:fuel_added`, `director pacing`, `choose lend`).
    pub what: String,
    /// What happened (`played tidewatch:fed_odile`, `silence: no candidate`, `rest: tension >= 3`).
    pub outcome: String,
    /// Each candidate considered and why it was or was not taken.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<String>,
}

/// A decision waiting for the game to apply its commands.
#[derive(Debug, Clone, PartialEq)]
enum Pending {
    /// The player's answer at a conversation line.
    Answer { line: u32, choice: Option<usize> },
    /// A bark heard in full.
    Heard { bark: u32 },
    /// A director's intervention.
    Intervene { director: u32, intervention: u32 },
}

/// What a presenter shows: the open conversation's line and the choices it offers now, and the bark being presented.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct View {
    /// What every answer must name ([`Story`]).
    pub revision: u64,
    /// The open conversation, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub talk: Option<TalkView>,
    /// The bark being presented, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bark: Option<LineView>,
}

/// A line as shown.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LineView {
    /// The line's id.
    pub id: String,
    /// The speaker's id (`None`: narration).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    /// The speaker's name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The words, with every `{value}` filled in.
    pub text: String,
    /// When a bark is heard in full (tick); conversations wait for the player.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<u64>,
}

/// The open conversation as shown.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TalkView {
    /// The conversation's id.
    pub conversation: String,
    /// Its line.
    pub line: LineView,
    /// The choices offered now (`(id, text)`), in order; empty: the player moves on.
    pub choices: Vec<(String, String)>,
    /// The line's choices not offered now, with why (for tools; a presenter shows only `choices`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unavailable: Vec<(String, String)>,
    /// A bark suspended it.
    pub suspended: bool,
    /// The game refused the last choice: why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,
}

/// A story, running.
pub struct Story {
    lp: Arc<Loaded>,
    st: State,
    pending: Option<Pending>,
    log: VecDeque<Decision>,
    /// Every decision's outcome kind, counted since the story was made or loaded (for walks and tests: not saved, not hashed).
    tally: std::collections::BTreeMap<String, u32>,
    problems: Vec<Diagnostic>,
    me: Id,
}

impl Story {
    /// The story of `lp`, at the start of a run.
    pub fn new(lp: Arc<Loaded>) -> Story {
        let st = State::fresh(&lp);
        let me = lp.id.clone();
        Story { lp, st, pending: None, log: VecDeque::new(), tally: Default::default(), problems: Vec::new(), me }
    }

    /// The package.
    pub fn loaded(&self) -> &Loaded {
        &self.lp
    }

    /// Its state.
    pub fn state(&self) -> &State {
        &self.st
    }

    /// The decisions it made lately (oldest first), each with why.
    pub fn log(&self) -> impl Iterator<Item = &Decision> {
        self.log.iter()
    }

    /// Whether a conversation is open (and not suspended): a game that holds the player still while one is, asks this.
    pub fn locks_input(&self) -> bool {
        self.st.talk.as_ref().is_some_and(|t| !t.suspended)
    }

    fn cmd(&self, name: &str) -> Id {
        Id::parse(&format!("{}:{name}", self.lp.package.story)).unwrap_or_else(|_| self.me.clone())
    }

    fn note(&mut self, tick: u64, what: impl Into<String>, outcome: impl Into<String>, candidates: Vec<String>) {
        if self.log.len() == LOG {
            self.log.pop_front();
        }
        let outcome = outcome.into();
        let kind =
            KINDS.iter().find(|k| outcome.starts_with(**k)).map_or_else(|| outcome.split_whitespace().next().unwrap_or("").to_string(), |k| k.to_string());
        *self.tally.entry(kind).or_default() += 1;
        let cooled = candidates.iter().filter(|c| c.contains("cooling down") || c.contains("quiet until") || c.contains("spoke lately")).count() as u32;
        if cooled > 0 {
            *self.tally.entry("held by a cooldown".into()).or_default() += cooled;
        }
        self.log.push_back(Decision { tick, what: what.into(), outcome, candidates });
    }

    /// How decisions ended since the story was made or loaded: outcome kind -> count.
    pub fn tally(&self) -> &std::collections::BTreeMap<String, u32> {
        &self.tally
    }

    // ---- reading ------------------------------------------------------------------------------------------------------------------

    /// Every variable's value now.
    fn values(&self, ask: &dyn Ask) -> Result<Vec<f64>, Diagnostic> {
        self.lp
            .package
            .vars
            .iter()
            .map(|v| {
                Ok(match v {
                    Var::Fact { fact } => self.st.facts[*fact as usize],
                    Var::Query { id, .. } => ask.ask(id)?,
                    Var::Knows { holder, info } => self.st.knows(*holder, *info, true) as u8 as f64,
                    Var::Believes { holder, info } => self.st.knows(*holder, *info, false) as u8 as f64,
                    Var::Heard { line } => (self.st.heard[*line as usize] > 0) as u8 as f64,
                })
            })
            .collect()
    }

    fn holds(&self, expr: Option<u32>, vals: &[f64]) -> bool {
        expr.is_none_or(|e| self.lp.exprs[e as usize].truthy(vals))
    }

    /// Why `expr` is false: the expression as written and the values it reads.
    fn why_not(&self, expr: u32, vals: &[f64]) -> String {
        let p = &self.lp.package;
        let mut reads: Vec<String> = Vec::new();
        let lowered = &p.exprs[expr as usize];
        for (i, v) in vals.iter().enumerate() {
            if contains_var(lowered, i) {
                reads.push(format!("{} = {}", self.var_name(i), number(*v)));
            }
        }
        format!("`{}` is false ({})", p.authored[expr as usize], reads.join(", "))
    }

    fn var_name(&self, i: usize) -> String {
        let p = &self.lp.package;
        match &p.vars[i] {
            Var::Fact { fact } => p.facts[*fact as usize].id.clone(),
            Var::Query { id, .. } => id.to_string(),
            Var::Knows { holder, info } => format!("knows({}, {})", p.holders[*holder as usize], p.infos[*info as usize].id),
            Var::Believes { holder, info } => format!("believes({}, {})", p.holders[*holder as usize], p.infos[*info as usize].id),
            Var::Heard { line } => format!("heard({})", p.lines[*line as usize].id),
        }
    }

    fn present(&self, speaker: u16, vals: &[f64]) -> bool {
        self.lp.package.speakers[speaker as usize].present.is_none_or(|v| vals[v as usize] != 0.0)
    }

    fn text(&self, pieces: &[Piece], vals: &[f64]) -> String {
        pieces
            .iter()
            .map(|x| match x {
                Piece::Text(t) => t.clone(),
                Piece::Var { var } => number(vals[*var as usize]),
            })
            .collect()
    }

    /// Why line `li` cannot be said now (`None`: it can): its condition, its speaker's absence, or a reveal its speaker does not know.
    fn line_blocked(&self, li: u32, vals: &[f64]) -> Option<String> {
        let p = &self.lp.package;
        let l = &p.lines[li as usize];
        if let Some(e) = l.cond.filter(|e| !self.lp.exprs[*e as usize].truthy(vals)) {
            return Some(self.why_not(e, vals));
        }
        if let Some(s) = l.speaker {
            let sp = &p.speakers[s as usize];
            if !self.present(s, vals) {
                return Some(format!("{} is not here{}", sp.id, sp.present.map(|v| format!(" ({} = 0)", self.var_name(v as usize))).unwrap_or_default()));
            }
            if let Some(i) = l.reveals.iter().find(|i| !self.st.knows(sp.holder, **i, true)) {
                return Some(format!("{} does not know {}, which the line reveals", sp.id, p.infos[*i as usize].id));
            }
        }
        None
    }

    /// The choices of the line at `at` (global index) offered now: `(index among the line's choices, id, text)`.
    fn offered(&self, li: u32, vals: &[f64]) -> Vec<(usize, String, String)> {
        let l = &self.lp.package.lines[li as usize];
        l.choices.iter().enumerate().filter(|(_, c)| self.holds(c.cond, vals)).map(|(k, c)| (k, c.id.clone(), self.text(&c.text, vals))).collect()
    }

    fn line_view(&self, li: u32, vals: &[f64], until: Option<u64>) -> LineView {
        let p = &self.lp.package;
        let l = &p.lines[li as usize];
        let sp = l.speaker.map(|s| &p.speakers[s as usize]);
        LineView { id: l.id.clone(), speaker: sp.map(|s| s.id.clone()), name: sp.map(|s| s.name.clone()), text: self.text(&l.text, vals), until }
    }

    /// What a presenter shows now.
    pub fn view(&self, ask: &dyn Ask) -> Result<View, Diagnostic> {
        let vals = self.values(ask)?;
        let p = &self.lp.package;
        let talk = self.st.talk.as_ref().map(|t| {
            let c = &p.conversations[t.conv as usize];
            let li = c.first + t.at;
            let line = &p.lines[li as usize];
            let unavailable = line
                .choices
                .iter()
                .filter_map(|ch| Some((ch.id.clone(), ch.cond.filter(|e| !self.lp.exprs[*e as usize].truthy(&vals)).map(|e| self.why_not(e, &vals))?)))
                .collect();
            TalkView {
                conversation: c.id.clone(),
                line: self.line_view(li, &vals, None),
                choices: self.offered(li, &vals).into_iter().map(|(_, id, text)| (id, text)).collect(),
                unavailable,
                suspended: t.suspended,
                refusal: t.refusal.clone(),
            }
        });
        let bark = self.st.playing.and_then(|b| p.barks[b.bark as usize].line.map(|l| self.line_view(l, &vals, Some(b.ends))));
        Ok(View { revision: self.st.revision, talk, bark })
    }

    // ---- effects --------------------------------------------------------------------------------------------------------------------

    fn commands_of(&self, effects: &[Effect], vals: &[f64]) -> Vec<Command> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::Run { command, subject, value } => Some(Command {
                    name: command.clone(),
                    subject: subject.clone(),
                    value: value.map_or(0, |v| self.lp.exprs[v as usize].eval(vals).round() as i64),
                }),
                _ => None,
            })
            .collect()
    }

    /// Applies the story's own effects (facts and knowledge; barks offered and conversations started) of `effects`, written at `origin`.
    fn apply(&mut self, effects: &[Effect], origin: &str, vals: &[f64], cx: &mut Cx) {
        for e in effects {
            match e {
                Effect::Set { fact, expr } => self.set_fact(*fact, self.lp.exprs[*expr as usize].eval(vals)),
                Effect::Add { fact, expr } => self.set_fact(*fact, self.st.facts[*fact as usize] + self.lp.exprs[*expr as usize].eval(vals)),
                Effect::Learn { who, info, knows } => {
                    for h in self.holders(*who, vals) {
                        self.learn(h, *info, *knows, Source::Effect(origin.to_string()), cx.tick, vals);
                    }
                }
                Effect::Run { .. } => {}
                Effect::Bark { bark } => {
                    let what = format!("offered by {origin}");
                    self.offer(&[*bark], &what, vals, cx);
                }
                Effect::Start { conversation } => {
                    if self.st.talk.is_none() {
                        let _ = self.open(*conversation, vals, cx.tick);
                    }
                }
            }
        }
    }

    fn set_fact(&mut self, fact: u32, v: f64) {
        let f = &self.lp.package.facts[fact as usize];
        self.st.facts[fact as usize] = if f.ty == crate::sim::systems::Ty::Bool { (v != 0.0) as u8 as f64 } else { v.round() };
    }

    fn holders(&self, who: Who, vals: &[f64]) -> Vec<u16> {
        let p = &self.lp.package;
        match who {
            Who::Holder(h) => vec![h],
            Who::Present => p.speakers.iter().enumerate().filter(|(i, _)| self.present(*i as u16, vals)).map(|(_, s)| s.holder).collect(),
            Who::Everyone => (0..p.holders.len() as u16).collect(),
        }
    }

    /// `holder` learns `info` (or comes to believe it). Knowing is never undone by a belief; an untrue info is only ever believed.
    fn learn(&mut self, holder: u16, info: u16, knows: bool, source: Source, tick: u64, vals: &[f64]) {
        let truth = self.lp.package.infos[info as usize].truth;
        let knows = knows && self.holds(truth, vals);
        match self.st.known.get(&(holder, info)) {
            Some(k) if k.knows || !knows => {}
            _ => {
                self.st.known.insert((holder, info), Known { knows, source, tick });
            }
        }
    }

    /// Everyone listening to line `li` (the player and every present character but its speaker) learns what it reveals and believes
    /// what it claims; the line counts as heard.
    fn hear(&mut self, li: u32, vals: &[f64], tick: u64) {
        let p = Arc::clone(&self.lp);
        let l = &p.package.lines[li as usize];
        let speaker = l.speaker.map(|s| p.package.speakers[s as usize].holder);
        let listeners: Vec<u16> = std::iter::once(0).chain(self.holders(Who::Present, vals)).filter(|h| Some(*h) != speaker).collect();
        for h in listeners {
            for i in &l.reveals {
                self.learn(h, *i, true, Source::Line(li), tick, vals);
            }
            for i in &l.claims {
                self.learn(h, *i, false, Source::Line(li), tick, vals);
            }
        }
        self.st.heard[li as usize] += 1;
    }

    // ---- conversations ------------------------------------------------------------------------------------------------------------

    /// Why conversation `ci` cannot open now (`None`: it can).
    fn conv_blocked(&self, ci: u32, vals: &[f64]) -> Option<String> {
        let p = &self.lp.package;
        let c = &p.conversations[ci as usize];
        if self.st.ended[ci as usize] && !c.repeat {
            return Some("it has ended (it does not repeat)".into());
        }
        if !self.present(c.with, vals) {
            return Some(format!("{} is not here", p.speakers[c.with as usize].id));
        }
        c.cond.filter(|e| !self.lp.exprs[*e as usize].truthy(vals)).map(|e| self.why_not(e, vals))
    }

    /// Opens conversation `ci` at its first line that can be said.
    fn open(&mut self, ci: u32, vals: &[f64], tick: u64) -> Result<(), String> {
        if let Some(why) = self.conv_blocked(ci, vals) {
            return Err(why);
        }
        self.st.talk = Some(Talk { conv: ci, at: 0, suspended: false, refusal: None });
        self.settle_cursor(vals, tick);
        self.st.revision += 1;
        let id = self.lp.package.conversations[ci as usize].id.clone();
        if self.st.talk.is_none() {
            self.note(tick, format!("talk {id}"), "closed at once: no line of it can be said now", vec![]);
        } else {
            self.note(tick, format!("talk {id}"), "opened", vec![]);
        }
        Ok(())
    }

    /// Moves the open conversation past every line that cannot be said now (no effects, nothing heard), closing it past the end.
    fn settle_cursor(&mut self, vals: &[f64], tick: u64) {
        let Some(t) = self.st.talk.clone() else { return };
        let p = Arc::clone(&self.lp);
        let c = &p.package.conversations[t.conv as usize];
        let mut at = t.at;
        // Each skip moves on (a goto to an earlier line is allowed): a bound of one pass per line is enough for any loop that can end.
        for _ in 0..=c.len {
            if at >= c.len {
                break;
            }
            let li = c.first + at;
            let Some(why) = self.line_blocked(li, vals) else { break };
            let line = &p.package.lines[li as usize];
            self.note(tick, format!("line {}", line.id), format!("passed over: {why}"), vec![]);
            // A line that cannot be said goes on to the next line of its node, or, as its node's last, where it would have gone.
            at = line.skip.map_or(at + 1, |g| local(c.first, g));
        }
        if at >= c.len {
            self.close(t.conv, tick);
        } else if let Some(t) = &mut self.st.talk {
            t.at = at;
        }
    }

    fn close(&mut self, ci: u32, tick: u64) {
        self.st.talk = None;
        self.st.ended[ci as usize] = true;
        let id = self.lp.package.conversations[ci as usize].id.clone();
        self.note(tick, format!("talk {id}"), "ended", vec![]);
    }

    /// The player's answer at the open line: checked, then applied (with its commands, as one batch).
    fn answer(&mut self, choice: Option<&Id>, revision: i64, cx: &mut Cx) -> Result<(), Diagnostic> {
        let refuse = |m: String| Diagnostic::new(code::EVENT, "story", m);
        if revision as u64 != self.st.revision {
            return Err(refuse(format!("stale: it answers what was shown at revision {revision}; it is now {}", self.st.revision)));
        }
        let Some(t) = self.st.talk.clone() else { return Err(refuse("no conversation is open".into())) };
        if t.suspended {
            return Err(refuse("the conversation is suspended (a bark is being presented)".into()));
        }
        let vals = self.values(cx)?;
        let p = Arc::clone(&self.lp);
        let c = &p.package.conversations[t.conv as usize];
        let li = c.first + t.at;
        let line = &p.package.lines[li as usize];
        let k = match choice {
            None => None,
            Some(id) => {
                let (conv, local) = id.name().rsplit_once('/').unwrap_or(("", id.name()));
                let k = line.choices.iter().position(|ch| ch.id == local).filter(|_| conv.is_empty() || conv == c.id);
                let k = k.ok_or_else(|| refuse(format!("`{id}` is not a choice of the line shown ({})", line.id)))?;
                // Checked again now: what was offered may no longer hold.
                if let Some(e) = line.choices[k].cond.filter(|e| !p.exprs[*e as usize].truthy(&vals)) {
                    let why = self.why_not(e, &vals);
                    self.note(cx.tick, format!("choose {}", line.choices[k].id), format!("refused: {why}"), vec![]);
                    return Err(refuse(format!("`{}` is no longer available: {why}", line.choices[k].id)));
                }
                Some(k)
            }
        };
        // A line whose choices are all held back goes where it says otherwise (`otherwise -> node`), or closes the conversation.
        if k.is_none() && !line.choices.is_empty() && self.offered(li, &vals).is_empty() {
            self.st.revision += 1;
            self.st.seen[li as usize] += 1;
            self.hear(li, &vals, cx.tick);
            self.note(cx.tick, format!("move on from {}", line.id), "no choice is offered now: it goes on otherwise", vec![]);
            match line.goto {
                Some(next) => self.goto(t.conv, next, &vals, cx.tick),
                None => self.close(t.conv, cx.tick),
            }
            return Ok(());
        }
        let script = ConvScript { p: &p.package, conv: t.conv };
        talk::answer(&script, t.at as usize, k.map_or(Answer::MoveOn, Answer::Choose)).map_err(|e| refuse(e.to_string()))?;
        let mut commands = self.commands_of(&line.effects, &vals);
        if let Some(k) = k {
            commands.extend(self.commands_of(&line.choices[k].effects, &vals));
        }
        if commands.is_empty() {
            self.settled_answer(li, k, Ok(()), cx);
        } else {
            cx.issue(self.st.revision, commands)?;
            self.pending = Some(Pending::Answer { line: li, choice: k });
        }
        Ok(())
    }

    fn settled_answer(&mut self, li: u32, k: Option<usize>, result: Result<(), Diagnostic>, cx: &mut Cx) {
        self.pending = None;
        let Ok(vals) = self.values(cx) else { return };
        let p = Arc::clone(&self.lp);
        let line = &p.package.lines[li as usize];
        let Some(t) = self.st.talk.clone() else { return };
        let what = k.map_or_else(|| format!("move on from {}", line.id), |k| format!("choose {}", line.choices[k].id));
        self.st.revision += 1;
        if let Err(e) = result {
            let to = k.and_then(|k| line.choices[k].refused);
            self.note(cx.tick, what, format!("the game refused: {}", e.message), vec![]);
            match to {
                Some(next) => self.goto(t.conv, next, &vals, cx.tick),
                None => {
                    if let Some(t) = &mut self.st.talk {
                        t.refusal = Some(e.message);
                    }
                }
            }
            return;
        }
        self.st.seen[li as usize] += 1;
        self.hear(li, &vals, cx.tick);
        self.apply(&line.effects, &line.id, &vals, cx);
        if let Some(k) = k {
            let origin = format!("{}/{}", p.package.conversations[t.conv as usize].id, line.choices[k].id);
            self.apply(&line.choices[k].effects, &origin, &vals, cx);
        }
        self.note(cx.tick, what, "taken", vec![]);
        let script = ConvScript { p: &p.package, conv: t.conv };
        let moved = talk::answer(&script, t.at as usize, k.map_or(Answer::MoveOn, Answer::Choose));
        if let Some(talk) = &mut self.st.talk {
            talk.refusal = None;
        }
        match moved {
            Ok(m) if m.closed => self.close(t.conv, cx.tick),
            Ok(m) => {
                if let Some(talk) = &mut self.st.talk {
                    talk.at = m.next as u32;
                }
                let vals = self.values(cx).unwrap_or(vals);
                self.settle_cursor(&vals, cx.tick);
            }
            Err(_) => self.close(t.conv, cx.tick),
        }
    }

    fn goto(&mut self, ci: u32, next: Next, vals: &[f64], tick: u64) {
        let c = &self.lp.package.conversations[ci as usize];
        match next {
            Next::End => self.close(ci, tick),
            Next::Line(l) => {
                let at = l - c.first;
                if let Some(t) = &mut self.st.talk {
                    t.at = at;
                    t.refusal = None;
                }
                self.settle_cursor(vals, tick);
            }
        }
    }
}

/// Conversation `conv` as the shared core sees it: local line indices.
struct ConvScript<'a> {
    p: &'a super::package::Package,
    conv: u32,
}

fn local(first: u32, n: Next) -> u32 {
    match n {
        Next::Line(l) => l - first,
        Next::End => u32::MAX,
    }
}

fn goto_of(first: u32, n: Option<Next>) -> Option<Goto> {
    n.map(|n| match n {
        Next::Line(l) => Goto::Line((l - first) as usize),
        Next::End => Goto::End,
    })
}

impl talk::Script for ConvScript<'_> {
    fn lines(&self) -> usize {
        self.p.conversations[self.conv as usize].len as usize
    }
    fn choices(&self, line: usize) -> usize {
        let c = &self.p.conversations[self.conv as usize];
        self.p.lines[c.first as usize + line].choices.len()
    }
    fn goto(&self, line: usize) -> Option<Goto> {
        let c = &self.p.conversations[self.conv as usize];
        goto_of(c.first, self.p.lines[c.first as usize + line].goto)
    }
    fn choice_goto(&self, line: usize, choice: usize) -> Option<Goto> {
        let c = &self.p.conversations[self.conv as usize];
        goto_of(c.first, self.p.lines[c.first as usize + line].choices[choice].goto)
    }
}

/// A number as a line shows it: whole numbers without a point, others to one decimal.
fn number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v:.1}")
    }
}

/// Whether lowered expression text names `v<i>` (not `v<i>` as a prefix of a longer one).
fn contains_var(lowered: &str, i: usize) -> bool {
    let name = format!("v{i}");
    lowered.match_indices(&name).any(|(at, _)| {
        let before = lowered[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric() && c != '_');
        let after = lowered[at + name.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '_');
        before && after
    })
}

impl System for Story {
    fn id(&self) -> Id {
        self.me.clone()
    }

    fn hears(&self) -> Vec<Id> {
        self.lp.package.hears()
    }

    fn reads(&self) -> Vec<Id> {
        self.lp.package.reads().into_iter().map(|(id, _)| id).collect()
    }

    /// Each story fact, for the game to read (`<story>:<fact>`: a HUD, a gameplay system that cares what was promised).
    fn queries(&self) -> Vec<crate::sim::systems::QueryDecl> {
        let p = &self.lp.package;
        p.facts.iter().filter_map(|f| Some(crate::sim::systems::QueryDecl::new(Id::parse(&format!("{}:{}", p.story, f.id)).ok()?, f.ty, &f.doc))).collect()
    }

    fn query(&self, query: &Id, _look: &Look) -> Option<f64> {
        let p = &self.lp.package;
        (query.namespace() == p.story).then(|| p.facts.iter().position(|f| f.id == query.name())).flatten().map(|f| self.st.facts[f])
    }

    fn issues(&self) -> Vec<Id> {
        self.lp.package.commands().into_iter().map(|(id, _)| id).collect()
    }

    fn commands(&self) -> Vec<Id> {
        ["talk", "advance", "choose", "leave", "skip", "new_run"].iter().map(|c| self.cmd(c)).collect()
    }

    fn tick(&mut self, cx: &mut Cx) {
        if self.st.playing.is_some_and(|b| cx.tick >= b.ends) {
            self.heard_bark(cx);
        } else if self.st.playing.is_none() && !self.st.queue.is_empty() {
            self.resume_or_next(cx);
        }
        if self.lp.package.directors.is_empty() {
            return;
        }
        let due: Vec<usize> = (0..self.lp.package.directors.len()).filter(|d| cx.tick >= self.st.directing[*d].next).collect();
        if due.is_empty() || self.pending.is_some() {
            return;
        }
        match self.values(cx) {
            Ok(vals) => {
                for d in due {
                    if self.pending.is_none() {
                        self.direct(d, &vals, cx);
                    }
                }
            }
            Err(e) => self.problems.push(e),
        }
    }

    fn event(&mut self, ev: &crate::sim::systems::Event, cx: &mut Cx) {
        let p = Arc::clone(&self.lp);
        if p.package.round.as_ref() == Some(&ev.name) {
            self.st.new_round(&self.lp);
            self.note(cx.tick, format!("round {}", self.st.round), "round facts reset", vec![]);
        }
        let answers = |b: &Bark| b.on.as_ref().is_some_and(|t| t.event == ev.name && t.subject.as_ref().is_none_or(|s| ev.subject.as_ref() == Some(s)));
        let (notices, candidates): (Vec<u32>, Vec<u32>) =
            p.package.barks.iter().enumerate().filter(|(_, b)| answers(b)).map(|(i, _)| i as u32).partition(|i| p.package.barks[*i as usize].notice);
        if notices.is_empty() && candidates.is_empty() {
            return;
        }
        let vals = match self.values(cx) {
            Ok(v) => v,
            Err(e) => return self.problems.push(e),
        };
        // Every notice that holds applies (they record what witnesses see); then the barks choose who speaks.
        for ni in notices {
            let b = &p.package.barks[ni as usize];
            if b.once && self.st.chosen[ni as usize] > 0 {
                continue;
            }
            match b.cond.filter(|e| !p.exprs[*e as usize].truthy(&vals)) {
                Some(e) => {
                    let why = self.why_not(e, &vals);
                    self.note(cx.tick, format!("notice {}", b.id), format!("not noticed: {why}"), vec![]);
                }
                None => {
                    self.st.chosen[ni as usize] += 1;
                    self.apply(&b.effects, &b.id, &vals, cx);
                    self.note(cx.tick, format!("notice {}", b.id), "noticed", vec![]);
                }
            }
        }
        if !candidates.is_empty() {
            let vals = self.values(cx).unwrap_or(vals);
            let what = format!("bark on {}{}", ev.name, ev.subject.as_ref().map(|s| format!(" about {s}")).unwrap_or_default());
            self.offer(&candidates, &what, &vals, cx);
        }
    }

    fn command(&mut self, c: &Command, cx: &mut Cx) -> Result<(), Diagnostic> {
        let refuse = |m: String| Diagnostic::new(code::EVENT, c.name.as_str(), m);
        if self.pending.is_some() {
            return Err(refuse("a decision is still being applied".into()));
        }
        match c.name.name() {
            "talk" => {
                if let Some(t) = &self.st.talk {
                    if t.suspended {
                        return Err(refuse("a conversation is suspended by a bark; it resumes by itself".into()));
                    }
                    return Err(refuse("a conversation is already open".into()));
                }
                let who = c.subject.as_ref().map(|s| s.name().to_string()).ok_or_else(|| refuse("talk to whom? (the subject is a speaker)".into()))?;
                let p = Arc::clone(&self.lp);
                let s = p.package.speakers.iter().position(|s| s.id == who).ok_or_else(|| refuse(format!("`{who}` is not a speaker of this story")))? as u16;
                let vals = self.values(cx)?;
                let mut options: Vec<(i32, std::cmp::Reverse<usize>, u32)> = Vec::new();
                let mut why = Vec::new();
                for (ci, conv) in p.package.conversations.iter().enumerate().filter(|(_, conv)| conv.with == s) {
                    match self.conv_blocked(ci as u32, &vals) {
                        None => options.push((conv.priority, std::cmp::Reverse(ci), ci as u32)),
                        Some(r) => why.push(format!("{}: {r}", conv.id)),
                    }
                }
                let Some((_, _, ci)) = options.into_iter().max() else {
                    self.note(cx.tick, format!("talk to {who}"), "nothing to say", why.clone());
                    return Err(refuse(format!("{who} has nothing to say now ({})", if why.is_empty() { "no conversation".into() } else { why.join("; ") })));
                };
                self.open(ci, &vals, cx.tick).map_err(refuse)
            }
            "advance" => self.answer(None, c.value, cx),
            "choose" => {
                let choice = c.subject.clone().ok_or_else(|| refuse("choose what? (the subject is a choice)".into()))?;
                self.answer(Some(&choice), c.value, cx)
            }
            "leave" => {
                let t = self.st.talk.clone().ok_or_else(|| refuse("no conversation is open".into()))?;
                self.st.talk = None;
                self.st.revision += 1;
                let id = self.lp.package.conversations[t.conv as usize].id.clone();
                self.note(cx.tick, format!("leave {id}"), "left before the end: it opens again from the start", vec![]);
                Ok(())
            }
            "skip" => {
                if c.value as u64 != self.st.revision {
                    return Err(refuse(format!("stale: revision {} is now {}", c.value, self.st.revision)));
                }
                if self.st.playing.is_none() {
                    return Err(refuse("no bark is being presented".into()));
                }
                self.heard_bark(cx);
                Ok(())
            }
            "new_run" => {
                self.st = self.st.new_run(&self.lp);
                self.note(cx.tick, "new run", "everything but profile facts reset", vec![]);
                Ok(())
            }
            other => Err(refuse(format!("the story takes no `{other}`"))),
        }
    }

    fn settled(&mut self, _tag: u64, result: Result<(), Diagnostic>, cx: &mut Cx) {
        match self.pending.take() {
            Some(Pending::Answer { line, choice }) => self.settled_answer(line, choice, result, cx),
            Some(Pending::Heard { bark }) => {
                let p = Arc::clone(&self.lp);
                let b = &p.package.barks[bark as usize];
                match (result, b.line, self.values(cx)) {
                    (Ok(()), Some(li), Ok(vals)) => self.apply(&p.package.lines[li as usize].effects, &b.id, &vals, cx),
                    (Err(e), _, _) => self.note(cx.tick, format!("bark {}", b.id), format!("heard, but the game refused its effects: {}", e.message), vec![]),
                    _ => {}
                }
            }
            Some(Pending::Intervene { director, intervention }) => {
                let id = self.lp.package.directors[director as usize].interventions[intervention as usize].id.clone();
                match (result, self.values(cx)) {
                    (Ok(()), Ok(vals)) => self.intervened(director as usize, intervention as usize, &vals, cx),
                    (Err(e), _) => {
                        self.note(cx.tick, format!("director intervention {id}"), format!("the game refused it (nothing used): {}", e.message), vec![])
                    }
                    _ => {}
                }
            }
            None => {}
        }
    }

    fn save(&self) -> Value {
        self.st.save(&self.lp)
    }

    fn load(&mut self, state: Value) -> Result<(), String> {
        let (st, notes) = State::load(&self.lp, &state).map_err(|e| e.message)?;
        self.st = st;
        self.pending = None;
        self.tally.clear();
        for n in notes {
            self.problems.push(Diagnostic::new(code::STATE_VERSION, "save#/story", n));
        }
        Ok(())
    }

    fn hash(&self) -> u64 {
        self.st.hash()
    }

    fn instances(&self) -> Vec<Id> {
        let p = &self.lp.package;
        let ns = &p.story;
        p.conversations
            .iter()
            .map(|c| &c.id)
            .chain(p.barks.iter().map(|b| &b.id))
            .chain(p.directors.iter().map(|d| &d.id))
            .filter_map(|i| Id::parse(&format!("{ns}:{i}")).ok())
            .collect()
    }

    fn inspect(&self, target: &Id, _look: &Look) -> Option<Inspection> {
        // Without the game's answers only what the story itself holds: games ask [`inspect`] (with the host and the match) for the live why.
        (target == &self.me).then(|| Inspection {
            id: self.me.clone(),
            system: self.me.clone(),
            instance_at: None,
            definition: None,
            defined_at: None,
            status: if self.st.talk.is_some() {
                "talking"
            } else if self.st.playing.is_some() {
                "barking"
            } else {
                "idle"
            }
            .into(),
            blocked_by: None,
            state: self.save(),
        })
    }
}

/// The story system in `systems`, if the game runs one.
pub fn story_of(systems: &Systems) -> Option<&Story> {
    systems.get::<Story>()
}

/// A player's [`Intent`] for the story (`story:talk` with a speaker, `story:advance`, `story:choose` with a choice id, `story:leave`,
/// `story:skip`, `story:new_run`; `value` is the revision the player was shown), as the game's `MatchGame::intend` passes it on.
pub fn intend(systems: &mut Systems, outside: &dyn Outside, intent: &Intent) -> Result<(), String> {
    let story = story_of(systems).ok_or("this game runs no story")?;
    let ns = story.lp.package.story.clone();
    let what = intent.name.strip_prefix("story:").ok_or_else(|| format!("`{}` is not a story intention (`story:talk`, `story:choose`, ...)", intent.name))?;
    let subject = match (&intent.subject, what) {
        (None, _) => None,
        (Some(s), "choose") if !s.contains(':') => {
            let conv = story.st.talk.as_ref().map(|t| story.lp.package.conversations[t.conv as usize].id.clone()).unwrap_or_default();
            Some(Id::parse(&format!("{ns}:{conv}/{s}"))?)
        }
        (Some(s), _) => Some(Id::resolve(&ns, s)?),
    };
    let name = Id::parse(&format!("{ns}:{what}"))?;
    systems.command_with(&Command { name, subject, value: intent.value }, outside).map_err(|e| e.message)
}

/// `target` explained live, for a game's `MatchGame::inspect` (`story` alone is the story itself): `None` when the game runs no story or
/// the story has no such id.
pub fn inspect(systems: &Systems, outside: &dyn Outside, target: &str) -> Option<Value> {
    let story = story_of(systems)?;
    let target = if target == "story" { story.me.to_string() } else { target.to_string() };
    story.explain(&target, &HostAsk { systems, outside })
}
