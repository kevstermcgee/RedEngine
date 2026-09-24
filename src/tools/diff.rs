//! `red_engine2 diff a.json b.json` (or `diff scene.json --git` against `HEAD`): a *semantic*
//! diff of two scenes — objects/lights/zones added, removed or changed **by id**, with the exact
//! fields that changed. Line diffs of scene JSON are useless after the edit tools re-format a
//! file; this is what to read when reviewing what an AI (or you) changed.

use serde_json::Value;
use std::collections::BTreeMap;

fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => (x.as_f64().unwrap_or(0.0) - y.as_f64().unwrap_or(0.0)).abs() < 1e-6,
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q)),
        (Value::Object(x), Value::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w))),
        _ => a == b,
    }
}

fn short(v: &Value) -> String {
    let s = serde_json::to_string(v).unwrap_or_default();
    if s.chars().count() > 60 {
        format!("{}…", s.chars().take(60).collect::<String>())
    } else {
        s
    }
}

/// Dotted paths that differ between `a` and `b` (stops descending at arrays of non-objects).
fn changes(path: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
    if same(a, b) {
        return;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                match (x.get(k), y.get(k)) {
                    (Some(p1), Some(p2)) => changes(&p, p1, p2, out),
                    (Some(p1), None) => out.push(format!("{p}: {} -> (removed)", short(p1))),
                    (None, Some(p2)) => out.push(format!("{p}: (new) {}", short(p2))),
                    (None, None) => {}
                }
            }
        }
        _ => out.push(format!("{path}: {} -> {}", short(a), short(b))),
    }
}

fn by_id(v: &Value, key: &str) -> BTreeMap<String, Value> {
    v.get(key).and_then(Value::as_array).map(|a| a.iter().enumerate().map(|(i, o)| (o.get("id").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("#{i}")), o.clone())).collect()).unwrap_or_default()
}

/// Result of a semantic scene diff: human-readable text and whether anything changed.
pub struct Diff {
    pub text: String,
    pub changed: bool,
}

/// Diffs two scene JSON values by object id, ignoring formatting and float noise.
pub fn diff(a: &Value, b: &Value) -> Diff {
    let mut text = String::new();
    let mut any = false;
    for key in ["objects", "lights", "zones"] {
        let (ma, mb) = (by_id(a, key), by_id(b, key));
        let added: Vec<&String> = mb.keys().filter(|k| !ma.contains_key(*k)).collect();
        let removed: Vec<&String> = ma.keys().filter(|k| !mb.contains_key(*k)).collect();
        let mut modified: Vec<(String, Vec<String>)> = Vec::new();
        for (id, va) in &ma {
            if let Some(vb) = mb.get(id) {
                let mut ch = Vec::new();
                changes("", va, vb, &mut ch);
                if !ch.is_empty() {
                    modified.push((id.clone(), ch));
                }
            }
        }
        if added.is_empty() && removed.is_empty() && modified.is_empty() {
            continue;
        }
        any = true;
        text.push_str(&format!("{key}: +{} -{} ~{}\n", added.len(), removed.len(), modified.len()));
        for id in &added {
            let kind = mb[*id].get("type").and_then(Value::as_str).unwrap_or("light/zone");
            let extra = mb[*id].get("prop").or_else(|| mb[*id].get("prefab")).and_then(Value::as_str).map(|p| format!(":{p}")).unwrap_or_default();
            text.push_str(&format!("  + {id}  ({kind}{extra})\n"));
        }
        for id in &removed {
            text.push_str(&format!("  - {id}\n"));
        }
        for (id, ch) in &modified {
            text.push_str(&format!("  ~ {id}\n"));
            for c in ch.iter().take(6) {
                text.push_str(&format!("      {c}\n"));
            }
            if ch.len() > 6 {
                text.push_str(&format!("      … {} more field(s)\n", ch.len() - 6));
            }
        }
    }
    // everything else at the top level
    let (oa, ob) = (a.as_object(), b.as_object());
    if let (Some(oa), Some(ob)) = (oa, ob) {
        let mut keys: Vec<&String> = oa.keys().chain(ob.keys()).filter(|k| !["objects", "lights", "zones"].contains(&k.as_str())).collect();
        keys.sort();
        keys.dedup();
        for k in keys {
            let mut ch = Vec::new();
            match (oa.get(k), ob.get(k)) {
                (Some(x), Some(y)) => changes(k, x, y, &mut ch),
                (Some(x), None) => ch.push(format!("{k}: {} -> (removed)", short(x))),
                (None, Some(y)) => ch.push(format!("{k}: (new) {}", short(y))),
                _ => {}
            }
            for c in ch.into_iter().take(6) {
                any = true;
                text.push_str(&format!("scene {c}\n"));
            }
        }
    }
    if !any {
        text.push_str("no semantic differences\n");
    }
    Diff { text, changed: any }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reports_added_removed_and_changed_fields_by_id() {
        let a = json!({"camera": {"fov": 50}, "objects": [{"id": "a", "type": "box", "position": [0, 0, 0]}, {"id": "b", "type": "prop", "prop": "crate"}]});
        let b = json!({"camera": {"fov": 60}, "objects": [{"id": "a", "type": "box", "position": [0, 1, 0]}, {"id": "c", "type": "prefab", "prefab": "apple_red"}]});
        let d = diff(&a, &b);
        assert!(d.changed);
        assert!(d.text.contains("+ c  (prefab:apple_red)") && d.text.contains("- b") && d.text.contains("~ a"), "{}", d.text);
        assert!(d.text.contains("position: [0,0,0] -> [0,1,0]") || d.text.contains("position: "), "{}", d.text);
        assert!(d.text.contains("scene camera.fov: 50 -> 60"), "{}", d.text);
    }

    #[test]
    fn formatting_and_float_noise_are_not_differences() {
        let a = json!({"objects": [{"id": "a", "position": [1.0, 2.0, 3.0]}]});
        let b = json!({"objects": [{"id": "a", "position": [1, 2.0000000001, 3]}]});
        assert!(!diff(&a, &b).changed);
    }
}
