//! `style-report GAME`: how much of a game's look is its own. For a 3D scene: what share of the placed objects come from the shared catalogue, how many of
//! those the palette re-skins, how many are the game's own prefabs or hand-built parts, how far its palette is from the defaults, what share of surfaces are
//! still default colours, its voxel models and sound files, and what `lint` says it kept at the defaults. For a 2D game: its sprites, sounds and colours.
//! `--sheet` draws the game's own prefabs in its style. The numbers feed the Idea Forge game score (ADR 2026-10-10-a-game-style-block-re-skins-shared-prefabs).

use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The measured look of one game.
#[derive(Debug, Clone, Serialize, Default)]
pub struct StyleReport {
    /// `3d` or `2d`.
    pub kind: String,
    /// Authored objects (top-level and inside groups; a prefab's own parts are not counted).
    pub placed: usize,
    /// Of those, instances of catalogue prefabs and built-in props: the shared look.
    pub shared: usize,
    /// Shared instances with at least one param the palette re-skins (bound to a role the palette names).
    pub shared_reskinned: usize,
    /// Instances of the game's own prefabs (its scene's `prefabs` block).
    pub own_prefab_instances: usize,
    /// Primitives, walls, groups and other parts the game built itself.
    pub hand_built: usize,
    /// The game's own prefab definitions.
    pub own_prefabs: Vec<String>,
    /// `shared / placed` (0..1).
    pub shared_share: f32,
    /// Roles in `style.palette`.
    pub palette_roles: usize,
    /// Roles whose colour is not (perceptibly) the default for that role.
    pub palette_custom_roles: usize,
    /// Mean distance (redmean, 0..765) of each palette role from its default colour (or the nearest default, for a role of the game's own); 0 without a palette.
    pub palette_distance: f32,
    /// Surfaces (leaf primitives with a material) and how many of them are default colours.
    pub surfaces: usize,
    pub default_surfaces: usize,
    /// Distinct sprites: voxel models (3D) or `sprites` (2D).
    pub sprites: usize,
    /// Sound files the game brings: score files under its folder and those its `audio` names (3D), or its `sounds` and `music` (2D).
    pub sounds: usize,
    /// What `lint` reports the game kept at the defaults (`style-palette`, `style-lamp`, `style-sky`).
    pub defaults_kept: Vec<String>,
}

fn prefab_names(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(|d| d.get("name").and_then(Value::as_str).map(str::to_string)).collect(),
        Some(Value::Object(o)) => o.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

/// Measures the game at `path` (a 3D scene or a `.game2d.json`).
pub fn report(path: &Path) -> Result<StyleReport, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let raw: Value = serde_json::from_str(&text).map_err(|e| format!("{}: invalid JSON: {e}", path.display()))?;
    if raw.get("game2d").is_some() || path.to_string_lossy().ends_with(".game2d.json") {
        return Ok(report_2d(&raw));
    }
    let scene = crate::scene_blocks::parse_scene_in(&text, path.parent()).map_err(|e| format!("{}: {}", path.display(), e.join("; ")))?;
    let mut r = StyleReport { kind: "3d".into(), ..Default::default() };
    let palette = crate::style::palette_of(&raw);
    r.own_prefabs = prefab_names(raw.get("prefabs"));
    let own: BTreeSet<&str> = r.own_prefabs.iter().map(String::as_str).collect();
    let lib = &crate::prefabs::builtin().0;
    let mut voxel_models: BTreeSet<String> = BTreeSet::new();
    let mut stack: Vec<&Value> = raw.get("objects").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    while let Some(o) = stack.pop() {
        r.placed += 1;
        match o.get("type").and_then(Value::as_str).unwrap_or("") {
            "prefab" => {
                let name = o.get("prefab").and_then(Value::as_str).unwrap_or("");
                if own.contains(name) {
                    r.own_prefab_instances += 1;
                } else {
                    r.shared += 1;
                    let reskinned = lib.find(name).is_some_and(|d| d.params.iter().any(|p| p.palette.as_ref().is_some_and(|role| palette.contains_key(role))))
                        || o.get("params").and_then(Value::as_object).is_some_and(|p| !p.is_empty());
                    if reskinned {
                        r.shared_reskinned += 1;
                    }
                }
            }
            "prop" => r.shared += 1,
            "voxels" => {
                r.hand_built += 1;
                voxel_models.insert(o.get("rows").map(Value::to_string).unwrap_or_default());
            }
            "group" => {
                r.placed -= 1; // a group is a container: its children are what is placed
                stack.extend(o.get("children").and_then(Value::as_array).into_iter().flatten());
            }
            _ => r.hand_built += 1,
        }
    }
    // Voxel models defined inside the game's own prefabs count too: that is where a game keeps a model it places many times.
    for def in raw.get("prefabs").map(|p| p.to_string()).into_iter() {
        for (i, _) in def.match_indices("\"voxels\"") {
            voxel_models.insert(format!("prefab-voxels-{i}"));
        }
    }
    r.sprites = voxel_models.len();
    r.shared_share = if r.placed > 0 { r.shared as f32 / r.placed as f32 } else { 0.0 };
    r.palette_roles = palette.len();
    let mut dist = Vec::new();
    for (role, hex) in &palette {
        let d = match crate::style::default_role(role) {
            Some(def) => crate::style::color_distance(def, hex).unwrap_or(0.0),
            None => crate::style::DEFAULT_PALETTE.iter().filter_map(|(_, def, _)| crate::style::color_distance(def, hex)).fold(f32::MAX, f32::min),
        };
        if d >= crate::style::SAME_COLOR {
            r.palette_custom_roles += 1;
        }
        dist.push(d);
    }
    r.palette_distance = if dist.is_empty() { 0.0 } else { dist.iter().sum::<f32>() / dist.len() as f32 };
    let (d, n) = crate::style::default_color_share(&scene.objects);
    r.default_surfaces = d;
    r.surfaces = n;
    r.sounds = sound_files(path, &raw);
    r.defaults_kept = crate::style::defaults_kept(&raw, &scene).into_iter().map(|(c, _)| c.to_string()).collect();
    Ok(r)
}

/// Score files the game brings: `*.json` under an `audio/` folder next to the scene, plus any file its `audio` block names that lives elsewhere.
fn sound_files(path: &Path, raw: &Value) -> usize {
    let mut files: BTreeSet<String> = BTreeSet::new();
    if let Some(dir) = path.parent().map(|p| p.join("audio")) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if e.path().extension().is_some_and(|x| x == "json") {
                files.insert(e.file_name().to_string_lossy().to_string());
            }
        }
    }
    fn strings(v: &Value, out: &mut BTreeSet<String>) {
        match v {
            Value::String(s) if s.ends_with(".json") => {
                out.insert(Path::new(s).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default());
            }
            Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
            Value::Object(o) => o.values().for_each(|x| strings(x, out)),
            _ => {}
        }
    }
    if let Some(a) = raw.get("audio") {
        strings(a, &mut files);
    }
    files.len()
}

fn report_2d(raw: &Value) -> StyleReport {
    let count = |k: &str| match raw.get(k) {
        Some(Value::Object(o)) => o.len(),
        Some(Value::Array(a)) => a.len(),
        _ => 0,
    };
    let mut colours: BTreeMap<String, ()> = BTreeMap::new();
    if let Some(sprites) = raw.get("sprites").and_then(Value::as_object) {
        for s in sprites.values() {
            if let Some(p) = s.get("palette").and_then(Value::as_object) {
                for c in p.values().filter_map(Value::as_str) {
                    colours.insert(c.to_lowercase(), ());
                }
            }
        }
    }
    StyleReport {
        kind: "2d".into(),
        sprites: count("sprites"),
        sounds: count("sounds") + usize::from(raw.get("music").is_some()),
        palette_roles: colours.len(),
        palette_custom_roles: colours.len(),
        ..Default::default()
    }
}

/// The report as text: one fact per line, then what to do about what it found.
pub fn text(path: &Path, r: &StyleReport) -> String {
    let mut s = format!("style-report {} ({})\n", path.display(), r.kind);
    if r.kind == "2d" {
        s.push_str(&format!("  sprites {}   sounds {}   sprite colours {}\n", r.sprites, r.sounds, r.palette_roles));
        s.push_str("  (a 2D game draws everything itself: the shared-catalogue questions do not apply)\n");
        return s;
    }
    let pct = |a: usize, b: usize| if b == 0 { 0.0 } else { 100.0 * a as f32 / b as f32 };
    s.push_str(&format!(
        "  placed {}: shared catalogue {} ({:.0}%, {} re-skinned by the palette), own prefabs {} ({:.0}%), hand-built {} ({:.0}%)\n",
        r.placed,
        r.shared,
        pct(r.shared, r.placed),
        r.shared_reskinned,
        r.own_prefab_instances,
        pct(r.own_prefab_instances, r.placed),
        r.hand_built,
        pct(r.hand_built, r.placed)
    ));
    s.push_str(&format!("  own prefabs {}: {}\n", r.own_prefabs.len(), if r.own_prefabs.is_empty() { "none".to_string() } else { r.own_prefabs.join(", ") }));
    s.push_str(&format!(
        "  palette {} role(s), {} not the default; mean distance from the defaults {:.0} (0 = the defaults; {:.0}+ reads as a different colour)\n",
        r.palette_roles,
        r.palette_custom_roles,
        r.palette_distance,
        crate::style::SAME_COLOR
    ));
    s.push_str(&format!("  surfaces {}: {} ({:.0}%) still default colours\n", r.surfaces, r.default_surfaces, pct(r.default_surfaces, r.surfaces)));
    s.push_str(&format!("  voxel models {}   sound files {}\n", r.sprites, r.sounds));
    if r.defaults_kept.is_empty() {
        s.push_str("  lint: no default palette, lamp or sky kept\n");
    } else {
        s.push_str(&format!("  lint: kept the defaults: {} (`red_engine2 lint` says how to fix each)\n", r.defaults_kept.join(", ")));
    }
    if r.own_prefabs.is_empty() {
        s.push_str("next: make a few prefabs of your own (scene `prefabs`; `describe objects` -> prefab) and look at them: red_engine2 catalog --scene GAME --local --sheet out/own.png\n");
    } else {
        s.push_str("look: red_engine2 style-report GAME --sheet out/own.png  (or catalog --scene GAME --local --sheet ...)\n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, v: &Value) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, serde_json::to_string(v).unwrap()).unwrap();
        p
    }

    #[test]
    fn it_tells_shared_from_own_from_hand_built_and_measures_the_palette() {
        let dir = std::env::temp_dir().join(format!("style_report_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("audio")).unwrap();
        std::fs::write(dir.join("audio").join("night.json"), "{}").unwrap();
        let p = write(
            &dir,
            "g.json",
            &serde_json::json!({
                "camera": {"position": [0, 1.7, 5], "target": [0, 1, 0]},
                "style": {"palette": {"wood": "#3b2418", "primary": "#3a6ea5"}},
                "rules": [],
                "prefabs": [{"name": "my_totem", "tags": ["deco"], "objects": [{"id": "b", "type": "box", "size": [0.3, 1, 0.3], "position": [0, 0.5, 0], "material": {"color": "@wood"}}]}],
                "objects": [
                    {"id": "c1", "type": "prefab", "prefab": "chair_wooden_1", "position": [0, 0, 0]},
                    {"id": "c2", "type": "prefab", "prefab": "apple_red", "position": [1, 0, 0]},
                    {"id": "t1", "type": "prefab", "prefab": "my_totem", "position": [2, 0, 0]},
                    {"id": "g", "type": "group", "position": [0, 0, 0], "children": [{"id": "floor", "type": "box", "size": [10, 0.1, 10], "position": [0, -0.05, 0], "material": {"color": "#202020"}}]}
                ]
            }),
        );
        let r = report(&p).unwrap();
        assert_eq!((r.placed, r.shared, r.shared_reskinned, r.own_prefab_instances, r.hand_built), (4, 2, 1, 1, 1), "{r:?}");
        assert_eq!(r.own_prefabs, vec!["my_totem"]);
        assert_eq!((r.palette_roles, r.palette_custom_roles), (2, 1), "primary is the default colour, wood is not");
        assert!(r.palette_distance > 20.0);
        assert_eq!(r.sounds, 1);
        let t = text(&p, &r);
        assert!(t.contains("shared catalogue 2 (50%, 1 re-skinned"), "{t}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_2d_game_reports_its_sprites_and_sounds() {
        let r = report_2d(
            &serde_json::json!({"game2d": 1, "sprites": {"a": {"rows": ["aa"], "palette": {"a": "#ff0000"}}, "b": {"rows": ["b"], "palette": {"b": "#00ff00"}}}, "sounds": {"jump": {}}, "music": {}}),
        );
        assert_eq!((r.kind.as_str(), r.sprites, r.sounds, r.palette_roles), ("2d", 2, 2, 2));
    }
}
