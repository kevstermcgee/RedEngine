//! Safe, structural edits to a scene file: `set`, `move`, `rm`, `add`, `clone`, `array`,
//! `rename`, `fmt`.
//!
//! These operate on the scene's *raw JSON* (so `wall`/`fence` macros stay macros, key order and
//! unknown keys like `zones` survive), address objects by `id` (searching nested groups too),
//! and refuse to write a file that no longer validates. The writer keeps the scene readable:
//! anything that fits on one line is one line, so diffs stay small.

use crate::schema::parse_scene;
use serde_json::{Map, Value};
use std::path::Path;

/// Result type of the edit operations; the error is a user-facing message.
pub type Res<T> = Result<T, String>;

/// A scene file loaded as raw JSON for structural editing.
pub struct SceneFile {
    pub root: Value,
}

impl SceneFile {
    /// Loads a scene file for editing.
    pub fn load(path: &Path) -> Res<SceneFile> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let root: Value = serde_json::from_str(&text).map_err(|e| format!("{}: not valid JSON: {e}", path.display()))?;
        if !root.is_object() {
            return Err(format!("{}: scene root must be an object", path.display()));
        }
        Ok(SceneFile { root })
    }

    /// Serializes, validates through the real schema parser, and (unless `dry_run`) writes.
    pub fn save(&self, path: &Path, dry_run: bool, force: bool) -> Res<()> {
        let text = format_scene(&self.root);
        if let Err(errs) = parse_scene(&text) {
            let mut msg = String::from("the edited scene would be invalid:");
            for e in &errs {
                msg.push_str(&format!("\n  error: {e}"));
            }
            if !force {
                msg.push_str("\n(nothing was written; pass --force to write it anyway)");
                return Err(msg);
            }
            eprintln!("{msg}\n(writing anyway because of --force)");
        }
        if dry_run {
            println!("(dry run: {} not modified)", path.display());
            return Ok(());
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("replace {}: {e}", path.display()))?;
        Ok(())
    }

    fn array_mut(&mut self, key: &str) -> Res<&mut Vec<Value>> {
        let obj = self.root.as_object_mut().unwrap();
        if !obj.contains_key(key) {
            obj.insert(key.to_string(), Value::Array(vec![]));
        }
        obj.get_mut(key).and_then(Value::as_array_mut).ok_or_else(|| format!("'{key}' exists but is not an array"))
    }

    /// Every object id in `objects` (nested included), in document order.
    pub fn all_ids(&self) -> Vec<String> {
        fn walk(v: &[Value], out: &mut Vec<String>) {
            for o in v {
                if let Some(id) = o.get("id").and_then(Value::as_str) {
                    out.push(id.to_string());
                }
                if let Some(k) = o.get("children").and_then(Value::as_array) {
                    walk(k, out);
                }
            }
        }
        let mut out = Vec::new();
        if let Some(a) = self.root.get("objects").and_then(Value::as_array) {
            walk(a, &mut out);
        }
        out
    }

    /// Finds an object by id, for mutation.
    pub fn find_mut(&mut self, id: &str) -> Option<&mut Value> {
        fn walk<'a>(v: &'a mut [Value], id: &str) -> Option<&'a mut Value> {
            for o in v.iter_mut() {
                if o.get("id").and_then(Value::as_str) == Some(id) {
                    return Some(o);
                }
                if let Some(k) = o.get_mut("children").and_then(Value::as_array_mut) {
                    if let Some(f) = walk(k, id) {
                        return Some(f);
                    }
                }
            }
            None
        }
        walk(self.root.get_mut("objects")?.as_array_mut()?, id)
    }

    /// Finds an object by id.
    pub fn find(&self, id: &str) -> Option<&Value> {
        fn walk<'a>(v: &'a [Value], id: &str) -> Option<&'a Value> {
            for o in v {
                if o.get("id").and_then(Value::as_str) == Some(id) {
                    return Some(o);
                }
                if let Some(k) = o.get("children").and_then(Value::as_array) {
                    if let Some(f) = walk(k, id) {
                        return Some(f);
                    }
                }
            }
            None
        }
        walk(self.root.get("objects")?.as_array()?, id)
    }

    fn require_free_id(&self, id: &str) -> Res<()> {
        if self.all_ids().iter().any(|i| i == id) {
            return Err(format!("id '{id}' already exists"));
        }
        Ok(())
    }

    /// `set <id> path=value`: `path` is dotted, with numeric segments indexing arrays
    /// (`position.1=2.5`, `material.color="#ff0000"`); `value` is JSON, or a bare string.
    pub fn set(&mut self, id: &str, assignment: &str) -> Res<()> {
        let (path, raw) = assignment.split_once('=').ok_or_else(|| format!("'{assignment}' must look like path=value"))?;
        let value: Value = serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()));
        let obj = self.find_mut(id).ok_or_else(|| format!("no object with id '{id}'"))?;
        set_path(obj, path, value)
    }

    /// Removes every object (at any depth) whose id equals `pattern` or matches it as a glob.
    pub fn remove(&mut self, pattern: &str) -> usize {
        fn walk(v: &mut Vec<Value>, pattern: &str) -> usize {
            let before = v.len();
            v.retain(|o| !o.get("id").and_then(Value::as_str).is_some_and(|id| glob(pattern, id)));
            let mut n = before - v.len();
            for o in v.iter_mut() {
                if let Some(k) = o.get_mut("children").and_then(Value::as_array_mut) {
                    n += walk(k, pattern);
                }
            }
            n
        }
        match self.root.get_mut("objects").and_then(Value::as_array_mut) {
            Some(a) => walk(a, pattern),
            None => 0,
        }
    }

    /// Adds `obj` to the top-level array `key` (`objects`, `lights`, `zones`), refusing duplicate ids.
    pub fn add(&mut self, key: &str, obj: Value) -> Res<()> {
        if key == "objects" {
            if let Some(id) = obj.get("id").and_then(Value::as_str) {
                self.require_free_id(id)?;
            } else {
                return Err("an object needs an \"id\"".to_string());
            }
        }
        self.array_mut(key)?.push(obj);
        Ok(())
    }

    /// Renames an object id, refusing collisions.
    pub fn rename(&mut self, old: &str, new: &str) -> Res<()> {
        self.require_free_id(new)?;
        let o = self.find_mut(old).ok_or_else(|| format!("no object with id '{old}'"))?;
        o["id"] = Value::String(new.to_string());
        Ok(())
    }

    /// Moves/copies `id` by `delta`, understanding each object type's own position fields.
    pub fn translate(&mut self, id: &str, delta: [f64; 3]) -> Res<()> {
        let o = self.find_mut(id).ok_or_else(|| format!("no object with id '{id}'"))?;
        translate_value(o, delta)
    }

    /// Moves an object to an absolute position (walls/fences shift their endpoints).
    pub fn place_at(&mut self, id: &str, to: [f64; 3]) -> Res<()> {
        let o = self.find(id).ok_or_else(|| format!("no object with id '{id}'"))?;
        let cur = anchor_of(o)?;
        self.translate(id, [to[0] - cur[0], to[1] - cur[1], to[2] - cur[2]])
    }

    /// Inserts a copy of `id` as `new_id` right after it (top-level or within its group).
    pub fn clone_object(&mut self, id: &str, new_id: &str, delta: [f64; 3], rot_y_add: f64) -> Res<()> {
        self.require_free_id(new_id)?;
        let mut copy = self.find(id).ok_or_else(|| format!("no object with id '{id}'"))?.clone();
        copy["id"] = Value::String(new_id.to_string());
        translate_value(&mut copy, delta)?;
        if rot_y_add != 0.0 {
            let cur = copy.get("rotation").and_then(Value::as_array).map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect::<Vec<_>>()).unwrap_or_else(|| vec![0.0, 0.0, 0.0]);
            if cur.len() == 3 {
                copy["rotation"] = num_array(&[cur[0], cur[1] + rot_y_add, cur[2]]);
            }
        }
        // Insert after the original.
        fn walk(v: &mut Vec<Value>, id: &str, copy: &Value) -> bool {
            if let Some(i) = v.iter().position(|o| o.get("id").and_then(Value::as_str) == Some(id)) {
                v.insert(i + 1, copy.clone());
                return true;
            }
            for o in v.iter_mut() {
                if let Some(k) = o.get_mut("children").and_then(Value::as_array_mut) {
                    if walk(k, id, copy) {
                        return true;
                    }
                }
            }
            false
        }
        walk(self.root.get_mut("objects").and_then(Value::as_array_mut).ok_or("scene has no objects array")?, id, &copy);
        Ok(())
    }
}

/// Builds a JSON array of numbers.
pub fn num_array(v: &[f64]) -> Value {
    Value::Array(v.iter().map(|x| num(*x)).collect())
}

/// A JSON number rounded to 4 decimals; whole numbers stay integers (`2`, not `2.0`).
pub fn num(x: f64) -> Value {
    let r = (x * 10000.0).round() / 10000.0;
    if r.fract() == 0.0 && r.abs() < 1e12 {
        return Value::from(r as i64);
    }
    serde_json::Number::from_f64(r).map(Value::Number).unwrap_or(Value::Null)
}

fn vec3_of(v: Option<&Value>, default: [f64; 3]) -> Res<[f64; 3]> {
    match v {
        None => Ok(default),
        Some(Value::Array(a)) if a.len() == 3 => {
            let f = |i: usize| a[i].as_f64().ok_or_else(|| "position component is not a number".to_string());
            Ok([f(0)?, f(1)?, f(2)?])
        }
        Some(_) => Err("position is animated (keyframes) or malformed; edit it by hand with `set`".to_string()),
    }
}

/// The `[x, y, z]` an object is "at": its `position`, or for `wall`/`fence` the start point.
pub fn anchor_of(o: &Value) -> Res<[f64; 3]> {
    match o.get("type").and_then(Value::as_str) {
        Some("wall") => {
            let f = o.get("from").and_then(Value::as_array).ok_or("wall has no 'from'")?;
            let y = o.get("y").and_then(Value::as_f64).unwrap_or(0.0);
            Ok([f[0].as_f64().unwrap_or(0.0), y, f[1].as_f64().unwrap_or(0.0)])
        }
        Some("fence") => {
            let p = o.get("points").and_then(Value::as_array).and_then(|p| p.first()).and_then(Value::as_array).ok_or("fence has no points")?;
            let y = o.get("y").and_then(Value::as_f64).unwrap_or(0.0);
            Ok([p[0].as_f64().unwrap_or(0.0), y, p[1].as_f64().unwrap_or(0.0)])
        }
        _ => vec3_of(o.get("position"), [0.0, 0.0, 0.0]),
    }
}

fn translate_value(o: &mut Value, d: [f64; 3]) -> Res<()> {
    let shift_xz = |v: &mut Value| {
        if let Some(a) = v.as_array_mut() {
            if a.len() == 2 {
                let (x, z) = (a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0));
                a[0] = num(x + d[0]);
                a[1] = num(z + d[2]);
            }
        }
    };
    match o.get("type").and_then(Value::as_str) {
        Some("wall") => {
            for k in ["from", "to"] {
                if let Some(v) = o.get_mut(k) {
                    shift_xz(v);
                }
            }
            let y = o.get("y").and_then(Value::as_f64).unwrap_or(0.0);
            if d[1] != 0.0 || o.get("y").is_some() {
                o["y"] = num(y + d[1]);
            }
        }
        Some("fence") => {
            if let Some(pts) = o.get_mut("points").and_then(Value::as_array_mut) {
                for p in pts {
                    shift_xz(p);
                }
            }
            if let Some(gaps) = o.get_mut("gaps").and_then(Value::as_array_mut) {
                for g in gaps {
                    if let Some(at) = g.get_mut("at") {
                        shift_xz(at);
                    }
                }
            }
            let y = o.get("y").and_then(Value::as_f64).unwrap_or(0.0);
            if d[1] != 0.0 || o.get("y").is_some() {
                o["y"] = num(y + d[1]);
            }
        }
        _ => {
            let cur = vec3_of(o.get("position"), [0.0, 0.0, 0.0])?;
            o["position"] = num_array(&[cur[0] + d[0], cur[1] + d[1], cur[2] + d[2]]);
        }
    }
    Ok(())
}

fn set_path(target: &mut Value, path: &str, value: Value) -> Res<()> {
    let mut cur = target;
    let segs: Vec<&str> = path.split('.').collect();
    for (i, seg) in segs.iter().enumerate() {
        let last = i + 1 == segs.len();
        if let Ok(idx) = seg.parse::<usize>() {
            let arr = cur.as_array_mut().ok_or_else(|| format!("'{}' is not an array (at segment '{seg}')", segs[..i].join(".")))?;
            if idx >= arr.len() {
                return Err(format!("index {idx} out of range (len {}) in '{path}'", arr.len()));
            }
            if last {
                arr[idx] = value;
                return Ok(());
            }
            cur = &mut arr[idx];
        } else {
            if !cur.is_object() {
                return Err(format!("'{}' is not an object (at segment '{seg}')", segs[..i].join(".")));
            }
            let map = cur.as_object_mut().unwrap();
            if last {
                map.insert(seg.to_string(), value);
                return Ok(());
            }
            cur = map.entry(seg.to_string()).or_insert_with(|| Value::Object(Map::new()));
        }
    }
    Ok(())
}

/// `*` matches any run of characters; everything else is literal.
pub fn glob(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut rest = text;
    for (i, p) in parts.iter().enumerate() {
        if i == 0 {
            if !rest.starts_with(p) {
                return false;
            }
            rest = &rest[p.len()..];
        } else if i + 1 == parts.len() {
            return rest.ends_with(p);
        } else if let Some(pos) = rest.find(p) {
            rest = &rest[pos + p.len()..];
        } else {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------------------------

const LINE_WIDTH: usize = 150;

fn inline(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            if m.is_empty() {
                return "{}".to_string();
            }
            let parts: Vec<String> = m.iter().map(|(k, v)| format!("{}: {}", Value::String(k.clone()), inline(v))).collect();
            format!("{{ {} }}", parts.join(", "))
        }
        Value::Array(a) => {
            let parts: Vec<String> = a.iter().map(inline).collect();
            format!("[{}]", parts.join(", "))
        }
        other => other.to_string(),
    }
}

fn pretty(v: &Value, indent: usize, out: &mut String) {
    let one = inline(v);
    if indent * 2 + one.len() <= LINE_WIDTH || !(v.is_object() || v.is_array()) {
        out.push_str(&one);
        return;
    }
    let pad = "  ".repeat(indent + 1);
    match v {
        Value::Object(m) => {
            out.push_str("{\n");
            for (i, (k, val)) in m.iter().enumerate() {
                out.push_str(&pad);
                out.push_str(&format!("{}: ", Value::String(k.clone())));
                pretty(val, indent + 1, out);
                out.push_str(if i + 1 == m.len() { "\n" } else { ",\n" });
            }
            out.push_str(&"  ".repeat(indent));
            out.push('}');
        }
        Value::Array(a) => {
            out.push_str("[\n");
            for (i, val) in a.iter().enumerate() {
                out.push_str(&pad);
                pretty(val, indent + 1, out);
                out.push_str(if i + 1 == a.len() { "\n" } else { ",\n" });
            }
            out.push_str(&"  ".repeat(indent));
            out.push(']');
        }
        _ => unreachable!(),
    }
}

/// Renders a scene: each top-level key on its own line(s), arrays of objects one element per
/// line, and any value that fits in [`LINE_WIDTH`] columns kept inline.
pub fn format_scene(root: &Value) -> String {
    let mut out = String::from("{\n");
    let map = root.as_object().expect("scene root is an object");
    for (i, (k, v)) in map.iter().enumerate() {
        out.push_str(&format!("  {}: ", Value::String(k.clone())));
        match v {
            Value::Array(a) if !a.is_empty() && a.iter().all(Value::is_object) => {
                out.push_str("[\n");
                for (j, o) in a.iter().enumerate() {
                    out.push_str("    ");
                    pretty(o, 2, &mut out);
                    out.push_str(if j + 1 == a.len() { "\n" } else { ",\n" });
                }
                out.push_str("  ]");
            }
            other => pretty(other, 1, &mut out),
        }
        out.push_str(if i + 1 == map.len() { "\n" } else { ",\n" });
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scene() -> SceneFile {
        SceneFile {
            root: json!({
                "camera": {},
                "objects": [
                    {"id": "a", "type": "box", "position": [1, 0, 2], "material": {"color": "#ffffff"}},
                    {"id": "g", "type": "group", "children": [{"id": "g.b", "type": "box", "position": [0, 0, 0]}]},
                    {"id": "w", "type": "wall", "from": [0, 0], "to": [4, 0]}
                ]
            }),
        }
    }

    #[test]
    fn set_reaches_into_arrays_and_nested_objects() {
        let mut s = scene();
        s.set("a", "position.1=2.5").unwrap();
        s.set("a", "material.color=#ff0000").unwrap();
        assert_eq!(s.find("a").unwrap()["position"], json!([1, 2.5, 2]));
        assert_eq!(s.find("a").unwrap()["material"]["color"], json!("#ff0000"));
        assert!(s.set("a", "position.7=1").is_err());
        assert!(s.set("nope", "x=1").is_err());
    }

    #[test]
    fn nested_objects_are_addressable_and_removable() {
        let mut s = scene();
        assert!(s.find("g.b").is_some());
        assert_eq!(s.remove("g.b"), 1);
        assert!(s.find("g.b").is_none());
        assert_eq!(s.remove("nothing*"), 0);
    }

    #[test]
    fn glob_matches_prefixes_and_infixes() {
        assert!(glob("fence_*", "fence_back_3"));
        assert!(glob("*_1", "chair_1"));
        assert!(glob("a*c*e", "abcde"));
        assert!(!glob("fence_*", "wall_1"));
        assert!(glob("exact", "exact") && !glob("exact", "exact2"));
    }

    #[test]
    fn translate_understands_walls_and_regular_objects() {
        let mut s = scene();
        s.translate("w", [1.0, 0.5, 2.0]).unwrap();
        assert_eq!(s.find("w").unwrap()["from"], json!([1, 2]));
        assert_eq!(s.find("w").unwrap()["to"], json!([5, 2]));
        assert_eq!(s.find("w").unwrap()["y"], json!(0.5));
        s.translate("a", [1.0, 0.0, 0.0]).unwrap();
        assert_eq!(s.find("a").unwrap()["position"], json!([2, 0, 2]));
    }

    #[test]
    fn clone_gets_a_new_id_and_offset() {
        let mut s = scene();
        s.clone_object("a", "a2", [3.0, 0.0, 0.0], 90.0).unwrap();
        assert_eq!(s.find("a2").unwrap()["position"], json!([4, 0, 2]));
        assert_eq!(s.find("a2").unwrap()["rotation"], json!([0, 90, 0]));
        assert!(s.clone_object("a", "a2", [0.0; 3], 0.0).is_err(), "duplicate id must be refused");
    }

    #[test]
    fn formatter_output_round_trips_and_keeps_key_order() {
        let s = scene();
        let text = format_scene(&s.root);
        let back: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(back, s.root);
        let keys: Vec<&String> = back["objects"][0].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["id", "type", "position", "material"]);
        assert!(text.contains("{ \"id\": \"a\""), "objects stay one-per-line and spaced:\n{text}");
    }
}
