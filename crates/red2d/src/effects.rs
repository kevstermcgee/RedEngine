//! Effects: named, parameterised groups of actions (ADR 2026-10-07-shared-effects-for-2d-games).
//!
//! ```json
//! "effects": {
//!   "heal":   { "params": { "amount": 1 }, "do": [ { "add": ["lives", "$amount"] }, { "play": "pick" } ] },
//!   "pickup": { "params": ["points"],      "do": [ { "add": ["score", "$points"] }, { "play": "pick" }, { "destroy": "self" } ] }
//! },
//! "rules": [ { "id": "take_coin", "when": { "touch": ["player", "coin"] }, "do": [ { "apply": "pickup", "with": { "points": 1 } } ] },
//!            { "id": "bandage",   "when": { "press": "secondary" }, "if": "score >= 3", "do": [ { "add": ["score", -3] }, { "apply": "heal" } ] } ]
//! ```
//!
//! An effect is a definition, not a runtime concept: `{"apply": "heal", "with": {...}}` is replaced by the effect's actions *when the file is parsed*, and every expanded action goes through
//! the same validation a hand-written one does, so the simulation, its hash and every existing game are untouched. What this adds is one place to say what "heal" or "pick up" means, used
//! by items, abilities, buttons and other effects alike.
//!
//! Rules: `params` is a list of names (all required) or an object of name -> default; `"$name"` as a whole string is replaced by the argument with its JSON type, and inside a longer string
//! (an expression) by its text; a `$name` that is not a parameter is an error where the effect is defined; effects may apply other effects up to four deep, never in a cycle; an effect
//! nobody applies is an error (it proves and does nothing). Every error names the call site and, for an expanded action, the effect definition it came from.

use serde_json::{Map, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

/// How deep effects may apply effects.
const MAX_DEPTH: usize = 4;

/// One effect: its parameters (name, default) and the actions it stands for, unexpanded.
pub(crate) struct Effect {
    params: Vec<(String, Option<Value>)>,
    body: Vec<Value>,
}

/// The effects of a game, and which of them have been applied so far.
#[derive(Default)]
pub(crate) struct Effects {
    map: BTreeMap<String, Effect>,
    used: RefCell<BTreeSet<String>>,
}

fn ident_ok(s: &str) -> bool {
    let mut cs = s.chars();
    cs.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && cs.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn near(name: &str, all: impl Iterator<Item = String>) -> String {
    let all: Vec<String> = all.collect();
    let hint = crate::suggest::suggest(name, all.iter().map(String::as_str)).first().map(|n| format!(" — did you mean `{n}`?")).unwrap_or_default();
    format!("{hint} (known: {})", if all.is_empty() { "none".to_string() } else { all.join(", ") })
}

fn describe(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 40 {
        format!("{}…", &s[..s.floor_char_boundary(40)])
    } else {
        s
    }
}

/// The `$name` tokens of a string, in order.
fn tokens(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'$' && b.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_') {
            let start = i + 1;
            let mut j = start;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            out.push(&s[start..j]);
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Every `$name` in a value (strings only, at any depth), with where it was found.
fn collect_tokens<'a>(v: &'a Value, path: &str, out: &mut Vec<(String, &'a str)>) {
    match v {
        Value::String(s) => out.extend(tokens(s).into_iter().map(|t| (path.to_string(), t))),
        Value::Array(a) => a.iter().enumerate().for_each(|(i, x)| collect_tokens(x, &format!("{path}[{i}]"), out)),
        Value::Object(o) => o.iter().for_each(|(k, x)| collect_tokens(x, &format!("{path}.{k}"), out)),
        _ => {}
    }
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { "1" } else { "0" }.to_string(),
        other => other.to_string(),
    }
}

/// `v` with every `$param` replaced: a string that is exactly one `$param` becomes the argument itself (its JSON type kept); `$param` inside a longer string becomes the argument's text.
fn substitute(v: &Value, args: &BTreeMap<String, Value>) -> Value {
    match v {
        Value::String(s) => {
            if let Some(name) = s.strip_prefix('$') {
                if let Some(a) = args.get(name).filter(|_| ident_ok(name)) {
                    return a.clone();
                }
            }
            let mut out = s.clone();
            // Longest names first, so `$points2` is not eaten by `$points`.
            let mut names: Vec<&String> = args.keys().collect();
            names.sort_by_key(|n| std::cmp::Reverse(n.len()));
            for n in names {
                out = out.replace(&format!("${n}"), &text_of(&args[n]));
            }
            Value::String(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| substitute(x, args)).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), substitute(x, args))).collect()),
        other => other.clone(),
    }
}

impl Effects {
    /// Reads and validates the `effects` block of a game (absent = none). Problems go to `errs` as `path: message`.
    pub(crate) fn parse(root: &Map<String, Value>, errs: &mut Vec<String>) -> Effects {
        let mut fx = Effects::default();
        let Some(block) = root.get("effects") else { return fx };
        let Some(defs) = block.as_object() else {
            errs.push(format!("effects: expected an object of named effects, got {}", describe(block)));
            return fx;
        };
        for (name, def) in defs {
            let path = format!("effects.{name}");
            if !ident_ok(name) {
                errs.push(format!("{path}: an effect name is letters, digits and `_`, not starting with a digit"));
                continue;
            }
            let Some(o) = def.as_object() else {
                errs.push(format!("{path}: expected {{\"params\": ..., \"do\": [actions]}}, got {}", describe(def)));
                continue;
            };
            for k in o.keys().filter(|k| !matches!(k.as_str(), "params" | "do" | "about") && !k.starts_with("x-")) {
                errs.push(format!("{path}.{k}: not a field of an effect{}", near(k, ["params", "do", "about"].iter().map(|s| s.to_string()))));
            }
            let mut params: Vec<(String, Option<Value>)> = Vec::new();
            match o.get("params") {
                None => {}
                Some(Value::Array(list)) => {
                    for (i, p) in list.iter().enumerate() {
                        match p.as_str().filter(|s| ident_ok(s)) {
                            Some(s) => params.push((s.to_string(), None)),
                            None => errs.push(format!("{path}.params[{i}]: a parameter name is letters, digits and `_`, got {}", describe(p))),
                        }
                    }
                }
                Some(Value::Object(m)) => {
                    for (k, dflt) in m {
                        if ident_ok(k) {
                            params.push((k.clone(), Some(dflt.clone())));
                        } else {
                            errs.push(format!("{path}.params.{k}: a parameter name is letters, digits and `_`"));
                        }
                    }
                }
                Some(other) => errs.push(format!("{path}.params: expected a list of names or an object of name -> default, got {}", describe(other))),
            }
            let body = match o.get("do") {
                Some(Value::Array(list)) if !list.is_empty() => list.clone(),
                Some(Value::Array(_)) => {
                    errs.push(format!("{path}.do: empty: an effect that does nothing proves nothing"));
                    continue;
                }
                Some(other) => {
                    errs.push(format!("{path}.do: expected a list of actions, got {}", describe(other)));
                    continue;
                }
                None => {
                    errs.push(format!("{path}.do: missing: the actions the effect stands for"));
                    continue;
                }
            };
            // A `$name` that is not a parameter is a typo; say so where the effect is defined, not at some call site.
            let mut found = Vec::new();
            for (i, a) in body.iter().enumerate() {
                collect_tokens(a, &format!("{path}.do[{i}]"), &mut found);
            }
            for (at, tok) in found {
                if !params.iter().any(|(p, _)| p == tok) {
                    errs.push(format!("{at}: `${tok}` is not a parameter of effect `{name}`{}", near(tok, params.iter().map(|(p, _)| p.clone()))));
                }
            }
            fx.map.insert(name.clone(), Effect { params, body });
        }
        fx
    }

    /// The actions of a `do` list with every `apply` replaced by the effect's actions (recursively), each with the path to report errors at: the call site, then the effect definition
    /// the action came from. An `apply` that cannot be expanded is reported in `errs` and contributes nothing.
    pub(crate) fn flatten(&self, path: &str, list: &[Value], errs: &mut Vec<String>) -> Vec<(String, Value)> {
        let mut out = Vec::new();
        let mut stack = Vec::new();
        for (i, item) in list.iter().enumerate() {
            self.flatten_one(&format!("{path}[{i}]"), item, errs, &mut stack, &mut out);
        }
        out
    }

    fn flatten_one(&self, path: &str, item: &Value, errs: &mut Vec<String>, stack: &mut Vec<String>, out: &mut Vec<(String, Value)>) {
        let Some(o) = item.as_object().filter(|o| o.contains_key("apply")) else {
            out.push((path.to_string(), item.clone()));
            return;
        };
        let ap = format!("{path}.apply");
        if let Some(k) = o.keys().find(|k| !matches!(k.as_str(), "apply" | "with")) {
            errs.push(format!(
                "{path}.{k}: an `apply` action has only `apply` and `with`; this key does nothing{}",
                near(k, ["apply", "with"].iter().map(|s| s.to_string()))
            ));
            return;
        }
        let Some(name) = o["apply"].as_str() else {
            errs.push(format!("{ap}: expected the name of an effect, got {}", describe(&o["apply"])));
            return;
        };
        let Some(effect) = self.map.get(name) else {
            let hint = if self.map.is_empty() {
                " (this game defines no `effects`: add \"effects\": {...})".to_string()
            } else {
                near(name, self.map.keys().cloned())
            };
            errs.push(format!("{ap}: no effect `{name}`{hint}"));
            return;
        };
        self.used.borrow_mut().insert(name.to_string());
        if stack.iter().any(|s| s == name) {
            errs.push(format!("{ap}: effect `{name}` applies itself ({} -> {name}): effects may not form a cycle", stack.join(" -> ")));
            return;
        }
        if stack.len() >= MAX_DEPTH {
            errs.push(format!("{ap}: effects apply effects more than {MAX_DEPTH} deep ({} -> {name}): flatten them", stack.join(" -> ")));
            return;
        }
        // Arguments: every `with` key must be a parameter; every parameter without a default must be given.
        let given: Map<String, Value> = match o.get("with") {
            None => Map::new(),
            Some(Value::Object(m)) => m.clone(),
            Some(other) => {
                errs.push(format!("{path}.with: expected an object of arguments, got {}", describe(other)));
                return;
            }
        };
        let mut bad = false;
        for k in given.keys() {
            if !effect.params.iter().any(|(p, _)| p == k) {
                errs.push(format!("{path}.with.{k}: effect `{name}` has no parameter `{k}`{}", near(k, effect.params.iter().map(|(p, _)| p.clone()))));
                bad = true;
            }
        }
        let mut args = BTreeMap::new();
        for (p, dflt) in &effect.params {
            match given.get(p).or(dflt.as_ref()) {
                Some(v) => {
                    args.insert(p.clone(), v.clone());
                }
                None => {
                    errs.push(format!("{path}: effect `{name}` needs the parameter `{p}` (effects.{name}.params): apply it with {{\"apply\": \"{name}\", \"with\": {{\"{p}\": ...}}}}"));
                    bad = true;
                }
            }
        }
        if bad {
            return;
        }
        stack.push(name.to_string());
        for (j, action) in effect.body.iter().enumerate() {
            self.flatten_one(&format!("{path} -> effects.{name}.do[{j}]"), &substitute(action, &args), errs, stack, out);
        }
        stack.pop();
    }

    /// Reports every effect that was never applied: it does nothing and proves nothing. Call after every `do` list of the game has been read.
    pub(crate) fn check_used(&self, errs: &mut Vec<String>) {
        let used = self.used.borrow();
        for name in self.map.keys().filter(|n| !used.contains(*n)) {
            errs.push(format!("effects.{name}: never applied by any rule or button: apply it with {{\"apply\": \"{name}\"}} or delete it"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fx(root: Value) -> (Effects, Vec<String>) {
        let mut errs = Vec::new();
        let e = Effects::parse(root.as_object().unwrap(), &mut errs);
        (e, errs)
    }

    #[test]
    fn an_apply_becomes_the_effects_actions_with_arguments_typed_and_defaults_filled() {
        let (e, errs) = fx(json!({"effects": {
            "pickup": {"params": ["points"], "do": [{"add": ["score", "$points"]}, {"play": "pick"}]},
            "heal": {"params": {"amount": 1}, "do": [{"add": ["lives", "$amount"]}, {"set": ["note", "$amount + 1"]}]}}}));
        assert!(errs.is_empty(), "{errs:?}");
        let mut errs = Vec::new();
        let out = e.flatten("rules[0].do", &[json!({"apply": "pickup", "with": {"points": 5}}), json!({"apply": "heal"}), json!({"shake": 3})], &mut errs);
        assert!(errs.is_empty(), "{errs:?}");
        let acts: Vec<&Value> = out.iter().map(|(_, v)| v).collect();
        assert_eq!(
            acts,
            [&json!({"add": ["score", 5]}), &json!({"play": "pick"}), &json!({"add": ["lives", 1]}), &json!({"set": ["note", "1 + 1"]}), &json!({"shake": 3})]
        );
        assert_eq!(out[0].0, "rules[0].do[0] -> effects.pickup.do[0]", "an expanded action reports the call site and the definition");
        assert_eq!(out[4].0, "rules[0].do[2]", "a plain action keeps its own path");
    }

    #[test]
    fn mistakes_name_the_effect_the_parameter_and_the_fix() {
        let (e, errs) = fx(
            json!({"effects": {"pickup": {"params": ["points"], "do": [{"add": ["score", "$point"]}]}, "empty": {"do": []}, "Bad Name": {"do": [{"emit": "x"}]}}}),
        );
        assert!(
            errs.iter().any(|m| m.starts_with("effects.pickup.do[0].add[1]: `$point` is not a parameter of effect `pickup` — did you mean `points`?")),
            "{errs:?}"
        );
        assert!(errs.iter().any(|m| m.contains("effects.empty.do: empty")), "{errs:?}");
        assert!(errs.iter().any(|m| m.contains("effects.Bad Name: an effect name")), "{errs:?}");
        let (e2, _) = fx(json!({"effects": {"pickup": {"params": ["points"], "do": [{"add": ["score", "$points"]}]}}}));
        let mut errs = Vec::new();
        e2.flatten(
            "r.do",
            &[
                json!({"apply": "pickp"}),
                json!({"apply": "pickup"}),
                json!({"apply": "pickup", "with": {"points": 1, "pionts": 2}}),
                json!({"apply": "pickup", "with": 3}),
            ],
            &mut errs,
        );
        assert!(errs[0].contains("no effect `pickp` — did you mean `pickup`?"), "{errs:?}");
        assert!(errs[1].contains("needs the parameter `points`") && errs[1].contains("with"), "{errs:?}");
        assert!(errs[2].contains("has no parameter `pionts` — did you mean `points`?"), "{errs:?}");
        assert!(errs[3].contains("expected an object of arguments"), "{errs:?}");
        let _ = e;
    }

    #[test]
    fn effects_nest_but_never_loop_and_an_unused_one_is_reported() {
        let (e, errs) = fx(json!({"effects": {
            "a": {"do": [{"apply": "b"}]}, "b": {"do": [{"apply": "a"}]},
            "inner": {"do": [{"emit": "x"}]}, "mid": {"do": [{"apply": "inner"}]}, "outer": {"do": [{"apply": "mid"}]}, "unused": {"do": [{"emit": "y"}]}}}));
        assert!(errs.is_empty(), "{errs:?}");
        let mut errs = Vec::new();
        e.flatten("r.do", &[json!({"apply": "a"})], &mut errs);
        assert!(errs.iter().any(|m| m.contains("effect `a` applies itself (a -> b -> a)") || m.contains("effects may not form a cycle")), "{errs:?}");
        let mut errs = Vec::new();
        let out = e.flatten("r.do", &[json!({"apply": "outer"})], &mut errs);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(out[0].1, json!({"emit": "x"}));
        assert_eq!(out[0].0, "r.do[0] -> effects.outer.do[0] -> effects.mid.do[0] -> effects.inner.do[0]");
        let mut errs = Vec::new();
        e.check_used(&mut errs);
        assert!(errs.iter().any(|m| m.starts_with("effects.unused: never applied")) && !errs.iter().any(|m| m.contains("effects.inner")), "{errs:?}");
    }

    #[test]
    fn a_game_without_effects_expands_nothing_and_says_so_when_one_is_applied() {
        let (e, errs) = fx(json!({}));
        assert!(errs.is_empty() && e.map.is_empty());
        let mut errs = Vec::new();
        let out = e.flatten("r.do", &[json!({"emit": "go"})], &mut errs);
        assert_eq!(out.len(), 1);
        e.flatten("r.do", &[json!({"apply": "x"})], &mut errs);
        assert!(errs[0].contains("this game defines no `effects`"), "{errs:?}");
    }
}
