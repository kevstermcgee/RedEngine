//! Effects as written (`do set f = e; run tw:lend 2; learn present info`) lowered to the package's: each checked against what the story and the
//! game declare (a fact's type, a command the game takes, one a director may issue unattended).

use super::*;

impl Lower<'_> {
    /// The effects of `a`, written at `what`; `director`: only the game's unattended commands may run.
    pub(super) fn effects(&mut self, a: &Attrs, what: &str, director: bool) -> Vec<Effect> {
        let mut out = Vec::new();
        for e in &a.effects {
            let words: Vec<&str> = e.text.split_whitespace().collect();
            let at = e.at;
            let effect = match words.as_slice() {
                ["set", fact, "=", ..] | ["add", fact, ..] => {
                    let set = words[0] == "set";
                    let Some(&f) = self.facts.get(*fact) else {
                        self.unknown(at, "fact", fact, self.facts.keys().cloned().collect());
                        continue;
                    };
                    let skip = if set { 3 } else { 2 };
                    let value = Spanned { text: words[skip..].join(" "), at };
                    if value.text.is_empty() {
                        self.err(pkg::code::SYNTAX, at, format!("`{}`: what value?", e.text), "");
                        continue;
                    }
                    let Some((x, t)) = self.expr(&value, what) else { continue };
                    let ty = self.p.facts[f as usize].ty;
                    let fits = match (ty, set) {
                        (Ty::Bool, true) => t.boolish(),
                        (Ty::Bool, false) => false,
                        _ => t.intish(),
                    };
                    if !fits {
                        let want = if ty == Ty::Bool { "a yes/no" } else { "a whole number (floor() or round() a number)" };
                        let m = if ty == Ty::Bool && !set {
                            format!("`add` needs an int fact; `{fact}` is a yes/no (use `set`)")
                        } else {
                            format!("`{}` gives {}; `{fact}` takes {want}", value.text, t.name())
                        };
                        self.err(pkg::code::EXPRESSION, at, m, "");
                        continue;
                    }
                    self.ix.writers.entry(f).or_default().push((what.to_string(), at));
                    if set {
                        Effect::Set { fact: f, expr: x }
                    } else {
                        Effect::Add { fact: f, expr: x }
                    }
                }
                [verb @ ("learn" | "believe"), who, info] => {
                    let who = match *who {
                        "present" => Who::Present,
                        "everyone" => Who::Everyone,
                        h => match self.holder(h, at) {
                            Some(h) => Who::Holder(h),
                            None => continue,
                        },
                    };
                    let Some(i) = self.info(info, at) else { continue };
                    self.ix.tellers.entry(i).or_default().push((what.to_string(), at));
                    Effect::Learn { who, info: i, knows: *verb == "learn" }
                }
                ["run", command, rest @ ..] => {
                    let id = match Id::parse(command) {
                        Ok(id) => id,
                        Err(m) => {
                            self.err(pkg::code::BINDING, at, m, "");
                            continue;
                        }
                    };
                    if !self.game.commands.contains(&id) {
                        let known = self.game.commands.iter().map(Id::to_string).collect();
                        self.unknown(at, "command the game takes named", command, known);
                        continue;
                    }
                    if director && !self.game.unattended.contains(&id) {
                        self.err(
                            pkg::code::BINDING,
                            at,
                            format!("a director cannot run `{id}`: the game does not let it be issued unattended"),
                            "a director may only run the game's unattended commands",
                        );
                        continue;
                    }
                    let (subject, rest) = match rest {
                        ["on", s, more @ ..] => match Id::parse(s) {
                            Ok(s) => (Some(s), more),
                            Err(m) => {
                                self.err(pkg::code::BINDING, at, m, "");
                                continue;
                            }
                        },
                        more => (None, more),
                    };
                    let value = if rest.is_empty() {
                        None
                    } else {
                        let v = Spanned { text: rest.join(" "), at };
                        match self.expr(&v, what) {
                            Some((x, t)) if t.intish() => Some(x),
                            Some((_, t)) => {
                                self.err(pkg::code::EXPRESSION, at, format!("a command's value is a whole number; `{}` is {}", v.text, t.name()), "");
                                continue;
                            }
                            None => continue,
                        }
                    };
                    Effect::Run { command: id, subject, value }
                }
                ["bark", id] => match self.barks.get(*id) {
                    Some(b) => Effect::Bark { bark: *b },
                    None => {
                        self.unknown(at, "bark", id, self.barks.keys().cloned().collect());
                        continue;
                    }
                },
                ["start", id] => match self.convs.get(*id) {
                    Some(c) => Effect::Start { conversation: *c },
                    None => {
                        self.unknown(at, "conversation", id, self.convs.keys().cloned().collect());
                        continue;
                    }
                },
                _ => {
                    self.err(
                        pkg::code::SYNTAX,
                        at,
                        format!("`{}` is not an effect", e.text),
                        "set <fact> = <expr> | add <fact> <n> | learn|believe <who> <info> | run <command> [on <id>] [<n>] | bark <id> | start <conversation>",
                    );
                    continue;
                }
            };
            out.push(effect);
        }
        out
    }
}
