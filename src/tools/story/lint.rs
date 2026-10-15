//! What a compiled story's structure can prove, beyond its syntax and types: conversations that cannot end, loops that never wait for the
//! player, someone revealing what they can never know (errors); writes nothing reads, setups nothing pays off, choices whose declared stakes
//! are not what they change, equivalent alternatives, repetition, missing fallbacks, long lines (warnings). A warning can be accepted
//! with `allow <warning> <id> "<why>"`, which every check lists. Nothing here judges the writing: `story review` asks a reader to.

use super::compile::Index;
use super::parse::{Ast, At};
use super::{Diag, Severity};
use crate::sim::story::package::{Catalog, Effect, Line, Next, Package, Stakes, Var, Who};
use std::collections::{BTreeMap, BTreeSet};

/// Warning kinds an `allow` may name, with what each means.
pub const WARNINGS: &[(&str, &str)] = &[
    ("unread", "a fact is written but nothing reads it (no condition, no text, not the game, not `shown`)"),
    ("unwritten", "a fact is read but nothing ever changes it"),
    ("unused", "a fact or info nothing uses"),
    ("untold", "a condition asks who knows an info nobody can ever learn"),
    ("unpaid", "a setup nothing acknowledges, resolves, complicates or deliberately leaves"),
    ("stakes", "a choice's declared kind is not what it changes"),
    ("equivalent", "two choices of a line do the same thing and go to the same place"),
    ("repeat", "the same words more than once, or one bark answering an event every time"),
    ("fallback", "when no condition holds nothing is offered or said"),
    ("unreached", "a node nothing leads to"),
    ("length", "a line longer than the story's `style max_line` (default 200)"),
    ("else", "an `else` that can never be taken"),
    ("request", "an implementation request the game has not answered yet (a mechanic the story wants)"),
];

fn at(s: crate::sim::story::package::Src) -> At {
    At { file: s.file as usize, line: s.line }
}

struct Lints<'a> {
    ast: &'a Ast,
    out: Vec<Diag>,
    used_allows: BTreeSet<usize>,
}

impl Lints<'_> {
    fn error(&mut self, code: &'static str, a: At, m: String, fix: &str) {
        self.out.push(Diag::error(code, a, m, fix));
    }

    fn warn(&mut self, kind: &str, target: &str, a: At, m: String, fix: &str) {
        if let Some(k) = self.ast.allows.iter().position(|(w, t, _, _)| w == kind && t == target) {
            self.used_allows.insert(k);
            let why = &self.ast.allows[k].2;
            self.out.push(Diag {
                severity: Severity::Allowed,
                code: "story-warning",
                at: a,
                message: format!("allowed {kind} {target}: {m} (because: {why})"),
                fix: None,
            });
            return;
        }
        self.out.push(Diag::warning(
            "story-warning",
            a,
            format!("[{kind}] {m}"),
            &format!("{fix}{}or `allow {kind} {target} \"<why>\"`", if fix.is_empty() { "" } else { "; " }),
        ));
    }
}

/// Each choice's stakes as traced from what it changes: `(conversation, line, choice, stakes, why)`.
pub fn trace_stakes(p: &Package, ix: &Index, game: &Catalog) -> Vec<(u32, u32, usize, Stakes, String)> {
    let mut out = Vec::new();
    let fact_var = |f: u32| p.vars.iter().position(|v| *v == Var::Fact { fact: f }).map(|k| k as u32);
    let read = |f: u32| fact_var(f).is_some_and(|k| ix.readers.get(&k).is_some_and(|r| !r.is_empty())) || consumed(p, game, f);
    for (ci, c) in p.conversations.iter().enumerate() {
        for li in c.first..c.first + c.len {
            let l = &p.lines[li as usize];
            for (k, ch) in l.choices.iter().enumerate() {
                let mut why = Vec::new();
                let mut stakes = Stakes::Expressive;
                for e in &ch.effects {
                    match e {
                        Effect::Run { command, .. } => {
                            stakes = Stakes::Consequential;
                            why.push(format!("runs {command}"));
                        }
                        Effect::Set { fact, .. } | Effect::Add { fact, .. } if read(*fact) => {
                            stakes = Stakes::Consequential;
                            why.push(format!("changes {}, which is read", p.facts[*fact as usize].id));
                        }
                        Effect::Set { fact, .. } | Effect::Add { fact, .. } => why.push(format!("changes {}, which nothing reads", p.facts[*fact as usize].id)),
                        Effect::Learn { who, info, .. } => {
                            // What someone knows is consequential when something other than this choice asks it.
                            let label = format!("choice {}/{}", c.id, ch.id);
                            let asked = p.vars.iter().enumerate().any(|(k, v)| {
                                matches!(v, Var::Knows { holder, info: i } | Var::Believes { holder, info: i }
                                    if i == info && reaches(p, *who, *holder))
                                    && ix.readers.get(&(k as u32)).is_some_and(|r| r.iter().any(|(w, _)| *w != label))
                            });
                            let name = format!("{} learns {}", who_name(p, *who), p.infos[*info as usize].id);
                            if asked {
                                stakes = Stakes::Consequential;
                                why.push(format!("{name}, which a later condition asks"));
                            } else {
                                if stakes != Stakes::Consequential {
                                    stakes = Stakes::Informational;
                                }
                                why.push(name);
                            }
                        }
                        Effect::Start { conversation } => why.push(format!("starts {}", p.conversations[*conversation as usize].id)),
                        Effect::Bark { bark } => why.push(format!("offers {}", p.barks[*bark as usize].id)),
                    }
                }
                if stakes != Stakes::Consequential {
                    if let Some(Next::Line(t)) = ch.goto {
                        let reveals: Vec<&str> =
                            node_lines(p, ci as u32, t).iter().flat_map(|l| l.reveals.iter().map(|i| p.infos[*i as usize].id.as_str())).collect();
                        if !reveals.is_empty() {
                            stakes = Stakes::Informational;
                            why.push(format!("leads to a reply that reveals {}", reveals.join(", ")));
                        } else if why.is_empty() {
                            why.push("changes only what is said next".into());
                        }
                    } else if why.is_empty() {
                        why.push("ends the conversation, changing nothing".into());
                    }
                }
                out.push((ci as u32, li, k, stakes, why.join("; ")));
            }
        }
    }
    out
}

/// Whether the game (or a presenter) reads story fact `f`: a system's `reads` names `<story>:<fact>`.
fn consumed(p: &Package, game: &Catalog, f: u32) -> bool {
    let id = format!("{}:{}", p.story, p.facts[f as usize].id);
    game.consumers.iter().any(|c| c.as_str() == id)
}

/// Whether learning for `who` reaches `holder`.
fn reaches(p: &Package, who: Who, holder: u16) -> bool {
    match who {
        Who::Holder(h) => h == holder,
        Who::Present => holder > 0 && (holder as usize) < p.holders.len(),
        Who::Everyone => true,
    }
}

fn who_name(p: &Package, w: Who) -> String {
    match w {
        Who::Holder(h) => p.holders[h as usize].clone(),
        Who::Present => "everyone present".into(),
        Who::Everyone => "everyone".into(),
    }
}

/// The lines of the node starting at `first` (up to the line with choices, or the conversation's end).
fn node_lines(p: &Package, ci: u32, first: u32) -> Vec<&Line> {
    let c = &p.conversations[ci as usize];
    let mut out = Vec::new();
    let mut li = first;
    while li < c.first + c.len {
        let l = &p.lines[li as usize];
        out.push(l);
        if !l.choices.is_empty() || l.goto.is_some() {
            break;
        }
        li += 1;
    }
    out
}

/// Every check, against the package `p` compiled from `ast` (with the compiler's `ix`) and the game's `game`.
pub fn lint(ast: &Ast, p: &Package, ix: &Index, game: &Catalog) -> Vec<Diag> {
    let mut l = Lints { ast, out: Vec::new(), used_allows: BTreeSet::new() };
    for (k, (kind, target, _, a)) in ast.allows.iter().enumerate() {
        if !WARNINGS.iter().any(|(w, _)| w == kind) {
            let known: Vec<&str> = WARNINGS.iter().map(|(w, _)| *w).collect();
            l.error("story-syntax", *a, format!("`allow {kind}`: no such warning"), &format!("one of: {}", known.join(", ")));
            l.used_allows.insert(k);
        } else if target.is_empty() {
            l.used_allows.insert(k);
        }
    }
    for (id, what, a) in &ast.requests {
        l.warn(
            "request",
            id,
            *a,
            format!("implementation request `{id}` is open: {what} (nothing in the story may depend on it until the game declares it)"),
            "ask for it in the game; then bind to what it declares",
        );
    }
    flow(&mut l, p);
    facts_and_infos(&mut l, p, ix, game);
    setups(&mut l, ast, p, ix);
    choices(&mut l, p, ix, game);
    barks(&mut l, ast, p);
    for (k, (kind, target, _, a)) in ast.allows.iter().enumerate() {
        if !l.used_allows.contains(&k) {
            l.out.push(Diag::warning(
                "story-warning",
                *a,
                format!("`allow {kind} {target}` allows nothing: there is no such warning about it"),
                "remove it (an allow stays only while it is needed)",
            ));
        }
    }
    l.out
}

fn flow(l: &mut Lints, p: &Package) {
    for c in &p.conversations {
        let n = c.len as usize;
        let local = |x: Option<Next>, i: usize| -> usize {
            match x {
                Some(Next::Line(t)) => (t - c.first) as usize,
                Some(Next::End) => n,
                None => i + 1,
            }
        };
        let line = |i: usize| &p.lines[c.first as usize + i];
        let skippable = |i: usize| line(i).cond.is_some() || !line(i).reveals.is_empty();
        // Where each line can go: answered (its choices, or moving on), or passed over.
        let edges = |i: usize| -> Vec<usize> {
            let li = line(i);
            let mut out: Vec<usize> = Vec::new();
            if li.choices.is_empty() {
                out.push(local(li.goto, i));
            } else {
                for ch in &li.choices {
                    out.push(local(ch.goto, i));
                    if ch.refused.is_some() {
                        out.push(local(ch.refused, i));
                    }
                }
                if li.goto.is_some() {
                    out.push(local(li.goto, i));
                }
            }
            if skippable(i) {
                out.push(local(li.skip, i));
            }
            out
        };
        let mut seen = vec![false; n + 1];
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            if i > n || seen[i] {
                continue;
            }
            seen[i] = true;
            if i < n {
                stack.extend(edges(i));
            }
        }
        if !seen[n] {
            l.error(
                "story-flow",
                at(c.at),
                format!("conversation `{}` can never end: no way through it reaches `-> end`", c.id),
                "give some path a `-> end` (a goodbye choice)",
            );
        }
        // Unreached nodes: a line after a goto or a choice line that nothing names.
        for i in 0..n {
            let starts_node = i == 0 || !line(i - 1).choices.is_empty() || line(i - 1).goto.is_some();
            if starts_node && !seen[i] {
                let id = line(i).id.clone();
                l.warn("unreached", &id, at(line(i).at), format!("`{id}` is never reached: nothing leads to its node"), "jump to it, or remove it");
            }
        }
        // Loops that never wait: lines that can each be passed over, leading back to themselves.
        for start in 0..n {
            let mut i = start;
            let mut path = vec![start];
            while i < n && skippable(i) {
                i = local(line(i).skip, i);
                if i == start {
                    let ids: Vec<String> = path.iter().map(|k| line(*k).id.clone()).collect();
                    l.error(
                        "story-flow",
                        at(line(start).at),
                        format!("{} can all be passed over and lead back: a loop that may never wait for the player", ids.join(" -> ")),
                        "give one of them no condition",
                    );
                    break;
                }
                if path.contains(&i) || path.len() > n {
                    break;
                }
                path.push(i);
            }
        }
        for i in 0..n {
            let li = line(i);
            if !li.choices.is_empty() && li.choices.iter().all(|ch| ch.cond.is_some()) && li.goto.is_none() {
                let id = li.id.clone();
                l.warn(
                    "fallback",
                    &id,
                    at(li.at),
                    format!("every choice of `{id}` has a condition: when none holds the conversation closes"),
                    "add one choice without a condition, or `otherwise -> <node>`",
                );
            }
            if !li.choices.is_empty() && skippable(i) {
                let id = li.id.clone();
                l.warn(
                    "fallback",
                    &id,
                    at(li.at),
                    format!("`{id}` offers choices but can be passed over (it has a condition or reveals): then its choices are never offered"),
                    "carry the choices on a line without a condition (a short prompt), and put conditions on the choices",
                );
            }
            for ch in &li.choices {
                let runs = li.effects.iter().chain(&ch.effects).any(|e| matches!(e, Effect::Run { .. }));
                if ch.refused.is_some() && !runs {
                    let id = format!("{}/{}", c.id, ch.id);
                    l.warn("else", &id, at(ch.at), format!("`{id}` has an `else`, but runs no command the game could refuse"), "drop the `else`");
                }
            }
        }
        if !(c.first..c.first + c.len).any(|li| p.lines[li as usize].speaker == Some(c.with)) {
            l.warn("unreached", &c.id, at(c.at), format!("conversation `{}` is with {}, who says nothing in it", c.id, p.speakers[c.with as usize].id), "");
        }
    }
}

fn facts_and_infos(l: &mut Lints, p: &Package, ix: &Index, game: &Catalog) {
    for (f, fact) in p.facts.iter().enumerate() {
        let f = f as u32;
        let var = p.vars.iter().position(|v| *v == Var::Fact { fact: f }).map(|k| k as u32);
        let readers = var.and_then(|k| ix.readers.get(&k)).map_or(0, Vec::len);
        let shown = l.ast.facts.iter().any(|d| d.id == fact.id && d.shown);
        let writes = ix.writers.get(&f).map_or(0, Vec::len);
        let read = readers > 0 || shown || consumed(p, game, f);
        match (writes > 0, read) {
            (true, false) => l.warn(
                "unread",
                &fact.id,
                at(fact.at),
                format!("`{}` is written ({writes} place(s)) but nothing reads it: it changes nothing a player can see", fact.id),
                "read it in a condition or a line, or mark it `shown`",
            ),
            (false, true) if fact.scope != crate::sim::story::package::Scope::Profile => l.warn(
                "unwritten",
                &fact.id,
                at(fact.at),
                format!("`{}` is read but never changes (always {})", fact.id, fact.default),
                "set it somewhere, or use the value",
            ),
            (false, false) => l.warn("unused", &fact.id, at(fact.at), format!("`{}` is declared and never used", fact.id), "remove it"),
            _ => {}
        }
    }
    // A speaker can only reveal what they could know: their sheet, a `learn` that reaches them, or hearing someone else reveal it.
    for li in &p.lines {
        let Some(s) = li.speaker else { continue };
        let sp = &p.speakers[s as usize];
        for i in &li.reveals {
            let heard_it = p.lines.iter().any(|o| o.reveals.contains(i) && o.speaker != Some(s));
            if !(sp.knows.contains(i) || heard_it || learns_by_effect(p, sp.holder, *i)) {
                l.error(
                    "story-flow",
                    at(li.at),
                    format!(
                        "{} reveals {} in `{}`, but can never know it (not on their sheet, never told, never taught)",
                        sp.id, p.infos[*i as usize].id, li.id
                    ),
                    "add it to their `knows:`, teach it (`do learn`), or make the line a `claims`",
                );
            }
        }
    }
    for (i, info) in p.infos.iter().enumerate() {
        let i = i as u16;
        let asked = p.vars.iter().enumerate().any(|(k, v)| {
            matches!(v, Var::Knows { info, .. } | Var::Believes { info, .. } if *info == i) && ix.readers.get(&(k as u32)).is_some_and(|r| !r.is_empty())
        });
        let told = ix.tellers.get(&i).is_some_and(|t| !t.is_empty());
        if asked && !told {
            l.warn(
                "untold",
                &info.id,
                at(info.at),
                format!("conditions ask who knows `{}`, but nothing ever tells it", info.id),
                "reveal it in a line, or teach it",
            );
        } else if !asked && !told {
            l.warn("unused", &info.id, at(info.at), format!("`{}` is declared and never used", info.id), "remove it");
        }
    }
}

fn learns_by_effect(p: &Package, holder: u16, info: u16) -> bool {
    let hit = |e: &Effect| matches!(e, Effect::Learn { who, info: i, .. } if *i == info && match who { Who::Holder(h) => *h == holder, Who::Present => holder > 0, Who::Everyone => true });
    p.lines.iter().any(|l| l.effects.iter().any(hit) || l.choices.iter().any(|c| c.effects.iter().any(hit)))
        || p.directors.iter().any(|d| d.interventions.iter().any(|i| i.effects.iter().any(hit)))
}

fn setups(l: &mut Lints, ast: &Ast, p: &Package, ix: &Index) {
    let known = |r: &str| -> bool {
        ix.line_ids.contains_key(r)
            || p.barks.iter().any(|b| b.id == r)
            || r.rsplit_once('/').is_some_and(|(conv, ch)| {
                p.conversations
                    .iter()
                    .filter(|c| c.id == conv)
                    .any(|c| (c.first..c.first + c.len).any(|li| p.lines[li as usize].choices.iter().any(|x| x.id == ch)))
            })
    };
    for s in &ast.setups {
        for r in s.by.iter().chain(s.pays.iter().map(|(_, r)| r)) {
            if !known(&r.text) {
                l.error(
                    "story-reference",
                    r.at,
                    format!("setup `{}`: `{}` is no line, choice or bark", s.id, r.text),
                    "name a line (`<conversation>/<id>`), a choice (`<conversation>/<choice>`) or a bark",
                );
            }
        }
        if s.by.is_empty() {
            l.error("story-reference", s.at, format!("setup `{}` does not say where it is established", s.id), "add `by <line or choice>`");
        }
        if s.pays.is_empty() {
            l.warn(
                "unpaid",
                &s.id,
                s.at,
                format!("setup `{}` ({}) is never acknowledged, resolved, complicated or deliberately left", s.id, s.doc),
                "add `pays <kind> <line, choice or bark>` (or `pays leave ...` to leave it open on purpose)",
            );
        }
    }
}

fn choices(l: &mut Lints, p: &Package, ix: &Index, game: &Catalog) {
    for (ci, li, k, traced, why) in trace_stakes(p, ix, game) {
        let ch = &p.lines[li as usize].choices[k];
        let id = format!("{}/{}", p.conversations[ci as usize].id, ch.id);
        if ix.declared.contains(&(li, k)) && ch.stakes != traced {
            let d = format!("{:?}", ch.stakes).to_lowercase();
            l.warn("stakes", &id, at(ch.at), format!("`{id}` is declared {d}, but it {why}"), "change what it does, or the kind it declares");
        }
    }
    for li in &p.lines {
        for (a, x) in li.choices.iter().enumerate() {
            for y in &li.choices[a + 1..] {
                let same = x.goto == y.goto && format!("{:?}", x.effects) == format!("{:?}", y.effects) && x.refused == y.refused;
                if same && x.stakes != Stakes::Expressive && y.stakes != Stakes::Expressive {
                    let id = format!("{}/{}", li.id.split('/').next().unwrap_or(""), y.id);
                    l.warn(
                        "equivalent",
                        &id,
                        at(y.at),
                        format!("`{}` and `{}` do the same and go to the same place", x.id, y.id),
                        "make them differ, or declare them `kind expressive` (a difference of voice is fine)",
                    );
                }
            }
        }
    }
}

fn barks(l: &mut Lints, ast: &Ast, p: &Package) {
    let max = ast.header.max_line.unwrap_or(200);
    for li in &p.lines {
        let len: usize = li.text.iter().map(|x| if let crate::sim::story::package::Piece::Text(t) = x { t.chars().count() } else { 3 }).sum();
        if len > max {
            l.warn("length", &li.id, at(li.at), format!("`{}` is {len} characters (the story's limit is {max})", li.id), "split it, or raise `style max_line`");
        }
    }
    let mut by_text: BTreeMap<(Option<u16>, String), &str> = BTreeMap::new();
    for b in &p.barks {
        let Some(li) = b.line else { continue };
        let line = &p.lines[li as usize];
        let text = format!("{:?}", line.text);
        if let Some(first) = by_text.insert((line.speaker, text), &b.id) {
            l.warn("repeat", &b.id, at(b.at), format!("bark `{}` says exactly what `{first}` says", b.id), "vary the words, or drop one");
        }
    }
    let mut events: BTreeMap<String, Vec<&crate::sim::story::package::Bark>> = BTreeMap::new();
    for b in p.barks.iter().filter(|b| !b.notice) {
        if let Some(t) = &b.on {
            events.entry(t.event.to_string()).or_default().push(b);
        }
    }
    for (ev, list) in events {
        let unconditional = list.iter().any(|b| b.cond.is_none() && !b.once);
        if !unconditional {
            let first = list[0];
            l.warn(
                "fallback",
                &ev,
                at(first.at),
                format!("every bark on `{ev}` has a condition or is `once`: some times it happens, nothing is said"),
                "fine if meant: add a `silence` bark to say so",
            );
        } else if list.len() == 1 && list[0].cooldown == 0 && list[0].category.is_none() && list[0].line.is_some() {
            let b = list[0];
            l.warn(
                "repeat",
                &b.id,
                at(b.at),
                format!("`{}` is the only bark on `{ev}`, with no cooldown: the same line every time it happens", b.id),
                "add a variant, a cooldown, or `once`",
            );
        }
    }
}
