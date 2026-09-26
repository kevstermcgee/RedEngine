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
    "rules",
    "weapons",
    "match",
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
/// `material` keys.
pub const MATERIAL_KEYS: &[&str] = &["color", "metallic", "roughness", "emissive"];
/// Point-light keys.
pub const POINT_LIGHT_KEYS: &[&str] = &["id", "type", "position", "color", "intensity", "range"];
/// Directional-light keys.
pub const DIRECTIONAL_LIGHT_KEYS: &[&str] = &["id", "type", "direction", "color", "intensity", "cast_shadows", "shadow_radius", "shadow_center"];
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
pub const PREFAB_INSTANCE_KEYS: &[&str] = &["id", "type", "prefab", "params", "material", "position", "rotation", "scale", "collide", "movable", "lint_ignore"];

/// Keys every ordinary object may carry.
const COMMON: &[&str] = &["id", "type", "position", "rotation", "scale", "collide", "movable", "lint_ignore"];

/// The keys allowed on an object of `ty` (`None` for a type this module does not know: the parser reports that itself).
pub fn object_keys(ty: &str) -> Option<Vec<&'static str>> {
    let extra: &[&str] = match ty {
        "box" => &["size", "material"],
        "sphere" => &["radius", "material"],
        "cylinder" | "cone" | "capsule" => &["radius", "height", "material"],
        "plane" => &["size", "material"],
        // A prefab instance expands to a group that remembers where it came from.
        "group" => &["children", "prefab_name", "mount"],
        "humanoid" => &["height", "build", "material", "pose", "skin", "hair", "pants", "shoes"],
        "rat" => &["material", "pose"],
        "prop" => &["prop", "material"],
        "stairs" => &["width", "run", "rise", "steps", "material"],
        "wall" => &["from", "to", "y", "height", "thickness", "material", "openings", "trim", "baseboard", "extend"],
        "fence" => &["points", "closed", "y", "height", "post_spacing", "style", "material", "post_color", "gaps"],
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
