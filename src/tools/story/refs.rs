//! Questions about a compiled story, answered from its package and the compiler's index: who reads this fact, what can this choice
//! change, why might this conversation be closed, what breaks if this line goes (`story refs`); and the setup/payoff ledger with every
//! choice's traced stakes (`story ledger`). Static answers: the live "why is it blocked right now" is `sim --inspect <story>:<id>`.

use super::{lint, Built};
use crate::sim::story::package::{Effect, Package, Var};
use serde_json::{json, Value};

fn place(b: &Built, a: super::parse::At) -> String {
    format!("{}:{}", b.ast.files.get(a.file).map_or("?", String::as_str), a.line)
}

fn readers(b: &Built, v: Option<usize>) -> Vec<Value> {
    v.and_then(|k| b.index.readers.get(&(k as u32))).into_iter().flatten().map(|(w, a)| json!({"by": w, "at": place(b, *a)})).collect()
}

fn src(p: &Package, s: crate::sim::story::package::Src) -> String {
    p.place(s)
}

/// Where `id` appears outside the story's own sources: the scene (its `checks.sim` expectations) and the folder's other JSON (walks), as
/// `file:line: text`. A rename must repair these too; they are found by name, so a match may be a coincidence.
fn mentioned_outside(b: &Built, id: &str) -> Vec<String> {
    let word = |line: &str| {
        line.match_indices(id).any(|(at, _)| {
            let before = line[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric() && c != '_');
            let after = line[at + id.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '_');
            before && after
        })
    };
    let mut out = Vec::new();
    for f in &b.outside {
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        for (n, line) in text.lines().enumerate().filter(|(_, l)| word(l)) {
            out.push(format!("{}:{}: {}", f.display(), n + 1, line.trim().chars().take(140).collect::<String>()));
        }
    }
    out
}

/// What `id` is and everything that depends on it, or what it depends on.
pub fn refs(b: &Built, id: &str) -> Result<Value, String> {
    let mut v = refs_in_story(b, id)?;
    if let Some(o) = v.as_object_mut() {
        o.insert("mentioned_outside_the_story".into(), json!(mentioned_outside(b, id)));
    }
    Ok(v)
}

fn refs_in_story(b: &Built, id: &str) -> Result<Value, String> {
    let p = b.package.as_ref().ok_or("the story has errors: `story check` first")?;
    let var_of = |v: &Var| p.vars.iter().position(|x| x == v);
    if let Some(f) = p.facts.iter().position(|x| x.id == id) {
        let fact = &p.facts[f];
        let writers: Vec<Value> = b.index.writers.get(&(f as u32)).into_iter().flatten().map(|(w, a)| json!({"by": w, "at": place(b, *a)})).collect();
        let game = format!("{}:{}", p.story, fact.id);
        return Ok(json!({"id": id, "kind": "fact", "means": fact.doc, "type": fact.ty, "scope": fact.scope, "defined_at": src(p, fact.at),
            "read_by": readers(b, var_of(&Var::Fact { fact: f as u32 })), "written_by": writers,
            "read_by_the_game": b.catalog.consumers.iter().any(|c| c.as_str() == game)}));
    }
    if id.contains(':') && !id.contains('/') {
        let v = p.vars.iter().position(|v| matches!(v, Var::Query { id: q, .. } if q.as_str() == id));
        let c = &b.catalog;
        let decl =
            match (c.queries.iter().find(|q| q.id.as_str() == id), c.commands.iter().any(|x| x.as_str() == id), c.events.iter().any(|x| x.as_str() == id)) {
                (Some(q), _, _) => Some(json!({"as": "query", "type": q.ty, "means": q.doc})),
                (None, true, _) => Some(json!({"as": "command", "unattended": c.unattended.iter().any(|x| x.as_str() == id), "means": b.notes.get(id)})),
                (None, false, true) => Some(json!({"as": "event", "means": b.notes.get(id)})),
                _ if id.starts_with("scene:") && c.scene_vars.iter().any(|s| Some(s.as_str()) == id.strip_prefix("scene:")) => {
                    Some(json!({"as": "scene variable"}))
                }
                _ => None,
            };
        let is_run = |e: &Effect| matches!(e, Effect::Run { command, .. } if command.as_str() == id);
        let mut runs: Vec<String> =
            p.lines.iter().filter(|l| l.effects.iter().chain(l.choices.iter().flat_map(|c| &c.effects)).any(is_run)).map(|l| l.id.clone()).collect();
        runs.extend(
            p.directors.iter().flat_map(|d| d.interventions.iter().filter(|i| i.effects.iter().any(is_run)).map(move |i| format!("{}/{}", d.id, i.id))),
        );
        let barks: Vec<&str> = p.barks.iter().filter(|x| x.on.as_ref().is_some_and(|t| t.event.as_str() == id)).map(|x| x.id.as_str()).collect();
        if v.is_none() && runs.is_empty() && barks.is_empty() && decl.is_none() {
            return Err(format!("`{id}`: the story neither reads, runs nor answers it, and the game's bindings do not declare it"));
        }
        return Ok(json!({"id": id, "kind": "binding", "declared_by_the_game": decl, "read_by": readers(b, v), "run_by": runs, "answered_by_barks": barks}));
    }
    if let Some(i) = p.infos.iter().position(|x| x.id == id) {
        let info = &p.infos[i];
        let told: Vec<Value> = b.index.tellers.get(&(i as u16)).into_iter().flatten().map(|(w, a)| json!({"by": w, "at": place(b, *a)})).collect();
        let mut asked = Vec::new();
        for (k, v) in p.vars.iter().enumerate() {
            if let Var::Knows { holder, info } | Var::Believes { holder, info } = v {
                if *info as usize == i {
                    for r in readers(b, Some(k)) {
                        asked.push(json!({"who": p.holders[*holder as usize], "knows_or_believes": matches!(v, Var::Knows { .. }), "asked_by": r}));
                    }
                }
            }
        }
        let at_start: Vec<&str> = p.speakers.iter().filter(|s| s.knows.contains(&(i as u16))).map(|s| s.id.as_str()).collect();
        return Ok(json!({"id": id, "kind": "info", "means": info.doc, "true_when": info.truth.map(|e| &p.authored[e as usize]),
            "known_at_start_by": at_start, "told_by": told, "asked_by": asked, "defined_at": src(p, info.at)}));
    }
    if let Some(c) = p.conversations.iter().position(|x| x.id == id) {
        let conv = &p.conversations[c];
        let opens: Vec<String> = p
            .lines
            .iter()
            .flat_map(|l| l.effects.iter().chain(l.choices.iter().flat_map(|c| &c.effects)).map(move |e| (l, e)))
            .filter(|(_, e)| matches!(e, Effect::Start { conversation } if *conversation as usize == c))
            .map(|(l, _)| l.id.clone())
            .chain(p.directors.iter().flat_map(|d| {
                d.interventions
                    .iter()
                    .filter(|i| i.effects.iter().any(|e| matches!(e, Effect::Start { conversation } if *conversation as usize == c)))
                    .map(move |i| format!("{}/{}", d.id, i.id))
            }))
            .collect();
        let cond = conv.cond.map(|e| &p.authored[e as usize]);
        let speaker = &p.speakers[conv.with as usize];
        return Ok(json!({"id": id, "kind": "conversation", "with": speaker.id, "defined_at": src(p, conv.at),
            "opens_when": {"talked_to": speaker.id, "they_are_present": speaker.present.map(|v| var_name(p, v as usize)), "condition": cond,
                "it_has_not_ended": !conv.repeat, "or_started_by": opens},
            "condition_reads": conv.cond.map(|e| reads_of(b, p, e)).unwrap_or_default(),
            "lines": (conv.first..conv.first + conv.len).map(|l| &p.lines[l as usize].id).collect::<Vec<_>>(),
            "live": format!("sim --inspect {}:{id}", p.story)}));
    }
    if let Some(&li) = b.index.line_ids.get(id) {
        let line = &p.lines[li as usize];
        let heard = readers(b, var_of(&Var::Heard { line: li }));
        let setups: Vec<Value> = b
            .ast
            .setups
            .iter()
            .flat_map(|s| {
                s.by.iter()
                    .map(|r| ("by", r))
                    .chain(s.pays.iter().map(|(k, r)| (k.as_str(), r)))
                    .filter(|(_, r)| r.text == id)
                    .map(move |(k, _)| json!({"setup": s.id, "as": k}))
            })
            .collect();
        let jumps: Vec<String> = jumps_to(p, li);
        let mut breaks = Vec::new();
        if !heard.is_empty() {
            breaks.push(format!("{} condition(s) ask `heard({id})`", heard.len()));
        }
        if !setups.is_empty() {
            breaks.push(format!("{} setup reference(s)", setups.len()));
        }
        if !jumps.is_empty() {
            breaks.push(format!("it starts a node {} jump(s) go to: they would land on the next line", jumps.len()));
        }
        if !line.reveals.is_empty() {
            breaks.push(format!("it is where {} is told", line.reveals.iter().map(|i| p.infos[*i as usize].id.as_str()).collect::<Vec<_>>().join(", ")));
        }
        return Ok(json!({"id": id, "kind": "line", "speaker": line.speaker.map(|s| &p.speakers[s as usize].id), "defined_at": src(p, line.at),
            "heard_by_conditions": heard, "setups": setups, "jumped_to_from": jumps, "removing_it_breaks": breaks}));
    }
    if let Some((conv, choice)) = id.rsplit_once('/') {
        for (ci, li, k, stakes, why) in lint::trace_stakes(p, &b.index, &b.catalog) {
            let ch = &p.lines[li as usize].choices[k];
            if p.conversations[ci as usize].id != conv || ch.id != choice {
                continue;
            }
            let mut downstream = Vec::new();
            for e in &ch.effects {
                if let Effect::Set { fact, .. } | Effect::Add { fact, .. } = e {
                    for r in readers(b, var_of(&Var::Fact { fact: *fact })) {
                        downstream.push(json!({"fact": p.facts[*fact as usize].id, "then": r}));
                    }
                }
            }
            return Ok(json!({"id": id, "kind": "choice", "stakes": stakes, "declared": b.index.declared.contains(&(li, k)), "why": why,
                "offered_when": ch.cond.map(|e| &p.authored[e as usize]), "effects": ch.effects.iter().map(|e| effect_text(p, e)).collect::<Vec<_>>(),
                "what_reads_what_it_changes": downstream, "goes_to": ch.goto.map(|g| next_text(p, g)), "if_the_game_refuses": ch.refused.map(|g| next_text(p, g)),
                "defined_at": src(p, ch.at)}));
        }
    }
    if let Some(bk) = p.barks.iter().find(|x| x.id == id) {
        return Ok(json!({"id": id, "kind": "bark", "on": bk.on.as_ref().map(|t| t.event.to_string()), "priority": bk.priority, "specificity": bk.specificity,
            "condition": bk.cond.map(|e| &p.authored[e as usize]), "condition_reads": bk.cond.map(|e| reads_of(b, p, e)).unwrap_or_default(),
            "heard_by_conditions": bk.line.map(|l| readers(b, var_of(&Var::Heard { line: l }))).unwrap_or_default(), "defined_at": src(p, bk.at)}));
    }
    let mut known: Vec<String> = p
        .facts
        .iter()
        .map(|f| f.id.clone())
        .chain(p.infos.iter().map(|i| i.id.clone()))
        .chain(p.conversations.iter().map(|c| c.id.clone()))
        .chain(p.barks.iter().map(|b| b.id.clone()))
        .collect();
    known.extend(b.index.line_ids.keys().cloned());
    let hint = crate::sim::defs::closest(id, known.iter().map(String::as_str)).map(|h| format!(" (did you mean `{h}`?)")).unwrap_or_default();
    Err(format!("the story has nothing called `{id}`{hint}"))
}

fn var_name(p: &Package, k: usize) -> String {
    match &p.vars[k] {
        Var::Fact { fact } => p.facts[*fact as usize].id.clone(),
        Var::Query { id, .. } => id.to_string(),
        Var::Knows { holder, info } => format!("knows({}, {})", p.holders[*holder as usize], p.infos[*info as usize].id),
        Var::Believes { holder, info } => format!("believes({}, {})", p.holders[*holder as usize], p.infos[*info as usize].id),
        Var::Heard { line } => format!("heard({})", p.lines[*line as usize].id),
    }
}

/// What expression `e` reads, and for each story fact among them what writes it (how to make the condition hold).
fn reads_of(b: &Built, p: &Package, e: u32) -> Vec<Value> {
    let lowered = &p.exprs[e as usize];
    (0..p.vars.len())
        .filter(|k| lowered.split(|c: char| !c.is_alphanumeric()).any(|w| w == format!("v{k}")))
        .map(|k| {
            let written: Vec<String> = match &p.vars[k] {
                Var::Fact { fact } => b.index.writers.get(fact).into_iter().flatten().map(|(w, a)| format!("{w} ({})", place(b, *a))).collect(),
                Var::Knows { info, .. } | Var::Believes { info, .. } => {
                    b.index.tellers.get(info).into_iter().flatten().map(|(w, a)| format!("{w} ({})", place(b, *a))).collect()
                }
                _ => Vec::new(),
            };
            json!({"reads": var_name(p, k), "changed_by": written})
        })
        .collect()
}

fn jumps_to(p: &Package, li: u32) -> Vec<String> {
    let mut out = Vec::new();
    for l in &p.lines {
        if l.goto == Some(crate::sim::story::package::Next::Line(li)) {
            out.push(l.id.clone());
        }
        for c in &l.choices {
            if c.goto == Some(crate::sim::story::package::Next::Line(li)) || c.refused == Some(crate::sim::story::package::Next::Line(li)) {
                out.push(format!("{} -> {}", l.id, c.id));
            }
        }
    }
    out
}

fn next_text(p: &Package, n: crate::sim::story::package::Next) -> String {
    match n {
        crate::sim::story::package::Next::End => "end".into(),
        crate::sim::story::package::Next::Line(l) => p.lines[l as usize].id.clone(),
    }
}

fn effect_text(p: &Package, e: &Effect) -> String {
    match e {
        Effect::Set { fact, expr } => format!("set {} = {}", p.facts[*fact as usize].id, p.authored[*expr as usize]),
        Effect::Add { fact, expr } => format!("add {} {}", p.facts[*fact as usize].id, p.authored[*expr as usize]),
        Effect::Learn { who, info, knows } => format!("{} {:?} {}", if *knows { "learn" } else { "believe" }, who, p.infos[*info as usize].id),
        Effect::Run { command, subject, value } => {
            format!(
                "run {command}{}{}",
                subject.as_ref().map(|s| format!(" on {s}")).unwrap_or_default(),
                value.map(|v| format!(" {}", p.authored[v as usize])).unwrap_or_default()
            )
        }
        Effect::Bark { bark } => format!("bark {}", p.barks[*bark as usize].id),
        Effect::Start { conversation } => format!("start {}", p.conversations[*conversation as usize].id),
    }
}

/// The setup/payoff ledger, each choice's traced stakes, each fact's writers and readers, and who can know what.
pub fn ledger(b: &Built) -> Result<Value, String> {
    let p = b.package.as_ref().ok_or("the story has errors: `story check` first")?;
    let cond_of = |r: &str| -> Option<String> {
        if let Some(&li) = b.index.line_ids.get(r) {
            let line = &p.lines[li as usize];
            let bark = p.barks.iter().find(|x| x.line == Some(li));
            let conv = p.conversations.iter().find(|c| li >= c.first && li < c.first + c.len);
            let parts: Vec<String> =
                [line.cond, bark.and_then(|x| x.cond), conv.and_then(|c| c.cond)].into_iter().flatten().map(|e| p.authored[e as usize].clone()).collect();
            return Some(if parts.is_empty() { "always, once reached".into() } else { parts.join(" && ") });
        }
        let (conv, choice) = r.rsplit_once('/')?;
        let c = p.conversations.iter().find(|c| c.id == conv)?;
        let ch = (c.first..c.first + c.len).flat_map(|li| &p.lines[li as usize].choices).find(|x| x.id == choice)?;
        Some(ch.cond.map_or_else(|| "always, when its line is shown".to_string(), |e| p.authored[e as usize].clone()))
    };
    let setups: Vec<Value> = b
        .ast
        .setups
        .iter()
        .map(|s| {
            json!({"id": s.id, "what": s.doc, "established_by": s.by.iter().map(|r| json!({"at": r.text, "when": cond_of(&r.text)})).collect::<Vec<_>>(),
                "outcomes": s.pays.iter().map(|(k, r)| json!({"kind": k, "at": r.text, "available_when": cond_of(&r.text)})).collect::<Vec<_>>(),
                "status": if s.pays.is_empty() { "unpaid" } else if s.pays.iter().all(|(k, _)| k == "leave") { "left open on purpose" } else { "paid" }})
        })
        .collect();
    let choices: Vec<Value> = lint::trace_stakes(p, &b.index, &b.catalog)
        .into_iter()
        .map(|(ci, li, k, stakes, why)| {
            let ch = &p.lines[li as usize].choices[k];
            json!({"id": format!("{}/{}", p.conversations[ci as usize].id, ch.id), "stakes": ch.stakes, "traced": stakes, "declared": b.index.declared.contains(&(li, k)), "why": why})
        })
        .collect();
    let facts: Vec<Value> = p
        .facts
        .iter()
        .enumerate()
        .map(|(f, fact)| {
            let w = b.index.writers.get(&(f as u32)).map_or(0, Vec::len);
            let r = p.vars.iter().position(|v| *v == Var::Fact { fact: f as u32 }).and_then(|k| b.index.readers.get(&(k as u32))).map_or(0, Vec::len);
            json!({"id": fact.id, "scope": fact.scope, "writes": w, "reads": r})
        })
        .collect();
    let knowledge: Vec<Value> = p
        .infos
        .iter()
        .enumerate()
        .map(|(i, info)| {
            json!({"info": info.id, "true_when": info.truth.map(|e| &p.authored[e as usize]),
                "known_at_start_by": p.speakers.iter().filter(|s| s.knows.contains(&(i as u16))).map(|s| &s.id).collect::<Vec<_>>(),
                "told_by": b.index.tellers.get(&(i as u16)).into_iter().flatten().map(|(w, _)| w).collect::<Vec<_>>()})
        })
        .collect();
    Ok(json!({"story": p.story, "setups": setups, "choices": choices, "facts": facts, "knowledge": knowledge}))
}
