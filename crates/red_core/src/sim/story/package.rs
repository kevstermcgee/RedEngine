//! A compiled story: what the content toolchain (`red_engine2 story build`) writes and the runtime ([`super::runtime`]) loads. Immutable,
//! indexed tables (lines, conversations, barks, directors, the expression variables), each entry with its human-readable id and the
//! `file:line` it was written at, so a diagnostic, a save and an inspection speak in ids while the runtime works in indices.
//!
//! Expressions are the rules' own language (`sim::rules_expr`), lowered by the toolchain: every name an author wrote (`tw:shed_fuel`,
//! `lent_fuel`, `knows(odile, gannet_lost)`, `heard(talk_bram/gannet)`) is one entry of [`Package::vars`], and the expression text names it
//! `v<index>`. [`Loaded::new`] checks the whole package ([`check`]: format, indices, gotos, expressions) and binds it to the game
//! ([`bind`]: every query, command and event is one the game declares, with the type the story expects); the toolchain runs the same two
//! functions before it writes a package, so the compiler and the runtime cannot disagree about what is valid.

use super::super::defs::{Diagnostic, Id};
use super::super::rules_expr::{self, Expr};
use super::super::systems::{QueryDecl, Ty};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The package format this runtime reads (`"story_package"`). A newer one is refused, never guessed at.
pub const FORMAT: u32 = 1;

/// Stable diagnostic codes of story content (also in `tools::envelope::CODES`).
pub mod code {
    /// The package (or a save) is of a format or content version this runtime cannot read.
    pub const VERSION: &str = "story-version";
    /// An index, id or goto that points at nothing.
    pub const REFERENCE: &str = "story-reference";
    /// A query, command or event the game does not declare, or of another type.
    pub const BINDING: &str = "story-binding";
    /// An expression that does not compile, or of the wrong type for where it is used.
    pub const EXPRESSION: &str = "story-expression";
    /// A source file that does not parse.
    pub const SYNTAX: &str = "story-syntax";
    /// A character sheet missing a field its profile requires.
    pub const PROFILE: &str = "story-profile";
    /// A conversation or setup that cannot go on: a dead end, a loop that never waits for the player, an unreachable required line.
    pub const FLOW: &str = "story-flow";
    /// A warning: content that is valid but probably not what was meant (a write nobody reads, a setup nothing pays off, ...).
    pub const WARNING: &str = "story-warning";
}

/// Where something was written: a source file (index into [`Package::sources`]) and a 1-based line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Src {
    /// Index into [`Package::sources`].
    pub file: u16,
    /// 1-based line.
    pub line: u32,
}

/// One entry of the expression variable table: what fills `v<index>` when an expression is evaluated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Var {
    /// One of the story's own facts ([`Package::facts`]).
    Fact {
        /// Index into the facts.
        fact: u32,
    },
    /// A query the game answers (a system's `ns:name`, or the scene's `scene:var`), of the type the story expects.
    Query {
        /// The query.
        id: Id,
        /// Its type.
        ty: Ty,
    },
    /// `knows(holder, info)`: 1 when the holder knows it.
    Knows {
        /// Index into [`Package::holders`].
        holder: u16,
        /// Index into [`Package::infos`].
        info: u16,
    },
    /// `believes(holder, info)`: 1 when the holder believes it (a claim they heard) or knows it.
    Believes {
        /// Index into the holders.
        holder: u16,
        /// Index into the infos.
        info: u16,
    },
    /// `heard(line)`: 1 once the line was presented in full.
    Heard {
        /// Index into [`Package::lines`].
        line: u32,
    },
}

/// How long a story fact lasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Reset when the package's round event happens.
    Round,
    /// The whole run (saved with it; reset by `story:new_run`).
    Run,
    /// Kept across runs (`story:new_run` keeps it).
    Profile,
}

/// A fact the story owns (narrative-only state: what was said, promised, decided). Gameplay state is never a fact: it is a query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    /// Its id (`lent_fuel`).
    pub id: String,
    /// `bool` or `int`.
    pub ty: Ty,
    /// Its value at the start.
    pub default: f64,
    /// How long it lasts.
    pub scope: Scope,
    /// What it means.
    pub doc: String,
    /// Where it is declared.
    pub at: Src,
}

/// Something that can be known: true or not in the world, told, believed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Info {
    /// Its id (`gannet_lost`).
    pub id: String,
    /// What it says.
    pub doc: String,
    /// Whether it is true in the world (an expression), when the story says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truth: Option<u32>,
    /// Where it is declared.
    pub at: Src,
}

/// Someone who speaks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Speaker {
    /// Its id (`odile`).
    pub id: String,
    /// The name a presenter shows.
    pub name: String,
    /// Its index in [`Package::holders`].
    pub holder: u16,
    /// The variable that says whether they are here now (a bool query); `None`: always present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub present: Option<u32>,
    /// Ticks between two barks of theirs.
    #[serde(default)]
    pub cooldown: u32,
    /// What they know when the run begins.
    #[serde(default)]
    pub knows: Vec<u16>,
    /// Where they are declared.
    pub at: Src,
}

/// A piece of a line's text: words, or the value of a variable (`{tw:shed_fuel}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Piece {
    /// Words.
    Text(String),
    /// A number from the variable table.
    Var {
        /// Index into [`Package::vars`].
        var: u32,
    },
}

/// Who learns (or comes to believe) something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Who {
    /// One holder ([`Package::holders`]: 0 is the player).
    Holder(u16),
    /// Every character who is present now (not the player).
    Present,
    /// The player and every character.
    Everyone,
}

/// One effect of a line, a choice, a bark or a director's intervention.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Effect {
    /// `set fact = expr`.
    Set {
        /// Index into the facts.
        fact: u32,
        /// Index into [`Package::exprs`].
        expr: u32,
    },
    /// `add fact expr`.
    Add {
        /// Index into the facts.
        fact: u32,
        /// Index into the expressions.
        expr: u32,
    },
    /// `learn who info` / `believe who info`.
    Learn {
        /// Who.
        who: Who,
        /// Index into the infos.
        info: u16,
        /// `false`: they come to believe it (a claim), not know it.
        knows: bool,
    },
    /// `run cmd [on subject] [value]`: a command the game takes, applied with the decision's others as one batch.
    Run {
        /// The command.
        command: Id,
        /// What about.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<Id>,
        /// The value (an expression), `None`: 0.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<u32>,
    },
    /// `bark id`: offer a bark now (it is still checked like any other).
    Bark {
        /// Index into [`Package::barks`].
        bark: u32,
    },
    /// `start conversation`: open a conversation (only when none is open and its speaker is here).
    Start {
        /// Index into [`Package::conversations`].
        conversation: u32,
    },
}

/// Where a line or a choice goes ([`super::talk::Goto`], with lines as indices into [`Package::lines`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Next {
    /// The line at this index (inside the same conversation).
    Line(u32),
    /// The conversation ends.
    End,
}

/// What a choice is for, as its author declared it (checked against what it actually changes, `story ledger`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stakes {
    /// It changes play: gameplay, a fact something reads, a relationship.
    Consequential,
    /// It reveals something (knowledge).
    Informational,
    /// It changes only how the moment feels (the player's voice).
    Expressive,
}

/// One choice offered with a line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    /// Its id inside the conversation (`lend`).
    pub id: String,
    /// What the player picks.
    pub text: Vec<Piece>,
    /// When it is offered (index into the expressions), checked again when it is picked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cond: Option<u32>,
    /// What picking it does (after the line's own effects).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    /// Where it goes (`None`: where its line goes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goto: Option<Next>,
    /// Where it goes when the game refuses its commands (`None`: it stays, and the refusal is shown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<Next>,
    /// What it is for.
    pub stakes: Stakes,
    /// Where it is written.
    pub at: Src,
}

/// One line: a conversation's or a bark's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Line {
    /// Its full id (`talk_bram/ask/l1`, or the bark's id).
    pub id: String,
    /// Who says it (index into [`Package::speakers`]); `None`: narration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<u16>,
    /// What is said.
    pub text: Vec<Piece>,
    /// When it is said (a conversation skips a line whose condition is false; a bark is not chosen).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cond: Option<u32>,
    /// What it does once heard.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    /// Infos the speaker tells as true (they must know them; every listener then knows them).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reveals: Vec<u16>,
    /// Infos the speaker claims without knowing (speculation, or a lie: listeners come to believe them).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub claims: Vec<u16>,
    /// The choices offered with it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<Choice>,
    /// Where it goes (`None`: the next line).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goto: Option<Next>,
    /// Where it goes when it is passed over (its condition false, its speaker away): `None` is the next line of its node; a node's last
    /// line goes where it would have gone (`goto`), or the conversation ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip: Option<Next>,
    /// Where it is written.
    pub at: Src,
}

/// A conversation: a contiguous run of [`Package::lines`], opened by talking to its speaker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    /// Its id (`talk_bram`).
    pub id: String,
    /// The speaker the player talks to, to open it.
    pub with: u16,
    /// Its first line (index into the lines) and how many it has.
    pub first: u32,
    /// How many lines it has.
    pub len: u32,
    /// When it may open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cond: Option<u32>,
    /// Several open at once: the highest first, then the first written.
    #[serde(default)]
    pub priority: i32,
    /// A bark of this priority or more suspends it (it resumes after); `None`: nothing does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interrupt: Option<i32>,
    /// Whether it may open again once it ended.
    #[serde(default)]
    pub repeat: bool,
    /// Where it is written.
    pub at: Src,
}

/// What a bark answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trigger {
    /// The event (`tw:fuel_added`).
    pub event: Id,
    /// Only for an event about this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Id>,
}

/// A contextual line (a bark): a rule answering an event, chosen among the others answering it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bark {
    /// Its id.
    pub id: String,
    /// What it answers (`None`: only a director or an effect offers it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<Trigger>,
    /// Its line (index into the lines); `None`: a deliberate silence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// When it may be chosen (beyond its line's speaker being here).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cond: Option<u32>,
    /// Higher first.
    #[serde(default)]
    pub priority: i32,
    /// How specific its condition is (more first, at equal priority): computed by the toolchain.
    #[serde(default)]
    pub specificity: u32,
    /// Ticks before it may be chosen again.
    #[serde(default)]
    pub cooldown: u32,
    /// Heard at most once.
    #[serde(default)]
    pub once: bool,
    /// Its category (index into [`Package::categories`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<u16>,
    /// A notice (`notice`, not `bark`): it never speaks and never competes; when its event happens and its condition holds, its
    /// effects apply (what witnesses learn, what the story records). Every eligible notice applies, before the barks choose.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notice: bool,
    /// A notice's effects (a bark with a line keeps its effects on the line, applied when it is heard).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    /// Where it is written.
    pub at: Src,
}

/// A category of barks sharing a cooldown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Category {
    /// Its id.
    pub id: String,
    /// Ticks between two barks of it.
    pub cooldown: u32,
}

/// One intervention a director may make.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Intervention {
    /// Its id.
    pub id: String,
    /// When it is eligible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cond: Option<u32>,
    /// Percent chance it happens when eligible (one draw of the host's random stream, only then); `None`: always.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chance: Option<u8>,
    /// At most one intervention of a group per round.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// At most once a run.
    #[serde(default)]
    pub once: bool,
    /// What it does (commands must be ones the game lets be issued unattended).
    pub effects: Vec<Effect>,
    /// Where it is written.
    pub at: Src,
}

/// A small pacing policy over live state: every so often, at most one intervention, within a budget, never during a conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Director {
    /// Its id.
    pub id: String,
    /// Ticks between evaluations.
    pub every: u32,
    /// Ticks after an intervention before the next.
    #[serde(default)]
    pub recover: u32,
    /// Interventions per round (`None`: no limit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<u32>,
    /// The game-defined pacing proxy it reports (an expression), not a measure of feeling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tension: Option<u32>,
    /// When any holds, it deliberately does nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rest: Vec<u32>,
    /// What it may do, in order of preference.
    pub interventions: Vec<Intervention>,
    /// Where it is written.
    pub at: Src,
}

/// A compiled story.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Package {
    /// The package format ([`FORMAT`]).
    pub story_package: u32,
    /// The story's namespace (`tidewatch`): its system is `<story>:story`, its ids `<story>:<id>`.
    pub story: String,
    /// The author's content version (raise it when saves of the old content should be read by id, not trusted).
    pub content_version: u32,
    /// A hash of the sources it was built from.
    pub source_hash: String,
    /// The source files, as the toolchain was given them.
    pub sources: Vec<String>,
    /// The event that resets round facts and director budgets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round: Option<Id>,
    /// Who can know things: `player` first, then every speaker.
    pub holders: Vec<String>,
    /// The speakers.
    pub speakers: Vec<Speaker>,
    /// The story's facts.
    pub facts: Vec<Fact>,
    /// What can be known.
    pub infos: Vec<Info>,
    /// The expression variables (`v<index>`).
    pub vars: Vec<Var>,
    /// The expressions, lowered (`v3 >= 2 && !v0`).
    pub exprs: Vec<String>,
    /// The expressions as written (`tw:shed_fuel >= 2 && !lent_fuel`), for diagnostics.
    pub authored: Vec<String>,
    /// Every line.
    pub lines: Vec<Line>,
    /// The conversations.
    pub conversations: Vec<Conversation>,
    /// The barks, in the order written (the last tie-break).
    pub barks: Vec<Bark>,
    /// Bark categories.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<Category>,
    /// The directors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub directors: Vec<Director>,
}

/// What the game offers a story to bind to: its systems' queries and commands, the events they emit, and the scene's variables.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    /// Queries the systems answer.
    pub queries: Vec<QueryDecl>,
    /// Commands the systems take.
    pub commands: Vec<Id>,
    /// The commands a director may issue unattended.
    pub unattended: Vec<Id>,
    /// Events the systems emit.
    pub events: Vec<Id>,
    /// The scene's declared variables (read as `scene:<name>`, numbers).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scene_vars: Vec<String>,
    /// The queries the game's systems read (so a story fact the game reads, `<story>:<fact>`, counts as read).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consumers: Vec<Id>,
}

impl Catalog {
    /// What `systems` (built, before they are hosted) offer a story, with the scene's declared variables: the one source a game binds its
    /// story to, at load and in the bindings file its authors compile against.
    pub fn of(systems: &[&dyn super::super::systems::System], scene_vars: Vec<String>) -> Catalog {
        let mut c = Catalog { scene_vars, ..Catalog::default() };
        for s in systems {
            c.queries.extend(s.queries());
            c.commands.extend(s.commands());
            c.unattended.extend(s.unattended());
            c.events.extend(s.emits());
            c.consumers.extend(s.reads());
        }
        c.queries.sort_by(|a, b| a.id.cmp(&b.id));
        for v in [&mut c.commands, &mut c.unattended, &mut c.events, &mut c.consumers] {
            v.sort();
            v.dedup();
        }
        c
    }
}

impl Package {
    /// `file:line` of `at`, for a diagnostic.
    pub fn place(&self, at: Src) -> String {
        format!("{}:{}", self.sources.get(at.file as usize).map_or("?", String::as_str), at.line)
    }

    /// The story system's id (`tidewatch:story`).
    pub fn system_id(&self) -> Result<Id, String> {
        Id::parse(&format!("{}:story", self.story))
    }

    /// Every query the story reads, with the type it expects.
    pub fn reads(&self) -> Vec<(Id, Ty)> {
        let mut all: BTreeMap<Id, Ty> = BTreeMap::new();
        for v in &self.vars {
            if let Var::Query { id, ty } = v {
                all.insert(id.clone(), *ty);
            }
        }
        all.into_iter().collect()
    }

    /// Every command the story may issue, and whether a director issues it.
    pub fn commands(&self) -> Vec<(Id, bool)> {
        let mut all: BTreeMap<Id, bool> = BTreeMap::new();
        let mut put = |effects: &[Effect], director: bool| {
            for e in effects {
                if let Effect::Run { command, .. } = e {
                    let d = all.entry(command.clone()).or_insert(false);
                    *d |= director;
                }
            }
        };
        for l in &self.lines {
            put(&l.effects, false);
            for c in &l.choices {
                put(&c.effects, false);
            }
        }
        for b in &self.barks {
            put(&b.effects, false);
        }
        for d in &self.directors {
            for i in &d.interventions {
                put(&i.effects, true);
            }
        }
        all.into_iter().collect()
    }

    /// Every event the story hears (its barks' triggers and its round event).
    pub fn hears(&self) -> Vec<Id> {
        let all: BTreeSet<Id> = self.barks.iter().filter_map(|b| b.on.as_ref().map(|t| t.event.clone())).chain(self.round.clone()).collect();
        all.into_iter().collect()
    }
}

/// Checks a package by itself: its format, that every index and goto points inside it, that every expression compiles. Every problem is
/// reported, `file:line` first.
pub fn check(p: &Package) -> Vec<Diagnostic> {
    let mut errs = Vec::new();
    let mut bad = |at: Src, code: &'static str, m: String| errs.push(Diagnostic::new(code, p.place(at), m));
    if p.story_package != FORMAT {
        bad(Src::default(), code::VERSION, format!("story package format {}; this runtime reads format {FORMAT}", p.story_package));
        return errs;
    }
    if p.system_id().is_err() {
        bad(Src::default(), code::REFERENCE, format!("`{}` is not a story namespace (lowercase letters, digits, `_`)", p.story));
    }
    if p.exprs.len() != p.authored.len() {
        bad(Src::default(), code::REFERENCE, "the lowered and authored expression tables differ in length".into());
    }
    let (nl, nf, ni, nh, ns, nv, ne) = (p.lines.len(), p.facts.len(), p.infos.len(), p.holders.len(), p.speakers.len(), p.vars.len(), p.exprs.len());
    let names = var_names(nv);
    for (k, e) in p.exprs.iter().enumerate() {
        if let Err(err) = rules_expr::parse(e, &names) {
            bad(
                Src::default(),
                code::EXPRESSION,
                format!("expression {k} (`{}`) does not compile: {}", p.authored.get(k).map_or("?", String::as_str), err.message),
            );
        }
    }
    for v in &p.vars {
        let ok = match v {
            Var::Fact { fact } => (*fact as usize) < nf,
            Var::Query { .. } => true,
            Var::Knows { holder, info } | Var::Believes { holder, info } => (*holder as usize) < nh && (*info as usize) < ni,
            Var::Heard { line } => (*line as usize) < nl,
        };
        if !ok {
            bad(Src::default(), code::REFERENCE, format!("variable {v:?} points outside the package"));
        }
    }
    let expr_ok = |e: &Option<u32>| e.is_none_or(|e| (e as usize) < ne);
    let effects_ok = |effects: &[Effect]| {
        effects.iter().all(|e| match e {
            Effect::Set { fact, expr } | Effect::Add { fact, expr } => (*fact as usize) < nf && (*expr as usize) < ne,
            Effect::Learn { who, info, .. } => (*info as usize) < ni && !matches!(who, Who::Holder(h) if (*h as usize) >= nh),
            Effect::Run { value, .. } => expr_ok(value),
            Effect::Bark { bark } => (*bark as usize) < p.barks.len(),
            Effect::Start { conversation } => (*conversation as usize) < p.conversations.len(),
        })
    };
    for s in &p.speakers {
        if s.holder as usize >= nh || s.present.is_some_and(|v| v as usize >= nv) || s.knows.iter().any(|i| *i as usize >= ni) {
            bad(s.at, code::REFERENCE, format!("speaker `{}` points outside the package", s.id));
        }
    }
    for i in &p.infos {
        if !expr_ok(&i.truth) {
            bad(i.at, code::REFERENCE, format!("info `{}`: its truth points outside the package", i.id));
        }
    }
    let pieces_ok = |t: &[Piece]| t.iter().all(|x| matches!(x, Piece::Text(_)) || matches!(x, Piece::Var { var } if (*var as usize) < nv));
    // Each line belongs to at most one conversation, and its gotos stay inside it.
    let mut owner: Vec<Option<usize>> = vec![None; nl];
    for (ci, c) in p.conversations.iter().enumerate() {
        let (a, b) = (c.first as usize, c.first as usize + c.len as usize);
        if c.len == 0 || b > nl || c.with as usize >= ns || !expr_ok(&c.cond) {
            bad(c.at, code::REFERENCE, format!("conversation `{}` points outside the package", c.id));
            continue;
        }
        for o in &mut owner[a..b] {
            *o = Some(ci);
        }
    }
    let inside = |line: usize, n: &Option<Next>| match (n, owner.get(line).copied().flatten()) {
        (Some(Next::Line(t)), Some(ci)) => owner.get(*t as usize).copied().flatten() == Some(ci),
        (Some(Next::Line(_)), None) => false,
        _ => true,
    };
    for (li, l) in p.lines.iter().enumerate() {
        let speaker_ok = l.speaker.is_none_or(|s| (s as usize) < ns);
        let ok = speaker_ok
            && pieces_ok(&l.text)
            && expr_ok(&l.cond)
            && effects_ok(&l.effects)
            && l.reveals.iter().chain(&l.claims).all(|i| (*i as usize) < ni)
            && inside(li, &l.goto)
            && inside(li, &l.skip)
            && l.choices.iter().all(|c| pieces_ok(&c.text) && expr_ok(&c.cond) && effects_ok(&c.effects) && inside(li, &c.goto) && inside(li, &c.refused));
        if !ok {
            bad(l.at, code::REFERENCE, format!("line `{}` points outside the package (or its conversation)", l.id));
        }
        if l.choices.len() > MAX_CHOICES {
            bad(l.at, code::FLOW, format!("line `{}` offers {} choices; at most {MAX_CHOICES}", l.id, l.choices.len()));
        }
    }
    for b in &p.barks {
        let ok = b.line.is_none_or(|l| (l as usize) < nl && owner[l as usize].is_none())
            && expr_ok(&b.cond)
            && effects_ok(&b.effects)
            && (b.line.is_none() || b.effects.is_empty())
            && (!b.notice || (b.line.is_none() && b.on.is_some() && !b.effects.iter().any(|e| matches!(e, Effect::Run { .. }))))
            && b.category.is_none_or(|c| (c as usize) < p.categories.len());
        if !ok {
            bad(b.at, code::REFERENCE, format!("bark `{}` points outside the package (or at a conversation's line)", b.id));
        }
    }
    for d in &p.directors {
        if d.every == 0 {
            bad(d.at, code::FLOW, format!("director `{}` evaluates every 0 ticks", d.id));
        }
        let ok = expr_ok(&d.tension) && d.rest.iter().all(|r| (*r as usize) < ne) && d.interventions.iter().all(|i| expr_ok(&i.cond) && effects_ok(&i.effects));
        if !ok {
            bad(d.at, code::REFERENCE, format!("director `{}` points outside the package", d.id));
        }
    }
    errs
}

/// The most choices one line may offer (a presenter numbers them 1-9).
pub const MAX_CHOICES: usize = 9;

/// The variable names lowered expressions use: `v0`, `v1`, ...
pub fn var_names(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("v{i}")).collect()
}

/// Binds a package to the game: every query it reads is one the game answers with the type the story expects (`scene:<var>` a declared
/// scene variable, read as a number), every command it issues one the game takes (a director's, one the game lets be issued unattended),
/// every event it hears one the game emits.
pub fn bind(p: &Package, game: &Catalog) -> Vec<Diagnostic> {
    let mut errs = Vec::new();
    let place = p.place(Src::default());
    let near =
        |w: &str, all: &mut dyn Iterator<Item = &str>| super::super::defs::closest(w, all).map(|c| format!(" (did you mean `{c}`?)")).unwrap_or_default();
    for (q, want) in p.reads() {
        // A scene variable is a number (its type is what the toolchain read from the scene's declared start value).
        if q.namespace() == super::super::systems::OUTSIDE_NS {
            if !game.scene_vars.iter().any(|v| v == q.name()) {
                let hint = near(q.name(), &mut game.scene_vars.iter().map(String::as_str));
                errs.push(
                    Diagnostic::new(code::BINDING, &place, format!("reads `{q}`, which the scene does not declare{hint}"))
                        .fix("declare it in the scene's `vars`"),
                );
            }
            continue;
        }
        match game.queries.iter().find(|d| d.id == q) {
            None => {
                let hint = near(q.as_str(), &mut game.queries.iter().map(|d| d.id.as_str()));
                errs.push(
                    Diagnostic::new(code::BINDING, &place, format!("reads `{q}`, which the game does not answer{hint}"))
                        .fix("use a query from the game's bindings (`story bindings`)"),
                );
            }
            Some(d) if d.ty != want => {
                errs.push(Diagnostic::new(code::BINDING, &place, format!("reads `{q}` as {want:?}, but the game answers it as {:?}", d.ty)));
            }
            Some(_) => {}
        }
    }
    for (c, director) in p.commands() {
        if !game.commands.contains(&c) {
            let hint = near(c.as_str(), &mut game.commands.iter().map(Id::as_str));
            errs.push(Diagnostic::new(code::BINDING, &place, format!("runs `{c}`, which the game does not take{hint}")));
        } else if director && !game.unattended.contains(&c) {
            errs.push(
                Diagnostic::new(code::BINDING, &place, format!("a director runs `{c}`, which the game does not let be issued unattended"))
                    .fix("a director may only run the game's unattended commands (pacing that never takes from the player)"),
            );
        }
    }
    for e in p.hears() {
        if !game.events.contains(&e) {
            let hint = near(e.as_str(), &mut game.events.iter().map(Id::as_str));
            errs.push(Diagnostic::new(code::BINDING, &place, format!("answers `{e}`, which the game does not emit{hint}")));
        }
    }
    errs
}

/// A checked, bound package with its expressions compiled: what the runtime runs.
#[derive(Debug, Clone)]
pub struct Loaded {
    /// The package.
    pub package: Package,
    /// Its expressions, compiled (same indices as [`Package::exprs`]).
    pub exprs: Vec<Expr>,
    /// The system's id (`<story>:story`).
    pub id: Id,
}

impl Loaded {
    /// Checks `package` and binds it to `game`; every problem is reported and nothing is half-loaded.
    pub fn new(package: Package, game: &Catalog) -> Result<Loaded, Vec<Diagnostic>> {
        let mut errs = check(&package);
        if errs.is_empty() {
            errs = bind(&package, game);
        }
        if !errs.is_empty() {
            return Err(errs);
        }
        let names = var_names(package.vars.len());
        let exprs = package.exprs.iter().filter_map(|e| rules_expr::parse(e, &names).ok()).collect();
        let id = package.system_id().map_err(|e| vec![Diagnostic::new(code::REFERENCE, "package", e)])?;
        Ok(Loaded { package, exprs, id })
    }

    /// Parses package JSON text and loads it.
    pub fn from_json(text: &str, game: &Catalog) -> Result<Loaded, Vec<Diagnostic>> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| vec![Diagnostic::new(code::SYNTAX, "package", format!("not JSON: {e}"))])?;
        match v.get("story_package").and_then(serde_json::Value::as_u64) {
            Some(f) if f == FORMAT as u64 => {}
            Some(f) => {
                return Err(vec![Diagnostic::new(code::VERSION, "package", format!("story package format {f}; this runtime reads format {FORMAT}"))
                    .fix("rebuild it with this engine's `story build`")])
            }
            None => return Err(vec![Diagnostic::new(code::VERSION, "package", "not a story package (no `story_package`)")]),
        }
        let package: Package =
            serde_json::from_value(v).map_err(|e| vec![Diagnostic::new(code::SYNTAX, "package", format!("malformed story package: {e}"))])?;
        Loaded::new(package, game)
    }
}
