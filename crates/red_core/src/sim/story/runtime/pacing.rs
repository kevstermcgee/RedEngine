//! The story's pacing: barks (choosing, presenting, queueing, interrupting, hearing) and directors (evaluating, intervening). Part of the
//! story system (`super`); see its module docs for the order, the cooldowns and the queue.

use super::*;

impl Story {
    // ---- barks --------------------------------------------------------------------------------------------------------------------

    /// Why bark `bi` cannot be chosen now (`None`: it can).
    pub(super) fn bark_blocked(&self, bi: u32, vals: &[f64], now: u64) -> Option<String> {
        let p = &self.lp.package;
        let b = &p.barks[bi as usize];
        let heard = b.line.map_or(self.st.chosen[bi as usize], |l| self.st.heard[l as usize]);
        if b.once && heard > 0 {
            return Some("once, and already heard".into());
        }
        if self.st.playing.is_some_and(|p| p.bark == bi) || self.st.queue.iter().any(|q| q.bark == bi) {
            return Some("already being said (presented or queued)".into());
        }
        if let Some(t) = self.st.last[bi as usize].filter(|t| t + b.cooldown as u64 > now) {
            return Some(format!("cooling down until tick {}", t + b.cooldown as u64));
        }
        if let Some(c) = b.category.filter(|c| self.st.category_quiet[*c as usize] > now) {
            return Some(format!("category {} quiet until tick {}", p.categories[c as usize].id, self.st.category_quiet[c as usize]));
        }
        if let Some(e) = b.cond.filter(|e| !self.lp.exprs[*e as usize].truthy(vals)) {
            return Some(self.why_not(e, vals));
        }
        let l = b.line?;
        // A speaker's cooldown holds back their lines that are no more important than the one that started it.
        let quiet = |s: u16| self.st.speaker_quiet[s as usize];
        if let Some(s) = p.lines[l as usize].speaker.filter(|s| quiet(*s).0 > now && b.priority <= quiet(*s).1) {
            let (until, pr) = quiet(s);
            return Some(format!("{} spoke lately (quiet until tick {until} for priority {pr} and below)", p.speakers[s as usize].id));
        }
        self.line_blocked(l, vals)
    }

    /// Chooses among `candidates` (bark indices, in written order) and commits the best: see the module docs for the order.
    pub(super) fn offer(&mut self, candidates: &[u32], what: &str, vals: &[f64], cx: &mut Cx) {
        let now = cx.tick;
        let p = Arc::clone(&self.lp);
        let mut ok: Vec<u32> = Vec::new();
        let mut why = Vec::new();
        for &bi in candidates {
            match self.bark_blocked(bi, vals, now) {
                Some(r) => why.push(format!("{}: {r}", p.package.barks[bi as usize].id)),
                None => {
                    why.push(format!("{}: eligible", p.package.barks[bi as usize].id));
                    ok.push(bi);
                }
            }
        }
        let key = |bi: u32| {
            let b: &Bark = &p.package.barks[bi as usize];
            let heard = b.line.map_or(self.st.chosen[bi as usize], |l| self.st.heard[l as usize]);
            (b.priority, b.specificity, std::cmp::Reverse(heard))
        };
        let Some(best) = ok.iter().map(|b| key(*b)).max() else {
            self.note(now, what, "silence: no candidate", why);
            return;
        };
        let tied: Vec<u32> = ok.into_iter().filter(|b| key(*b) == best).collect();
        // The one commitment point that draws: only a real tie, only when one is being committed.
        let pick = if tied.len() > 1 { tied[cx.random(tied.len() as u32) as usize] } else { tied[0] };
        let b = &p.package.barks[pick as usize];
        self.st.chosen[pick as usize] += 1;
        self.st.last[pick as usize] = Some(now);
        if let Some(c) = b.category {
            self.st.category_quiet[c as usize] = now + p.package.categories[c as usize].cooldown as u64;
        }
        let Some(li) = b.line else {
            self.note(now, what, format!("silence: chose {} (a deliberate silence)", b.id), why);
            return;
        };
        if let Some(s) = p.package.lines[li as usize].speaker {
            self.st.speaker_quiet[s as usize] = (now + p.package.speakers[s as usize].cooldown as u64, b.priority);
        }
        let tie = if tied.len() > 1 { format!(" (drawn from {} tied)", tied.len()) } else { String::new() };
        let outcome = self.present_bark(pick, b.priority, now);
        self.note(now, what, format!("{outcome} {}{tie}", b.id), why);
    }

    /// Presents a chosen bark now, interrupting, suspending or queueing as its priority says; returns what it did.
    pub(super) fn present_bark(&mut self, bark: u32, priority: i32, now: u64) -> &'static str {
        let p = Arc::clone(&self.lp);
        if let Some(t) = &self.st.talk {
            let c = &p.package.conversations[t.conv as usize];
            if !t.suspended {
                if c.interrupt.is_some_and(|at| priority >= at) && self.st.playing.is_none() {
                    if let Some(t) = &mut self.st.talk {
                        t.suspended = true;
                    }
                    self.st.revision += 1;
                    self.play(bark, priority, now);
                    return "suspended the conversation for";
                }
                return self.enqueue(bark, priority, now);
            }
        }
        match self.st.playing {
            None => {
                self.play(bark, priority, now);
                "played"
            }
            Some(cur) if priority > cur.priority => {
                let id = p.package.barks[cur.bark as usize].id.clone();
                self.note(now, format!("bark {id}"), "interrupted: seen, not heard", vec![]);
                self.play(bark, priority, now);
                "interrupted the bark before for"
            }
            Some(_) => self.enqueue(bark, priority, now),
        }
    }

    pub(super) fn enqueue(&mut self, bark: u32, priority: i32, now: u64) -> &'static str {
        if self.st.queue.len() >= QUEUE_MAX {
            let lowest = self.st.queue.last().map_or(i32::MAX, |q| q.priority);
            if priority <= lowest {
                return "dropped (the queue is full)";
            }
            self.st.queue.pop();
        }
        let at = self.st.queue.iter().position(|q| q.priority < priority).unwrap_or(self.st.queue.len());
        self.st.queue.insert(at, Queued { bark, priority, chosen: now });
        "queued"
    }

    pub(super) fn play(&mut self, bark: u32, priority: i32, now: u64) {
        let p = &self.lp.package;
        let Some(li) = p.barks[bark as usize].line else { return };
        let text: String = p.lines[li as usize].text.iter().map(|x| if let Piece::Text(t) = x { t.as_str() } else { "00" }).collect();
        self.st.seen[li as usize] += 1;
        self.st.playing = Some(Playing { bark, started: now, ends: now + ticks_for(&text), priority });
        self.st.revision += 1;
    }

    /// The bark being presented is heard in full: its reveals and effects apply (its commands as one batch first).
    pub(super) fn heard_bark(&mut self, cx: &mut Cx) {
        let Some(cur) = self.st.playing.take() else { return };
        self.st.revision += 1;
        let p = Arc::clone(&self.lp);
        let b = &p.package.barks[cur.bark as usize];
        let Some(li) = b.line else { return };
        let Ok(vals) = self.values(cx) else { return };
        self.hear(li, &vals, cx.tick);
        let line = &p.package.lines[li as usize];
        let commands = self.commands_of(&line.effects, &vals);
        if commands.is_empty() {
            self.apply(&line.effects, &b.id, &vals, cx);
        } else {
            self.pending = Some(Pending::Heard { bark: cur.bark });
            if let Err(e) = cx.issue(cur.bark as u64, commands) {
                self.problems.push(e);
                self.pending = None;
            }
        }
        self.resume_or_next(cx);
    }

    /// After a bark: resume a conversation it suspended, else play the next queued bark that is still fresh and still eligible.
    pub(super) fn resume_or_next(&mut self, cx: &mut Cx) {
        if self.st.playing.is_some() {
            return;
        }
        if let Some(t) = &mut self.st.talk {
            if t.suspended {
                t.suspended = false;
                self.st.revision += 1;
                return;
            }
        }
        let Ok(vals) = self.values(cx) else { return };
        while !self.st.queue.is_empty() {
            let q = self.st.queue.remove(0);
            let id = self.lp.package.barks[q.bark as usize].id.clone();
            if cx.tick > q.chosen + QUEUE_TTL {
                self.note(cx.tick, format!("bark {id}"), "dropped: stale in the queue", vec![]);
                continue;
            }
            // Its cooldowns started when it was chosen: only what it says (its condition, its speaker) is checked again.
            let b = &self.lp.package.barks[q.bark as usize];
            let blocked = b
                .cond
                .filter(|e| !self.lp.exprs[*e as usize].truthy(&vals))
                .map(|e| self.why_not(e, &vals))
                .or_else(|| b.line.and_then(|l| self.line_blocked(l, &vals)));
            if let Some(why) = blocked {
                self.note(cx.tick, format!("bark {id}"), format!("dropped from the queue: {why}"), vec![]);
                continue;
            }
            if self.st.talk.is_some() {
                self.st.queue.insert(0, q);
                return;
            }
            self.play(q.bark, q.priority, cx.tick);
            self.note(cx.tick, format!("bark {id}"), "played from the queue", vec![]);
            return;
        }
    }

    // ---- directors ----------------------------------------------------------------------------------------------------------------

    pub(super) fn direct(&mut self, di: usize, vals: &[f64], cx: &mut Cx) {
        let p = Arc::clone(&self.lp);
        let d = &p.package.directors[di];
        let now = cx.tick;
        self.st.directing[di].next = now + d.every as u64;
        let what = format!("director {}", d.id);
        let tension = d.tension.map(|e| number(p.exprs[e as usize].eval(vals))).map(|t| format!(" (tension {t})")).unwrap_or_default();
        let st = &self.st.directing[di];
        let wait = if self.st.talk.is_some() {
            Some("waits: a conversation is open".to_string())
        } else if now < st.recover_until {
            Some(format!("recovering until tick {}", st.recover_until))
        } else if d.budget.is_some_and(|b| st.this_round >= b) {
            Some(format!("budget spent ({} this round)", st.this_round))
        } else {
            d.rest.iter().find(|e| p.exprs[**e as usize].truthy(vals)).map(|e| format!("rests: `{}` holds", p.package.authored[*e as usize]))
        };
        if let Some(w) = wait {
            self.note(now, what, format!("{w}{tension}"), vec![]);
            return;
        }
        let mut why = Vec::new();
        let mut exhausted = true;
        for (ii, i) in d.interventions.iter().enumerate() {
            let st = &self.st.directing[di];
            let blocked = if i.once && st.used.contains(&(ii as u32)) {
                Some("once, and used".to_string())
            } else if i.group.as_ref().is_some_and(|g| st.groups.contains(g)) {
                Some(format!("its group {} was used this round", i.group.as_deref().unwrap_or("")))
            } else {
                exhausted = false;
                i.cond.filter(|e| !p.exprs[*e as usize].truthy(vals)).map(|e| self.why_not(e, vals))
            };
            if let Some(b) = blocked {
                why.push(format!("{}: {b}", i.id));
                continue;
            }
            if let Some(chance) = i.chance {
                let rolled = cx.random(100);
                if rolled >= chance as u32 {
                    why.push(format!("{}: eligible", i.id));
                    self.note(now, what, format!("does nothing: {} rolled {rolled}, needs under {chance}{tension}", i.id), why);
                    return;
                }
            }
            why.push(format!("{}: eligible", i.id));
            let commands = self.commands_of(&i.effects, vals);
            self.note(now, what, format!("intervenes: {}{tension}", i.id), why);
            if commands.is_empty() {
                self.intervened(di, ii, vals, cx);
            } else {
                self.pending = Some(Pending::Intervene { director: di as u32, intervention: ii as u32 });
                if let Err(e) = cx.issue(ii as u64, commands) {
                    self.problems.push(e);
                    self.pending = None;
                }
            }
            return;
        }
        let outcome = if exhausted { "exhausted: every intervention is used up (the pool is spent, not endless)" } else { "nothing eligible" };
        self.note(now, what, format!("{outcome}{tension}"), why);
    }

    pub(super) fn intervened(&mut self, di: usize, ii: usize, vals: &[f64], cx: &mut Cx) {
        let p = Arc::clone(&self.lp);
        let d = &p.package.directors[di];
        let i = &d.interventions[ii];
        let st = &mut self.st.directing[di];
        st.this_round += 1;
        st.recover_until = cx.tick + d.recover as u64;
        if i.once {
            st.used.insert(ii as u32);
        }
        if let Some(g) = &i.group {
            st.groups.insert(g.clone());
        }
        let origin = format!("{}/{}", d.id, i.id);
        self.apply(&i.effects, &origin, vals, cx);
    }
}
