//! The museum collection's typed definition (`frameless::content::Collection`) and its hand-written checker (`museum::spec::parse_collection`) agree,
//! as the engine's `content_drift` checks its own formats: both accept the collection Frameless ships, and an unknown key put into any object of it
//! is refused by both or by neither. It moved here with the museum (ADR 2026-10-14-a-game-ships-as-its-own-binary-the-client-is-a-library).

use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

const PROBE: &str = "zz_drift_probe";

/// Every object in `v` (the root included) as a path of keys and indices, one per distinct shape (an array's elements are alike).
fn object_paths(v: &Value) -> Vec<(Vec<Value>, String)> {
    fn go(v: &Value, path: &mut Vec<Value>, sig: String, out: &mut Vec<(Vec<Value>, String)>) {
        match v {
            Value::Object(o) => {
                out.push((path.clone(), sig.clone()));
                for (k, x) in o {
                    if red_engine2::fields::is_extension_key(k) {
                        continue;
                    }
                    path.push(Value::from(k.as_str()));
                    go(x, path, format!("{sig}.{k}"), out);
                    path.pop();
                }
            }
            Value::Array(a) => {
                if let Some(x) = a.first() {
                    path.push(Value::from(0));
                    go(x, path, format!("{sig}[]"), out);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    go(v, &mut Vec::new(), String::new(), &mut out);
    out
}

fn with_probe(v: &Value, path: &[Value]) -> Value {
    let mut out = v.clone();
    let mut node = &mut out;
    for s in path {
        node = match s {
            Value::String(k) => &mut node[k.as_str()],
            i => &mut node[i.as_u64().unwrap_or(0) as usize],
        };
    }
    if let Some(o) = node.as_object_mut() {
        o.insert(PROBE.to_string(), Value::from(1));
    }
    out
}

#[test]
fn the_collections_typed_definition_and_its_checker_agree() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("content/collection.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let checker = |v: &Value| frameless::museum::spec::parse_collection(v, None).is_ok();
    let typed = |v: &Value| red_engine2::content::typed::<frameless::content::Collection>(v).is_ok();
    assert!(checker(&v), "the checker accepts the collection Frameless ships");
    assert!(typed(&v), "{:?}", red_engine2::content::typed::<frameless::content::Collection>(&v).err());
    let mut problems = Vec::new();
    let mut seen = BTreeSet::new();
    for (path, sig) in object_paths(&v) {
        if !seen.insert(sig.clone()) {
            continue;
        }
        let m = with_probe(&v, &path);
        let (c, t) = (checker(&m), typed(&m));
        if c != t {
            problems.push(format!(
                "an unknown key at `{}` is {} by the checker but {} by the typed definition",
                if sig.is_empty() { "<root>" } else { &sig },
                if c { "accepted" } else { "refused" },
                if t { "accepted" } else { "refused" }
            ));
        }
    }
    assert!(seen.len() > 3, "the probe reached the collection's objects ({} shapes)", seen.len());
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
