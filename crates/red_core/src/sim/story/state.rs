//! The story's own state, as the runtime ([`super::runtime`]) keeps it: indices into its [`Loaded`] package. It is saved by **id**
//! ([`State::save`], [`State::load`]): a save names facts, lines, barks and conversations as the author wrote them, so a save outlives an
//! edit that moves things around, and what an edit removed is reported, never guessed at.

use super::package::{Loaded, Scope};
use crate::sim::defs::{code, Diagnostic};
use crate::sim::ledger::Fnv;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// How someone came to know (or believe) something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Their character sheet: they knew it when the run began.
    Sheet,
    /// A line they heard (index into the lines).
    Line(u32),
    /// An effect: `learn` in a line, a choice, a bark or an intervention (its id).
    Effect(String),
}

impl Source {
    fn save(&self, lines: &[super::package::Line]) -> String {
        match self {
            Source::Sheet => "sheet".into(),
            Source::Line(l) => format!("line:{}", lines.get(*l as usize).map_or("?", |l| l.id.as_str())),
            Source::Effect(at) => format!("effect:{at}"),
        }
    }

    fn load(s: &str, line_of: &BTreeMap<&str, u32>) -> Option<Source> {
        match s.split_once(':') {
            None if s == "sheet" => Some(Source::Sheet),
            Some(("line", id)) => line_of.get(id).map(|l| Source::Line(*l)),
            Some(("effect", at)) => Some(Source::Effect(at.to_string())),
            _ => None,
        }
    }
}

/// What a holder knows or believes about one info.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    /// `true`: knows it; `false`: believes it (a claim, or a reveal of something untrue).
    pub knows: bool,
    /// How.
    pub source: Source,
    /// When (tick).
    pub tick: u64,
}

/// The open conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Talk {
    /// Index into the conversations.
    pub conv: u32,
    /// The line shown, as an index inside the conversation.
    pub at: u32,
    /// A bark suspended it; it resumes when that bark ends.
    pub suspended: bool,
    /// The game refused the last choice's commands: why (shown until the next answer).
    pub refusal: Option<String>,
}

/// The bark being presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Playing {
    /// Index into the barks.
    pub bark: u32,
    /// When it started (tick).
    pub started: u64,
    /// When it is heard in full (tick).
    pub ends: u64,
    /// Its priority (a higher one interrupts it).
    pub priority: i32,
}

/// A bark waiting its turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Queued {
    /// Index into the barks.
    pub bark: u32,
    /// Its priority.
    pub priority: i32,
    /// When it was chosen (tick): it goes stale after [`super::runtime::QUEUE_TTL`].
    pub chosen: u64,
}

/// One director's bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Directing {
    /// The tick it next evaluates at.
    pub next: u64,
    /// No intervention before this tick.
    pub recover_until: u64,
    /// Interventions this round.
    pub this_round: u32,
    /// `once` interventions already made (by index).
    pub used: BTreeSet<u32>,
    /// Groups used this round.
    pub groups: BTreeSet<String>,
}

/// Everything the story remembers.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct State {
    /// Each fact's value.
    pub facts: Vec<f64>,
    /// `(holder, info)` -> what they know.
    pub known: BTreeMap<(u16, u16), Known>,
    /// Per line: times heard in full.
    pub heard: Vec<u32>,
    /// Per line: times shown (heard, interrupted or skipped past).
    pub seen: Vec<u32>,
    /// Per bark: times chosen.
    pub chosen: Vec<u32>,
    /// Per bark: the tick it was last chosen.
    pub last: Vec<Option<u64>>,
    /// Per speaker: no bark of theirs before this tick, unless it is more important than the priority after the tick.
    pub speaker_quiet: Vec<(u64, i32)>,
    /// Per category: no bark of it before this tick.
    pub category_quiet: Vec<u64>,
    /// Per conversation: it ended (and does not repeat).
    pub ended: Vec<bool>,
    /// The open conversation.
    pub talk: Option<Talk>,
    /// The bark being presented.
    pub playing: Option<Playing>,
    /// Barks waiting, highest priority first (then the earliest chosen).
    pub queue: Vec<Queued>,
    /// Per director.
    pub directing: Vec<Directing>,
    /// Rises with every change a presenter shows: an answer must name the revision it was given ([`super::runtime`]).
    pub revision: u64,
    /// Rounds begun (the package's round event).
    pub round: u32,
}

impl State {
    /// The state a run begins in: every fact at its default, each speaker's sheet knowledge, nothing heard.
    pub fn fresh(lp: &Loaded) -> State {
        let p = &lp.package;
        let mut s = State {
            facts: p.facts.iter().map(|f| f.default).collect(),
            heard: vec![0; p.lines.len()],
            seen: vec![0; p.lines.len()],
            chosen: vec![0; p.barks.len()],
            last: vec![None; p.barks.len()],
            speaker_quiet: vec![(0, i32::MIN); p.speakers.len()],
            category_quiet: vec![0; p.categories.len()],
            ended: vec![false; p.conversations.len()],
            directing: vec![Directing::default(); p.directors.len()],
            ..State::default()
        };
        for sp in &p.speakers {
            for info in &sp.knows {
                s.known.insert((sp.holder, *info), Known { knows: true, source: Source::Sheet, tick: 0 });
            }
        }
        s
    }

    /// A new run: everything as [`fresh`](State::fresh), but the profile facts kept.
    pub fn new_run(&self, lp: &Loaded) -> State {
        let mut s = State::fresh(lp);
        for (i, f) in lp.package.facts.iter().enumerate() {
            if f.scope == Scope::Profile {
                s.facts[i] = self.facts[i];
            }
        }
        s.revision = self.revision + 1;
        s
    }

    /// A new round: round facts back to their defaults, the directors' round budgets and groups cleared.
    pub fn new_round(&mut self, lp: &Loaded) {
        for (i, f) in lp.package.facts.iter().enumerate() {
            if f.scope == Scope::Round {
                self.facts[i] = f.default;
            }
        }
        for d in &mut self.directing {
            d.this_round = 0;
            d.groups.clear();
        }
        self.round += 1;
    }

    /// Whether `holder` knows (`knows`) or at least believes `info`.
    pub fn knows(&self, holder: u16, info: u16, knows: bool) -> bool {
        self.known.get(&(holder, info)).is_some_and(|k| k.knows || !knows)
    }

    /// A hash of everything that decides what happens next (the match checksum folds it in).
    pub fn hash(&self) -> u64 {
        let mut h = Fnv::new();
        for f in &self.facts {
            h.u64(f.to_bits());
        }
        for ((holder, info), k) in &self.known {
            h.u64(((*holder as u64) << 32) | *info as u64);
            h.u64(k.knows as u64);
            h.u64(k.tick);
        }
        for v in [&self.heard, &self.seen, &self.chosen] {
            for n in v {
                h.u64(*n as u64);
            }
        }
        for l in &self.last {
            h.u64(l.unwrap_or(u64::MAX));
        }
        for (t, pr) in &self.speaker_quiet {
            h.u64(*t);
            h.u64(*pr as u64);
        }
        for t in &self.category_quiet {
            h.u64(*t);
        }
        for e in &self.ended {
            h.u64(*e as u64);
        }
        if let Some(t) = &self.talk {
            h.u64(t.conv as u64);
            h.u64(t.at as u64);
            h.u64(t.suspended as u64);
        }
        if let Some(p) = &self.playing {
            h.u64(p.bark as u64);
            h.u64(p.ends);
        }
        for q in &self.queue {
            h.u64(q.bark as u64);
            h.u64(q.chosen);
        }
        for d in &self.directing {
            h.u64(d.next);
            h.u64(d.recover_until);
            h.u64(d.this_round as u64);
            h.u64(d.used.len() as u64);
            h.u64(d.groups.len() as u64);
        }
        h.u64(self.revision);
        h.u64(self.round as u64);
        h.0
    }

    /// The state by id: `{"content": {...}, "facts": {id: n}, "known": [[holder, info, knows, source, tick]], "heard": {line: n}, ...}`.
    pub fn save(&self, lp: &Loaded) -> Value {
        let p = &lp.package;
        let counts = |v: &[u32], id: &dyn Fn(usize) -> String| -> Map<String, Value> {
            v.iter().enumerate().filter(|(_, n)| **n > 0).map(|(i, n)| (id(i), json!(n))).collect()
        };
        let line_id = |i: usize| p.lines[i].id.clone();
        let bark_id = |i: usize| p.barks[i].id.clone();
        json!({
            "content": {"story": p.story, "version": p.content_version, "hash": p.source_hash},
            "facts": p.facts.iter().zip(&self.facts).map(|(f, v)| (f.id.clone(), json!(v))).collect::<Map<_, _>>(),
            "known": self.known.iter().map(|((h, i), k)| json!([p.holders[*h as usize], p.infos[*i as usize].id, k.knows, k.source.save(&p.lines), k.tick])).collect::<Vec<_>>(),
            "heard": counts(&self.heard, &line_id),
            "seen": counts(&self.seen, &line_id),
            "chosen": counts(&self.chosen, &bark_id),
            "last": self.last.iter().enumerate().filter_map(|(i, t)| Some((bark_id(i), json!((*t)?)))).collect::<Map<_, _>>(),
            "speaker_quiet": p.speakers.iter().zip(&self.speaker_quiet).filter(|(_, (t, _))| *t > 0).map(|(s, (t, pr))| (s.id.clone(), json!([t, pr]))).collect::<Map<_, _>>(),
            "category_quiet": p.categories.iter().zip(&self.category_quiet).filter(|(_, t)| **t > 0).map(|(c, t)| (c.id.clone(), json!(t))).collect::<Map<_, _>>(),
            "ended": p.conversations.iter().zip(&self.ended).filter(|(_, e)| **e).map(|(c, _)| json!(c.id)).collect::<Vec<_>>(),
            "talk": self.talk.as_ref().map(|t| json!({
                "conversation": p.conversations[t.conv as usize].id,
                "line": p.lines[(p.conversations[t.conv as usize].first + t.at) as usize].id,
                "suspended": t.suspended,
                "refusal": t.refusal,
            })),
            "playing": self.playing.map(|b| json!({"bark": bark_id(b.bark as usize), "started": b.started, "ends": b.ends, "priority": b.priority})),
            "queue": self.queue.iter().map(|q| json!({"bark": bark_id(q.bark as usize), "priority": q.priority, "chosen": q.chosen})).collect::<Vec<_>>(),
            "directing": p.directors.iter().zip(&self.directing).map(|(d, s)| (d.id.clone(), json!({
                "next": s.next, "recover_until": s.recover_until, "this_round": s.this_round,
                "used": s.used.iter().map(|i| d.interventions[*i as usize].id.clone()).collect::<Vec<_>>(),
                "groups": s.groups,
            }))).collect::<Map<_, _>>(),
            "revision": self.revision,
            "round": self.round,
        })
    }

    /// Reads a [`save`](State::save) against `lp`: by id, so content that changed since is matched where it can be. Returns the state and
    /// what could not be matched (each a note); a save of another story, of newer content, or malformed, is refused.
    pub fn load(lp: &Loaded, v: &Value) -> Result<(State, Vec<String>), Diagnostic> {
        let p = &lp.package;
        let bad = |m: String| Diagnostic::new(code::STATE_VERSION, "save#/story", m);
        let content = v.get("content").ok_or_else(|| bad("no `content`".into()))?;
        let story = content.get("story").and_then(Value::as_str).unwrap_or("");
        if story != p.story {
            return Err(bad(format!("a save of story `{story}`; this is `{}`", p.story)));
        }
        let version = content.get("version").and_then(Value::as_u64).ok_or_else(|| bad("no content `version`".into()))? as u32;
        if version > p.content_version {
            return Err(bad(format!("saved with content version {version}; this package is version {}", p.content_version))
                .fix("load it with the content that saved it, or newer"));
        }
        let mut notes = Vec::new();
        let mut s = State::fresh(lp);
        s.known.clear();
        let line_of: BTreeMap<&str, u32> = p.lines.iter().enumerate().map(|(i, l)| (l.id.as_str(), i as u32)).collect();
        let bark_of: BTreeMap<&str, u32> = p.barks.iter().enumerate().map(|(i, b)| (b.id.as_str(), i as u32)).collect();
        let obj = |k: &str| v.get(k).and_then(Value::as_object).cloned().unwrap_or_default();
        let num = |x: &Value, what: &str| x.as_f64().ok_or_else(|| bad(format!("{what}: not a number")));
        for (id, x) in obj("facts") {
            match p.facts.iter().position(|f| f.id == id) {
                Some(i) => s.facts[i] = num(&x, &id)?,
                None => notes.push(format!("fact `{id}` is no longer in the story: dropped")),
            }
        }
        for row in v.get("known").and_then(Value::as_array).into_iter().flatten() {
            let r = row.as_array().filter(|r| r.len() == 5).ok_or_else(|| bad(format!("known: malformed entry {row}")))?;
            let (holder, info) = (r[0].as_str().unwrap_or(""), r[1].as_str().unwrap_or(""));
            let h = p.holders.iter().position(|x| x == holder);
            let i = p.infos.iter().position(|x| x.id == info);
            let source = Source::load(r[3].as_str().unwrap_or(""), &line_of);
            match (h, i, source) {
                (Some(h), Some(i), Some(source)) => {
                    s.known.insert((h as u16, i as u16), Known { knows: r[2].as_bool().unwrap_or(false), source, tick: r[4].as_u64().unwrap_or(0) });
                }
                _ => notes.push(format!("what `{holder}` knew of `{info}` no longer matches the story: dropped")),
            }
        }
        let mut counts = |key: &str, of: &BTreeMap<&str, u32>, into: &mut Vec<u32>| -> Result<(), Diagnostic> {
            for (id, n) in obj(key) {
                match of.get(id.as_str()) {
                    Some(i) => into[*i as usize] = num(&n, &id)? as u32,
                    None => notes.push(format!("{key} `{id}` is no longer in the story: dropped")),
                }
            }
            Ok(())
        };
        counts("heard", &line_of, &mut s.heard)?;
        counts("seen", &line_of, &mut s.seen)?;
        counts("chosen", &bark_of, &mut s.chosen)?;
        for (id, t) in obj("last") {
            if let Some(i) = bark_of.get(id.as_str()) {
                s.last[*i as usize] = t.as_u64();
            }
        }
        for (id, t) in obj("speaker_quiet") {
            if let Some(i) = p.speakers.iter().position(|x| x.id == id) {
                s.speaker_quiet[i] = (t.get(0).and_then(Value::as_u64).unwrap_or(0), t.get(1).and_then(Value::as_i64).unwrap_or(0) as i32);
            }
        }
        for (id, t) in obj("category_quiet") {
            if let Some(i) = p.categories.iter().position(|x| x.id == id) {
                s.category_quiet[i] = t.as_u64().unwrap_or(0);
            }
        }
        for id in v.get("ended").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            if let Some(i) = p.conversations.iter().position(|c| c.id == id) {
                s.ended[i] = true;
            }
        }
        if let Some(t) = v.get("talk").filter(|t| !t.is_null()) {
            let conv = t.get("conversation").and_then(Value::as_str).unwrap_or("");
            let line = t.get("line").and_then(Value::as_str).unwrap_or("");
            match (p.conversations.iter().position(|c| c.id == conv), line_of.get(line)) {
                (Some(ci), Some(&li)) if li >= p.conversations[ci].first && li < p.conversations[ci].first + p.conversations[ci].len => {
                    s.talk = Some(Talk {
                        conv: ci as u32,
                        at: li - p.conversations[ci].first,
                        suspended: t.get("suspended").and_then(Value::as_bool).unwrap_or(false),
                        refusal: t.get("refusal").and_then(Value::as_str).map(str::to_string),
                    });
                }
                _ => notes.push(format!("the open conversation (`{conv}` at `{line}`) is no longer in the story: it is closed")),
            }
        }
        if let Some(b) = v.get("playing").filter(|b| !b.is_null()) {
            match b.get("bark").and_then(Value::as_str).and_then(|id| bark_of.get(id)) {
                Some(&bark) => {
                    let t = |k: &str| b.get(k).and_then(Value::as_u64).unwrap_or(0);
                    s.playing =
                        Some(Playing { bark, started: t("started"), ends: t("ends"), priority: b.get("priority").and_then(Value::as_i64).unwrap_or(0) as i32 });
                }
                None => notes.push("the bark being presented is no longer in the story: dropped".into()),
            }
        }
        for q in v.get("queue").and_then(Value::as_array).into_iter().flatten() {
            if let Some(&bark) = q.get("bark").and_then(Value::as_str).and_then(|id| bark_of.get(id)) {
                let priority = q.get("priority").and_then(Value::as_i64).unwrap_or(0) as i32;
                s.queue.push(Queued { bark, priority, chosen: q.get("chosen").and_then(Value::as_u64).unwrap_or(0) });
            }
        }
        let directing = obj("directing");
        for (di, d) in p.directors.iter().enumerate() {
            let Some(x) = directing.get(&d.id) else { continue };
            let t = |k: &str| x.get(k).and_then(Value::as_u64).unwrap_or(0);
            let st = &mut s.directing[di];
            (st.next, st.recover_until, st.this_round) = (t("next"), t("recover_until"), t("this_round") as u32);
            for id in x.get("used").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                if let Some(i) = d.interventions.iter().position(|i| i.id == id) {
                    st.used.insert(i as u32);
                }
            }
            st.groups = x.get("groups").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        }
        s.revision = v.get("revision").and_then(Value::as_u64).unwrap_or(0);
        s.round = v.get("round").and_then(Value::as_u64).unwrap_or(0) as u32;
        Ok((s, notes))
    }
}
