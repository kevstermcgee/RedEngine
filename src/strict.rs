//! Strict scene fields: a key the engine does not know is an **error with a fix**, never silently ignored.
//!
//! The mistake an AI (or a person) makes most often in a JSON scene is a plausible-looking field that does
//! nothing: `"pos"` for `"position"`, `"color"` on an object instead of `material.color`, `"raduis"`. A parser
//! that skips unknown keys turns each into an invisible bug. Here every section has an allow-list, and a stray
//! key reports `path.key: unknown field — did you mean ...`.
//!
//! Notes and tool data belong in the **extension namespace**: any key starting with `_`, `x-` or `x_`, plus
//! `$comment` and `notes`, is always allowed and never interpreted. See `SPEC.md` ("Strict fields").
//! The allow-lists are checked against every example, recipe and catalogue asset by tests, so a key a real
//! scene uses cannot be forgotten here.

use serde_json::{Map, Value};

/// Keys allowed at the scene root.
pub const ROOT_KEYS: &[&str] = &[
    "schema_version",
    "recipe",
    "meta",
    "background",
    "ambient",
    "camera",
    "post",
    "lights",
    "zones",
    "spawns",
    "portals",
    "interest",
    "checks",
    "prefabs",
    "vars",
    "persist",
    "rules",
    "phases",
    "fields",
    "player",
    "jump_pads",
    "weapons",
    "match",
    "race",
    "combat",
    "shooter",
    "bots",
    "nav",
    "music",
    "flashlight",
    "death_text",
    "teams",
    "hud",
    "ui",
    "world",
    "sky",
    "clock",
    "procgen",
    "audio",
    "ocean",
    "objects",
];
/// `meta` keys.
pub const META_KEYS: &[&str] = &["fps", "duration", "resolution"];
/// `background` keys.
pub const BACKGROUND_KEYS: &[&str] = &["sky_top", "sky_bottom", "color"];
/// `ambient` keys.
pub const AMBIENT_KEYS: &[&str] = &["color", "intensity"];
/// `post` keys.
pub const POST_KEYS: &[&str] = &["enabled", "ao", "outline", "ao_radius"];
/// `camera` keys.
pub const CAMERA_KEYS: &[&str] = &["fov", "near", "far", "position", "target", "roll"];
/// Live-player tuning keys.
pub const PLAYER_KEYS: &[&str] = &[
    "humans_play_as",
    "character",
    "fov",
    "walk_speed",
    "sprint_speed",
    "crouch_multiplier",
    "jump_speed",
    "gravity",
    "acceleration",
    "air_acceleration",
    "friction",
    "max_speed",
    "throw_speed",
    "mode",
    "view",
    "fade_in",
];
/// One `jump_pads` entry.
pub const JUMP_PAD_KEYS: &[&str] = &["id", "position", "size", "launch_speed"];
/// `material` keys.
pub const MATERIAL_KEYS: &[&str] = &["color", "metallic", "roughness", "emissive", "opacity"];
/// Point-light keys.
pub const POINT_LIGHT_KEYS: &[&str] = &["id", "type", "position", "color", "intensity", "range"];
/// Directional-light keys.
pub const DIRECTIONAL_LIGHT_KEYS: &[&str] =
    &["id", "type", "direction", "color", "intensity", "cast_shadows", "shadow_radius", "shadow_center", "shadow_follow"];
/// Keys of a humanoid's `pose`.
pub const HUMANOID_POSE_KEYS: &[&str] = &["spine", "head", "l_shoulder", "r_shoulder", "l_elbow", "r_elbow", "l_hip", "r_hip", "l_knee", "r_knee"];
/// Keys of a rat's `pose`.
pub const RAT_POSE_KEYS: &[&str] = &["gait", "stride", "sway"];
/// Keys of one wall `openings` entry.
pub const OPENING_KEYS: &[&str] = &["kind", "at", "width", "height", "sill", "glass", "trim"];
/// Keys of a wall `baseboard` object.
pub const BASEBOARD_KEYS: &[&str] = &["color", "height"];
/// Keys of one fence `gaps` entry.
pub const GAP_KEYS: &[&str] = &["at", "width"];
/// Keys of a prefab instance.
pub const PREFAB_INSTANCE_KEYS: &[&str] =
    &["id", "type", "prefab", "params", "material", "position", "rotation", "scale", "collide", "movable", "lint_ignore", "on_terrain"];

/// Keys every ordinary object may carry.
const COMMON: &[&str] = &["id", "type", "position", "rotation", "scale", "collide", "movable", "lint_ignore", "on_terrain"];

/// The keys allowed on an object of `ty` (`None` for a type this module does not know: the parser reports that itself).
pub fn object_keys(ty: &str) -> Option<Vec<&'static str>> {
    let extra: &[&str] = match ty {
        "box" => &["size", "material"],
        "sphere" => &["radius", "material"],
        "cylinder" | "cone" | "capsule" => &["radius", "height", "material"],
        "plane" => &["size", "material"],
        // A prefab instance expands to a group that remembers where it came from.
        "group" => &["children", "prefab_name", "mount"],
        "humanoid" => &["height", "build", "material", "pose", "skin", "hair", "pants", "shoes", "style"],
        "rat" => &["material", "pose"],
        "prop" => &["prop", "material"],
        "stairs" => &["width", "run", "rise", "steps", "material"],
        "terrain" => crate::terrain::TERRAIN_KEYS,
        "wall" => &["from", "to", "y", "height", "thickness", "material", "openings", "trim", "baseboard", "extend"],
        "fence" => &["points", "closed", "y", "height", "post_spacing", "style", "material", "post_color", "gaps"],
        "array" => &["template", "count", "step", "positions", "rotation_step"],
        "text" => &["text", "height", "depth", "align", "spacing", "line_gap", "backing", "material"],
        "prefab" => return Some(PREFAB_INSTANCE_KEYS.to_vec()),
        _ => return None,
    };
    Some(COMMON.iter().chain(extra.iter()).copied().collect())
}

/// `recipe` block keys (what a known-good example map teaches).
pub const RECIPE_KEYS: &[&str] = &["title", "summary", "teaches", "tips"];
/// One `zones` entry.
pub const ZONE_KEYS: &[&str] = &["id", "rect", "y", "kind"];
/// One `spawns` entry.
pub const SPAWN_KEYS: &[&str] = &["id", "position", "yaw_deg", "group"];
/// One `portals` entry.
pub const PORTAL_KEYS: &[&str] = &["id", "between", "center", "width", "height", "open"];
/// The `interest` block.
pub const INTEREST_KEYS: &[&str] = &["cell_size", "note", "hops"];

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
            let near = crate::prefabs::suggest(key, allowed.iter().copied());
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
                    let near = crate::prefabs::suggest(s, all.iter().copied());
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

/// Checks the data sections that other modules read from the raw JSON (`recipe`, `zones`, `spawns`, `portals`,
/// `interest`): a typo there would otherwise mean "no zone", "default spawn" or "no portal" with no complaint.
pub fn check_sections(errs: &mut Vec<String>, root: &Map<String, Value>) {
    if let Some(r) = root.get("recipe").and_then(Value::as_object) {
        check_keys(errs, "recipe", r, RECIPE_KEYS);
    }
    if let Some(i) = root.get("interest").and_then(Value::as_object) {
        check_keys(errs, "interest", i, INTEREST_KEYS);
    }
    if let Some(m) = root.get("match") {
        if let Err(e) = crate::sim::flow::MatchSettings::from_json(m) {
            errs.push(e);
        }
    }
    if root.contains_key("race") {
        if let Err(e) = crate::sim::race::RaceCourse::from_scene_json(root) {
            errs.push(e);
        }
    }
    for (section, allowed) in [("zones", ZONE_KEYS), ("spawns", SPAWN_KEYS), ("portals", PORTAL_KEYS)] {
        let Some(list) = root.get(section) else { continue };
        let Some(arr) = list.as_array() else {
            errs.push(format!("{section}: must be an array"));
            continue;
        };
        for (i, item) in arr.iter().enumerate() {
            let name = item.get("id").and_then(Value::as_str).map_or_else(|| format!("{section}[{i}]"), |id| format!("{section}[{i}] ({id})"));
            match item.as_object() {
                Some(o) => check_keys(errs, &name, o, allowed),
                None => errs.push(format!("{name}: must be an object")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn errs_for(v: Value, allowed: &[&str]) -> Vec<String> {
        let mut e = Vec::new();
        check_keys(&mut e, "crate_1", v.as_object().unwrap(), allowed);
        e
    }

    #[test]
    fn a_typo_names_the_field_and_the_fix() {
        let allowed = object_keys("box").unwrap();
        let e = errs_for(json!({"id":"b","type":"box","size":[1,1,1],"pos":[0,0,0]}), &allowed);
        assert_eq!(e, vec!["crate_1.pos: unknown field — did you mean `position`?"]);
        let e = errs_for(json!({"id":"b","type":"box","siez":[1,1,1]}), &allowed);
        assert!(e[0].contains("did you mean `size`"), "{e:?}");
    }

    #[test]
    fn color_on_the_object_points_at_material() {
        let e = errs_for(json!({"id":"b","type":"sphere","color":"#f00"}), &object_keys("sphere").unwrap());
        assert!(e[0].contains("\"material\": { \"color\": ... }"), "{e:?}");
    }

    #[test]
    fn the_extension_namespace_is_always_allowed() {
        let allowed = object_keys("prop").unwrap();
        assert!(errs_for(json!({"id":"p","type":"prop","prop":"crate","x-owner":"ai","_todo":1,"notes":"hi","$comment":"c"}), &allowed).is_empty());
    }

    #[test]
    fn an_unrelated_key_lists_what_is_valid() {
        let e = errs_for(json!({"id":"b","type":"box","wibble":1}), &object_keys("box").unwrap());
        assert!(e[0].contains("valid here:") && e[0].contains("size"), "{e:?}");
    }
}
