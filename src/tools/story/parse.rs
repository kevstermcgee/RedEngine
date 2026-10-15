//! The `.story` text format, read into a syntax tree with a `file:line` on everything (`docs/STORY.md` is its contract). Line-oriented and
//! indentation-free: a keyword opens an item (`character`, `fact`, `conversation`, `bark`, ...), and an attribute line (`if`, `do`,
//! `reveals`, `->`, ...) belongs to the nearest item above it. Ink and Yarn syntax is refused by name, never half-understood.

use super::Diag;

/// Where something was written: index into the files, 1-based line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct At {
    /// Index into [`Ast::files`].
    pub file: usize,
    /// 1-based line.
    pub line: u32,
}

/// A piece of text with where it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spanned {
    /// The text.
    pub text: String,
    /// Where.
    pub at: At,
}

/// The header: which story, which version, which game bindings and scene, which event starts a round.
#[derive(Debug, Clone, Default)]
pub struct Header {
    /// `story <ns>` (every file says it; they must agree).
    pub story: Option<Spanned>,
    /// `story <ns> version <n>` (one file).
    pub version: Option<(u32, At)>,
    /// `uses <bindings.json>`.
    pub uses: Option<Spanned>,
    /// `map <scene.json>`.
    pub map: Option<Spanned>,
    /// `round <event>`.
    pub round: Option<Spanned>,
    /// `style max_line <n>`.
    pub max_line: Option<usize>,
}

/// A character sheet (`character`, major) or a lighter profile (`extra`, minor).
#[derive(Debug, Clone)]
pub struct Character {
    /// Its id.
    pub id: String,
    /// The name shown.
    pub name: String,
    /// An `extra`.
    pub minor: bool,
    /// `key: value` fields, in order (`example` may repeat).
    pub fields: Vec<(String, Spanned)>,
    /// Where.
    pub at: At,
}

impl Character {
    /// Every value of field `key`.
    pub fn all<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a Spanned> + 'a {
        self.fields.iter().filter(move |(k, _)| k == key).map(|(_, v)| v)
    }

    /// The first value of field `key`.
    pub fn get(&self, key: &str) -> Option<&Spanned> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

/// `fact <id> bool|int = <value> [round|run|profile] [shown] "<meaning>"`.
#[derive(Debug, Clone)]
pub struct FactDecl {
    /// Its id.
    pub id: String,
    /// `bool` or `int`.
    pub ty: String,
    /// The start value as written.
    pub default: String,
    /// `round`, `run` (the default) or `profile`.
    pub scope: String,
    /// A presenter or the game's UI reads it (`shown`).
    pub shown: bool,
    /// What it means.
    pub doc: String,
    /// Where.
    pub at: At,
}

/// `info <id> "<meaning>"`, with `true if <expr>`.
#[derive(Debug, Clone)]
pub struct InfoDecl {
    /// Its id.
    pub id: String,
    /// What it says.
    pub doc: String,
    /// When it is true in the world.
    pub truth: Option<Spanned>,
    /// Where.
    pub at: At,
}

/// `setup <id> "<what>"` with `by <ref>` and `pays <kind> <ref>` lines.
#[derive(Debug, Clone)]
pub struct Setup {
    /// Its id.
    pub id: String,
    /// What was established or promised.
    pub doc: String,
    /// Where it is established (line or choice ids).
    pub by: Vec<Spanned>,
    /// `(kind, ref)`: acknowledge, resolve, complicate or leave, at a line, choice or bark.
    pub pays: Vec<(String, Spanned)>,
    /// Where.
    pub at: At,
}

/// What lines, choices, barks and interventions share: a condition, effects, knowledge.
#[derive(Debug, Clone, Default)]
pub struct Attrs {
    /// `if <expr>`.
    pub cond: Option<Spanned>,
    /// `do <effect>; <effect>` (each effect separately).
    pub effects: Vec<Spanned>,
    /// `reveals <info>, ...`.
    pub reveals: Vec<Spanned>,
    /// `claims <info>, ...`.
    pub claims: Vec<Spanned>,
}

/// `* <id>: <text>`.
#[derive(Debug, Clone)]
pub struct ChoiceAst {
    /// Its id (inside its conversation).
    pub id: String,
    /// What the player picks.
    pub text: Spanned,
    /// Condition and effects.
    pub attrs: Attrs,
    /// `kind consequential|informational|expressive`.
    pub kind: Option<Spanned>,
    /// `-> node|end`.
    pub goto: Option<Spanned>,
    /// `else -> node|end`.
    pub refused: Option<Spanned>,
    /// Where.
    pub at: At,
}

/// `<speaker>: <text>` in a node.
#[derive(Debug, Clone)]
pub struct LineAst {
    /// The speaker's id (`narrator`: none).
    pub speaker: Spanned,
    /// What is said.
    pub text: Spanned,
    /// `id <name>`.
    pub id: Option<Spanned>,
    /// Condition, effects, knowledge.
    pub attrs: Attrs,
    /// Its choices.
    pub choices: Vec<ChoiceAst>,
    /// `-> node|end` (a line without choices), or `otherwise -> node|end` (one with: where it goes when none is offered).
    pub goto: Option<Spanned>,
    /// Where.
    pub at: At,
}

/// `node <id>`.
#[derive(Debug, Clone)]
pub struct Node {
    /// Its id.
    pub id: String,
    /// Its lines.
    pub lines: Vec<LineAst>,
    /// Where.
    pub at: At,
}

/// `conversation <id> with <speaker>`.
#[derive(Debug, Clone)]
pub struct Conv {
    /// Its id.
    pub id: String,
    /// Who the player talks to, to open it.
    pub with: Spanned,
    /// `if <expr>`.
    pub cond: Option<Spanned>,
    /// `priority <n>`.
    pub priority: i32,
    /// `interrupt <n>`.
    pub interrupt: Option<i32>,
    /// `repeat`.
    pub repeat: bool,
    /// Its nodes.
    pub nodes: Vec<Node>,
    /// Where.
    pub at: At,
}

/// `bark <id> [on <event> [about <subject>]]`.
#[derive(Debug, Clone)]
pub struct BarkDecl {
    /// Its id.
    pub id: String,
    /// `(event, subject)`.
    pub on: Option<(Spanned, Option<Spanned>)>,
    /// `(speaker, text)`; `None` with `silence`.
    pub line: Option<(Spanned, Spanned)>,
    /// `silence`.
    pub silence: bool,
    /// Declared with `notice` (it never speaks; its effects apply when its event happens).
    pub notice: bool,
    /// Condition, effects, knowledge.
    pub attrs: Attrs,
    /// `priority <n>`.
    pub priority: i32,
    /// `cooldown <secs>s`, in ticks.
    pub cooldown: u32,
    /// `once`.
    pub once: bool,
    /// `category <id>`.
    pub category: Option<Spanned>,
    /// Where.
    pub at: At,
}

/// `intervene <id>` in a director.
#[derive(Debug, Clone)]
pub struct Interv {
    /// Its id.
    pub id: String,
    /// Condition and effects.
    pub attrs: Attrs,
    /// `chance <n>%`.
    pub chance: Option<u8>,
    /// `group <id>`.
    pub group: Option<String>,
    /// `once`.
    pub once: bool,
    /// Where.
    pub at: At,
}

/// `director <id>`.
#[derive(Debug, Clone)]
pub struct DirDecl {
    /// Its id.
    pub id: String,
    /// `every <secs>s`, in ticks.
    pub every: u32,
    /// `recover <secs>s`, in ticks.
    pub recover: u32,
    /// `budget <n> per round`.
    pub budget: Option<u32>,
    /// `tension <expr>`.
    pub tension: Option<Spanned>,
    /// `rest if <expr>`.
    pub rest: Vec<Spanned>,
    /// Its interventions.
    pub interventions: Vec<Interv>,
    /// Where.
    pub at: At,
}

/// Every source file of one story, read.
#[derive(Debug, Clone, Default)]
pub struct Ast {
    /// The files, as given.
    pub files: Vec<String>,
    /// The header.
    pub header: Header,
    /// Characters, in order.
    pub characters: Vec<Character>,
    /// Facts.
    pub facts: Vec<FactDecl>,
    /// Infos.
    pub infos: Vec<InfoDecl>,
    /// `category <id> cooldown <secs>s`: `(id, ticks, at)`.
    pub categories: Vec<(String, u32, At)>,
    /// Setups.
    pub setups: Vec<Setup>,
    /// Conversations.
    pub convs: Vec<Conv>,
    /// Barks.
    pub barks: Vec<BarkDecl>,
    /// Directors.
    pub directors: Vec<DirDecl>,
    /// `allow <warning> <id> "<why>"`: one warning about one thing, accepted with its reason (listed by every check, never silent).
    pub allows: Vec<(String, String, String, At)>,
    /// `request <id> "<what the game would need>"`: a mechanic the story wants and the game does not have yet (listed by every check).
    pub requests: Vec<(String, String, At)>,
}

/// Ticks a second (the simulation's rate).
pub const TICKS: u32 = 60;

/// What an attribute line belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cur {
    None,
    Character,
    Info,
    Setup,
    Conv,
    Node,
    Line,
    Choice,
    Bark,
    Director,
    Interv,
}

/// The foreign syntax that is refused by name, with what to write instead.
const FOREIGN: &[(&str, &str)] = &[
    ("===", "an Ink knot: write `conversation <id> with <speaker>` and `node <id>`"),
    ("->->", "an Ink tunnel: jump with `-> node`"),
    ("-> DONE", "Ink's DONE: write `-> end`"),
    ("-> END", "Ink's END: write `-> end`"),
    ("~ ", "Ink logic: write `do set fact = value`"),
    ("VAR ", "an Ink variable: write `fact <id> bool|int = <value> \"<meaning>\"`"),
    ("CONST ", "an Ink constant: write the number where it is used"),
    ("INCLUDE ", "an Ink include: give `story build` the folder (every .story file in it is read)"),
    ("<<", "a Yarn command: conditions are `if <expr>`, effects `do <effect>`"),
    ("---", "a Yarn node separator: write `node <id>`"),
    ("===", "a Yarn node end: nodes end where the next begins"),
    ("+ ", "an Ink sticky choice: choices are `* <id>: <text>` (a conversation that repeats offers them again)"),
];

/// Reads `files` (`(name, text)`) as one story. Every problem is reported; the tree holds what could be read.
pub fn parse(files: &[(String, String)]) -> (Ast, Vec<Diag>) {
    let mut ast = Ast { files: files.iter().map(|(n, _)| n.clone()).collect(), ..Ast::default() };
    let mut errs = Vec::new();
    for (fi, (_, text)) in files.iter().enumerate() {
        parse_file(fi, text, &mut ast, &mut errs);
    }
    (ast, errs)
}

fn quoted(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start();
    let rest = s.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some((rest[..end].to_string(), rest[end + 1..].trim()))
}

fn is_name(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_lowercase()) && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Seconds as written (`12s`, `1.5s`) in ticks.
fn secs(s: &str) -> Option<u32> {
    let n: f32 = s.strip_suffix('s')?.parse().ok()?;
    (n >= 0.0).then(|| (n * TICKS as f32).round() as u32)
}

fn parse_file(fi: usize, text: &str, ast: &mut Ast, errs: &mut Vec<Diag>) {
    let mut cur = Cur::None;
    let mut said_story = false;
    for (n, raw) in text.lines().enumerate() {
        let at = At { file: fi, line: n as u32 + 1 };
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let err = |errs: &mut Vec<Diag>, m: String, fix: &str| errs.push(Diag::error("story-syntax", at, m, fix));
        if let Some((_, what)) = FOREIGN.iter().find(|(p, _)| line.starts_with(p)) {
            err(errs, format!("not supported: {what} (this is a Red story, not Ink or Yarn)"), "see `describe story` for the supported subset");
            continue;
        }
        let (word, rest) = line.split_once(char::is_whitespace).map_or((line, ""), |(w, r)| (w, r.trim()));
        if !said_story && word != "story" {
            err(errs, "a story file begins with `story <name>`".into(), "add `story <name>` (and `version <n>` in one file) as the first line");
            said_story = true;
        }
        match word {
            "story" => {
                said_story = true;
                let parts: Vec<&str> = rest.split_whitespace().collect();
                let (name, version) = match parts.as_slice() {
                    [name] => (*name, None),
                    [name, "version", v] => (*name, v.parse::<u32>().ok().map(Some).unwrap_or(None)),
                    _ => {
                        err(errs, format!("`story {rest}`: write `story <name>` or `story <name> version <n>`"), "");
                        continue;
                    }
                };
                if !is_name(name) {
                    err(errs, format!("`{name}` is not a story name (lowercase letters, digits, `_`)"), "");
                }
                match &ast.header.story {
                    Some(s) if s.text != name => err(errs, format!("this file is story `{name}`, another is `{}`", s.text), "one story per folder"),
                    _ => ast.header.story = Some(Spanned { text: name.into(), at }),
                }
                if let Some(v) = version {
                    if ast.header.version.replace((v, at)).is_some() {
                        err(errs, "the version is given twice".into(), "say `version` in one file only");
                    }
                } else if parts.len() == 3 {
                    err(errs, format!("`{}` is not a version number", parts[2]), "a whole number");
                }
                cur = Cur::None;
            }
            "uses" | "map" | "round" => {
                let slot = match word {
                    "uses" => &mut ast.header.uses,
                    "map" => &mut ast.header.map,
                    _ => &mut ast.header.round,
                };
                if slot.replace(Spanned { text: rest.into(), at }).is_some() {
                    err(errs, format!("`{word}` is given twice"), "say it once");
                }
                cur = Cur::None;
            }
            "style" => match rest.split_whitespace().collect::<Vec<_>>().as_slice() {
                ["max_line", n] if n.parse::<usize>().is_ok() => ast.header.max_line = n.parse().ok(),
                _ => err(errs, format!("`style {rest}`: the one style setting is `style max_line <n>`"), ""),
            },
            "character" | "extra" => {
                let (id, after) = rest.split_once(char::is_whitespace).map_or((rest, ""), |(a, b)| (a, b));
                let Some((name, _)) = quoted(after) else {
                    err(errs, format!("`{word} {rest}`: write `{word} <id> \"<Name>\"`"), "");
                    continue;
                };
                if !is_name(id) {
                    err(errs, format!("`{id}` is not an id (lowercase letters, digits, `_`)"), "");
                }
                ast.characters.push(Character { id: id.into(), name, minor: word == "extra", fields: Vec::new(), at });
                cur = Cur::Character;
            }
            "fact" => {
                // fact <id> bool|int = <value> [round|run|profile] [shown] "<meaning>"
                let (head, doc) = match line.find('"') {
                    Some(q) => (&line[..q], quoted(&line[q..]).map(|(d, _)| d)),
                    None => (line, None),
                };
                let parts: Vec<&str> = head.split_whitespace().collect();
                let ok = parts.len() >= 5 && parts[3] == "=" && matches!(parts[2], "bool" | "int");
                if !ok || doc.is_none() {
                    err(errs, format!("`{line}`: write `fact <id> bool|int = <value> [round|run|profile] \"<what it means>\"`"), "");
                    continue;
                }
                let mut scope = "run".to_string();
                let mut shown = false;
                for extra in &parts[5..] {
                    match *extra {
                        "round" | "run" | "profile" => scope = extra.to_string(),
                        "shown" => shown = true,
                        other => err(errs, format!("`{other}`: a fact's scope is round, run or profile; `shown` marks one a presenter reads"), ""),
                    }
                }
                ast.facts.push(FactDecl { id: parts[1].into(), ty: parts[2].into(), default: parts[4].into(), scope, shown, doc: doc.unwrap_or_default(), at });
                cur = Cur::None;
            }
            "info" => {
                let (id, after) = rest.split_once(char::is_whitespace).map_or((rest, ""), |(a, b)| (a, b));
                let Some((doc, _)) = quoted(after) else {
                    err(errs, format!("`{line}`: write `info <id> \"<what it says>\"`"), "");
                    continue;
                };
                ast.infos.push(InfoDecl { id: id.into(), doc, truth: None, at });
                cur = Cur::Info;
            }
            // In a bark, `category <id>` names its category; at the top, `category <id> cooldown <secs>s` declares one.
            "category" if cur == Cur::Bark && !rest.contains(char::is_whitespace) => attribute(word, rest, line, at, &mut cur, ast, errs),
            "category" => match rest.split_whitespace().collect::<Vec<_>>().as_slice() {
                [id, "cooldown", s] if secs(s).is_some() => {
                    ast.categories.push((id.to_string(), secs(s).unwrap_or(0), at));
                    cur = Cur::None;
                }
                _ => err(errs, format!("`{line}`: write `category <id> cooldown <secs>s`"), ""),
            },
            "setup" => {
                let (id, after) = rest.split_once(char::is_whitespace).map_or((rest, ""), |(a, b)| (a, b));
                let Some((doc, _)) = quoted(after) else {
                    err(errs, format!("`{line}`: write `setup <id> \"<what was established or promised>\"`"), "");
                    continue;
                };
                ast.setups.push(Setup { id: id.into(), doc, by: Vec::new(), pays: Vec::new(), at });
                cur = Cur::Setup;
            }
            "conversation" => match rest.split_whitespace().collect::<Vec<_>>().as_slice() {
                [id, "with", who] => {
                    ast.convs.push(Conv {
                        id: id.to_string(),
                        with: Spanned { text: who.to_string(), at },
                        cond: None,
                        priority: 0,
                        interrupt: None,
                        repeat: false,
                        nodes: Vec::new(),
                        at,
                    });
                    cur = Cur::Conv;
                }
                _ => err(errs, format!("`{line}`: write `conversation <id> with <speaker>`"), ""),
            },
            "node" => {
                let Some(c) = ast.convs.last_mut().filter(|_| matches!(cur, Cur::Conv | Cur::Node | Cur::Line | Cur::Choice)) else {
                    err(errs, "a `node` belongs to a conversation".into(), "write `conversation <id> with <speaker>` above it");
                    continue;
                };
                c.nodes.push(Node { id: rest.into(), lines: Vec::new(), at });
                cur = Cur::Node;
            }
            "bark" | "notice" => {
                let parts: Vec<&str> = rest.split_whitespace().collect();
                let on = match parts.as_slice() {
                    [_] => Some(None),
                    [_, "on", ev] => Some(Some((Spanned { text: ev.to_string(), at }, None))),
                    [_, "on", ev, "about", s] => Some(Some((Spanned { text: ev.to_string(), at }, Some(Spanned { text: s.to_string(), at })))),
                    _ => None,
                };
                let Some(on) = on else {
                    err(
                        errs,
                        format!(
                            "`{line}`: write `{word} <id> on <event> [about <subject>]`{}",
                            if word == "bark" { " (or `bark <id>` for one a director offers)" } else { "" }
                        ),
                        "",
                    );
                    continue;
                };
                ast.barks.push(BarkDecl {
                    id: parts[0].into(),
                    on,
                    line: None,
                    silence: false,
                    notice: word == "notice",
                    attrs: Attrs::default(),
                    priority: 0,
                    cooldown: 0,
                    once: false,
                    category: None,
                    at,
                });
                cur = Cur::Bark;
            }
            "request" => {
                let (id, after) = rest.split_once(char::is_whitespace).map_or((rest, ""), |(a, b)| (a, b));
                match quoted(after) {
                    Some((what, _)) if !id.is_empty() && !what.trim().is_empty() => ast.requests.push((id.to_string(), what, at)),
                    _ => err(errs, format!("`{line}`: write `request <id> \"<what the game would need to offer>\"`"), ""),
                }
                cur = Cur::None;
            }
            "allow" => {
                let (head, why) = match line.find('"') {
                    Some(q) => (&line[..q], quoted(&line[q..]).map(|(d, _)| d)),
                    None => (line, None),
                };
                match (head.split_whitespace().collect::<Vec<_>>().as_slice(), why) {
                    (["allow", kind, target], Some(why)) if !why.trim().is_empty() => ast.allows.push((kind.to_string(), target.to_string(), why, at)),
                    _ => err(errs, format!("`{line}`: write `allow <warning> <id> \"<why it is right here>\"` (one warning, one thing, a reason)"), ""),
                }
                cur = Cur::None;
            }
            "director" => {
                ast.directors.push(DirDecl {
                    id: rest.into(),
                    every: 0,
                    recover: 0,
                    budget: None,
                    tension: None,
                    rest: Vec::new(),
                    interventions: Vec::new(),
                    at,
                });
                cur = Cur::Director;
            }
            _ => attribute(word, rest, line, at, &mut cur, ast, errs),
        }
    }
}

/// An attribute line (or a line of speech, or a choice): it belongs to the nearest item above it.
fn attribute(word: &str, rest: &str, line: &str, at: At, cur: &mut Cur, ast: &mut Ast, errs: &mut Vec<Diag>) {
    let sp = |t: &str| Spanned { text: t.trim().to_string(), at };
    let err = |errs: &mut Vec<Diag>, m: String, fix: &str| errs.push(Diag::error("story-syntax", at, m, fix));
    // A choice: `* <id>: <text>`.
    if let Some(choice) = line.strip_prefix('*') {
        let Some((id, text)) = choice.split_once(':') else {
            err(errs, format!("`{line}`: write `* <id>: <what the player says>`"), "");
            return;
        };
        let Some(l) =
            ast.convs.last_mut().and_then(|c| c.nodes.last_mut()).and_then(|n| n.lines.last_mut()).filter(|_| matches!(*cur, Cur::Line | Cur::Choice))
        else {
            err(errs, "a choice follows the line it answers".into(), "put a `<speaker>: <text>` line above it");
            return;
        };
        l.choices.push(ChoiceAst { id: id.trim().into(), text: sp(text), attrs: Attrs::default(), kind: None, goto: None, refused: None, at });
        *cur = Cur::Choice;
        return;
    }
    // Speech: `<speaker>: <text>`, in a node or a bark.
    if let Some(speaker) = word.strip_suffix(':').filter(|s| is_name(s)) {
        if *cur == Cur::Character {
            if let Some(c) = ast.characters.last_mut() {
                c.fields.push((speaker.to_string(), sp(rest)));
            }
            return;
        }
        match *cur {
            Cur::Node | Cur::Line | Cur::Choice => {
                if let Some(n) = ast.convs.last_mut().and_then(|c| c.nodes.last_mut()) {
                    n.lines.push(LineAst { speaker: sp(speaker), text: sp(rest), id: None, attrs: Attrs::default(), choices: Vec::new(), goto: None, at });
                    *cur = Cur::Line;
                }
            }
            Cur::Bark => {
                if let Some(b) = ast.barks.last_mut() {
                    if b.notice {
                        err(errs, format!("notice `{}` speaks: a notice only records what witnesses see", b.id), "make it a `bark` to say something");
                    } else if b.line.is_some() || b.silence {
                        err(errs, format!("bark `{}` already has its line: a bark says one line", b.id), "write another bark (same `on`) for a variant");
                    } else {
                        b.line = Some((sp(speaker), sp(rest)));
                    }
                }
            }
            Cur::Conv => err(errs, "a line of a conversation belongs to a node".into(), "write `node <id>` above it"),
            _ => err(errs, format!("`{line}`: a line of speech belongs to a node or a bark"), ""),
        }
        return;
    }
    let attrs: Option<&mut Attrs> = match *cur {
        Cur::Line => ast.convs.last_mut().and_then(|c| c.nodes.last_mut()).and_then(|n| n.lines.last_mut()).map(|l| &mut l.attrs),
        Cur::Choice => {
            ast.convs.last_mut().and_then(|c| c.nodes.last_mut()).and_then(|n| n.lines.last_mut()).and_then(|l| l.choices.last_mut()).map(|c| &mut c.attrs)
        }
        Cur::Bark => ast.barks.last_mut().map(|b| &mut b.attrs),
        Cur::Interv => ast.directors.last_mut().and_then(|d| d.interventions.last_mut()).map(|i| &mut i.attrs),
        _ => None,
    };
    match (word, attrs) {
        ("if", Some(a)) => {
            if a.cond.replace(sp(rest)).is_some() {
                err(errs, "a second `if`: join the conditions with `&&`".into(), "");
            }
            return;
        }
        ("do", Some(a)) => {
            a.effects.extend(rest.split(';').filter(|e| !e.trim().is_empty()).map(sp));
            return;
        }
        ("reveals" | "claims", Some(a)) => {
            let into = if word == "reveals" { &mut a.reveals } else { &mut a.claims };
            into.extend(rest.split(',').filter(|e| !e.trim().is_empty()).map(sp));
            return;
        }
        _ => {}
    }
    let goto = |rest: &str| rest.strip_prefix("->").map(sp);
    match (*cur, word) {
        (Cur::Conv, "if") => {
            if let Some(c) = ast.convs.last_mut() {
                c.cond = Some(sp(rest));
            }
        }
        (Cur::Conv, "priority") | (Cur::Bark, "priority") => match rest.parse::<i32>() {
            Ok(n) if *cur == Cur::Conv => ast.convs.last_mut().into_iter().for_each(|c| c.priority = n),
            Ok(n) => ast.barks.last_mut().into_iter().for_each(|b| b.priority = n),
            Err(_) => err(errs, format!("`priority {rest}`: a whole number"), ""),
        },
        (Cur::Conv, "interrupt") => match rest.parse::<i32>() {
            Ok(n) => ast.convs.last_mut().into_iter().for_each(|c| c.interrupt = Some(n)),
            Err(_) => err(errs, format!("`interrupt {rest}`: a bark priority (a whole number): barks of it or more suspend the conversation"), ""),
        },
        (Cur::Conv, "repeat") => ast.convs.last_mut().into_iter().for_each(|c| c.repeat = true),
        (Cur::Line | Cur::Choice, "->") | (Cur::Line | Cur::Choice, "otherwise") | (Cur::Line | Cur::Choice, "else") => {
            let Some(l) = ast.convs.last_mut().and_then(|c| c.nodes.last_mut()).and_then(|n| n.lines.last_mut()) else { return };
            let target = if word == "->" { Some(sp(rest)) } else { goto(rest) };
            let Some(target) = target else {
                err(errs, format!("`{line}`: write `{word} -> <node>` (or `-> end`)"), "");
                return;
            };
            match (word, *cur, l.choices.last_mut()) {
                ("->", Cur::Choice, Some(c)) => {
                    if c.goto.replace(target).is_some() {
                        err(errs, format!("choice `{}` already says where it goes", c.id), "");
                    }
                }
                ("else", Cur::Choice, Some(c)) => c.refused = Some(target),
                ("otherwise", _, _) | ("->", Cur::Line, _) => {
                    if l.goto.replace(target).is_some() {
                        err(errs, "this line already says where it goes".into(), "");
                    }
                }
                _ => {
                    err(errs, format!("`{word}` belongs to a {}", if word == "else" { "choice (where it goes when the game refuses it)" } else { "line" }), "")
                }
            }
        }
        (Cur::Line, "id") => {
            if let Some(l) = ast.convs.last_mut().and_then(|c| c.nodes.last_mut()).and_then(|n| n.lines.last_mut()) {
                l.id = Some(sp(rest));
            }
        }
        (Cur::Choice, "kind") => {
            if let Some(c) = ast.convs.last_mut().and_then(|c| c.nodes.last_mut()).and_then(|n| n.lines.last_mut()).and_then(|l| l.choices.last_mut()) {
                c.kind = Some(sp(rest));
            }
        }
        (Cur::Bark, "silence") => {
            if let Some(b) = ast.barks.last_mut() {
                if b.line.is_some() {
                    err(errs, format!("bark `{}` has a line: `silence` is a bark with none", b.id), "");
                }
                b.silence = true;
            }
        }
        (Cur::Bark, "cooldown") => match secs(rest) {
            Some(t) => ast.barks.last_mut().into_iter().for_each(|b| b.cooldown = t),
            None => err(errs, format!("`cooldown {rest}`: seconds, like `cooldown 20s`"), ""),
        },
        (Cur::Bark, "once") => ast.barks.last_mut().into_iter().for_each(|b| b.once = true),
        (Cur::Bark, "category") => ast.barks.last_mut().into_iter().for_each(|b| b.category = Some(sp(rest))),
        (Cur::Info, "true") => match rest.strip_prefix("if ") {
            Some(e) => ast.infos.last_mut().into_iter().for_each(|i| i.truth = Some(sp(e))),
            None => err(errs, format!("`{line}`: write `true if <expr>`"), ""),
        },
        (Cur::Setup, "by") => ast.setups.last_mut().into_iter().for_each(|s| s.by.push(sp(rest))),
        (Cur::Setup, "pays") => match rest.split_once(char::is_whitespace) {
            Some((kind, r)) if matches!(kind, "acknowledge" | "resolve" | "complicate" | "leave") => {
                ast.setups.last_mut().into_iter().for_each(|s| s.pays.push((kind.to_string(), sp(r))));
            }
            _ => err(errs, format!("`{line}`: write `pays acknowledge|resolve|complicate|leave <line, choice or bark>`"), ""),
        },
        (Cur::Director | Cur::Interv, "every" | "recover") => match secs(rest) {
            Some(t) => {
                if let Some(d) = ast.directors.last_mut() {
                    if word == "every" {
                        d.every = t;
                    } else {
                        d.recover = t;
                    }
                }
            }
            None => err(errs, format!("`{word} {rest}`: seconds, like `{word} 15s`"), ""),
        },
        (Cur::Director | Cur::Interv, "budget") => match rest.split_whitespace().collect::<Vec<_>>().as_slice() {
            [n, "per", "round"] if n.parse::<u32>().is_ok() => ast.directors.last_mut().into_iter().for_each(|d| d.budget = n.parse().ok()),
            _ => err(errs, format!("`{line}`: write `budget <n> per round`"), ""),
        },
        (Cur::Director | Cur::Interv, "tension") => ast.directors.last_mut().into_iter().for_each(|d| d.tension = Some(sp(rest))),
        (Cur::Director | Cur::Interv, "rest") => match rest.strip_prefix("if ") {
            Some(e) => ast.directors.last_mut().into_iter().for_each(|d| d.rest.push(sp(e))),
            None => err(errs, format!("`{line}`: write `rest if <expr>`"), ""),
        },
        (Cur::Director | Cur::Interv, "intervene") => {
            if let Some(d) = ast.directors.last_mut() {
                d.interventions.push(Interv { id: rest.into(), attrs: Attrs::default(), chance: None, group: None, once: false, at });
                *cur = Cur::Interv;
            }
        }
        (Cur::Interv, "chance") => match rest.strip_suffix('%').and_then(|n| n.trim().parse::<u8>().ok()).filter(|n| *n <= 100) {
            Some(n) => ast.directors.last_mut().and_then(|d| d.interventions.last_mut()).into_iter().for_each(|i| i.chance = Some(n)),
            None => err(errs, format!("`chance {rest}`: a percent, like `chance 40%`"), ""),
        },
        (Cur::Interv, "group") => ast.directors.last_mut().and_then(|d| d.interventions.last_mut()).into_iter().for_each(|i| i.group = Some(rest.into())),
        (Cur::Interv, "once") => ast.directors.last_mut().and_then(|d| d.interventions.last_mut()).into_iter().for_each(|i| i.once = true),
        _ => {
            let place = match *cur {
                Cur::None => "at the top of the file".to_string(),
                c => format!("after a {}", format!("{c:?}").to_lowercase()),
            };
            err(errs, format!("`{line}` is not understood {place}"), "see `describe story` for what each item takes");
        }
    }
}
