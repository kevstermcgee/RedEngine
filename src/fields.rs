//! Generic field validation for hand-written JSON: an unknown key is an error with a fix (`check_keys`), a value of the wrong type is an error that says what was
//! expected and found (`Ty`, `Field`, `check_fields`), and notes live in the extension namespace. Scene-specific tables stay in `strict.rs`; this file depends only on
//! `serde_json` and `crate::suggest`, so the 2D game crate (`crates/red2d`) includes it unchanged and the two validate with one vocabulary.

use serde_json::{Map, Value};

/// Keys that are always allowed: the extension namespace.
pub fn is_extension_key(k: &str) -> bool {
    k.starts_with('_') || k.starts_with("x-") || k.starts_with("x_") || k == "$comment" || k == "notes"
}

/// Common wrong spellings and the right place for the value (checked before edit distance).
fn alias(key: &str, allowed: &[&str]) -> Option<String> {
    let direct = match key {
        "pos" | "loc" | "location" | "translation" => Some("position"),
        "rot" | "euler" | "angle" | "angles" => Some("rotation"),
        "scl" => Some("scale"),
        "kids" | "child" | "items" => Some("children"),
        "w" | "d" | "l" => Some("size"),
        "r" => Some("radius"),
        "h" => Some("height"),
        "field_of_view" | "fov_deg" => Some("fov"),
        _ => None,
    };
    if let Some(d) = direct.filter(|d| allowed.contains(d)) {
        return Some(format!("did you mean `{d}`?"));
    }
    if ["color", "colour", "roughness", "metallic", "emissive"].contains(&key) && allowed.contains(&"material") {
        let name = if key == "colour" { "color" } else { key };
        return Some(format!("material properties go inside `material`: \"material\": {{ \"{name}\": ... }}"));
    }
    None
}

/// Pushes `path.key: unknown field ...` for every key of `obj` that is neither in `allowed` nor an extension key.
pub fn check_keys(errs: &mut Vec<String>, path: &str, obj: &Map<String, Value>, allowed: &[&str]) {
    for key in obj.keys() {
        if allowed.contains(&key.as_str()) || is_extension_key(key) {
            continue;
        }
        let fix = alias(key, allowed).or_else(|| {
            let near = crate::suggest::suggest(key, allowed.iter().copied());
            (!near.is_empty()).then(|| format!("did you mean {}?", near.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(" or ")))
        });
        let tail = match fix {
            Some(f) => f,
            None => {
                let shown: Vec<&str> = allowed.iter().copied().filter(|k| !["id", "type"].contains(k)).take(14).collect();
                format!("valid here: {}; prefix a note with `x-` to keep it", shown.join(", "))
            }
        };
        let at = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
        errs.push(format!("{at}: unknown field — {tail}"));
    }
}

/// What a value in an AI-written block must be. A value of the wrong type is an error that says what was expected and what was found: it is never read as a default.
#[derive(Clone, Copy)]
pub enum Ty {
    /// A finite number, optionally within an inclusive range.
    Num(Option<(f64, f64)>),
    /// A whole number, 0 or more.
    Count,
    /// `true` or `false` (never the strings `"true"`/`"false"`).
    Bool,
    /// Any string.
    Str,
    /// One of these strings.
    OneOf(&'static [&'static str]),
    /// `[min, max]`: two numbers, `min <= max`.
    Range,
    /// A list of numbers whose length is one of these (a point is `[x, z]` or `[x, y, z]`).
    Nums(&'static [usize]),
    /// A list of strings.
    Strs,
    /// Anything the function accepts (`Err` is the complaint, which is appended to `path.key: `).
    Custom(fn(&Value) -> Result<(), String>),
}

/// One field of a block: its key, its type, what the number means (shown in the diagnostic) and whether it must be present.
#[derive(Clone, Copy)]
pub struct Field {
    /// The key.
    pub key: &'static str,
    /// What its value must be.
    pub ty: Ty,
    /// A unit or meaning, like `dBFS`, appended to the expectation.
    pub hint: &'static str,
    /// Whether the block is meaningless without it.
    pub required: bool,
}

/// A field that may be left out.
pub const fn opt(key: &'static str, ty: Ty, hint: &'static str) -> Field {
    Field { key, ty, hint, required: false }
}

/// A field that must be there.
pub const fn req(key: &'static str, ty: Ty, hint: &'static str) -> Field {
    Field { key, ty, hint, required: true }
}

/// A short description of a JSON value for a diagnostic: `string "-3"`, `number 5`, `list of 3`, `null`.
pub fn describe_value(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => format!("boolean {b}"),
        Value::Number(n) => format!("number {n}"),
        Value::String(s) => format!("string \"{s}\""),
        Value::Array(a) => format!("a list of {}", a.len()),
        Value::Object(_) => "an object".into(),
    }
}

impl Ty {
    fn expects(&self, hint: &str) -> String {
        let h = if hint.is_empty() { String::new() } else { format!(" ({hint})") };
        match self {
            Ty::Num(None) => format!("a number{h}"),
            Ty::Num(Some((lo, hi))) => format!("a number from {lo} to {hi}{h}"),
            Ty::Count => format!("a whole number, 0 or more{h}"),
            Ty::Bool => format!("true or false{h}"),
            Ty::Str => format!("a string{h}"),
            Ty::OneOf(all) => format!("one of {}{h}", all.join(", ")),
            Ty::Range => format!("[min, max]: two numbers with min <= max{h}"),
            Ty::Nums(lens) => format!("a list of {} numbers{h}", lens.iter().map(usize::to_string).collect::<Vec<_>>().join(" or ")),
            Ty::Strs => format!("a list of strings{h}"),
            Ty::Custom(_) => format!("a valid value{h}"),
        }
    }

    /// `Err(what is wrong)` when `v` is not what this type asks for.
    pub fn check(&self, v: &Value, hint: &str) -> Result<(), String> {
        let wrong = || format!("expected {}, got {}", self.expects(hint), describe_value(v));
        let finite = |x: &Value| x.as_f64().filter(|n| n.is_finite());
        match self {
            Ty::Num(range) => match finite(v) {
                Some(n) if range.is_none_or(|(lo, hi)| (lo..=hi).contains(&n)) => Ok(()),
                _ => Err(wrong()),
            },
            Ty::Count => v.as_u64().map(|_| ()).ok_or_else(wrong),
            Ty::Bool => v.as_bool().map(|_| ()).ok_or_else(wrong),
            Ty::Str => v.as_str().map(|_| ()).ok_or_else(wrong),
            Ty::OneOf(all) => match v.as_str() {
                Some(s) if all.contains(&s) => Ok(()),
                Some(s) => {
                    let near = crate::suggest::suggest(s, all.iter().copied());
                    let tail = if near.is_empty() {
                        String::new()
                    } else {
                        format!(" — did you mean {}?", near.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(" or "))
                    };
                    Err(format!("`{s}` is not {}{tail}", self.expects(hint)))
                }
                None => Err(wrong()),
            },
            Ty::Range => match v.as_array().map(|a| a.iter().map(finite).collect::<Vec<_>>()).as_deref() {
                Some([Some(lo), Some(hi)]) if lo <= hi => Ok(()),
                _ => Err(wrong()),
            },
            Ty::Nums(lens) => match v.as_array() {
                Some(a) if lens.contains(&a.len()) && a.iter().all(|x| finite(x).is_some()) => Ok(()),
                _ => Err(wrong()),
            },
            Ty::Strs => match v.as_array() {
                Some(a) if a.iter().all(Value::is_string) => Ok(()),
                _ => Err(wrong()),
            },
            Ty::Custom(f) => f(v),
        }
    }
}

/// Validates `obj` against `fields`: an unknown key (with a likely fix), a value of the wrong type (with what was expected and found), and a missing required key are
/// each one message at the exact path. Nothing in a validated block is left to a silent default.
pub fn check_fields(errs: &mut Vec<String>, path: &str, obj: &Map<String, Value>, fields: &[Field]) {
    let keys: Vec<&str> = fields.iter().map(|f| f.key).collect();
    check_keys(errs, path, obj, &keys);
    for f in fields {
        match obj.get(f.key) {
            Some(v) => {
                if let Err(e) = f.ty.check(v, f.hint) {
                    errs.push(format!("{path}.{}: {e}", f.key));
                }
            }
            None if f.required => errs.push(format!("{path}: needs `{}`: {}", f.key, f.ty.expects(f.hint))),
            None => {}
        }
    }
}
