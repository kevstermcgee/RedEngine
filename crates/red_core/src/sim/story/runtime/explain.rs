//! A story explained with live values (`sim --inspect <story>:<id>`, `sim --inspect story`): the story itself, a conversation, a bark, a
//! director, a fact, an info, a speaker; each with its status, why it is blocked now, and where it was written.

use super::*;

impl Story {
    /// `target` explained with live values: the story itself (`<story>:story`), a conversation, a bark, a director, a fact, an info, a
    /// speaker. `None` for an id it does not have.
    pub fn explain(&self, target: &str, ask: &dyn Ask) -> Option<Value> {
        let p = &self.lp.package;
        let id = Id::parse(target).ok().filter(|i| i.namespace() == p.story)?;
        let vals = match self.values(ask) {
            Ok(v) => v,
            Err(e) => return Some(json!({"id": target, "status": "unanswerable", "blocked_by": e})),
        };
        let name = id.name();
        let at = |s| p.place(s);
        if id == self.me {
            let view = self.view(ask).ok();
            return Some(json!({
                "id": target,
                "status": if self.st.talk.is_some() { "talking" } else if self.st.playing.is_some() { "barking" } else { "idle" },
                "revision": self.st.revision,
                "round": self.st.round,
                "content": {"story": p.story, "version": p.content_version, "hash": p.source_hash},
                "view": view,
                "queue": self.st.queue.iter().map(|q| &p.barks[q.bark as usize].id).collect::<Vec<_>>(),
                "facts": p.facts.iter().zip(&self.st.facts).map(|(f, v)| (f.id.clone(), json!(v))).collect::<serde_json::Map<_, _>>(),
                "reads": p.vars.iter().zip(&vals).filter_map(|(v, x)| match v { Var::Query { id, .. } => Some((id.to_string(), json!(x))), _ => None }).collect::<serde_json::Map<_, _>>(),
                "heard": p.lines.iter().zip(&self.st.heard).filter(|(_, n)| **n > 0).map(|(l, n)| (l.id.clone(), json!(n))).collect::<serde_json::Map<_, _>>(),
                "chosen": p.barks.iter().zip(&self.st.chosen).filter(|(_, n)| **n > 0).map(|(b, n)| (b.id.clone(), json!(n))).collect::<serde_json::Map<_, _>>(),
                "ended": p.conversations.iter().zip(&self.st.ended).filter(|(_, e)| **e).map(|(c, _)| &c.id).collect::<Vec<_>>(),
                "known": self.st.known.iter().map(|((h, i), k)| json!({"who": p.holders[*h as usize], "info": p.infos[*i as usize].id, "knows": k.knows, "from": source_text(&k.source, p), "tick": k.tick})).collect::<Vec<_>>(),
                "log": self.log.iter().collect::<Vec<_>>(),
                "tally": self.tally,
                "problems": self.problems,
                "content": {
                    "lines": p.lines.iter().filter(|l| !p.barks.iter().any(|b| b.line.is_some_and(|x| p.lines[x as usize].id == l.id))).map(|l| &l.id).collect::<Vec<_>>(),
                    "barks": p.barks.iter().map(|b| &b.id).collect::<Vec<_>>(),
                    "choices": p.conversations.iter().flat_map(|c| (c.first..c.first + c.len).flat_map(move |li| p.lines[li as usize].choices.iter().map(move |ch| format!("{}/{}", c.id, ch.id)))).collect::<Vec<_>>(),
                },
            }));
        }
        if let Some(ci) = p.conversations.iter().position(|c| c.id == name) {
            let c = &p.conversations[ci];
            let open = self.st.talk.as_ref().is_some_and(|t| t.conv as usize == ci);
            let blocked = self.conv_blocked(ci as u32, &vals);
            let status = if open {
                "open"
            } else if blocked.is_some() {
                "blocked"
            } else {
                "available"
            };
            return Some(json!({"id": target, "kind": "conversation", "status": status, "blocked_by": blocked, "with": p.speakers[c.with as usize].id,
                "defined_at": at(c.at), "ended": self.st.ended[ci],
                "lines": (c.first..c.first + c.len).map(|l| json!({"id": p.lines[l as usize].id, "heard": self.st.heard[l as usize], "blocked_by": self.line_blocked(l, &vals)})).collect::<Vec<_>>()}));
        }
        if let Some(bi) = p.barks.iter().position(|b| b.id == name) {
            let b = &p.barks[bi];
            let blocked = self.bark_blocked(bi as u32, &vals, ask.now());
            return Some(json!({"id": target, "kind": "bark", "status": if blocked.is_some() { "blocked" } else { "eligible" }, "blocked_by": blocked,
                "on": b.on.as_ref().map(|t| t.event.to_string()), "chosen": self.st.chosen[bi], "heard": b.line.map(|l| self.st.heard[l as usize]),
                "last": self.st.last[bi], "defined_at": at(b.at)}));
        }
        if let Some(di) = p.directors.iter().position(|d| d.id == name) {
            let d = &p.directors[di];
            let st = &self.st.directing[di];
            return Some(json!({"id": target, "kind": "director", "next": st.next, "recover_until": st.recover_until, "this_round": st.this_round,
                "tension": d.tension.map(|e| self.lp.exprs[e as usize].eval(&vals)),
                "interventions": d.interventions.iter().enumerate().map(|(k, i)| json!({"id": i.id, "used": st.used.contains(&(k as u32)),
                    "blocked_by": i.cond.filter(|e| !self.lp.exprs[*e as usize].truthy(&vals)).map(|e| self.why_not(e, &vals))})).collect::<Vec<_>>(),
                "outcomes": self.log.iter().filter(|l| l.what == format!("director {}", d.id)).map(|l| json!({"tick": l.tick, "outcome": l.outcome})).collect::<Vec<_>>(),
                "defined_at": at(d.at)}));
        }
        if let Some(fi) = p.facts.iter().position(|f| f.id == name) {
            let f = &p.facts[fi];
            return Some(
                json!({"id": target, "kind": "fact", "value": self.st.facts[fi], "type": f.ty, "scope": f.scope, "means": f.doc, "defined_at": at(f.at)}),
            );
        }
        if let Some(ii) = p.infos.iter().position(|i| i.id == name) {
            let i = &p.infos[ii];
            let who: Vec<Value> = self
                .st
                .known
                .iter()
                .filter(|((_, x), _)| *x as usize == ii)
                .map(|((h, _), k)| json!({"who": p.holders[*h as usize], "knows": k.knows, "from": source_text(&k.source, p), "tick": k.tick}))
                .collect();
            return Some(
                json!({"id": target, "kind": "info", "means": i.doc, "true": i.truth.map(|e| self.lp.exprs[e as usize].truthy(&vals)), "known_by": who, "defined_at": at(i.at)}),
            );
        }
        if let Some(si) = p.speakers.iter().position(|s| s.id == name) {
            let s = &p.speakers[si];
            return Some(json!({"id": target, "kind": "speaker", "name": s.name, "present": self.present(si as u16, &vals),
                "knows": self.st.known.iter().filter(|((h, _), _)| *h == s.holder).map(|((_, i), k)| json!({"info": p.infos[*i as usize].id, "knows": k.knows, "from": source_text(&k.source, p)})).collect::<Vec<_>>(),
                "quiet_until": self.st.speaker_quiet[si].0, "defined_at": at(s.at)}));
        }
        None
    }
}

fn source_text(s: &Source, p: &crate::sim::story::package::Package) -> String {
    match s {
        Source::Sheet => "their character sheet".into(),
        Source::Line(l) => format!("hearing {}", p.lines[*l as usize].id),
        Source::Effect(at) => format!("`learn` in {at}"),
    }
}
