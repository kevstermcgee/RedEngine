//! The shape of a scene's `checks` block: every group, item and field an author may write, with its type.
//!
//! `verify` is how an AI learns whether its game works, so a check that is *ignored* is worse than one that fails: a misspelled field, a number written as a string or an
//! empty group would each leave a green result that proves nothing. Here every field of every group is typed (`"reachable": "false"` is an error, not a silent `true`), an unknown key
//! reports `path.key: unknown field — did you mean ...`, a required field must be there, and a group or item that asserts nothing (`{}`, `[]`, an object check with no
//! assertion) is an error that says so. The audio group has its own table next to its runner ([`super::audio_checks::invalid_fields`]); `sim` scenarios are parsed
//! (and held to the same standard) by `sim::scenario`.

use crate::strict::{check_fields, check_keys, describe_value, opt, req, Field, Ty};
use serde_json::{Map, Value};

/// Keys of a `checks` block.
pub const CHECK_GROUPS: &[&str] = &["lint", "reach", "walk", "objects", "views", "sim", "perf", "nav", "audio"];

fn lint_code(v: &Value) -> Result<(), String> {
    let codes: Vec<&str> = super::describe::LINT_CODES.iter().map(|(c, _, _)| *c).collect();
    let list = v.as_array().ok_or_else(|| format!("expected a list of lint codes, got {}", describe_value(v)))?;
    let mut bad = Vec::new();
    for item in list {
        match item.as_str() {
            Some(c) if codes.contains(&c) => {}
            Some(c) => {
                let near = crate::prefabs::suggest(c, codes.iter().copied());
                bad.push(format!(
                    "`{c}` is not a lint code{} (`describe lint` lists them)",
                    near.first().map_or(String::new(), |n| format!(" — did you mean `{n}`?"))
                ));
            }
            None => bad.push(format!("expected a lint code, got {}", describe_value(item))),
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad.join("; "))
    }
}

const LINT: &[Field] = &[
    opt("max_errors", Ty::Count, "errors allowed, default 0"),
    opt("max_warnings", Ty::Count, "warnings allowed, default unlimited"),
    opt("forbid", Ty::Custom(lint_code), "lint codes that fail the check even when within the budget"),
    opt("ignore", Ty::Custom(lint_code), "lint codes not counted"),
];
const REACH: &[Field] = &[
    req("to", Ty::Nums(&[2, 3]), "[x, z] or [x, y, z]"),
    opt("from", Ty::Nums(&[2]), "[x, z]; default the spawn"),
    opt("from_y", Ty::Num(None), "foot height to start at"),
    opt("why", Ty::Str, "shown in the row's name"),
    opt("phase", Ty::Str, "a scene `phases` entry"),
    opt("reachable", Ty::Bool, "false asserts the point CANNOT be reached; default true"),
];
const WALK: &[Field] = &[
    opt("name", Ty::Str, ""),
    opt("path", Ty::Str, "waypoints as \"x,z x,z ...\""),
    opt("from", Ty::Nums(&[2]), "[x, z]; default the spawn"),
    opt("from_y", Ty::Num(None), "foot height to start at"),
    opt("to", Ty::Nums(&[2]), "[x, z]"),
    opt("to_y", Ty::Num(None), "foot height to end at"),
    opt("auto", Ty::Bool, "plan the route instead of giving `path`"),
    opt("ends_near", Ty::Nums(&[2]), "[x, z], default `to`"),
    opt("tol", Ty::Num(Some((0.0, 100.0))), "metres, default 0.35"),
    opt("floor_y", Ty::Num(None), "the floor height the walk must end on"),
    opt("phase", Ty::Str, "a scene `phases` entry"),
];
const VIEWS: &[Field] = &[
    opt("name", Ty::Str, ""),
    req("eye", Ty::Nums(&[3]), "[x, y, z]"),
    req("at", Ty::Nums(&[3]), "[x, y, z]"),
    opt("fov", Ty::Num(Some((1.0, 179.0))), "degrees, default 70"),
    opt("max_diff", Ty::Num(Some((0.0, 1.0))), "share of pixels allowed to differ from the golden, default 0.01"),
];
const OBJECTS: &[Field] = &[
    opt("exist", Ty::Strs, "object ids that must exist"),
    opt("absent", Ty::Strs, "object ids that must not exist"),
    opt("min_count", Ty::Count, "at least this many objects"),
    opt("max_count", Ty::Count, "at most this many objects"),
    opt("count", Ty::Custom(object_counts), "[{ kind, min?, max? }]: objects of a kind"),
];
const COUNT_ITEM: &[Field] = &[req("kind", Ty::Str, "a type, prop or prefab name"), opt("min", Ty::Count, ""), opt("max", Ty::Count, "")];
const NAV: &[Field] = &[opt("max_failures", Ty::Count, "default 0")];

fn object_counts(v: &Value) -> Result<(), String> {
    let list = v.as_array().ok_or_else(|| format!("expected a list of {{\"kind\": ..., \"min\": ...}}, got {}", describe_value(v)))?;
    let mut errs = Vec::new();
    for (i, item) in list.iter().enumerate() {
        match item.as_object() {
            Some(o) => {
                check_fields(&mut errs, &format!("[{i}]"), o, COUNT_ITEM);
                if !o.contains_key("min") && !o.contains_key("max") {
                    errs.push(format!("[{i}]: asserts nothing; give `min` and/or `max`"));
                }
            }
            None => errs.push(format!("[{i}]: expected an object, got {}", describe_value(item))),
        }
    }
    if list.is_empty() {
        errs.push("is empty".into());
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs.join("; "))
    }
}

/// Validates a list-of-objects group: not empty, each item an object against `fields`, and `rule` (a complaint about an item that asserts nothing).
fn each(errs: &mut Vec<String>, checks: &Map<String, Value>, group: &str, fields: &[Field], rule: fn(&Map<String, Value>) -> Option<String>) {
    let Some(v) = checks.get(group) else { return };
    let Some(list) = v.as_array() else {
        errs.push(format!("checks.{group}: expected a list of objects, got {}", describe_value(v)));
        return;
    };
    if list.is_empty() {
        errs.push(format!("checks.{group}: is empty, so it checks nothing; add an item or remove it"));
    }
    for (i, item) in list.iter().enumerate() {
        let path = format!("checks.{group}[{i}]");
        match item.as_object() {
            Some(o) => {
                check_fields(errs, &path, o, fields);
                if let Some(e) = rule(o) {
                    errs.push(format!("{path}: {e}"));
                }
            }
            None => errs.push(format!("{path}: expected an object, got {}", describe_value(item))),
        }
    }
}

/// Everything wrong with a `checks` block that can be known without running a single check, as `path: message` lines (empty = well formed). `audio` is not included:
/// its runner validates it (so the complaint appears once).
pub fn invalid_fields(checks: &Value) -> Vec<String> {
    let mut errs = Vec::new();
    let Some(root) = checks.as_object() else {
        return vec![format!("checks: expected an object like {{\"lint\": {{\"max_errors\": 0}}}}, got {}", describe_value(checks))];
    };
    check_keys(&mut errs, "checks", root, CHECK_GROUPS);
    if !CHECK_GROUPS.iter().any(|g| root.contains_key(*g)) {
        errs.push("checks: has no group, so it proves nothing; at minimum add {\"lint\": {\"max_errors\": 0}} (groups: lint, reach, walk, objects, views, sim, perf, nav, audio)".into());
    }
    if let Some(v) = root.get("lint") {
        match v.as_object() {
            Some(o) => check_fields(&mut errs, "checks.lint", o, LINT),
            None => errs.push(format!("checks.lint: expected an object like {{\"max_errors\": 0}}, got {}", describe_value(v))),
        }
    }
    if let Some(v) = root.get("nav") {
        match v.as_object() {
            Some(o) => check_fields(&mut errs, "checks.nav", o, NAV),
            None => errs.push(format!("checks.nav: expected an object like {{\"max_failures\": 0}}, got {}", describe_value(v))),
        }
    }
    if let Some(v) = root.get("perf") {
        match v.as_object() {
            Some(o) => check_keys(&mut errs, "checks.perf", o, super::perf::PERF_KEYS),
            None => errs.push(format!("checks.perf: expected an object of budgets, got {}", describe_value(v))),
        }
    }
    each(&mut errs, root, "reach", REACH, |_| None);
    each(&mut errs, root, "walk", WALK, |o| {
        (!o.contains_key("path") && !o.contains_key("to")).then(|| "asserts nothing: give `to` (and `auto`) or a `path`".to_string())
    });
    each(&mut errs, root, "views", VIEWS, |_| None);
    if let Some(v) = root.get("sim") {
        match v.as_array() {
            Some(list) if list.is_empty() => errs.push("checks.sim: is empty, so it checks nothing; add a scenario or remove it".into()),
            Some(list) => {
                for (i, item) in list.iter().enumerate().filter(|(_, it)| !it.is_object()) {
                    errs.push(format!("checks.sim[{i}]: expected a scenario object, got {}", describe_value(item)));
                }
            }
            None => errs.push(format!("checks.sim: expected a list of scenarios, got {}", describe_value(v))),
        }
    }
    if let Some(v) = root.get("objects") {
        match v.as_object() {
            Some(o) => {
                check_fields(&mut errs, "checks.objects", o, OBJECTS);
                if !["exist", "absent", "min_count", "max_count", "count"].iter().any(|k| o.contains_key(*k)) {
                    errs.push("checks.objects: asserts nothing; give `exist`, `absent`, `min_count`, `max_count` or `count`".into());
                }
            }
            None => errs.push(format!("checks.objects: expected an object like {{\"exist\": [\"door\"]}}, got {}", describe_value(v))),
        }
    }
    errs
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Messages for a block, joined: what an author reads.
    fn problems(checks: Value) -> String {
        invalid_fields(&checks).join(" | ")
    }

    #[test]
    fn a_well_formed_block_has_no_problems() {
        let good = json!({
            "lint": {"max_errors": 0, "max_warnings": 3, "forbid": ["leak"], "ignore": ["carry"]},
            "reach": [{"to": [3.0, 4.0], "why": "gate", "reachable": false}, {"to": [1, 2, 3], "from": [0, 0]}],
            "walk": [{"name": "a", "to": [3, 4], "auto": true, "tol": 0.5}, {"path": "0,0 1,1"}],
            "views": [{"eye": [0, 2, 5], "at": [0, 1, 0], "fov": 60}],
            "objects": {"exist": ["door"], "min_count": 2, "count": [{"kind": "box", "min": 1}]},
            "sim": [{"name": "x"}], "nav": {"max_failures": 0}, "perf": {"secs": 5}
        });
        assert_eq!(problems(good), "");
    }

    /// The plausible mistakes: each used to be silently ignored (or read as a default) and is now one clear message at its path.
    #[test]
    fn plausible_mistakes_are_each_reported_where_they_are() {
        let cases = [
            ("a misspelled group", json!({"lint": {}, "reech": []}), "checks.reech: unknown field"),
            (
                "a misspelled field",
                json!({"reach": [{"to": [1, 2], "reachble": false}]}),
                "checks.reach[0].reachble: unknown field — did you mean `reachable`?",
            ),
            (
                "a boolean as a string",
                json!({"reach": [{"to": [1, 2], "reachable": "false"}]}),
                "checks.reach[0].reachable: expected true or false (false asserts the point CANNOT be reached; default true), got string \"false\"",
            ),
            ("a count as a string", json!({"lint": {"max_warnings": "5"}}), "checks.lint.max_warnings: expected a whole number"),
            ("a negative count", json!({"lint": {"max_errors": -1}}), "checks.lint.max_errors: expected a whole number"),
            ("a lint code typo", json!({"lint": {"forbid": ["leek"]}}), "`leek` is not a lint code — did you mean `leak`?"),
            ("a lint code in the wrong shape", json!({"lint": {"ignore": "leak"}}), "checks.lint.ignore: expected a list of lint codes, got string \"leak\""),
            ("a target with the wrong length", json!({"reach": [{"to": [1]}]}), "checks.reach[0].to: expected a list of 2 or 3 numbers"),
            (
                "a target as a string",
                json!({"reach": [{"to": "3,4"}]}),
                "checks.reach[0].to: expected a list of 2 or 3 numbers ([x, z] or [x, y, z]), got string \"3,4\"",
            ),
            ("a missing target", json!({"reach": [{"why": "gate"}]}), "checks.reach[0]: needs `to`"),
            ("a walk with neither path nor target", json!({"walk": [{"name": "w"}]}), "checks.walk[0]: asserts nothing"),
            ("a tolerance as a string", json!({"walk": [{"to": [1, 2], "tol": "0.5"}]}), "checks.walk[0].tol: expected a number"),
            ("a view without a camera", json!({"views": [{"name": "v"}]}), "checks.views[0]: needs `eye`"),
            (
                "a diff above 1",
                json!({"views": [{"eye": [0, 1, 2], "at": [0, 0, 0], "max_diff": 5}]}),
                "checks.views[0].max_diff: expected a number from 0 to 1",
            ),
            ("an object check that asserts nothing", json!({"objects": {}}), "checks.objects: asserts nothing"),
            ("a count with neither min nor max", json!({"objects": {"count": [{"kind": "box"}]}}), "asserts nothing; give `min` and/or `max`"),
            ("a count without a kind", json!({"objects": {"count": [{"min": 1}]}}), "needs `kind`"),
            ("an empty list", json!({"reach": []}), "checks.reach: is empty, so it checks nothing"),
            ("an empty scenario list", json!({"sim": []}), "checks.sim: is empty"),
            ("a list that is a lone object", json!({"walk": {"to": [1, 2]}}), "checks.walk: expected a list of objects, got an object"),
            ("a block with no group", json!({}), "checks: has no group"),
            ("a note-only block", json!({"x-note": "later"}), "checks: has no group"),
            ("not an object", json!([1]), "checks: expected an object"),
        ];
        for (what, block, want) in cases {
            let got = problems(block);
            assert!(got.contains(want), "{what}: wanted `{want}` in `{got}`");
        }
    }

    #[test]
    fn extension_keys_are_notes_and_do_not_count_as_checks() {
        assert_eq!(problems(json!({"lint": {"x-why": "keep it clean"}, "_todo": 1})), "");
    }
}
