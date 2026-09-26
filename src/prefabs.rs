//! Prefabs: reusable, parametric object groups authored in plain JSON — no Rust required.
//!
//! A prefab is a named template: a list of ordinary scene objects (boxes, spheres, `prop`s, even
//! other prefabs) plus optional `params` with defaults. A scene places one with a single line:
//!
//! ```json
//! { "id": "snack_1", "type": "prefab", "prefab": "apple_red",
//!   "position": [2, 0.78, 3], "params": { "color": "#7fb83a" } }
//! ```
//!
//! Like the `wall`/`fence` macros (`crate::macros`) an instance **expands at parse time into a
//! plain `group`**, so rendering, collision and every analysis tool only ever see primitives.
//! Expansion is JSON-to-JSON and unit-testable without a GPU.
//!
//! Where prefabs come from:
//! * the built-in catalogue, `assets/*.json` (embedded in the binary, see [`BUILTIN_FILES`]);
//! * a scene's own top-level `"prefabs"` (an object `{name: def}` or an array of defs with
//!   `name`), which shadows a built-in of the same name.
//!
//! Template language (inside a def's `objects`):
//! * a string that is exactly `"$name"` is replaced by that param's value (any JSON type);
//! * a string starting with `"="` is an arithmetic expression over params, e.g.
//!   `"=$w/2 - 0.05"` (operators `+ - * /`, parentheses, unary `-`, `min() max() abs()`).
//!
//! Conventions every catalogue prefab follows: the origin is the middle of the base and the front
//! faces local `+Z` (same as `prop`s). Wall/ceiling-mounted ones set `"mount": "wall"` and put
//! the origin at the middle of the back face.

use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// The built-in catalogue files: `(category, JSON text)`. Each file is an array of prefab defs.
pub const BUILTIN_FILES: &[(&str, &str)] = &[
    ("food", include_str!("../assets/food.json")),
    ("kitchen", include_str!("../assets/kitchen.json")),
    ("furniture", include_str!("../assets/furniture.json")),
    ("office", include_str!("../assets/office.json")),
    ("school", include_str!("../assets/school.json")),
    ("store", include_str!("../assets/store.json")),
    ("decor", include_str!("../assets/decor.json")),
    ("art", include_str!("../assets/art.json")),
    ("lamps", include_str!("../assets/lamps.json")),
    ("outdoor", include_str!("../assets/outdoor.json")),
];

const MAX_DEPTH: usize = 6;

/// One template parameter: its name, default value and description.
#[derive(Debug, Clone)]
pub struct ParamDef {
    pub name: String,
    pub default: Value,
    pub desc: String,
}

/// A prefab template: name, category, tags, description, params and the object JSON it expands to.
#[derive(Debug, Clone)]
pub struct PrefabDef {
    pub name: String,
    pub category: String,
    pub tags: Vec<String>,
    pub desc: String,
    /// `floor` (origin at the base), `wall` (origin at the back face), `ceiling`, `surface`.
    pub mount: String,
    pub params: Vec<ParamDef>,
    pub objects: Vec<Value>,
    /// `false`: instances are walk-through (small decor). Instances can override with `"collide"`.
    pub collide: bool,
    /// The prefab this one extends, if any (a variant).
    pub extends: Option<String>,
    /// Alternative names and vocabulary used only for discovery.
    pub aliases: Vec<String>,
    /// Gameplay/level-design jobs this asset commonly fills (for example `seating` or `cover`).
    pub roles: Vec<String>,
    /// Visual families this asset fits. Kept separate from free-form tags so tools can filter it.
    pub styles: Vec<String>,
    /// `stable`, `experimental`, or `deprecated`.
    pub status: String,
    /// Monotonic metadata/shape revision for caches and downstream tooling.
    pub revision: u64,
    /// SPDX license identifier for the definition and any imported source material.
    pub license: String,
    /// How the asset entered the library: `authored`, `generated`, `modified`, or `imported`.
    pub origin: String,
    /// Optional human-readable provenance (generator prompt/tool, upstream URL, or parent asset).
    pub provenance: Option<String>,
}

/// A set of prefab definitions (the built-ins from `assets/*.json` plus any scene-local ones).
#[derive(Debug, Clone, Default)]
pub struct Library {
    pub defs: Vec<PrefabDef>,
}

impl Library {
    /// Looks a prefab up by name.
    pub fn find(&self, name: &str) -> Option<&PrefabDef> {
        self.defs.iter().rev().find(|d| d.name == name)
    }

    /// All prefab names, sorted.
    pub fn names(&self) -> Vec<&str> {
        self.defs.iter().map(|d| d.name.as_str()).collect()
    }

    /// Adds defs parsed from `v` (an array of defs, or an object `{name: def}`). Defs may
    /// `extends` any def already in the library or later in the same batch.
    pub fn add_json(&mut self, v: &Value, category: &str) -> Vec<String> {
        let mut errs = Vec::new();
        let mut pending: Vec<(String, Map<String, Value>)> = Vec::new();
        match v {
            Value::Array(a) => {
                for (i, d) in a.iter().enumerate() {
                    match (d.as_object(), d.get("name").and_then(Value::as_str)) {
                        (Some(o), Some(n)) => pending.push((n.to_string(), o.clone())),
                        _ => errs.push(format!("prefabs[{i}]: a prefab def needs an object with a \"name\"")),
                    }
                }
            }
            Value::Object(m) => {
                for (n, d) in m {
                    match d.as_object() {
                        Some(o) => pending.push((n.clone(), o.clone())),
                        None => errs.push(format!("prefabs.{n}: must be an object")),
                    }
                }
            }
            _ => errs.push("prefabs: must be an array of defs or an object {name: def}".to_string()),
        }
        while !pending.is_empty() {
            let before = pending.len();
            let mut still = Vec::new();
            for (name, o) in pending {
                let base = o.get("extends").and_then(Value::as_str).map(str::to_string);
                if let Some(b) = &base {
                    if self.find(b).is_none() && b != &name {
                        still.push((name, o));
                        continue;
                    }
                }
                match build_def(&name, &o, category, self) {
                    Ok(d) => self.defs.push(d),
                    Err(e) => errs.push(e),
                }
            }
            pending = still;
            if pending.len() == before {
                for (n, o) in &pending {
                    errs.push(format!("prefab '{n}': extends unknown prefab '{}'", o.get("extends").and_then(Value::as_str).unwrap_or("?")));
                }
                break;
            }
        }
        errs
    }
}

fn param_spec(name: &str, v: &Value) -> ParamDef {
    // `{"default": .., "desc": ..}` is a spec; anything else is just the default value.
    match v.as_object().filter(|o| o.contains_key("default")) {
        Some(o) => ParamDef { name: name.to_string(), default: o["default"].clone(), desc: o.get("desc").and_then(Value::as_str).unwrap_or("").to_string() },
        None => ParamDef { name: name.to_string(), default: v.clone(), desc: String::new() },
    }
}

fn build_def(name: &str, o: &Map<String, Value>, category: &str, lib: &Library) -> Result<PrefabDef, String> {
    let base = o.get("extends").and_then(Value::as_str).and_then(|b| lib.find(b)).cloned();
    let meta = match o.get("meta") {
        Some(Value::Object(m)) => Some(m),
        Some(_) => return Err(format!("prefab '{name}'.meta: must be an object")),
        None => None,
    };
    if let Some(meta) = meta {
        const KEYS: &[&str] = &["aliases", "roles", "styles", "status", "revision", "license", "origin", "provenance"];
        if let Some(key) = meta
            .keys()
            .find(|key| !KEYS.contains(&key.as_str()) && !key.starts_with("x-") && !key.starts_with("x_") && !key.starts_with('_') && key.as_str() != "notes")
        {
            return Err(format!("prefab '{name}'.meta.{key}: unknown metadata field"));
        }
    }
    let strings = |key: &str, inherited: Vec<String>| -> Result<Vec<String>, String> {
        let Some(v) = meta.and_then(|m| m.get(key)) else { return Ok(inherited) };
        let Some(a) = v.as_array() else { return Err(format!("prefab '{name}'.meta.{key}: must be an array of strings")) };
        let mut out = Vec::new();
        for (i, value) in a.iter().enumerate() {
            let Some(s) = value.as_str() else { return Err(format!("prefab '{name}'.meta.{key}[{i}]: must be a string")) };
            if s.trim().is_empty() {
                return Err(format!("prefab '{name}'.meta.{key}[{i}]: must not be empty"));
            }
            if !out.iter().any(|x| x == s) {
                out.push(s.to_string());
            }
        }
        Ok(out)
    };
    let tags: Vec<String> = {
        let mut t: Vec<String> = base.as_ref().map(|b| b.tags.clone()).unwrap_or_default();
        if let Some(a) = o.get("tags").and_then(Value::as_array) {
            for x in a.iter().filter_map(Value::as_str) {
                if !t.iter().any(|e| e == x) {
                    t.push(x.to_string());
                }
            }
        }
        t
    };
    let mut params: Vec<ParamDef> = base.as_ref().map(|b| b.params.clone()).unwrap_or_default();
    if let Some(p) = o.get("params").and_then(Value::as_object) {
        for (k, v) in p {
            let spec = param_spec(k, v);
            match params.iter_mut().find(|e| e.name == *k) {
                Some(e) => {
                    e.default = spec.default;
                    if !spec.desc.is_empty() {
                        e.desc = spec.desc;
                    }
                }
                None => params.push(spec),
            }
        }
    }
    let objects = match o.get("objects") {
        Some(Value::Array(a)) => a.clone(),
        Some(_) => return Err(format!("prefab '{name}'.objects: must be an array")),
        None => match &base {
            Some(b) => b.objects.clone(),
            None => return Err(format!("prefab '{name}': needs an \"objects\" array (or \"extends\")")),
        },
    };
    let status = meta
        .and_then(|m| m.get("status"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| base.as_ref().map(|b| b.status.clone()))
        .unwrap_or_else(|| "stable".to_string());
    if !["stable", "experimental", "deprecated"].contains(&status.as_str()) {
        return Err(format!("prefab '{name}'.meta.status: expected stable, experimental, or deprecated, got '{status}'"));
    }
    let revision = meta.and_then(|m| m.get("revision")).and_then(Value::as_u64).or_else(|| base.as_ref().map(|b| b.revision)).unwrap_or(1);
    if revision == 0 {
        return Err(format!("prefab '{name}'.meta.revision: must be at least 1"));
    }
    let origin = meta
        .and_then(|m| m.get("origin"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| base.as_ref().map(|b| b.origin.clone()))
        .unwrap_or_else(|| "authored".to_string());
    if !["authored", "generated", "modified", "imported"].contains(&origin.as_str()) {
        return Err(format!("prefab '{name}'.meta.origin: expected authored, generated, modified, or imported, got '{origin}'"));
    }
    let license = meta
        .and_then(|m| m.get("license"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| base.as_ref().map(|b| b.license.clone()))
        .unwrap_or_else(|| "MIT".to_string());
    if license.trim().is_empty() {
        return Err(format!("prefab '{name}'.meta.license: must not be empty"));
    }
    Ok(PrefabDef {
        name: name.to_string(),
        category: category.to_string(),
        tags,
        desc: o.get("desc").and_then(Value::as_str).map(str::to_string).or_else(|| base.as_ref().map(|b| b.desc.clone())).unwrap_or_default(),
        mount: o
            .get("mount")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| base.as_ref().map(|b| b.mount.clone()))
            .unwrap_or_else(|| "floor".to_string()),
        params,
        objects,
        collide: o.get("collide").and_then(Value::as_bool).or_else(|| base.as_ref().map(|b| b.collide)).unwrap_or(true),
        extends: o.get("extends").and_then(Value::as_str).map(str::to_string),
        aliases: strings("aliases", Vec::new())?,
        roles: strings("roles", base.as_ref().map(|b| b.roles.clone()).unwrap_or_default())?,
        styles: strings("styles", base.as_ref().map(|b| b.styles.clone()).unwrap_or_default())?,
        status,
        revision,
        license,
        origin,
        provenance: meta
            .and_then(|m| m.get("provenance"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| base.as_ref().and_then(|b| b.provenance.clone())),
    })
}

/// The embedded catalogue, parsed once. Errors (a broken asset file) are kept, not panicked on;
/// `tests::builtin_catalogue_loads_clean` fails the build if there are any.
pub fn builtin() -> &'static (Library, Vec<String>) {
    static LIB: OnceLock<(Library, Vec<String>)> = OnceLock::new();
    LIB.get_or_init(|| {
        let mut lib = Library::default();
        let mut errs = Vec::new();
        for (cat, text) in BUILTIN_FILES {
            match serde_json::from_str::<Value>(text) {
                Ok(v) => errs.extend(lib.add_json(&v, cat).into_iter().map(|e| format!("assets/{cat}.json: {e}"))),
                Err(e) => errs.push(format!("assets/{cat}.json: invalid JSON: {e}")),
            }
        }
        (lib, errs)
    })
}

// ---------------------------------------------------------------------------------------------
// Expression language
// ---------------------------------------------------------------------------------------------

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    params: &'a BTreeMap<String, Value>,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }

    fn expr(&mut self) -> Result<f64, String> {
        let mut v = self.term()?;
        while let Some(c) = self.peek() {
            match c {
                b'+' => {
                    self.i += 1;
                    v += self.term()?;
                }
                b'-' => {
                    self.i += 1;
                    v -= self.term()?;
                }
                _ => break,
            }
        }
        Ok(v)
    }

    fn term(&mut self) -> Result<f64, String> {
        let mut v = self.factor()?;
        while let Some(c) = self.peek() {
            match c {
                b'*' => {
                    self.i += 1;
                    v *= self.factor()?;
                }
                b'/' => {
                    self.i += 1;
                    let d = self.factor()?;
                    if d == 0.0 {
                        return Err("division by zero".to_string());
                    }
                    v /= d;
                }
                _ => break,
            }
        }
        Ok(v)
    }

    fn ident(&mut self) -> String {
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_') {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.i]).to_string()
    }

    fn factor(&mut self) -> Result<f64, String> {
        match self.peek() {
            None => Err("expression ends early".to_string()),
            Some(b'-') => {
                self.i += 1;
                Ok(-self.factor()?)
            }
            Some(b'+') => {
                self.i += 1;
                self.factor()
            }
            Some(b'(') => {
                self.i += 1;
                let v = self.expr()?;
                if self.peek() != Some(b')') {
                    return Err("missing ')'".to_string());
                }
                self.i += 1;
                Ok(v)
            }
            Some(b'$') => {
                self.i += 1;
                let name = self.ident();
                match self.params.get(&name) {
                    Some(v) => v.as_f64().ok_or_else(|| format!("param '${name}' is not a number (it is {v})")),
                    None => Err(format!("unknown param '${name}' (declared: {})", self.params.keys().cloned().collect::<Vec<_>>().join(", "))),
                }
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => {
                let start = self.i;
                while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.') {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[start..self.i]).unwrap();
                t.parse::<f64>().map_err(|_| format!("bad number '{t}'"))
            }
            Some(c) if c.is_ascii_alphabetic() => {
                let f = self.ident();
                if self.peek() != Some(b'(') {
                    return Err(format!("unknown name '{f}' (params are written $name)"));
                }
                self.i += 1;
                let mut args = vec![self.expr()?];
                while self.peek() == Some(b',') {
                    self.i += 1;
                    args.push(self.expr()?);
                }
                if self.peek() != Some(b')') {
                    return Err(format!("missing ')' after {f}("));
                }
                self.i += 1;
                match (f.as_str(), args.as_slice()) {
                    ("min", [a, b]) => Ok(a.min(*b)),
                    ("max", [a, b]) => Ok(a.max(*b)),
                    ("abs", [a]) => Ok(a.abs()),
                    _ => Err(format!("unknown function {f}/{} (have min/2, max/2, abs/1)", args.len())),
                }
            }
            Some(c) => Err(format!("unexpected '{}'", c as char)),
        }
    }
}

/// Evaluates an arithmetic expression over numeric params. Public for tests/tools.
pub fn eval_expr(src: &str, params: &BTreeMap<String, Value>) -> Result<f64, String> {
    let mut p = Parser { s: src.as_bytes(), i: 0, params };
    let v = p.expr()?;
    if p.peek().is_some() {
        return Err(format!("unexpected '{}' after the expression", p.s[p.i] as char));
    }
    Ok(v)
}

fn round4(x: f64) -> Value {
    let r = (x * 10000.0).round() / 10000.0;
    if r.fract() == 0.0 && r.abs() < 1e9 {
        json!(r as i64)
    } else {
        json!(r)
    }
}

fn is_param_ref(s: &str) -> Option<&str> {
    let name = s.strip_prefix('$')?;
    (!name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')).then_some(name)
}

/// Replaces `"$param"` strings and `"=expr"` strings throughout `v`.
fn subst(v: &Value, params: &BTreeMap<String, Value>, path: &str, errs: &mut Vec<String>) -> Value {
    match v {
        Value::String(s) => {
            if let Some(name) = is_param_ref(s) {
                return match params.get(name) {
                    Some(p) => p.clone(),
                    None => {
                        errs.push(format!("{path}: unknown param '${name}' (declared: {})", params.keys().cloned().collect::<Vec<_>>().join(", ")));
                        Value::Null
                    }
                };
            }
            if let Some(e) = s.strip_prefix('=') {
                return match eval_expr(e, params) {
                    Ok(x) => round4(x),
                    Err(m) => {
                        errs.push(format!("{path}: expression '{s}': {m}"));
                        Value::Null
                    }
                };
            }
            v.clone()
        }
        Value::Array(a) => Value::Array(a.iter().enumerate().map(|(i, x)| subst(x, params, &format!("{path}[{i}]"), errs)).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), subst(x, params, &format!("{path}.{k}"), errs))).collect()),
        other => other.clone(),
    }
}

/// Levenshtein-ish "did you mean": names containing the query or within edit distance 2.
pub fn suggest<'a>(query: &str, candidates: impl Iterator<Item = &'a str>) -> Vec<String> {
    fn dist(a: &str, b: &str) -> usize {
        let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        for i in 1..=a.len() {
            let mut cur = vec![i];
            for j in 1..=b.len() {
                let c = usize::from(a[i - 1] != b[j - 1]);
                cur.push((prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + c));
            }
            prev = cur;
        }
        prev[b.len()]
    }
    let q = query.to_lowercase();
    let mut out: Vec<(usize, String)> = candidates
        .filter_map(|c| {
            let cl = c.to_lowercase();
            let d = dist(&q, &cl);
            (cl.contains(&q) || q.contains(&cl) || d <= 2).then(|| (d, c.to_string()))
        })
        .collect();
    out.sort();
    out.into_iter().take(6).map(|(_, c)| c).collect()
}

// ---------------------------------------------------------------------------------------------
// Instance expansion
// ---------------------------------------------------------------------------------------------

/// Expands one `{"type":"prefab", ...}` instance into a `group`.
pub fn expand_instance(lib: &Library, inst: &Map<String, Value>, id: &str, depth: usize) -> Result<Value, Vec<String>> {
    let Some(name) = inst.get("prefab").and_then(Value::as_str) else {
        return Err(vec![format!("{id}.prefab: missing (the prefab's name — run `red_engine2 catalog` to list them)")]);
    };
    let Some(def) = lib.find(name) else {
        let hint = suggest(name, lib.defs.iter().map(|d| d.name.as_str()));
        return Err(vec![format!(
            "{id}.prefab: unknown prefab '{name}'{} (run `red_engine2 catalog`)",
            if hint.is_empty() { String::new() } else { format!(" — did you mean: {}?", hint.join(", ")) }
        )]);
    };
    if depth > MAX_DEPTH {
        return Err(vec![format!("{id}: prefabs nest more than {MAX_DEPTH} deep (does '{name}' contain itself?)")]);
    }
    let mut params: BTreeMap<String, Value> = def.params.iter().map(|p| (p.name.clone(), p.default.clone())).collect();
    let mut errs = Vec::new();
    crate::strict::check_keys(&mut errs, id, inst, crate::strict::PREFAB_INSTANCE_KEYS);
    if let Some(given) = inst.get("params") {
        match given.as_object() {
            Some(g) => {
                for (k, v) in g {
                    if !params.contains_key(k) {
                        let hint = suggest(k, params.keys().map(String::as_str));
                        errs.push(format!(
                            "{id}.params.{k}: '{name}' has no such param (has: {}){}",
                            if params.is_empty() { "none".to_string() } else { params.keys().cloned().collect::<Vec<_>>().join(", ") },
                            if hint.is_empty() { String::new() } else { format!(" — did you mean {}?", hint[0]) }
                        ));
                    } else {
                        params.insert(k.clone(), v.clone());
                    }
                }
            }
            None => errs.push(format!("{id}.params: must be an object")),
        }
    }
    let mut children: Vec<Value> = Vec::new();
    for (i, o) in def.objects.iter().enumerate() {
        let mut c = subst(o, &params, &format!("{id}[{name}].objects[{i}]"), &mut errs);
        if let Some(cid) = c.get("id").and_then(Value::as_str).map(str::to_string) {
            c["id"] = json!(format!("{id}.{cid}"));
        }
        children.push(c);
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    errs.extend(expand_list(lib, &mut children, depth + 1));
    if !errs.is_empty() {
        return Err(errs);
    }
    let mut g = json!({ "id": id, "type": "group", "children": children });
    for k in ["position", "rotation", "scale"] {
        if let Some(v) = inst.get(k) {
            g[k] = v.clone();
        }
    }
    let collide = inst.get("collide").and_then(Value::as_bool).unwrap_or(def.collide);
    if !collide {
        g["collide"] = json!(false);
    }
    // Remember where it came from: `physics::classify` decides from this whether it is a loose prop.
    g["prefab_name"] = json!(name);
    g["mount"] = json!(def.mount);
    if let Some(m) = inst.get("movable") {
        g["movable"] = m.clone();
    }
    Ok(g)
}

/// Replaces every prefab instance in `objects` (recursing into group children) with its group.
fn expand_list(lib: &Library, objects: &mut [Value], depth: usize) -> Vec<String> {
    let mut errs = Vec::new();
    for o in objects.iter_mut() {
        if o.get("type").and_then(Value::as_str) == Some("prefab") {
            let id = o.get("id").and_then(Value::as_str).unwrap_or("?").to_string();
            match expand_instance(lib, o.as_object().unwrap(), &id, depth) {
                Ok(g) => *o = g,
                Err(e) => errs.extend(e),
            }
        } else if let Some(kids) = o.get_mut("children").and_then(Value::as_array_mut) {
            errs.extend(expand_list(lib, kids, depth));
        }
    }
    errs
}

fn contains_prefab(objects: &[Value]) -> bool {
    objects
        .iter()
        .any(|o| o.get("type").and_then(Value::as_str) == Some("prefab") || o.get("children").and_then(Value::as_array).is_some_and(|k| contains_prefab(k)))
}

/// The pre-pass `schema::parse_scene` runs: registers the scene's own `prefabs` and replaces
/// every `type: "prefab"` object with its expanded group. No-op (and no allocation of the
/// library) for scenes that use neither.
pub fn expand_scene(root: &mut Value) -> Result<(), Vec<String>> {
    let has_local = root.get("prefabs").is_some();
    let has_use = root.get("objects").and_then(Value::as_array).is_some_and(|a| contains_prefab(a));
    if !has_local && !has_use {
        return Ok(());
    }
    let (built, load_errs) = builtin();
    let mut errs: Vec<String> = load_errs.clone();
    let mut lib = built.clone();
    if let Some(local) = root.get("prefabs") {
        errs.extend(lib.add_json(local, "scene").into_iter().map(|e| format!("prefabs: {e}")));
    }
    if let Some(objs) = root.get_mut("objects").and_then(Value::as_array_mut) {
        errs.extend(expand_list(&lib, objs, 0));
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

/// A complete one-object scene containing `prefab` at the origin — used to measure, render and
/// lint catalogue entries.
pub fn preview_scene(name: &str, params: Option<&Value>) -> Value {
    preview_scene_in(&builtin().0, name, params)
}

/// A complete one-object scene expanded with an explicit library. This is the catalogue API's
/// bridge for inspecting project-local packs without first promoting them into the engine.
pub fn preview_scene_in(lib: &Library, name: &str, params: Option<&Value>) -> Value {
    let mut inst = json!({ "id": "item", "type": "prefab", "prefab": name });
    if let Some(p) = params {
        inst["params"] = p.clone();
    }
    let object = expand_instance(lib, inst.as_object().unwrap(), "item", 0)
        .unwrap_or_else(|errors| json!({"id": "catalog_error", "type": "group", "children": [], "x-errors": errors}));
    json!({
        "meta": { "fps": 30, "duration": 1, "resolution": [640, 480] },
        "camera": { "fov": 40, "position": [2, 1.5, 3], "target": [0, 0.4, 0] },
        "lights": [{ "type": "directional", "direction": [-0.4, -1, -0.5], "intensity": 1.0 }],
        "objects": [object],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn expressions_follow_precedence_and_params() {
        let ps = p(&[("w", json!(2.0)), ("h", json!(0.5))]);
        assert_eq!(eval_expr("$w/2 - 0.05", &ps).unwrap(), 0.95);
        assert_eq!(eval_expr("-($w + $h) * 2", &ps).unwrap(), -5.0);
        assert_eq!(eval_expr("max($h, 0.7)", &ps).unwrap(), 0.7);
        assert!(eval_expr("$nope", &ps).unwrap_err().contains("unknown param"));
        assert!(eval_expr("1 / 0", &ps).is_err());
        assert!(eval_expr("2 3", &ps).is_err());
    }

    #[test]
    fn instance_expands_params_expressions_and_prefixes_ids() {
        let mut lib = Library::default();
        let errs = lib.add_json(
            &json!([{ "name": "post", "params": { "h": 1.0, "color": "#ff0000" },
                "objects": [{ "id": "p", "type": "box", "size": [0.1, "$h", 0.1], "position": [0, "=$h/2", 0],
                             "material": { "color": "$color" } }] }]),
            "t",
        );
        assert!(errs.is_empty(), "{errs:?}");
        let inst = json!({ "id": "a", "type": "prefab", "prefab": "post", "position": [1, 0, 2], "params": { "h": 3 } });
        let g = expand_instance(&lib, inst.as_object().unwrap(), "a", 0).unwrap();
        assert_eq!(g["type"], "group");
        assert_eq!(g["position"], json!([1, 0, 2]));
        let c = &g["children"][0];
        assert_eq!(c["id"], "a.p");
        assert_eq!(c["size"], json!([0.1, 3, 0.1]));
        assert_eq!(c["position"], json!([0, 1.5, 0]));
        assert_eq!(c["material"]["color"], "#ff0000");
    }

    #[test]
    fn variants_inherit_and_override_defaults() {
        let mut lib = Library::default();
        let errs = lib.add_json(
            &json!([
                { "name": "ball_red", "extends": "ball", "params": { "color": "#ff0000" }, "tags": ["red"] },
                { "name": "ball", "tags": ["toy"], "params": { "color": { "default": "#0000ff", "desc": "skin" } },
                  "objects": [{ "id": "b", "type": "sphere", "radius": 0.1, "material": { "color": "$color" } }] }
            ]),
            "t",
        );
        assert!(errs.is_empty(), "{errs:?}");
        let v = lib.find("ball_red").unwrap();
        assert_eq!(v.params[0].default, json!("#ff0000"));
        assert_eq!(v.params[0].desc, "skin");
        assert_eq!(v.tags, vec!["toy", "red"]);
    }

    #[test]
    fn structured_metadata_is_normalized_and_validated() {
        let mut lib = Library::default();
        let errs = lib.add_json(
            &json!([{"name":"locker","tags":["storage"],"desc":"locker","meta":{
                "aliases":["school locker"],"roles":["storage","cover"],"styles":["institutional"],
                "status":"experimental","revision":3,"license":"CC0-1.0","origin":"generated","provenance":"shape generator v2"
            },"objects":[{"id":"body","type":"box","size":[1,2,1],"position":[0,1,0]}]}]),
            "game",
        );
        assert!(errs.is_empty(), "{errs:?}");
        let d = lib.find("locker").unwrap();
        assert_eq!(d.aliases, vec!["school locker"]);
        assert_eq!(d.roles, vec!["storage", "cover"]);
        assert_eq!(d.status, "experimental");
        assert_eq!(d.revision, 3);
        assert_eq!(d.origin, "generated");

        let errors = lib.add_json(&json!([{"name":"bad","meta":{"status":"finished","origin":"downloaded","revision":0},"objects":[]}]), "game");
        assert!(errors.iter().any(|e| e.contains("meta.status")), "{errors:?}");
    }

    #[test]
    fn errors_point_at_the_instance_and_suggest() {
        let mut lib = Library::default();
        lib.add_json(&json!([{ "name": "apple", "params": { "color": "#f00" }, "objects": [] }]), "t");
        let e = expand_instance(&lib, json!({"id":"x","type":"prefab","prefab":"aple"}).as_object().unwrap(), "x", 0).unwrap_err();
        assert!(e[0].contains("x.prefab") && e[0].contains("did you mean: apple"), "{e:?}");
        let e = expand_instance(&lib, json!({"id":"x","type":"prefab","prefab":"apple","params":{"colour":"#fff"}}).as_object().unwrap(), "x", 0).unwrap_err();
        assert!(e[0].contains("x.params.colour") && e[0].contains("did you mean color"), "{e:?}");
    }

    #[test]
    fn self_containing_prefab_is_rejected_not_infinite() {
        let mut lib = Library::default();
        lib.add_json(&json!([{ "name": "loop", "objects": [{ "id": "again", "type": "prefab", "prefab": "loop" }] }]), "t");
        let e = expand_instance(&lib, json!({"id":"x","type":"prefab","prefab":"loop"}).as_object().unwrap(), "x", 0).unwrap_err();
        assert!(e.iter().any(|m| m.contains("nest")), "{e:?}");
    }

    #[test]
    fn scene_local_prefabs_shadow_and_expand_in_groups() {
        let mut root = json!({
            "prefabs": { "widget": { "objects": [{ "id": "w", "type": "box", "size": [1, 1, 1], "material": { "color": "#fff" } }] } },
            "objects": [{ "id": "g", "type": "group", "children": [{ "id": "wid", "type": "prefab", "prefab": "widget" }] }]
        });
        expand_scene(&mut root).unwrap();
        assert_eq!(root["objects"][0]["children"][0]["type"], "group");
        assert_eq!(root["objects"][0]["children"][0]["children"][0]["id"], "wid.w");
    }

    #[test]
    fn builtin_catalogue_loads_clean() {
        let (lib, errs) = builtin();
        assert!(errs.is_empty(), "{errs:#?}");
        assert!(!lib.defs.is_empty());
        let mut names = lib.names();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(n, names.len(), "duplicate prefab names in assets/");
    }
}
