//! Typed definitions of every content format (ADR 2026-10-11-content-formats-are-typed-and-checked-twice): one `serde` struct per JSON shape,
//! `deny_unknown_fields`, so each format is self-describing (its JSON Schema derives with the `schema` feature) and a second,
//! independent definition of what the hand-written checkers accept. The checkers stay the source of the error messages; `tests/content_drift.rs`
//! fails when the two disagree about any content file in the repository or about any key in it.
//!
//! The extension namespace (`x-*`, `_*`, `notes`, `$comment`) is allowed anywhere and never interpreted, exactly as the checkers allow it:
//! [`strip_extensions`] removes it before a typed deserialize.

pub mod blueprint;
pub mod checks;
pub mod common;
pub mod drivers;
pub mod game2d;
#[cfg(feature = "schema")]
pub mod json_schema;
pub mod museum;
pub mod objects;
pub mod packs;
pub mod playscript;
pub mod prefab;
pub mod project;
pub mod scene;
pub mod score;
pub mod trace;
pub mod validate;
pub mod voice;

use serde_json::Value;

/// Removes every extension key (`x-*`, `x_*`, `_*`, `notes`, `$comment`) at any depth, as the checkers ignore them.
pub fn strip_extensions(v: &mut Value) {
    match v {
        Value::Object(o) => {
            o.retain(|k, _| !crate::fields::is_extension_key(k));
            o.values_mut().for_each(strip_extensions);
        }
        Value::Array(a) => a.iter_mut().for_each(strip_extensions),
        _ => {}
    }
}

/// Deserializes `v` (extension keys stripped first) as `T`; the error names the JSON path to the bad field (`packs[3].tag: unknown field ...`).
pub fn typed<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, String> {
    let mut v = v.clone();
    strip_extensions(&mut v);
    serde_path_to_error::deserialize(v).map_err(|e| {
        let path = e.path().to_string();
        let path = if path == "." { String::new() } else { path };
        format!("{}{}", if path.is_empty() { String::new() } else { format!("{path}: ") }, e.into_inner())
    })
}
