//! `groups` and `for_each`: rule templates a scene expands into plain rules when it loads (parse-time sugar, like `wall` and `prefab`; ADR 0006).
//!
//! A game with a board or a row of doors used to need a copy of every rule per tile (Gentle Carousel: 252 generated rules for a 5x5 board). A group
//! names its members; a template is written once and repeated for each member, with `$name` replaced by the member and `$name.field` by one of
//! its fields. What comes out is exactly the rules (and variables) an author would have written by hand, so the simulation, the match checksum,
//! replays and the wire never see a template (ADR 2026-10-17-rules-repeat-over-groups-by-parse-time-expansion).
//!
//! ```json
//! "groups": {
//!   "door": ["door_*"],                                            ids, `*` matching the scene's top-level object ids in scene order
//!   "tile": {"t00": {"box": [0,0,0,1,2,1], "near": ["t01"]}, ...}   keyed members, each with fields of its own
//! },
//! "vars": {"_state_$tile": 0},                                     one variable per member of `tile`
//! "rules": [
//!   {"id": "open_$door", "for_each": "door", "when": {"enter": {"object": "$door", "pad": 1}}, "do": [{"deactivate": "$door"}]},
//!   {"for_each": "tile", "rules": [                                 a block: its rules repeat per member, in order
//!     {"id": "step_$tile", "when": {"enter": {"box": "$tile.box"}}, "do": [{"set": ["_state_$tile", 1]}]},
//!     {"id": "wake_$tile_$n", "for_each": "n", "in": "$tile.near", "when": {"enter": {"box": "$tile.box"}}, "do": [{"activate": "$n"}]}
//!   ]},
//!   {"id": "init", "when": {"start": true}, "do": [{"for_each": "tile", "do": [{"deactivate": "$tile"}]}]}
//! ]
//! ```
//!
//! `in` is a group's name, a list, or a placeholder that holds one (`$tile.near`); without it `for_each` names the group and the placeholder both.
//! A string that is one placeholder becomes the field's value (`"box": "$tile.box"` is the list); inside a longer string a member id or a scalar
//! field is spliced in. A placeholder names the longest binding in scope (`$tile_$n` is `$tile`, `_`, `$n`). A template rule's id must name its
//! members so the expanded ids differ. Errors keep the template's path and say which member failed: `rules[3].rules[2] (step_t04, tile = t04)`.

use crate::strict::check_keys;
use serde_json::{Map, Value};

/// Keys of a template block: `{"for_each", "in"?, "rules"}`.
const BLOCK_KEYS: &[&str] = &["for_each", "in", "rules"];
/// Keys of an action block inside `do`: `{"for_each", "in"?, "do"}`.
const ACTION_BLOCK_KEYS: &[&str] = &["for_each", "in", "do"];
/// The most rules a scene's templates may expand into (a typo in a nested `for_each` should not build a million rules).
pub const MAX_EXPANDED_RULES: usize = 4096;

/// One member of a group: its id (what `$name` becomes) and its fields (`$name.field`).
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    /// The member's id.
    pub id: String,
    /// Its own data (keyed members only).
    pub fields: Map<String, Value>,
}

/// Where each rule of the expanded list came from: its template's path in the file and the members it was made for (empty for a rule written
/// as it is). `rules[i]` of the expanded scene is `origins[i]`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Expansion {
    /// Per expanded rule: (template path such as `rules[3].rules[2]`, bindings such as `tile = t04`).
    pub origins: Vec<(String, String)>,
}

impl Expansion {
    /// A rule checker's error about `rules[i]` re-addressed to the template it was made from: `rules[40] (step_t04).if: ...` becomes
    /// `rules[3].rules[2] (step_t04, tile = t04).if: ...`. Other errors pass through unchanged.
    pub fn relabel(&self, err: &str) -> String {
        let Some(rest) = err.strip_prefix("rules[") else { return err.to_string() };
        let Some(close) = rest.find(']') else { return err.to_string() };
        let Ok(i) = rest[..close].parse::<usize>() else { return err.to_string() };
        let Some((path, bound)) = self.origins.get(i) else { return err.to_string() };
        let tail = &rest[close + 1..];
        if bound.is_empty() && *path == format!("rules[{i}]") {
            return err.to_string();
        }
        let (id, tail) = match tail.strip_prefix(" (").and_then(|t| t.find(')').map(|c| (&t[..c], &t[c + 1..]))) {
            Some((id, t)) => (Some(id), t),
            None => (None, tail),
        };
        let label: Vec<&str> = id.into_iter().chain((!bound.is_empty()).then_some(bound.as_str())).collect();
        if label.is_empty() {
            format!("{path}{tail}")
        } else {
            format!("{path} ({}){tail}", label.join(", "))
        }
    }
}

/// Whether the scene uses any of this (a `groups` block, a `for_each` anywhere in `rules`, or a `$` in a variable's name): most scenes do not,
/// and they are left byte for byte alone.
pub fn uses_templates(root: &Map<String, Value>) -> bool {
    fn has_for_each(v: &Value) -> bool {
        match v {
            Value::Object(o) => o.contains_key("for_each") || o.values().any(has_for_each),
            Value::Array(a) => a.iter().any(has_for_each),
            _ => false,
        }
    }
    root.contains_key("groups")
        || root.get("rules").is_some_and(has_for_each)
        || root.get("vars").and_then(Value::as_object).is_some_and(|o| o.keys().any(|k| k.contains('$')))
}

/// Expands the scene's templates in place: `groups` is removed, every variable named with a `$group` becomes one per member, and every
/// template in `rules` (and every `for_each` inside a `do`) becomes the plain rules (actions) it stands for. A scene without templates is not touched.
pub fn expand(value: &mut Value) -> Result<Expansion, Vec<String>> {
    let Some(root) = value.as_object_mut() else { return Ok(Expansion::default()) };
    let plain = root.get("rules").and_then(Value::as_array).map_or(0, Vec::len);
    if !uses_templates(root) {
        return Ok(Expansion { origins: (0..plain).map(|i| (format!("rules[{i}]"), String::new())).collect() });
    }
    let mut errs = Vec::new();
    let object_ids: Vec<String> =
        root.get("objects").and_then(Value::as_array).into_iter().flatten().filter_map(|o| o.get("id").and_then(Value::as_str).map(str::to_string)).collect();
    let groups = parse_groups(root.remove("groups").as_ref(), &object_ids, &mut errs);
    if let Some(vars) = root.get("vars").and_then(Value::as_object).cloned() {
        root.insert("vars".into(), Value::Object(expand_vars(&vars, &groups, &mut errs)));
    }
    let mut out = Vec::new();
    if let Some(list) = root.get("rules").and_then(Value::as_array).cloned() {
        let mut cx = Cx { groups: &groups, errs: &mut errs, out: &mut out };
        cx.rules(&list, &[], "rules");
        if out.len() > MAX_EXPANDED_RULES {
            errs.push(format!("rules: the templates expand into {} rules, more than {MAX_EXPANDED_RULES}: check the `for_each` ranges", out.len()));
        }
        root.insert("rules".into(), Value::Array(out.iter().map(|(r, _, _)| r.clone()).collect()));
    }
    if errs.is_empty() {
        Ok(Expansion { origins: out.into_iter().map(|(_, p, b)| (p, b)).collect() })
    } else {
        Err(errs)
    }
}

fn is_name(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn parse_groups(raw: Option<&Value>, object_ids: &[String], errs: &mut Vec<String>) -> Vec<(String, Vec<Member>)> {
    let mut out = Vec::new();
    let Some(raw) = raw else { return out };
    let Some(o) = raw.as_object() else {
        errs.push("groups: must be an object of name -> members, like {\"door\": [\"door_*\"]}".to_string());
        return out;
    };
    for (name, v) in o.iter().filter(|(k, _)| !crate::fields::is_extension_key(k)) {
        let p = format!("groups.{name}");
        if !is_name(name) {
            errs.push(format!("{p}: a group name is letters, digits and `_`, not starting with a digit (it is written `${name}` in a template)"));
            continue;
        }
        let mut members: Vec<Member> = Vec::new();
        match v {
            Value::Array(list) => {
                for (i, m) in list.iter().enumerate() {
                    let Some(s) = m.as_str().filter(|s| !s.is_empty()) else {
                        errs.push(format!(
                            "{p}[{i}]: must be an id string (or a pattern with `*`); for members with fields write {{\"id\": {{\"field\": value}}}}"
                        ));
                        continue;
                    };
                    if s.contains('*') {
                        let found: Vec<&String> = object_ids.iter().filter(|id| glob(s, id)).collect();
                        if found.is_empty() {
                            errs.push(format!("{p}[{i}]: `{s}` matches no top-level object id"));
                        }
                        members.extend(found.into_iter().map(|id| Member { id: id.clone(), fields: Map::new() }));
                    } else {
                        members.push(Member { id: s.to_string(), fields: Map::new() });
                    }
                }
            }
            Value::Object(keyed) => {
                for (id, f) in keyed {
                    match f.as_object() {
                        Some(fields) => members.push(Member { id: id.clone(), fields: fields.clone() }),
                        None => errs.push(format!("{p}.{id}: must be an object of this member's fields, like {{\"box\": [0,0,0,1,2,1]}}")),
                    }
                }
            }
            _ => errs.push(format!("{p}: must be a list of ids, like [\"door_a\", \"door_*\"], or an object of id -> fields")),
        }
        let mut seen = std::collections::HashSet::new();
        for m in &members {
            if !seen.insert(m.id.clone()) {
                errs.push(format!("{p}: `{}` is a member twice", m.id));
            }
        }
        if members.is_empty() && !errs.iter().any(|e| e.starts_with(&p)) {
            errs.push(format!("{p}: has no members"));
        }
        out.push((name.clone(), members));
    }
    out
}

/// `*` matches any run of characters; nothing else is special.
fn glob(pattern: &str, s: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !s.starts_with(first) || s.len() < first.len() + last.len() || !s.ends_with(last) {
        return false;
    }
    let mut at = first.len();
    for mid in &parts[1..parts.len() - 1] {
        match s[at..s.len() - last.len()].find(mid) {
            Some(i) => at += i + mid.len(),
            None => return false,
        }
    }
    true
}

fn expand_vars(vars: &Map<String, Value>, groups: &[(String, Vec<Member>)], errs: &mut Vec<String>) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, v) in vars {
        if !key.contains('$') {
            out.insert(key.clone(), v.clone());
            continue;
        }
        let names: Vec<&str> = groups.iter().map(|(n, _)| n.as_str()).collect();
        let used: Vec<&str> = placeholders(key, &names).into_iter().map(|(n, _)| n).collect();
        let Some(group) = used.first() else {
            errs.push(format!("vars.{key}: `$` names no group (groups: {})", if names.is_empty() { "none".to_string() } else { names.join(", ") }));
            continue;
        };
        if used.iter().any(|n| n != group) {
            errs.push(format!("vars.{key}: a variable template repeats over one group; this names {}", used.join(" and ")));
            continue;
        }
        let members = &groups.iter().find(|(n, _)| n == group).map(|(_, m)| m.clone()).unwrap_or_default();
        for m in members {
            let scope = [(group.to_string(), m.clone())];
            match (subst_str(key, &scope), subst(v, &scope)) {
                (Err(e), _) | (_, Err(e)) => errs.push(format!("vars.{key} ({group} = {}): {e}", m.id)),
                (Ok(Value::String(k)), Ok(val)) => {
                    if out.insert(k.clone(), val).is_some() {
                        errs.push(format!("vars.{key}: `{k}` is declared twice"));
                    }
                }
                (Ok(_), Ok(_)) => errs.push(format!("vars.{key} ({group} = {}): the name is not text: write the member's id or a text field into it", m.id)),
            }
        }
    }
    out
}

/// The placeholders in `s`, as (binding name, field) in order: at each `$`, the longest of `names` that follows it, then `.field` when the
/// characters after it are a name.
fn placeholders<'a>(s: &str, names: &[&'a str]) -> Vec<(&'a str, Option<String>)> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(d) = rest.find('$') {
        rest = &rest[d + 1..];
        let Some(name) = names.iter().filter(|n| rest.starts_with(**n)).max_by_key(|n| n.len()) else { continue };
        rest = &rest[name.len()..];
        let field =
            rest.strip_prefix('.').map(|r| r.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect::<String>()).filter(|f| !f.is_empty());
        if let Some(f) = &field {
            rest = &rest[f.len() + 1..];
        }
        out.push((*name, field));
    }
    out
}

type Scope = [(String, Member)];

fn lookup(scope: &Scope, name: &str, field: Option<&str>) -> Result<Value, String> {
    let (_, m) = scope.iter().rev().find(|(n, _)| n == name).ok_or_else(|| format!("`${name}` is not in scope"))?;
    match field {
        None => Ok(Value::String(m.id.clone())),
        Some(f) => m.fields.get(f).cloned().ok_or_else(|| {
            let have: Vec<&str> = m.fields.keys().map(String::as_str).collect();
            format!("`${name}.{f}`: member `{}` has no field `{f}` (its fields: {})", m.id, if have.is_empty() { "none".to_string() } else { have.join(", ") })
        }),
    }
}

/// `s` with its placeholders filled: one placeholder that is the whole string becomes the value itself; in a longer string each is spliced in as text.
fn subst_str(s: &str, scope: &Scope) -> Result<Value, String> {
    if !s.contains('$') {
        return Ok(Value::String(s.to_string()));
    }
    let names: Vec<&str> = scope.iter().map(|(n, _)| n.as_str()).collect();
    let found = placeholders(s, &names);
    let unknown = s.matches('$').count() > found.len();
    if unknown {
        return Err(format!(
            "`{s}` has a `$` that names nothing in scope (in scope: {})",
            if names.is_empty() { "nothing".to_string() } else { names.join(", ") }
        ));
    }
    if let [(name, field)] = found.as_slice() {
        let whole = format!("${name}{}", field.as_ref().map(|f| format!(".{f}")).unwrap_or_default());
        if s == whole {
            return lookup(scope, name, field.as_deref());
        }
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    for (name, field) in found {
        let d = rest.find('$').unwrap_or(rest.len());
        out.push_str(&rest[..d]);
        let token_len = 1 + name.len() + field.as_ref().map_or(0, |f| f.len() + 1);
        rest = &rest[(d + token_len).min(rest.len())..];
        match lookup(scope, name, field.as_deref())? {
            Value::String(t) => out.push_str(&t),
            Value::Number(n) => out.push_str(&n.to_string()),
            Value::Bool(b) => out.push_str(if b { "true" } else { "false" }),
            other => {
                return Err(format!(
                    "`${name}{}` is {other} and cannot be part of the text `{s}` (write it as the whole string to use the value)",
                    field.map(|f| format!(".{f}")).unwrap_or_default()
                ))
            }
        }
    }
    out.push_str(rest);
    Ok(Value::String(out))
}

/// `v` with its placeholders filled. Outside any template (an empty scope) nothing is filled, so a `$` in a plain rule stays text; notes
/// (the extension keys: `notes`, `$comment`, `x-*`, `_*`) are never filled.
fn subst(v: &Value, scope: &Scope) -> Result<Value, String> {
    if scope.is_empty() {
        return Ok(v.clone());
    }
    match v {
        Value::String(s) => subst_str(s, scope),
        Value::Array(a) => a.iter().map(|x| subst(x, scope)).collect::<Result<Vec<_>, _>>().map(Value::Array),
        Value::Object(o) => o
            .iter()
            .map(|(k, x)| Ok((k.clone(), if crate::fields::is_extension_key(k) { x.clone() } else { subst(x, scope)? })))
            .collect::<Result<Map<_, _>, String>>()
            .map(Value::Object),
        other => Ok(other.clone()),
    }
}

struct Cx<'a> {
    groups: &'a [(String, Vec<Member>)],
    errs: &'a mut Vec<String>,
    /// (rule, template path, bindings).
    out: &'a mut Vec<(Value, String, String)>,
}

impl Cx<'_> {
    /// The members a `for_each` repeats over: `in` (a group's name, a list, or a placeholder holding one), else the group the binding is named after.
    fn range(&mut self, o: &Map<String, Value>, scope: &Scope, p: &str) -> Option<(String, Vec<Member>)> {
        let Some(name) = o.get("for_each").and_then(Value::as_str).filter(|n| is_name(n)) else {
            self.errs.push(format!("{p}.for_each: must be a name (the group to repeat over, or with `in` the name its members get), like \"tile\""));
            return None;
        };
        let group = |n: &str| self.groups.iter().find(|(g, _)| g == n).map(|(_, m)| m.clone());
        let members = match o.get("in") {
            None => group(name).ok_or_else(|| self.no_group(name)),
            Some(Value::String(s)) if !s.contains('$') => group(s).ok_or_else(|| self.no_group(s)),
            Some(v) => match subst(v, scope) {
                Ok(Value::Array(list)) => list
                    .iter()
                    .map(|m| match m {
                        Value::String(s) => Ok(Member { id: s.clone(), fields: Map::new() }),
                        Value::Number(n) => Ok(Member { id: n.to_string(), fields: Map::new() }),
                        other => Err(format!("a member of `in` is {other}: members are ids (strings or numbers)")),
                    })
                    .collect(),
                Ok(other) => Err(format!("must be a group's name or a list of ids (got {other})")),
                Err(e) => Err(e),
            },
        };
        match members {
            Ok(m) => Some((name.to_string(), m)),
            Err(e) => {
                self.errs.push(format!("{p}.{}: {e}", if o.contains_key("in") { "in" } else { "for_each" }));
                None
            }
        }
    }

    fn no_group(&self, name: &str) -> String {
        let names: Vec<&str> = self.groups.iter().map(|(n, _)| n.as_str()).collect();
        let hint = crate::suggest::suggest(name, names.iter().copied()).first().map(|h| format!(" — did you mean `{h}`?")).unwrap_or_default();
        format!("no group `{name}`{hint} (groups: {})", if names.is_empty() { "none: declare them under `groups`".to_string() } else { names.join(", ") })
    }

    fn rules(&mut self, list: &[Value], scope: &Scope, path: &str) {
        for (i, entry) in list.iter().enumerate() {
            let p = format!("{path}[{i}]");
            let Some(o) = entry.as_object() else {
                self.push(entry.clone(), p, scope);
                continue;
            };
            if !o.contains_key("for_each") {
                self.rule(o, scope, &p);
                continue;
            }
            let Some((name, members)) = self.range(o, scope, &p) else { continue };
            if let Some(inner) = o.get("rules") {
                check_keys(self.errs, &p, o, BLOCK_KEYS);
                let Some(inner) = inner.as_array() else {
                    self.errs.push(format!("{p}.rules: must be a list of rules"));
                    continue;
                };
                for m in members {
                    let mut s = scope.to_vec();
                    s.push((name.clone(), m));
                    self.rules(inner, &s, &format!("{p}.rules"));
                }
            } else {
                let id = o.get("id").and_then(Value::as_str).unwrap_or_default();
                if !placeholders(id, &[name.as_str()]).iter().any(|(n, _)| *n == name) {
                    self.errs.push(format!(
                        "{p}.id: a rule repeated for each `{name}` needs `${name}` in its id (like \"{}_${name}\") so each copy has its own",
                        if id.is_empty() { "rule" } else { id }
                    ));
                    continue;
                }
                let mut body = o.clone();
                body.remove("for_each");
                body.remove("in");
                for m in members {
                    let mut s = scope.to_vec();
                    s.push((name.clone(), m));
                    self.rule(&body, &s, &p);
                }
            }
        }
    }

    /// One rule (inside a template's scope or not): its placeholders filled, the `for_each` blocks in its `do` expanded.
    fn rule(&mut self, o: &Map<String, Value>, scope: &Scope, p: &str) {
        let mut r = Map::new();
        for (k, v) in o {
            let filled = if k == "do" { self.actions(v, scope, &format!("{p}.do")) } else { subst(v, scope).map_err(|e| format!("{p}.{k}: {e}")) };
            // (`notes` and friends are copied by `subst` untouched.)
            match filled {
                Ok(v) => {
                    r.insert(k.clone(), v);
                }
                Err(e) => {
                    self.errs.push(self.bound(e, scope));
                    return;
                }
            }
        }
        self.push(Value::Object(r), p.to_string(), scope);
    }

    fn actions(&mut self, v: &Value, scope: &Scope, p: &str) -> Result<Value, String> {
        let Some(list) = v.as_array() else { return subst(v, scope).map_err(|e| format!("{p}: {e}")) };
        let mut out = Vec::new();
        for (i, a) in list.iter().enumerate() {
            let ap = format!("{p}[{i}]");
            match a.as_object().filter(|o| o.contains_key("for_each")) {
                None => out.push(subst(a, scope).map_err(|e| format!("{ap}: {e}"))?),
                Some(o) => {
                    let before = self.errs.len();
                    check_keys(self.errs, &ap, o, ACTION_BLOCK_KEYS);
                    let Some((name, members)) = self.range(o, scope, &ap) else { return Err(format!("{ap}: see the error above")) };
                    if self.errs.len() > before {
                        return Err(format!("{ap}: see the error above"));
                    }
                    for m in members {
                        let mut s = scope.to_vec();
                        s.push((name.clone(), m));
                        match self.actions(o.get("do").unwrap_or(&Value::Null), &s, &format!("{ap}.do"))? {
                            Value::Array(inner) => out.extend(inner),
                            _ => return Err(format!("{ap}.do: must be a list of actions")),
                        }
                    }
                }
            }
        }
        Ok(Value::Array(out))
    }

    fn bound(&self, e: String, scope: &Scope) -> String {
        if scope.is_empty() {
            return e;
        }
        let b: Vec<String> = scope.iter().map(|(n, m)| format!("{n} = {}", m.id)).collect();
        match e.split_once(": ") {
            Some((head, msg)) => format!("{head} ({}): {msg}", b.join(", ")),
            None => e,
        }
    }

    fn push(&mut self, rule: Value, path: String, scope: &Scope) {
        let b: Vec<String> = scope.iter().map(|(n, m)| format!("{n} = {}", m.id)).collect();
        self.out.push((rule, path, b.join(", ")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(v: Value) -> Result<(Value, Expansion), Vec<String>> {
        let mut v = v;
        let e = expand(&mut v)?;
        Ok((v, e))
    }

    fn ids(v: &Value) -> Vec<&str> {
        v["rules"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect()
    }

    #[test]
    fn a_scene_without_templates_is_left_alone() {
        let scene = json!({"vars": {"a": 1}, "rules": [{"id": "r", "when": {"start": true}, "do": [{"emit": "x"}]}]});
        let (v, e) = run(scene.clone()).unwrap();
        assert_eq!(v, scene);
        assert_eq!(e.origins, [("rules[0]".to_string(), String::new())]);
    }

    #[test]
    fn a_rule_repeats_per_member_with_ids_fields_and_values_filled() {
        let (v, e) = run(json!({
            "groups": {"tile": {"a": {"box": [0, 0, 0, 1, 2, 1], "n": 3}, "b": {"box": [1, 0, 0, 2, 2, 1], "n": 4}}},
            "vars": {"x": 0, "_s_$tile": "$tile.n", "y": 1},
            "rules": [{"id": "step_$tile", "for_each": "tile", "when": {"enter": {"box": "$tile.box"}}, "if": "_s_$tile < $tile.n", "do": [{"set": ["_s_$tile", "$tile.n"]}]}]
        }))
        .unwrap();
        assert!(v.get("groups").is_none());
        assert_eq!(v["vars"], json!({"x": 0, "_s_a": 3, "_s_b": 4, "y": 1}), "per-member variables in declaration order, values from a field");
        assert_eq!(ids(&v), ["step_a", "step_b"]);
        assert_eq!(v["rules"][1], json!({"id": "step_b", "when": {"enter": {"box": [1, 0, 0, 2, 2, 1]}}, "if": "_s_b < 4", "do": [{"set": ["_s_b", 4]}]}));
        assert_eq!(e.origins[1], ("rules[0]".to_string(), "tile = b".to_string()));
    }

    #[test]
    fn blocks_keep_each_members_rules_together_and_nest_over_a_field() {
        let (v, _) = run(json!({
            "groups": {"tile": {"t0": {"near": ["t1"]}, "t1": {"near": ["t0", "t2"]}}},
            "rules": [{"for_each": "tile", "rules": [
                {"id": "chalk_$tile", "when": {"start": true}, "do": [{"emit": "c"}]},
                {"id": "rev_$tile_$n", "for_each": "n", "in": "$tile.near", "when": {"start": true}, "do": [{"activate": "$n"}]}
            ]}]
        }))
        .unwrap();
        assert_eq!(ids(&v), ["chalk_t0", "rev_t0_t1", "chalk_t1", "rev_t1_t0", "rev_t1_t2"], "`$tile_$n` is `$tile`, `_`, `$n`: the longest binding wins");
    }

    #[test]
    fn a_for_each_in_a_do_list_repeats_its_actions_in_place() {
        let (v, _) = run(json!({
            "groups": {"lamp": ["l1", "l2"]},
            "rules": [{"id": "init", "when": {"start": true}, "do": [{"emit": "a"}, {"for_each": "lamp", "do": [{"hide": "$lamp"}, {"emit": "off_$lamp"}]}, {"emit": "b"}]}]
        }))
        .unwrap();
        assert_eq!(v["rules"][0]["do"], json!([{"emit": "a"}, {"hide": "l1"}, {"emit": "off_l1"}, {"hide": "l2"}, {"emit": "off_l2"}, {"emit": "b"}]));
    }

    #[test]
    fn a_pattern_matches_object_ids_in_scene_order_and_in_can_rename_or_list() {
        let (v, _) = run(json!({
            "objects": [{"id": "door_b"}, {"id": "wall"}, {"id": "door_a"}],
            "groups": {"door": ["door_*"]},
            "rules": [
                {"id": "open_$d", "for_each": "d", "in": "door", "when": {"start": true}, "do": [{"deactivate": "$d"}]},
                {"id": "ring_$k", "for_each": "k", "in": [1, 2], "when": {"every": 1}, "if": "t == $k", "do": [{"emit": "ring"}]}
            ]
        }))
        .unwrap();
        assert_eq!(ids(&v), ["open_door_b", "open_door_a", "ring_1", "ring_2"]);
        assert_eq!(v["rules"][3]["if"], "t == 2");
    }

    #[test]
    fn every_mistake_names_the_template_the_member_and_the_fix() {
        let e = run(json!({
            "objects": [{"id": "door_a"}],
            "groups": {"tile": {"a": {"box": [0, 0, 0, 1, 1, 1]}}, "door": ["gate_*"], "9bad": ["x"], "empty": []},
            "vars": {"v_$nope": 0, "w_$tile": "$tile.start"},
            "rules": [
                {"id": "same", "for_each": "tile", "when": {"start": true}, "do": []},
                {"id": "r_$tile", "for_each": "tiles", "when": {"start": true}, "do": []},
                {"id": "w_$tile", "for_each": "tile", "when": {"enter": {"box": "$tile.size"}}, "do": []},
                {"id": "x_$tile", "for_each": "tile", "when": {"start": true}, "do": [{"emit": "$other"}]},
                {"for_each": "tile", "rules": [], "colour": "red"},
                {"id": "t_$tile", "for_each": "tile", "when": {"start": true}, "do": [{"emit": "in $tile.box"}]}
            ]
        }))
        .unwrap_err()
        .join("\n");
        for want in [
            "groups.door[0]: `gate_*` matches no top-level object id",
            "groups.9bad: a group name is letters, digits and `_`",
            "groups.empty: has no members",
            "vars.v_$nope: `$` names no group",
            "vars.w_$tile (tile = a): `$tile.start`: member `a` has no field `start` (its fields: box)",
            "rules[0].id: a rule repeated for each `tile` needs `$tile` in its id",
            "rules[1].for_each: no group `tiles` — did you mean `tile`?",
            "rules[2].when (tile = a): `$tile.size`: member `a` has no field `size` (its fields: box)",
            "rules[3].do[0] (tile = a): `$other` has a `$` that names nothing in scope (in scope: tile)",
            "rules[4].colour",
            "cannot be part of the text `in $tile.box`",
        ] {
            assert!(e.contains(want), "missing `{want}` in:\n{e}");
        }
    }

    #[test]
    fn a_checkers_error_is_readdressed_to_the_template_and_its_member() {
        let e = Expansion { origins: vec![("rules[0]".into(), String::new()), ("rules[3].rules[2]".into(), "tile = t04".into())] };
        assert_eq!(e.relabel("rules[1] (step_t04).if: unknown variable `scor`"), "rules[3].rules[2] (step_t04, tile = t04).if: unknown variable `scor`");
        assert_eq!(e.relabel("rules[0] (init).do[0]: bad"), "rules[0] (init).do[0]: bad", "a rule written as it is keeps its address");
        assert_eq!(e.relabel("vars.x: must be a number"), "vars.x: must be a number");
    }

    #[test]
    fn a_runaway_nesting_is_an_error_not_a_million_rules() {
        let many: Vec<Value> = (0..70).map(|i| json!(format!("m{i}"))).collect();
        let e = run(json!({
            "groups": {"a": many.clone(), "b": many},
            "rules": [{"for_each": "a", "rules": [{"id": "r_$a_$b", "for_each": "b", "when": {"start": true}, "do": []}]}]
        }))
        .unwrap_err();
        assert!(e[0].contains("expand into 4900 rules, more than 4096"), "{e:?}");
    }

    #[test]
    fn glob_star_is_any_run() {
        assert!(glob("door_*", "door_a") && glob("*_a", "door_a") && glob("d*r*a", "door_a") && glob("*", "x"));
        assert!(!glob("door_*", "dor_a") && !glob("a*a", "a"));
    }
}
