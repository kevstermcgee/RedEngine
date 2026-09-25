//! `red_engine2 patch`: many edits, one atomic, validated call.
//!
//! The single-edit commands (`set`, `move`, `add`, `rm`, ...) validate after every call, which is safe but costs an AI one
//! round trip per change. A patch is a JSON list of operations applied in order to an in-memory copy of the scene; the scene
//! is validated once at the end and written only if it is still valid, so a patch either lands completely or not at all.
//! Unknown fields in an operation are errors (with did-you-mean), and every error names the operation (`ops[3] (move): ...`).
//!
//! ```json
//! [ {"op": "add",    "object": {"id": "crate_9", "type": "prop", "prop": "crate", "position": [2, 0, 3]}},
//!   {"op": "set",    "id": "crate_9", "values": {"material.color": "#aa3322", "rotation": [0, 45, 0]}},
//!   {"op": "move",   "id": "lamp_a", "by": [0, -0.2, 0]},
//!   {"op": "clone",  "id": "crate_9", "new": "crate_10", "by": [1.2, 0, 0]},
//!   {"op": "rename", "id": "crate_10", "to": "crate_spare"},
//!   {"op": "rm",     "id": "old_*"} ]
//! ```

use super::edit::SceneFile;
use crate::strict::check_keys;
use serde_json::Value;

fn vec3(v: &Value, what: &str) -> Result<[f64; 3], String> {
    let a = v.as_array().filter(|a| a.len() == 3).ok_or_else(|| format!("{what} must be [x, y, z]"))?;
    let mut out = [0.0; 3];
    for (i, x) in a.iter().enumerate() {
        out[i] = x.as_f64().ok_or_else(|| format!("{what}[{i}] is not a number"))?;
    }
    Ok(out)
}

fn need<'a>(op: &'a Value, key: &str) -> Result<&'a str, String> {
    op.get(key).and_then(Value::as_str).ok_or_else(|| format!("needs \"{key}\": a string"))
}

/// Applies one operation.
fn apply_one(f: &mut SceneFile, op: &Value) -> Result<(), String> {
    let obj = op.as_object().ok_or("each operation must be an object with an \"op\"")?;
    let kind = need(op, "op")?;
    let mut errs = Vec::new();
    let allowed: &[&str] = match kind {
        "add" => &["op", "object", "objects", "into"],
        "set" => &["op", "id", "values"],
        "move" => &["op", "id", "by", "to"],
        "rm" => &["op", "id", "ids"],
        "clone" => &["op", "id", "new", "by", "rot_y"],
        "rename" => &["op", "id", "to"],
        other => return Err(format!("unknown op '{other}' (use add, set, move, rm, clone or rename)")),
    };
    check_keys(&mut errs, "", obj, allowed);
    if let Some(e) = errs.into_iter().next() {
        return Err(e);
    }
    match kind {
        "add" => {
            let into = op.get("into").and_then(Value::as_str).unwrap_or("objects");
            let list: Vec<Value> = match (op.get("object"), op.get("objects")) {
                (Some(o), None) => vec![o.clone()],
                (None, Some(Value::Array(a))) => a.clone(),
                _ => return Err("give exactly one of \"object\" (one) or \"objects\" (a list)".into()),
            };
            for o in list {
                f.add(into, o)?;
            }
        }
        "set" => {
            let id = need(op, "id")?;
            let values = op.get("values").and_then(Value::as_object).ok_or("needs \"values\": {\"path\": value, ...}")?;
            for (path, v) in values {
                f.set(id, &format!("{path}={v}"))?;
            }
        }
        "move" => {
            let id = need(op, "id")?;
            match (op.get("by"), op.get("to")) {
                (Some(b), None) => f.translate(id, vec3(b, "by")?)?,
                (None, Some(t)) => f.place_at(id, vec3(t, "to")?)?,
                _ => return Err("give exactly one of \"by\" [dx,dy,dz] or \"to\" [x,y,z]".into()),
            }
        }
        "rm" => {
            let mut pats: Vec<&str> = op.get("ids").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            if let Some(id) = op.get("id").and_then(Value::as_str) {
                pats.push(id);
            }
            if pats.is_empty() {
                return Err("needs \"id\" (an id or glob) or \"ids\"".into());
            }
            for p in pats {
                if f.remove(p) == 0 {
                    return Err(format!("nothing matches '{p}'"));
                }
            }
        }
        "clone" => {
            let by = op.get("by").map(|b| vec3(b, "by")).transpose()?.unwrap_or([0.0; 3]);
            f.clone_object(need(op, "id")?, need(op, "new")?, by, op.get("rot_y").and_then(Value::as_f64).unwrap_or(0.0))?;
        }
        "rename" => f.rename(need(op, "id")?, need(op, "to")?)?,
        _ => unreachable!("op kinds are matched above"),
    }
    Ok(())
}

/// Applies a patch (a list of ops, or `{"ops": [...]}`) to `f` in order; stops at the first failing op and names it.
pub fn apply(f: &mut SceneFile, patch: &Value) -> Result<String, String> {
    let ops = match patch {
        Value::Array(a) => a,
        Value::Object(o) => o.get("ops").and_then(Value::as_array).ok_or("a patch is a JSON list of operations (or {\"ops\": [...]})")?,
        _ => return Err("a patch is a JSON list of operations".into()),
    };
    if ops.is_empty() {
        return Err("the patch has no operations".into());
    }
    for (i, op) in ops.iter().enumerate() {
        let kind = op.get("op").and_then(Value::as_str).unwrap_or("?");
        apply_one(f, op).map_err(|e| format!("ops[{i}] ({kind}): {e}"))?;
    }
    Ok(format!("applied {} operation(s)", ops.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene_file(name: &str) -> (std::path::PathBuf, SceneFile) {
        let dir = std::env::temp_dir().join(format!("re2_patch_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.json");
        std::fs::write(
            &p,
            r##"{"camera":{"position":[0,1.7,-3]},"objects":[
              {"id":"floor","type":"plane","size":[10,10],"position":[0,0.01,0]},
              {"id":"old_1","type":"box","size":[1,1,1],"position":[3,0.5,3]},
              {"id":"old_2","type":"box","size":[1,1,1],"position":[-3,0.5,3]}]}"##,
        )
        .unwrap();
        (p.clone(), SceneFile::load(&p).unwrap())
    }

    #[test]
    fn a_patch_applies_every_kind_of_edit_in_order() {
        let (p, mut f) = scene_file("ok");
        let patch: Value = serde_json::from_str(
            r##"[{"op":"add","object":{"id":"crate_9","type":"prop","prop":"crate","position":[2,0,-2]}},
                 {"op":"set","id":"crate_9","values":{"material.color":"#aa3322","rotation":[0,45,0]}},
                 {"op":"clone","id":"crate_9","new":"crate_10","by":[1.5,0,0]},
                 {"op":"rename","id":"crate_10","to":"crate_spare"},
                 {"op":"move","id":"crate_spare","by":[0,0,1]},
                 {"op":"rm","id":"old_*"}]"##,
        )
        .unwrap();
        assert_eq!(apply(&mut f, &patch).unwrap(), "applied 6 operation(s)");
        f.save(&p, false, false).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("crate_spare") && text.contains("#aa3322") && !text.contains("old_1") && !text.contains("old_2"), "{text}");
    }

    #[test]
    fn a_failing_op_names_itself_and_nothing_is_written() {
        let (p, mut f) = scene_file("bad");
        let before = std::fs::read_to_string(&p).unwrap();
        let patch: Value = serde_json::from_str(r##"[{"op":"move","id":"old_1","by":[1,0,0]},{"op":"move","id":"ghost","by":[1,0,0]}]"##).unwrap();
        let e = apply(&mut f, &patch).unwrap_err();
        assert!(e.starts_with("ops[1] (move)") && e.contains("ghost"), "{e}");
        // The caller only saves on Ok, so the file on disk is untouched.
        assert_eq!(std::fs::read_to_string(&p).unwrap(), before);
        for (bad, want) in [
            (r##"[{"op":"teleport"}]"##, "unknown op"),
            (r##"[{"op":"move","id":"old_1","byy":[1,0,0]}]"##, "unknown field"),
            (r##"[{"op":"add"}]"##, "exactly one"),
            (r##"[{"op":"rm","id":"zzz_*"}]"##, "nothing matches"),
            (r##"[]"##, "no operations"),
        ] {
            let e = apply(&mut f, &serde_json::from_str(bad).unwrap()).unwrap_err();
            assert!(e.contains(want), "{bad}: {e}");
        }
    }
}
