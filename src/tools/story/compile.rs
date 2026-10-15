//! Lowers a parsed story ([`super::parse::Ast`]) into a package (`sim::story::package::Package`): names become variable-table indices,
//! expressions become the rules' own language (`v<index>`), node names become line indices, seconds become ticks. Every name is checked
//! against what the story declares and what the game declares (its bindings and its scene's variables); every expression is type-checked
//! (a condition is a yes/no; an int fact takes a whole number). The package then goes through the runtime's own `check` and `bind`.

use super::parse::{Ast, At, Attrs, Spanned, TICKS};
use super::types::{conjuncts, type_of, T};
use super::Diag;
use crate::sim::defs::{closest, Id};
use crate::sim::rules_expr::{self, MathFn};
use crate::sim::story::package::{
    self as pkg, Bark, Catalog, Category, Choice, Conversation, Director, Effect, Fact, Info, Intervention, Line, Next, Package, Piece, Scope, Speaker, Src,
    Stakes, Trigger, Var, Who,
};
use crate::sim::systems::Ty;
use std::collections::{BTreeMap, BTreeSet};

mod effects;

/// Profile fields a major character (`character`) must have, with what each is for.
pub const MAJOR_FIELDS: &[(&str, &str)] = &[
    ("want", "what they are after right now"),
    ("need", "the deeper need or tension under it (or `arc: stable, <why they do not change>`)"),
    ("flaw", "a flaw, limit or pressure point"),
    ("contradiction", "a meaningful contradiction"),
    ("values", "what they hold to, and the line they will not cross"),
    ("relationships", "who matters to them, and how"),
    ("voice", "how they talk, from their perspective (not a catchphrase)"),
];
/// Profile fields a minor speaker (`extra`) must have.
pub const MINOR_FIELDS: &[(&str, &str)] = &[("role", "what they are in this place"), ("voice", "how they talk")];
/// Every field a profile may have (the rest are for people, the machine reads `knows`, `present` and `cooldown`).
pub const FIELDS: &[&str] = &[
    "want",
    "need",
    "arc",
    "flaw",
    "contradiction",
    "values",
    "boundaries",
    "relationships",
    "voice",
    "example",
    "knows",
    "beliefs",
    "secret",
    "present",
    "cooldown",
    "role",
];
/// Names an author cannot use for an id.
const RESERVED: &[&str] = &["player", "narrator", "present", "everyone", "story", "end"];
/// Functions a story's expressions add to the rules' own (each a variable lookup, never a call into the game).
const STORY_FNS: &[&str] = &["knows", "believes", "heard", "present"];

/// What the compiler learned beyond the package: where each expression is read, for the linter and `story refs`.
#[derive(Debug, Clone, Default)]
pub struct Index {
    /// Per variable: the places that read it (`(what, at)`).
    pub readers: BTreeMap<u32, Vec<(String, At)>>,
    /// Per fact: the places that write it.
    pub writers: BTreeMap<u32, Vec<(String, At)>>,
    /// Per info: places that reveal, claim or teach it.
    pub tellers: BTreeMap<u16, Vec<(String, At)>>,
    /// Choices: `(conversation, line index, choice index)` with their traced stakes and why.
    pub stakes: Vec<(u32, u32, usize, Stakes, String)>,
    /// Lines, by full id.
    pub line_ids: BTreeMap<String, u32>,
    /// Each node's first line: `(conversation, node id)` -> line index.
    pub nodes: BTreeMap<(u32, String), u32>,
    /// Choices whose author declared their stakes (`kind`): `(line index, choice index)`.
    pub declared: BTreeSet<(u32, usize)>,
}

struct Lower<'a> {
    ast: &'a Ast,
    game: &'a Catalog,
    scene: &'a BTreeMap<String, Ty>,
    p: Package,
    types: Vec<Ty>,
    errs: Vec<Diag>,
    ix: Index,
    holders: BTreeMap<String, u16>,
    facts: BTreeMap<String, u32>,
    infos: BTreeMap<String, u16>,
    barks: BTreeMap<String, u32>,
    convs: BTreeMap<String, u32>,
    categories: BTreeMap<String, u16>,
}

fn src(at: At) -> Src {
    Src { file: at.file as u16, line: at.line }
}

/// Compiles `ast` against the game's `catalog` and the scene's variables (`scene`: name -> type from its start value).
pub fn compile(ast: &Ast, catalog: &Catalog, scene: &BTreeMap<String, Ty>, source_hash: &str) -> (Option<Package>, Index, Vec<Diag>) {
    let story = ast.header.story.as_ref().map_or_else(String::new, |s| s.text.clone());
    let p = Package {
        story_package: pkg::FORMAT,
        story,
        content_version: ast.header.version.map_or(0, |(v, _)| v),
        source_hash: source_hash.to_string(),
        sources: ast.files.clone(),
        round: None,
        holders: vec!["player".into()],
        speakers: Vec::new(),
        facts: Vec::new(),
        infos: Vec::new(),
        vars: Vec::new(),
        exprs: Vec::new(),
        authored: Vec::new(),
        lines: Vec::new(),
        conversations: Vec::new(),
        barks: Vec::new(),
        categories: Vec::new(),
        directors: Vec::new(),
    };
    let mut l = Lower {
        ast,
        game: catalog,
        scene,
        p,
        types: Vec::new(),
        errs: Vec::new(),
        ix: Index::default(),
        holders: BTreeMap::new(),
        facts: BTreeMap::new(),
        infos: BTreeMap::new(),
        barks: BTreeMap::new(),
        convs: BTreeMap::new(),
        categories: BTreeMap::new(),
    };
    l.header();
    l.declarations();
    l.conversations();
    l.barks_and_directors();
    let ok = !l.errs.iter().any(Diag::is_error);
    let Lower { p, ix, mut errs, .. } = l;
    if !ok {
        return (None, ix, errs);
    }
    // The runtime's own checks: what it would refuse, the toolchain refuses first.
    for d in pkg::check(&p).into_iter().chain(pkg::bind(&p, catalog)) {
        errs.push(Diag::error(d.code, At::default(), d.message, d.fix.as_deref().unwrap_or("")));
    }
    let ok = !errs.iter().any(Diag::is_error);
    (ok.then_some(p), ix, errs)
}

impl Lower<'_> {
    fn err(&mut self, code: &'static str, at: At, m: impl Into<String>, fix: &str) {
        self.errs.push(Diag::error(code, at, m.into(), fix));
    }

    fn header(&mut self) {
        let h = &self.ast.header;
        if h.story.is_none() {
            self.err(pkg::code::SYNTAX, At::default(), "no `story <name>` line", "begin every file with `story <name>`");
        }
        if h.version.is_none() {
            self.err(pkg::code::VERSION, At::default(), "no content version", "write `story <name> version 1` in one file");
        }
        if let Some(r) = &h.round {
            match Id::parse(&r.text) {
                Ok(id) => self.p.round = Some(id),
                Err(e) => self.err(pkg::code::BINDING, r.at, e, ""),
            }
        }
    }

    fn unique(&mut self, seen: &mut BTreeMap<String, At>, id: &str, at: At) {
        if RESERVED.contains(&id) {
            self.err(pkg::code::REFERENCE, at, format!("`{id}` is a reserved word, not an id"), "rename it");
        } else if !id.starts_with(|c: char| c.is_ascii_lowercase()) || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            self.err(pkg::code::REFERENCE, at, format!("`{id}` is not an id (lowercase letters, digits, `_`, starting with a letter)"), "");
        } else if let Some(first) = seen.insert(id.to_string(), at) {
            let place = format!("{}:{}", self.ast.files[first.file], first.line);
            self.err(pkg::code::REFERENCE, at, format!("`{id}` is already the id of something at {place}"), "every id in a story is unique");
        }
    }

    fn declarations(&mut self) {
        let ast = self.ast;
        let mut seen: BTreeMap<String, At> = BTreeMap::new();
        for c in &ast.characters {
            self.unique(&mut seen, &c.id, c.at);
            self.holders.insert(c.id.clone(), self.p.holders.len() as u16);
            self.p.holders.push(c.id.clone());
        }
        for f in &ast.facts {
            self.unique(&mut seen, &f.id, f.at);
            let ty = if f.ty == "bool" { Ty::Bool } else { Ty::Int };
            let default = match (ty, f.default.as_str()) {
                (Ty::Bool, "true") => 1.0,
                (Ty::Bool, "false") => 0.0,
                (Ty::Int, d) if d.parse::<i64>().is_ok() => d.parse::<i64>().unwrap_or(0) as f64,
                _ => {
                    self.err(pkg::code::EXPRESSION, f.at, format!("`{}` is not a {} start value", f.default, f.ty), "bool: true or false; int: a whole number");
                    0.0
                }
            };
            let scope = match f.scope.as_str() {
                "round" => Scope::Round,
                "profile" => Scope::Profile,
                _ => Scope::Run,
            };
            if scope == Scope::Round && ast.header.round.is_none() {
                self.err(
                    pkg::code::REFERENCE,
                    f.at,
                    format!("`{}` is a round fact, but the story names no round event", f.id),
                    "add `round <event>` to the header",
                );
            }
            self.facts.insert(f.id.clone(), self.p.facts.len() as u32);
            self.p.facts.push(Fact { id: f.id.clone(), ty, default, scope, doc: f.doc.clone(), at: src(f.at) });
        }
        for i in &ast.infos {
            self.unique(&mut seen, &i.id, i.at);
            self.infos.insert(i.id.clone(), self.p.infos.len() as u16);
            self.p.infos.push(Info { id: i.id.clone(), doc: i.doc.clone(), truth: None, at: src(i.at) });
        }
        for (id, ticks, at) in &ast.categories {
            self.unique(&mut seen, id, *at);
            self.categories.insert(id.clone(), self.p.categories.len() as u16);
            self.p.categories.push(Category { id: id.clone(), cooldown: *ticks });
        }
        for s in &ast.setups {
            self.unique(&mut seen, &s.id, s.at);
        }
        for (i, c) in ast.convs.iter().enumerate() {
            self.unique(&mut seen, &c.id, c.at);
            self.convs.insert(c.id.clone(), i as u32);
        }
        for (i, b) in ast.barks.iter().enumerate() {
            self.unique(&mut seen, &b.id, b.at);
            self.barks.insert(b.id.clone(), i as u32);
        }
        for d in &ast.directors {
            self.unique(&mut seen, &d.id, d.at);
        }
        // Infos' truths and the characters' sheets, now that every name is known.
        for (k, i) in ast.infos.iter().enumerate() {
            if let Some(t) = &i.truth {
                self.p.infos[k].truth = self.cond(t, &format!("info {}", i.id));
            }
        }
        for c in &ast.characters {
            self.sheet(c);
        }
    }

    fn sheet(&mut self, c: &super::parse::Character) {
        for (k, v) in &c.fields {
            if !FIELDS.contains(&k.as_str()) {
                let hint = closest(k, FIELDS.iter().copied()).map(|h| format!(" (did you mean `{h}`?)")).unwrap_or_default();
                self.err(pkg::code::PROFILE, v.at, format!("`{k}` is not a profile field{hint}"), &format!("one of: {}", FIELDS.join(", ")));
            }
        }
        let required = if c.minor { MINOR_FIELDS } else { MAJOR_FIELDS };
        for (field, what) in required {
            let has = c.get(field).is_some_and(|v| !v.text.is_empty()) || (*field == "need" && c.get("arc").is_some_and(|v| v.text.starts_with("stable")));
            if !has {
                let kind = if c.minor { "an extra" } else { "a character" };
                self.err(pkg::code::PROFILE, c.at, format!("{} `{}` has no `{field}:` ({what})", kind, c.id), &format!("add `{field}: ...` under it"));
            }
        }
        if !c.minor && c.all("example").count() < 2 {
            self.err(
                pkg::code::PROFILE,
                c.at,
                format!("character `{}` has {} `example:` lines; a voice needs at least two of its own", c.id, c.all("example").count()),
                "add original example lines in their voice",
            );
        }
        let holder = self.holders[&c.id];
        let present = c.get("present").cloned().and_then(|p| {
            let (k, ty) = self.name_var(&p.text, p.at, &format!("{}'s presence", c.id))?;
            if ty != T::Bool {
                self.err(pkg::code::EXPRESSION, p.at, format!("`present: {}` is {}, not a yes/no", p.text, ty.name()), "name a yes/no query or scene variable");
            }
            Some(k)
        });
        let cooldown = match c.get("cooldown") {
            None => 0,
            Some(v) => match v.text.strip_suffix('s').and_then(|n| n.parse::<f32>().ok()) {
                Some(s) => (s * TICKS as f32).round() as u32,
                None => {
                    self.err(pkg::code::SYNTAX, v.at, format!("`cooldown: {}`: seconds, like `cooldown: 8s`", v.text), "");
                    0
                }
            },
        };
        let mut knows = Vec::new();
        for v in c.all("knows").cloned().collect::<Vec<_>>() {
            for name in v.text.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                match self.infos.get(name) {
                    Some(i) => {
                        knows.push(*i);
                        self.ix.tellers.entry(*i).or_default().push((format!("{}'s sheet", c.id), v.at));
                    }
                    None => self.unknown(v.at, "info", name, self.infos.keys().cloned().collect()),
                }
            }
        }
        self.p.speakers.push(Speaker { id: c.id.clone(), name: c.name.clone(), holder, present, cooldown, knows, at: src(c.at) });
    }

    fn unknown(&mut self, at: At, what: &str, name: &str, known: Vec<String>) {
        let hint = closest(name, known.iter().map(String::as_str)).map(|h| format!(" (did you mean `{h}`?)")).unwrap_or_default();
        self.err(pkg::code::REFERENCE, at, format!("no {what} `{name}`{hint}"), "");
    }

    fn var(&mut self, v: Var, ty: Ty) -> u32 {
        if let Some(k) = self.p.vars.iter().position(|x| *x == v) {
            return k as u32;
        }
        self.p.vars.push(v);
        self.types.push(ty);
        (self.p.vars.len() - 1) as u32
    }

    /// A bare name (a fact) or a namespaced one (a game query, `scene:var`) as a variable.
    fn name_var(&mut self, name: &str, at: At, what: &str) -> Option<(u32, T)> {
        if let Some((ns, rest)) = name.split_once(':') {
            let id = match Id::parse(name) {
                Ok(id) => id,
                Err(e) => {
                    self.err(pkg::code::BINDING, at, e, "");
                    return None;
                }
            };
            if ns == crate::sim::systems::OUTSIDE_NS {
                let Some(ty) = self.scene.get(rest).copied() else {
                    self.unknown(at, "scene variable", rest, self.scene.keys().cloned().collect());
                    return None;
                };
                let k = self.var(Var::Query { id, ty }, ty);
                self.read(k, what, at);
                return Some((k, T::of(ty)));
            }
            let Some(d) = self.game.queries.iter().find(|q| q.id == id) else {
                let known = self.game.queries.iter().map(|q| q.id.to_string()).collect();
                self.unknown(at, "query the game answers named", name, known);
                return None;
            };
            let k = self.var(Var::Query { id, ty: d.ty }, d.ty);
            self.read(k, what, at);
            return Some((k, T::of(d.ty)));
        }
        let Some(&f) = self.facts.get(name) else {
            let mut known: Vec<String> = self.facts.keys().cloned().collect();
            known.extend(self.game.queries.iter().map(|q| q.id.to_string()));
            self.unknown(at, "fact", name, known);
            return None;
        };
        let ty = self.p.facts[f as usize].ty;
        let k = self.var(Var::Fact { fact: f }, ty);
        self.read(k, what, at);
        Some((k, T::of(ty)))
    }

    fn read(&mut self, k: u32, what: &str, at: At) {
        let r = self.ix.readers.entry(k).or_default();
        if !r.iter().any(|(w, a)| w == what && *a == at) {
            r.push((what.to_string(), at));
        }
    }

    fn holder(&mut self, name: &str, at: At) -> Option<u16> {
        if let Some(h) = self.holders.get(name) {
            return Some(*h);
        }
        if name == "player" {
            return Some(0);
        }
        self.unknown(at, "character (or `player`)", name, self.holders.keys().cloned().collect());
        None
    }

    fn info(&mut self, name: &str, at: At) -> Option<u16> {
        let i = self.infos.get(name).copied();
        if i.is_none() {
            self.unknown(at, "info", name, self.infos.keys().cloned().collect());
        }
        i
    }

    /// A story function (`knows(h, i)`, `believes(h, i)`, `heard(line)`, `present(speaker)`) as a variable, or a constant.
    fn story_fn(&mut self, f: &str, args: &[&str], at: At, what: &str) -> Option<String> {
        let want = if matches!(f, "knows" | "believes") { 2 } else { 1 };
        if args.len() != want {
            self.err(pkg::code::EXPRESSION, at, format!("`{f}` takes {want} argument(s), got {}", args.len()), "");
            return None;
        }
        let k = match f {
            "knows" | "believes" => {
                let (h, i) = (self.holder(args[0], at)?, self.info(args[1], at)?);
                let v = if f == "knows" { Var::Knows { holder: h, info: i } } else { Var::Believes { holder: h, info: i } };
                self.var(v, Ty::Bool)
            }
            "heard" => {
                let Some(&li) = self.ix.line_ids.get(args[0]) else {
                    self.unknown(
                        at,
                        "line (give the line an `id`, then `heard(<conversation>/<id>)` or `heard(<bark>)`)",
                        args[0],
                        self.ix.line_ids.keys().cloned().collect(),
                    );
                    return None;
                };
                self.var(Var::Heard { line: li }, Ty::Bool)
            }
            _ => {
                let Some(&h) = self.holders.get(args[0]) else {
                    self.unknown(at, "character", args[0], self.holders.keys().cloned().collect());
                    return None;
                };
                match self.ast.characters.get(h as usize - 1).and_then(|c| c.get("present")).cloned() {
                    Some(p) => self.name_var(&p.text, p.at, what)?.0,
                    None => return Some("1".into()),
                }
            }
        };
        self.read(k, what, at);
        Some(format!("v{k}"))
    }

    /// Lowers authored expression text: every name to `v<index>`, story functions to their variables. Returns the lowered text.
    fn lower_text(&mut self, e: &Spanned, what: &str) -> Option<String> {
        let s: Vec<char> = e.text.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        let mut ok = true;
        while i < s.len() {
            let c = s[i];
            if c.is_ascii_alphabetic() || c == '_' {
                let start = i;
                while i < s.len() && (s[i].is_ascii_alphanumeric() || s[i] == '_') {
                    i += 1;
                }
                let mut word: String = s[start..i].iter().collect();
                if i + 1 < s.len() && s[i] == ':' && s[i + 1].is_ascii_lowercase() {
                    i += 1;
                    let rest = i;
                    while i < s.len() && (s[i].is_ascii_alphanumeric() || matches!(s[i], '_' | '/' | '-')) {
                        i += 1;
                    }
                    word = format!("{word}:{}", s[rest..i].iter().collect::<String>());
                }
                let mut j = i;
                while j < s.len() && s[j] == ' ' {
                    j += 1;
                }
                let call = j < s.len() && s[j] == '(';
                if call && STORY_FNS.contains(&word.as_str()) {
                    let Some(close) = s[j..].iter().position(|c| *c == ')') else {
                        self.err(pkg::code::EXPRESSION, e.at, format!("`{word}(` is never closed in `{}`", e.text), "");
                        return None;
                    };
                    let inner: String = s[j + 1..j + close].iter().collect();
                    let args: Vec<&str> = inner.split(',').map(str::trim).filter(|a| !a.is_empty()).collect();
                    match self.story_fn(&word, &args, e.at, what) {
                        Some(v) => out.push_str(&v),
                        None => ok = false,
                    }
                    i = j + close + 1;
                    continue;
                }
                if word == "random" {
                    self.err(
                        pkg::code::EXPRESSION,
                        e.at,
                        "`random` is not allowed in a story: a condition must not draw".to_string(),
                        "vary with several barks (a tie draws once) or a director's `chance`",
                    );
                    ok = false;
                } else if word == "me" {
                    self.err(pkg::code::EXPRESSION, e.at, "`me.` (a per-player variable) is not something a story reads".to_string(), "read a game query");
                    ok = false;
                } else if call || matches!(word.as_str(), "true" | "false") {
                    if call && !MathFn::ALL.iter().any(|(n, ..)| *n == word) {
                        let known: Vec<&str> = MathFn::ALL.iter().map(|(n, ..)| *n).chain(STORY_FNS.iter().copied()).collect();
                        let hint = closest(&word, known.iter().copied()).map(|h| format!(" (did you mean `{h}`?)")).unwrap_or_default();
                        self.err(pkg::code::EXPRESSION, e.at, format!("no function `{word}`{hint}"), &format!("one of: {}", known.join(", ")));
                        ok = false;
                    }
                    out.push_str(&word);
                } else {
                    match self.name_var(&word, e.at, what) {
                        Some((k, _)) => out.push_str(&format!("v{k}")),
                        None => ok = false,
                    }
                }
                continue;
            }
            out.push(c);
            i += 1;
        }
        ok.then_some(out)
    }

    /// Lowers and type-checks an expression; returns its index and type.
    fn expr(&mut self, e: &Spanned, what: &str) -> Option<(u32, T)> {
        let lowered = self.lower_text(e, what)?;
        let names = pkg::var_names(self.p.vars.len());
        let parsed = match rules_expr::parse(&lowered, &names) {
            Ok(x) => x,
            Err(err) => {
                self.err(pkg::code::EXPRESSION, e.at, format!("`{}` does not read as an expression: {}", e.text, err.message), "");
                return None;
            }
        };
        let ty = match type_of(&parsed, &self.types) {
            Ok(t) => t,
            Err(m) => {
                self.err(pkg::code::EXPRESSION, e.at, format!("`{}`: {m}", e.text), "");
                return None;
            }
        };
        self.p.exprs.push(lowered);
        self.p.authored.push(e.text.clone());
        Some(((self.p.exprs.len() - 1) as u32, ty))
    }

    fn cond(&mut self, e: &Spanned, what: &str) -> Option<u32> {
        let (k, t) = self.expr(e, what)?;
        if !t.boolish() {
            self.err(pkg::code::EXPRESSION, e.at, format!("`if {}` is {}, not a yes/no", e.text, t.name()), "compare it (`x > 0`)");
            return None;
        }
        Some(k)
    }

    /// Text with `{name}` holes: a fact or a query, shown as a number.
    fn text(&mut self, t: &Spanned, what: &str) -> Vec<Piece> {
        let mut out = Vec::new();
        let mut rest = t.text.as_str();
        while let Some(open) = rest.find('{') {
            if rest[open..].starts_with("{{") {
                out.push(Piece::Text(format!("{}{{", &rest[..open])));
                rest = &rest[open + 2..];
                continue;
            }
            let Some(close) = rest[open..].find('}') else {
                self.err(pkg::code::SYNTAX, t.at, format!("`{{` is never closed in `{}`", t.text), "write `{{` for a brace");
                break;
            };
            if open > 0 {
                out.push(Piece::Text(rest[..open].to_string()));
            }
            let name = rest[open + 1..open + close].trim();
            if let Some((k, ty)) = self.name_var(name, t.at, what) {
                if ty == T::Bool {
                    self.err(pkg::code::EXPRESSION, t.at, format!("`{{{name}}}` is a yes/no: a line says it in words"), "write two lines with `if` instead");
                }
                out.push(Piece::Var { var: k });
            }
            rest = &rest[open + close + 1..];
        }
        if !rest.is_empty() {
            out.push(Piece::Text(rest.replace("}}", "}")));
        }
        out
    }

    fn knowledge(&mut self, list: &[Spanned], what: &str) -> Vec<u16> {
        let mut out = Vec::new();
        for i in list {
            if let Some(k) = self.info(&i.text, i.at) {
                self.ix.tellers.entry(k).or_default().push((what.to_string(), i.at));
                out.push(k);
            }
        }
        out
    }

    fn speaker(&mut self, s: &Spanned) -> Option<Option<u16>> {
        if s.text == "narrator" {
            return Some(None);
        }
        match self.holders.get(&s.text) {
            Some(h) => Some(Some(h - 1)),
            None => {
                self.unknown(s.at, "speaker (a `character` or `extra`, or `narrator`)", &s.text, self.holders.keys().cloned().collect());
                None
            }
        }
    }

    fn conversations(&mut self) {
        let ast = self.ast;
        // Pass 1: every line's index and id, so `heard(...)` and gotos can name any of them.
        let mut first = 0u32;
        let mut plan: Vec<(u32, BTreeMap<String, u32>)> = Vec::new();
        for c in &ast.convs {
            let mut nodes = BTreeMap::new();
            let mut k = first;
            for n in &c.nodes {
                if nodes.insert(n.id.clone(), k).is_some() {
                    self.err(pkg::code::REFERENCE, n.at, format!("conversation `{}` has two nodes `{}`", c.id, n.id), "");
                }
                for (j, l) in n.lines.iter().enumerate() {
                    let local = l.id.as_ref().map_or_else(|| format!("{}/l{}", n.id, j + 1), |i| i.text.clone());
                    let full = format!("{}/{local}", c.id);
                    if self.ix.line_ids.insert(full.clone(), k).is_some() {
                        self.err(pkg::code::REFERENCE, l.at, format!("two lines are `{full}`"), "give one another `id`");
                    }
                    k += 1;
                }
            }
            plan.push((first, nodes));
            first = k;
        }
        // Barks' lines come after every conversation's, in the order written (so `heard(<bark>)` can be named before the bark is read).
        for decl in ast.barks.iter().filter(|b| b.line.is_some()) {
            self.ix.line_ids.insert(decl.id.clone(), first);
            first += 1;
        }
        // Pass 2: the lines themselves.
        for (ci, c) in ast.convs.iter().enumerate() {
            let (start, nodes) = plan[ci].clone();
            // Every conversation and line is still lowered after a problem, so the indices of the rest stay right.
            let with = match self.speaker(&c.with) {
                Some(Some(w)) => w,
                Some(None) => {
                    self.err(pkg::code::REFERENCE, c.at, "a conversation is with a character, not the narrator", "");
                    0
                }
                None => 0,
            };
            let cond = c.cond.as_ref().and_then(|e| self.cond(e, &format!("conversation {}", c.id)));
            if c.nodes.is_empty() {
                self.err(pkg::code::FLOW, c.at, format!("conversation `{}` has no nodes", c.id), "add `node <id>` and its lines");
            }
            let resolve = |l: &mut Lower, t: &Spanned| -> Option<Next> {
                if t.text == "end" {
                    return Some(Next::End);
                }
                match nodes.get(&t.text) {
                    Some(k) => Some(Next::Line(*k)),
                    None => {
                        l.unknown(t.at, &format!("node of `{}`", c.id), &t.text, nodes.keys().cloned().collect());
                        None
                    }
                }
            };
            for n in &c.nodes {
                if n.lines.is_empty() {
                    self.err(pkg::code::FLOW, n.at, format!("node `{}` has no lines", n.id), "a node says at least one line");
                }
                for (j, l) in n.lines.iter().enumerate() {
                    let local = l.id.as_ref().map_or_else(|| format!("{}/l{}", n.id, j + 1), |i| i.text.clone());
                    let id = format!("{}/{local}", c.id);
                    let what = format!("line {id}");
                    let speaker = self.speaker(&l.speaker).flatten();
                    let text = self.text(&l.text, &what);
                    let lc = l.attrs.cond.as_ref().and_then(|e| self.cond(e, &what));
                    let effects = self.effects(&l.attrs, &what, false);
                    let reveals = self.knowledge(&l.attrs.reveals, &what);
                    let claims = self.knowledge(&l.attrs.claims, &what);
                    let goto = l.goto.as_ref().and_then(|t| resolve(self, t));
                    let last = j + 1 == n.lines.len();
                    if !l.choices.is_empty() && !last {
                        self.err(
                            pkg::code::FLOW,
                            l.at,
                            format!("`{id}` offers choices but is not the last line of node `{}`", n.id),
                            "a line with choices ends its node: put what follows in a new node",
                        );
                    }
                    if l.choices.is_empty() && last && l.goto.is_none() {
                        self.err(
                            pkg::code::FLOW,
                            l.at,
                            format!("node `{}` ends without saying where it goes", n.id),
                            "add `-> <node>` or `-> end` under its last line",
                        );
                    }
                    let mut choices = Vec::new();
                    for ch in &l.choices {
                        let cw = format!("choice {}/{}", c.id, ch.id);
                        let ctext = self.text(&ch.text, &cw);
                        let ccond = ch.attrs.cond.as_ref().and_then(|e| self.cond(e, &cw));
                        let ceffects = self.effects(&ch.attrs, &cw, false);
                        if !ch.attrs.reveals.is_empty() || !ch.attrs.claims.is_empty() {
                            self.err(
                                pkg::code::SYNTAX,
                                ch.at,
                                format!("`{cw}`: the player's choice reveals nothing to anyone by itself"),
                                "use `do learn <who> <info>`, or a reply line that `reveals` it",
                            );
                        }
                        let cgoto = ch.goto.as_ref().and_then(|t| resolve(self, t));
                        if ch.goto.is_none() {
                            self.err(pkg::code::FLOW, ch.at, format!("`{cw}` does not say where it goes"), "add `-> <node>` or `-> end` under it");
                        }
                        let refused = ch.refused.as_ref().and_then(|t| resolve(self, t));
                        let stakes = match ch.kind.as_ref().map(|k| k.text.as_str()) {
                            None => Stakes::Expressive,
                            Some("consequential") => Stakes::Consequential,
                            Some("informational") => Stakes::Informational,
                            Some("expressive") => Stakes::Expressive,
                            Some(other) => {
                                self.err(pkg::code::SYNTAX, ch.at, format!("`kind {other}`: consequential, informational or expressive"), "");
                                Stakes::Expressive
                            }
                        };
                        if !ch.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') || ch.id.is_empty() {
                            self.err(pkg::code::REFERENCE, ch.at, format!("`{}` is not a choice id (lowercase letters, digits, `_`)", ch.id), "");
                        }
                        choices.push(Choice { id: ch.id.clone(), text: ctext, cond: ccond, effects: ceffects, goto: cgoto, refused, stakes, at: src(ch.at) });
                    }
                    for (k, ch) in l.choices.iter().enumerate() {
                        if ch.kind.is_some() {
                            self.ix.declared.insert((self.p.lines.len() as u32, k));
                        }
                    }
                    let mut seen_choice = BTreeSet::new();
                    for ch in &choices {
                        if !seen_choice.insert(ch.id.clone()) {
                            self.err(pkg::code::REFERENCE, l.at, format!("`{id}` has two choices `{}`", ch.id), "");
                        }
                    }
                    // Passed over, a line goes on to the next of its node; its node's last goes where it would have gone (or ends).
                    let skip = if last { Some(goto.unwrap_or(Next::End)) } else { None };
                    self.p.lines.push(Line { id, speaker, text, cond: lc, effects, reveals, claims, choices, goto, skip, at: src(l.at) });
                }
            }
            let len = self.p.lines.len() as u32 - start;
            for (name, k) in &nodes {
                self.ix.nodes.insert((ci as u32, name.clone()), *k);
            }
            self.p.conversations.push(Conversation {
                id: c.id.clone(),
                with,
                first: start,
                len,
                cond,
                priority: c.priority,
                interrupt: c.interrupt,
                repeat: c.repeat,
                at: src(c.at),
            });
        }
    }

    fn barks_and_directors(&mut self) {
        let ast = self.ast;
        for b in &ast.barks {
            let what = format!("bark {}", b.id);
            let on = b.on.as_ref().and_then(|(ev, about)| {
                let event = Id::parse(&ev.text).map_err(|e| self.err(pkg::code::BINDING, ev.at, e, "")).ok()?;
                if !self.game.events.contains(&event) {
                    self.unknown(ev.at, "event the game emits named", &ev.text, self.game.events.iter().map(Id::to_string).collect());
                    return None;
                }
                let subject = about.as_ref().and_then(|s| Id::parse(&s.text).map_err(|e| self.err(pkg::code::BINDING, s.at, e, "")).ok());
                Some(Trigger { event, subject })
            });
            if b.notice {
                if b.on.is_none() {
                    self.err(pkg::code::SYNTAX, b.at, format!("notice `{}` answers no event", b.id), "write `notice <id> on <event>`");
                }
                if b.attrs.effects.is_empty() {
                    self.err(pkg::code::SYNTAX, b.at, format!("notice `{}` does nothing", b.id), "give it `do learn present <info>` (or a fact it records)");
                }
            } else if b.line.is_none() && !b.silence {
                self.err(
                    pkg::code::SYNTAX,
                    b.at,
                    format!("bark `{}` says nothing", b.id),
                    "give it a `<speaker>: <text>` line, or `silence` for a deliberate nothing",
                );
            }
            let cond = b.attrs.cond.as_ref().and_then(|e| self.cond(e, &what));
            let line = b.line.as_ref().map(|(s, t)| {
                let speaker = self.speaker(s).flatten();
                let text = self.text(t, &what);
                let effects = self.effects(&b.attrs, &what, false);
                let reveals = self.knowledge(&b.attrs.reveals, &what);
                let claims = self.knowledge(&b.attrs.claims, &what);
                self.p.lines.push(Line {
                    id: b.id.clone(),
                    speaker,
                    text,
                    cond: None,
                    effects,
                    reveals,
                    claims,
                    choices: vec![],
                    goto: None,
                    skip: None,
                    at: src(t.at),
                });
                let k = (self.p.lines.len() - 1) as u32;
                self.ix.line_ids.insert(b.id.clone(), k);
                k
            });
            // A notice's effects apply when its event happens; a line's when it is heard; a silence has none.
            let effects = if b.notice { self.effects(&b.attrs, &format!("notice {}", b.id), false) } else { Vec::new() };
            if effects.iter().any(|e| matches!(e, Effect::Run { .. })) {
                self.err(
                    pkg::code::SYNTAX,
                    b.at,
                    format!("notice `{}` runs a command: a notice records what is seen, it does not act", b.id),
                    "move the command to a line or a choice",
                );
            }
            if (b.silence || b.notice) && (!b.attrs.reveals.is_empty() || !b.attrs.claims.is_empty()) {
                self.err(
                    pkg::code::SYNTAX,
                    b.at,
                    format!("`{}` says nothing, so it reveals nothing", b.id),
                    "use `do learn present <info>` for what witnesses notice",
                );
            }
            if b.silence && !b.attrs.effects.is_empty() {
                self.err(pkg::code::SYNTAX, b.at, format!("bark `{}` is a silence: it does nothing", b.id), "use a `notice` for what witnesses learn");
            }
            let category = b.category.as_ref().and_then(|c| match self.categories.get(&c.text) {
                Some(k) => Some(*k),
                None => {
                    self.unknown(c.at, "category", &c.text, self.categories.keys().cloned().collect());
                    None
                }
            });
            let specificity =
                cond.map_or(0, |e| conjuncts(&self.p.exprs[e as usize], self.p.vars.len())) + on.as_ref().map_or(0, |t| t.subject.is_some() as u32);
            self.p.barks.push(Bark {
                id: b.id.clone(),
                on,
                line,
                cond,
                priority: b.priority,
                specificity,
                cooldown: b.cooldown,
                once: b.once,
                category,
                notice: b.notice,
                effects,
                at: src(b.at),
            });
        }
        for d in &ast.directors {
            let what = format!("director {}", d.id);
            if d.every == 0 {
                self.err(pkg::code::SYNTAX, d.at, format!("director `{}` has no `every <secs>s`", d.id), "say how often it looks");
            }
            let tension = d.tension.as_ref().and_then(|e| self.expr(e, &what)).map(|(k, t)| {
                if t == T::Bool {
                    self.err(pkg::code::EXPRESSION, d.at, "a tension is a number, not a yes/no".to_string(), "");
                }
                k
            });
            let rest = d.rest.iter().filter_map(|e| self.cond(e, &format!("{what} rest"))).collect();
            let mut interventions = Vec::new();
            for i in &d.interventions {
                let iw = format!("intervention {}/{}", d.id, i.id);
                let cond = i.attrs.cond.as_ref().and_then(|e| self.cond(e, &iw));
                let effects = self.effects(&i.attrs, &iw, true);
                if effects.is_empty() {
                    self.err(pkg::code::SYNTAX, i.at, format!("`{iw}` does nothing"), "give it `do ...` (to do nothing on purpose, use `rest if`)");
                }
                interventions.push(Intervention { id: i.id.clone(), cond, chance: i.chance, group: i.group.clone(), once: i.once, effects, at: src(i.at) });
            }
            self.p.directors.push(Director {
                id: d.id.clone(),
                every: d.every.max(1),
                recover: d.recover,
                budget: d.budget,
                tension,
                rest,
                interventions,
                at: src(d.at),
            });
        }
    }
}
