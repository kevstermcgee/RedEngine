//! Game-level visual style (`style`): a palette of named colour roles, material presets, a light rig, sky, fog and post settings that give one game its own
//! look with the shared prefabs. Resolved at parse time into the scene's ordinary fields (ADR 2026-10-10-a-game-style-block-re-skins-shared-prefabs), so every
//! tool downstream sees a plain scene: `"@wood"` anywhere a colour is accepted becomes the palette's hex, a prefab param that declares `"palette": "wood"` takes
//! the palette's colour as its default, and the light rig, sky, fog and post fill the scene's own blocks only where the scene left them out.

use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// Keys of the `style` block.
pub const STYLE_KEYS: &[&str] = &["palette", "materials", "light", "sky", "fog", "post"];
/// Keys of one `style.materials` preset (the same fields as an object's `material`).
pub const PRESET_KEYS: &[&str] = &["color", "metallic", "roughness", "emissive", "opacity"];
/// Keys of `style.light`.
pub const LIGHT_KEYS: &[&str] = &["sun", "ambient", "lamp"];
/// Keys of `style.sky`.
pub const SKY_KEYS: &[&str] = &["top", "bottom", "color"];
/// Keys of `style.fog`.
pub const FOG_KEYS: &[&str] = &["color", "density"];
/// The most roles a palette may name (the brief asks for 6-8; a few more leave room for a game's own roles).
pub const MAX_ROLES: usize = 16;

/// The standard roles, what each one colours, and the colour the catalogue, the recipes and the blueprint compiler use for it when a game names none
/// (measured from `assets/*.json` param defaults and `tools/blueprint.rs`). Shared prefabs bind their params to these names; `lint` and `style-report` compare a
/// game's palette with these colours.
pub const DEFAULT_PALETTE: &[(&str, &str, &str)] = &[
    ("primary", "#3a6ea5", "the game's main colour: signage, key props, the player's eye-line"),
    ("secondary", "#b9783a", "the colour that sits next to primary"),
    ("accent", "#f5d24a", "pickups, highlights, things to notice"),
    ("wood", "#7a5230", "furniture and other wooden parts"),
    ("metal", "#c0c4c8", "fittings, frames, appliances"),
    ("fabric", "#d9c9a3", "cushions, upholstery, curtains"),
    ("floor", "#8f9aa8", "floors (blueprint rooms)"),
    ("wall", "#cfcfd6", "walls (blueprint rooms)"),
    ("trim", "#e8e4dc", "ceilings, lamp shades, mouldings"),
    ("foliage", "#3f7a3a", "leaves and plants"),
    ("stone", "#b6b0a4", "stone, ceramic and concrete"),
    ("lamp", "#fff4e0", "the colour of point lights (the light rig's lamp)"),
];

/// The skies a scene gets when nobody chose one: the engine's own gradient, the one the recipes copy, the blueprint compiler's, and the variant several
/// generated games share. `(top, bottom)`.
pub const DEFAULT_SKIES: &[(&str, &str)] = &[("#adcef1", "#f3f7fd"), ("#8fc7ff", "#eef6ff"), ("#9fb4c8", "#dfe6ee"), ("#5b8fd4", "#dce8f5")];

/// The point-light colours that are "no choice was made": the blueprint compiler's lamp and plain white.
pub const DEFAULT_LAMPS: &[&str] = &["#fff4e0", "#ffffff"];

/// `describe style`: the block, the standard roles with their default colours, and how prefabs bind to them. Generated from the tables above.
pub fn describe_text() -> String {
    let mut s = String::from(
        "style: a game's own look, so the shared prefabs do not make every game look the same (resolved at load time into ordinary fields).\n\
         \x20 \"style\": {\n\
         \x20   \"palette\":   {\"primary\": \"#2e5e4e\", \"accent\": \"#e8a33d\", \"wood\": \"#3b2418\", \"wall\": \"#e6d8bf\", \"floor\": \"#4a3b30\", \"lamp\": \"#ffb36b\"},\n\
         \x20   \"materials\": {\"brass\": {\"color\": \"@metal\", \"metallic\": 0.9, \"roughness\": 0.3}},\n\
         \x20   \"light\":     {\"sun\": {\"color\", \"intensity\", \"direction\"}, \"ambient\": {\"color\", \"intensity\"}, \"lamp\": {\"color\", \"intensity\"}},\n\
         \x20   \"sky\": {\"top\", \"bottom\"} | {\"color\"},  \"fog\": {\"color\", \"density\"},  \"post\": {\"ao\", \"outline\"}\n\
         \x20 }\n\
         \"@role\" works anywhere a colour does (materials, prefab params, lights, background); `material: {\"preset\": \"brass\", ...}` merges a preset under the object's own fields.\n\
         Shared prefabs re-skin: a param declared {\"default\": \"#8b5a2b\", \"palette\": \"wood\"} takes the palette's wood when the game names one (`catalog <name>` shows bindings).\n\
         light/sky/fog/post only fill what the scene left out; point lights left at the default lamp colour take `lamp`. Blueprint rooms use @wall, @floor, @trim.\n\
         Standard roles and the defaults a game gets when it names none (lint `style-palette`/`style-lamp`/`style-sky` warn when a game keeps them):\n",
    );
    for (role, hex, what) in DEFAULT_PALETTE {
        s.push_str(&format!("  {role:<10} {hex}  {what}\n"));
    }
    s.push_str(
        "Look at the result: `frame`/`tour`, and `style-report GAME` (shared vs game-made assets, palette distance, a sheet of the game's own prefabs).\n",
    );
    s
}

/// The style lint applies to games (a scene with `rules` or `ui`), not to reference maps and test content.
pub fn is_game(raw: &Value) -> bool {
    raw.get("rules").is_some() || raw.get("ui").is_some()
}

/// How close (redmean distance, 0..765) a colour must be to a default to count as "the default": below this two colours read as the same.
pub const SAME_COLOR: f32 = 20.0;

fn linear(hex: &str) -> Option<glam::Vec3> {
    crate::color::parse_hex_to_linear(hex).ok()
}

/// What a game left at the engine's defaults, as `(lint code, message)`: no palette or the default one (`style-palette`), only default-coloured lamps
/// (`style-lamp`), a default sky (`style-sky`). Read from the raw file (did it choose?) and the parsed scene (what it renders). Empty for a non-game.
pub fn defaults_kept(raw: &Value, scene: &crate::schema::Scene) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if !is_game(raw) {
        return out;
    }
    let palette = palette_of(raw);
    let custom = palette.iter().filter(|(role, hex)| default_role(role).and_then(|d| color_distance(d, hex)).is_none_or(|d| d >= SAME_COLOR)).count();
    let (default_n, total_n) = default_color_share(&scene.objects);
    // No palette is fine for a game that chose its colours by hand (Marcel); it is the default look only when most surfaces are default colours.
    if palette.is_empty() && total_n > 0 && default_n * 2 >= total_n {
        out.push((
            "style-palette",
            format!(
                "{default_n} of {total_n} surfaces are default colours (the catalogue's, the blueprint's) and the game has no style.palette: it looks like every other game made with them -> add \"style\": {{\"palette\": {{...}}}} with 6-8 roles of your own (`describe style`)"
            ),
        ));
    } else if !palette.is_empty() && custom == 0 {
        out.push((
            "style-palette",
            format!(
                "every role of style.palette ({}) is the default colour -> choose colours of your own (`describe style` lists the defaults)",
                palette.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
        ));
    }
    let close = |c: glam::Vec3, hex: &str| linear(hex).is_some_and(|d| (c - d).length() < 0.02);
    let lamps: Vec<glam::Vec3> =
        scene.lights.iter().filter(|l| matches!(l.kind, crate::schema::LightKind::Point { .. })).map(|l| l.color.sample(0.0)).collect();
    if !lamps.is_empty() && lamps.iter().all(|c| DEFAULT_LAMPS.iter().any(|d| close(*c, d))) {
        out.push(("style-lamp", format!("all {} point light(s) are the default lamp colour ({}) -> give the palette a `lamp` role or style.light.lamp (warm, cold, coloured: the light is half the mood)", lamps.len(), DEFAULT_LAMPS.join(" / "))));
    }
    if scene.sky.is_none() && scene.clock.is_none() && scene.procgen.is_none() {
        let default_sky = match &scene.background {
            crate::schema::Background::Gradient { top, bottom } => DEFAULT_SKIES.iter().any(|(t, b)| close(*top, t) && close(*bottom, b)),
            crate::schema::Background::Flat(_) => false,
        };
        if default_sky {
            out.push((
                "style-sky",
                "the sky is a default gradient (the engine's, a recipe's or the blueprint's) -> set style.sky {top, bottom} (or a `sky` / `clock` block)"
                    .to_string(),
            ));
        }
    }
    out
}

/// Every colour a game gets without choosing one (linear RGB): the default palette, every colour default of a catalogue prefab param and the blueprint
/// compiler's room colours.
fn default_colors() -> &'static [glam::Vec3] {
    static ALL: std::sync::OnceLock<Vec<glam::Vec3>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        let mut hexes: Vec<String> = DEFAULT_PALETTE.iter().map(|(_, h, _)| h.to_string()).collect();
        hexes.extend(["#cfcfd6", "#b8b8c0", "#e8e4dc", "#8f9aa8", "#a89f8f", "#8fa895", "#a88f9d", "#9fa8b8", "#b0a688"].map(String::from));
        for d in &crate::prefabs::builtin().0.defs {
            hexes.extend(d.params.iter().filter_map(|p| p.default.as_str()).filter(|s| s.starts_with('#')).map(str::to_string));
        }
        hexes.sort();
        hexes.dedup();
        hexes.iter().filter_map(|h| linear(h)).collect()
    })
}

/// `(surfaces in a default colour, surfaces)`: leaf primitives with a material, counted once each (a prefab's parts count separately).
pub fn default_color_share(objects: &[crate::schema::Object]) -> (usize, usize) {
    let defaults = default_colors();
    let (mut d, mut n) = (0, 0);
    let mut stack: Vec<&crate::schema::Object> = objects.iter().collect();
    while let Some(o) = stack.pop() {
        match &o.kind {
            crate::schema::ObjectKind::Group(children) => stack.extend(children.iter()),
            crate::schema::ObjectKind::Prim(_) => {
                if let Some(m) = &o.material {
                    n += 1;
                    let c = m.color.sample(0.0);
                    if defaults.iter().any(|x| (*x - c).length() < 0.02) {
                        d += 1;
                    }
                }
            }
            _ => {}
        }
    }
    (d, n)
}

/// The colour of a standard role in the default palette.
pub fn default_role(role: &str) -> Option<&'static str> {
    DEFAULT_PALETTE.iter().find(|(r, _, _)| *r == role).map(|(_, c, _)| *c)
}

/// sRGB `#rrggbb` -> `[r, g, b]` in 0..=255, or None.
pub fn rgb(hex: &str) -> Option<[f32; 3]> {
    let h = hex.trim().strip_prefix('#')?;
    if h.len() != 6 && h.len() != 8 {
        return None;
    }
    let b = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(f32::from);
    Some([b(0)?, b(2)?, b(4)?])
}

/// Perceptual distance between two sRGB colours (the "redmean" approximation; 0 = identical, about 765 = black vs white). Below about 20 a person sees the
/// same colour.
pub fn color_distance(a: &str, b: &str) -> Option<f32> {
    let (a, b) = (rgb(a)?, rgb(b)?);
    let rm = (a[0] + b[0]) / 2.0;
    let (dr, dg, db) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    Some(((2.0 + rm / 256.0) * dr * dr + 4.0 * dg * dg + (2.0 + (255.0 - rm) / 256.0) * db * db).sqrt())
}

/// A palette as `role -> #hex`, read from a scene root's `style.palette` (no validation: `resolve` reports problems).
pub fn palette_of(root: &Value) -> BTreeMap<String, String> {
    root.get("style")
        .and_then(|s| s.get("palette"))
        .and_then(Value::as_object)
        .map(|p| p.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
        .unwrap_or_default()
}

fn valid_role_name(r: &str) -> bool {
    let mut c = r.chars();
    c.next().is_some_and(|f| f.is_ascii_lowercase()) && c.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

fn is_hex(s: &str) -> bool {
    crate::color::parse_hex_to_linear(s).is_ok()
}

/// Sections whose strings are words for people or rules, never colours: a `@` there is text.
const TEXT_SECTIONS: &[&str] =
    &["style", "rules", "ui", "recipe", "notes", "$comment", "death_text", "vars", "player_vars", "phases", "checks", "match", "nav", "audio", "music"];

/// The parse-time pass `schema::parse_scene_text` runs before prefab expansion. Applies the `style` block to `root` in place: `@role` colour references,
/// `material.preset`, then the light rig, sky, fog and post where the scene has none of its own. A scene without `style` and without `@` references is left as it is.
/// `Err` lists every problem as `path: message`.
pub fn resolve(root: &mut Value) -> Result<(), Vec<String>> {
    let mut errs = Vec::new();
    let Some(obj) = root.as_object() else { return Ok(()) };
    let style = obj.get("style").cloned();
    let palette = check_style(style.as_ref(), &mut errs);
    let presets = style.as_ref().and_then(|s| s.get("materials")).and_then(Value::as_object).cloned().unwrap_or_default();
    if let Some(root) = root.as_object_mut() {
        for (k, v) in root.iter_mut() {
            if TEXT_SECTIONS.contains(&k.as_str()) || crate::fields::is_extension_key(k) {
                continue;
            }
            substitute(v, k, &palette, &presets, &mut errs);
        }
        if let Some(s) = style.as_ref().and_then(Value::as_object) {
            apply_defaults(root, s, &palette);
        }
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

/// Validates the `style` block and returns its palette (role -> hex), roles resolved.
fn check_style(style: Option<&Value>, errs: &mut Vec<String>) -> BTreeMap<String, String> {
    let mut palette = BTreeMap::new();
    let Some(style) = style else { return palette };
    let Some(s) = style.as_object() else {
        errs.push("style: must be an object {palette, materials, light, sky, fog, post}".into());
        return palette;
    };
    crate::fields::check_keys(errs, "style", s, STYLE_KEYS);
    if let Some(p) = s.get("palette") {
        match p.as_object() {
            None => errs.push("style.palette: must be an object {role: \"#rrggbb\"}, e.g. {\"primary\": \"#c0392b\", \"wood\": \"#5a3d2b\"}".into()),
            Some(p) => {
                if p.len() > MAX_ROLES {
                    errs.push(format!("style.palette: {} roles; at most {MAX_ROLES} (6-8 is the useful range)", p.len()));
                }
                for (role, v) in p {
                    if !valid_role_name(role) {
                        errs.push(format!("style.palette.{role}: a role name is lower-case letters, digits and _ (standard roles: {})", standard_roles()));
                        continue;
                    }
                    match v.as_str() {
                        Some(h) if is_hex(h) => {
                            palette.insert(role.clone(), h.to_string());
                        }
                        _ => errs.push(format!("style.palette.{role}: must be a colour \"#rrggbb\" (a palette role cannot point at another role)")),
                    }
                }
            }
        }
    }
    if let Some(m) = s.get("materials") {
        match m.as_object() {
            None => errs.push("style.materials: must be an object {preset: {color, roughness, metallic, emissive, opacity}}".into()),
            Some(m) => {
                for (name, def) in m {
                    let path = format!("style.materials.{name}");
                    match def.as_object() {
                        None => errs.push(format!("{path}: must be an object like an object's material: {{color, roughness, metallic, emissive, opacity}}")),
                        Some(d) => {
                            crate::fields::check_keys(errs, &path, d, PRESET_KEYS);
                            if let Some(c) = d.get("color") {
                                check_color(c, &format!("{path}.color"), &palette, errs);
                            }
                        }
                    }
                }
            }
        }
    }
    for (key, keys) in [("light", LIGHT_KEYS), ("sky", SKY_KEYS), ("fog", FOG_KEYS)] {
        if let Some(v) = s.get(key) {
            match v.as_object() {
                None => errs.push(format!("style.{key}: must be an object {{{}}}", keys.join(", "))),
                Some(o) => crate::fields::check_keys(errs, &format!("style.{key}"), o, keys),
            }
        }
    }
    if let Some(f) = s.get("fog").and_then(Value::as_object) {
        match f.get("density").map(|d| d.as_f64()) {
            None | Some(Some(_)) => {}
            Some(None) => errs.push("style.fog.density: must be a number (0.01 light haze ... 0.1 thick fog; 0 turns it off)".into()),
        }
    }
    palette
}

fn check_color(v: &Value, path: &str, palette: &BTreeMap<String, String>, errs: &mut Vec<String>) {
    match v.as_str() {
        Some(s) if s.starts_with('@') => {
            if !palette.contains_key(&s[1..]) {
                errs.push(unknown_role(path, &s[1..], palette));
            }
        }
        Some(s) if is_hex(s) => {}
        _ => errs.push(format!("{path}: must be \"#rrggbb\" or a palette role \"@name\"")),
    }
}

fn standard_roles() -> String {
    DEFAULT_PALETTE.iter().map(|(r, _, _)| *r).collect::<Vec<_>>().join(", ")
}

fn unknown_role(path: &str, role: &str, palette: &BTreeMap<String, String>) -> String {
    if palette.is_empty() {
        return format!(
            "{path}: \"@{role}\" names a palette role, but the scene has no style.palette (add \"style\": {{\"palette\": {{\"{role}\": \"#rrggbb\"}}}})"
        );
    }
    let hint = crate::suggest::suggest(role, palette.keys().map(String::as_str));
    format!(
        "{path}: no palette role '{role}' (style.palette has: {}){}",
        palette.keys().cloned().collect::<Vec<_>>().join(", "),
        hint.first().map(|h| format!(" — did you mean @{h}?")).unwrap_or_default()
    )
}

/// Replaces `"@role"` strings with the palette's hex and merges `material.preset` presets, everywhere under `v` (objects, prefab definitions, lights, sky...).
fn substitute(v: &mut Value, path: &str, palette: &BTreeMap<String, String>, presets: &Map<String, Value>, errs: &mut Vec<String>) {
    match v {
        Value::String(s) if s.starts_with('@') && s.len() > 1 && valid_role_name(&s[1..]) => match palette.get(&s[1..]) {
            Some(hex) => *s = hex.clone(),
            None => errs.push(unknown_role(path, &s[1..], palette)),
        },
        Value::Array(a) => {
            for (i, x) in a.iter_mut().enumerate() {
                let p = match x.get("id").and_then(Value::as_str) {
                    Some(id) => id.to_string(),
                    None => format!("{path}[{i}]"),
                };
                substitute(x, &p, palette, presets, errs);
            }
        }
        Value::Object(o) => {
            if let Some(m) = o.get_mut("material").and_then(Value::as_object_mut) {
                if let Some(name) = m.remove("preset") {
                    match name.as_str().and_then(|n| presets.get(n).map(|p| (n, p))) {
                        Some((_, Value::Object(p))) => {
                            for (k, pv) in p {
                                m.entry(k.clone()).or_insert_with(|| pv.clone());
                            }
                        }
                        _ => {
                            let n = name.as_str().unwrap_or("?");
                            let hint = crate::suggest::suggest(n, presets.keys().map(String::as_str));
                            errs.push(format!(
                                "{path}.material.preset: no material preset '{n}' in style.materials{}{}",
                                if presets.is_empty() {
                                    " (the scene defines none)".to_string()
                                } else {
                                    format!(" (has: {})", presets.keys().cloned().collect::<Vec<_>>().join(", "))
                                },
                                hint.first().map(|h| format!(" — did you mean {h}?")).unwrap_or_default()
                            ));
                        }
                    }
                }
            }
            for (k, x) in o.iter_mut() {
                if k == "text" || k == "label" || k == "desc" || crate::fields::is_extension_key(k) {
                    continue;
                }
                substitute(x, &format!("{path}.{k}"), palette, presets, errs);
            }
        }
        _ => {}
    }
}

/// The light rig, sky, fog and post: each fills the scene's own block only when the scene has none (the scene's choice always wins).
fn apply_defaults(root: &mut Map<String, Value>, s: &Map<String, Value>, palette: &BTreeMap<String, String>) {
    let color = |v: Option<&Value>| -> Option<String> {
        let s = v?.as_str()?;
        match s.strip_prefix('@') {
            Some(role) => palette.get(role).cloned(),
            None => Some(s.to_string()),
        }
    };
    if let Some(light) = s.get("light").and_then(Value::as_object) {
        if let Some(a) = light.get("ambient").and_then(Value::as_object) {
            if !root.contains_key("ambient") {
                let mut amb = Map::new();
                if let Some(c) = color(a.get("color")) {
                    amb.insert("color".into(), json!(c));
                }
                if let Some(i) = a.get("intensity") {
                    amb.insert("intensity".into(), i.clone());
                }
                root.insert("ambient".into(), Value::Object(amb));
            }
        }
        let lights = root.entry("lights").or_insert_with(|| json!([]));
        if let Some(lights) = lights.as_array_mut() {
            if let Some(sun) = light.get("sun").and_then(Value::as_object) {
                if !lights.iter().any(|l| l.get("type").and_then(Value::as_str) == Some("directional")) {
                    let mut l = Map::new();
                    l.insert("id".into(), json!("style_sun"));
                    l.insert("type".into(), json!("directional"));
                    l.insert("direction".into(), sun.get("direction").cloned().unwrap_or(json!([-0.4, -1.0, -0.3])));
                    l.insert("color".into(), json!(color(sun.get("color")).unwrap_or_else(|| "#ffffff".into())));
                    l.insert("intensity".into(), sun.get("intensity").cloned().unwrap_or(json!(1.0)));
                    if let Some(cs) = sun.get("cast_shadows") {
                        l.insert("cast_shadows".into(), cs.clone());
                    }
                    lights.insert(0, Value::Object(l));
                }
            }
            // The lamp: point lights left at the "no choice was made" colour (the blueprint's lamp, plain white) take the rig's lamp colour and intensity.
            let lamp = light.get("lamp").and_then(Value::as_object);
            let lamp_color = color(lamp.and_then(|l| l.get("color"))).or_else(|| palette.get("lamp").cloned());
            recolor_default_lamps(lights, lamp_color.as_deref(), lamp.and_then(|l| l.get("intensity")));
        }
        if root.get("lights").and_then(Value::as_array).is_some_and(Vec::is_empty) {
            root.remove("lights");
        }
    } else if let Some(lights) = root.get_mut("lights").and_then(Value::as_array_mut) {
        recolor_default_lamps(lights, palette.get("lamp").map(String::as_str), None);
    }
    if let Some(sky) = s.get("sky").and_then(Value::as_object) {
        if !root.contains_key("background") {
            let bg = match color(sky.get("color")) {
                Some(c) => json!({"color": c}),
                None => {
                    let mut b = Map::new();
                    if let Some(t) = color(sky.get("top")) {
                        b.insert("sky_top".into(), json!(t));
                    }
                    if let Some(t) = color(sky.get("bottom")) {
                        b.insert("sky_bottom".into(), json!(t));
                    }
                    Value::Object(b)
                }
            };
            root.insert("background".into(), bg);
        }
    }
    if let Some(fog) = s.get("fog").and_then(Value::as_object) {
        if !root.contains_key("fog") {
            let mut f = Map::new();
            if let Some(c) = color(fog.get("color")) {
                f.insert("color".into(), json!(c));
            }
            if let Some(d) = fog.get("density") {
                f.insert("density".into(), d.clone());
            }
            root.insert("fog".into(), Value::Object(f));
        }
    }
    if let Some(post) = s.get("post").and_then(Value::as_object) {
        let target = root.entry("post").or_insert_with(|| json!({}));
        if let Some(t) = target.as_object_mut() {
            for (k, v) in post {
                t.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
    }
}

/// Point lights whose colour is missing or one of `DEFAULT_LAMPS` take `color` (and `intensity`, when given); a lamp the scene coloured itself is left alone.
fn recolor_default_lamps(lights: &mut [Value], color: Option<&str>, intensity: Option<&Value>) {
    for l in lights.iter_mut().filter(|l| l.get("type").and_then(Value::as_str) == Some("point")) {
        let default_colored = l.get("color").and_then(Value::as_str).is_none_or(|c| DEFAULT_LAMPS.iter().any(|d| d.eq_ignore_ascii_case(c)));
        if let (true, Some(o)) = (default_colored, l.as_object_mut()) {
            if let Some(c) = color {
                o.insert("color".into(), json!(c));
            }
            if let Some(i) = intensity {
                o.insert("intensity".into(), i.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(v: Value) -> Result<Value, Vec<String>> {
        let mut v = v;
        resolve(&mut v).map(|_| v)
    }

    #[test]
    fn a_role_reference_becomes_the_palette_colour_anywhere_a_colour_is_accepted() {
        let v = run(json!({
            "style": {"palette": {"wood": "#5a3d2b", "lamp": "#ffb070"}},
            "ambient": {"color": "@wood", "intensity": 0.3},
            "objects": [{"id": "t", "type": "box", "material": {"color": "@wood"}}, {"id": "g", "type": "group", "children": [{"id": "c", "type": "box", "material": {"color": "@lamp"}}]}]
        }))
        .unwrap();
        assert_eq!(v["objects"][0]["material"]["color"], "#5a3d2b");
        assert_eq!(v["objects"][1]["children"][0]["material"]["color"], "#ffb070");
        assert_eq!(v["ambient"]["color"], "#5a3d2b");
    }

    #[test]
    fn an_unknown_role_is_an_error_with_a_fix_and_text_is_left_alone() {
        let e = run(json!({"style": {"palette": {"wood": "#5a3d2b"}}, "objects": [{"id": "t", "type": "box", "material": {"color": "@wod"}}]})).unwrap_err();
        assert!(e[0].contains("no palette role 'wod'") && e[0].contains("did you mean @wood"), "{e:?}");
        let e = run(json!({"objects": [{"id": "t", "type": "box", "material": {"color": "@wood"}}]})).unwrap_err();
        assert!(e[0].contains("no style.palette"), "{e:?}");
        let v = run(json!({"style": {"palette": {"wood": "#5a3d2b"}}, "ui": {"title": "@wood"}, "objects": [{"id": "s", "type": "text", "text": "@wood"}]}))
            .unwrap();
        assert_eq!(v["ui"]["title"], "@wood");
        assert_eq!(v["objects"][0]["text"], "@wood");
    }

    #[test]
    fn a_material_preset_fills_what_the_object_did_not_say() {
        let v = run(json!({
            "style": {"palette": {"metal": "#b08d57"}, "materials": {"brass": {"color": "@metal", "metallic": 0.9, "roughness": 0.3}}},
            "objects": [{"id": "k", "type": "box", "material": {"preset": "brass", "roughness": 0.6}}]
        }))
        .unwrap();
        let m = &v["objects"][0]["material"];
        assert_eq!((m["color"].as_str(), m["metallic"].as_f64(), m["roughness"].as_f64(), m.get("preset")), (Some("#b08d57"), Some(0.9), Some(0.6), None));
        let e = run(json!({"style": {"materials": {"brass": {}}}, "objects": [{"id": "k", "type": "box", "material": {"preset": "bras"}}]})).unwrap_err();
        assert!(e[0].contains("did you mean brass"), "{e:?}");
    }

    #[test]
    fn the_light_rig_sky_fog_and_post_fill_only_what_the_scene_left_out() {
        let v = run(json!({
            "style": {"palette": {"lamp": "#ff9a50"}, "light": {"sun": {"color": "#ffd2a0", "intensity": 1.4}, "ambient": {"color": "#405070", "intensity": 0.2}},
                      "sky": {"top": "#1a1030", "bottom": "#c06040"}, "fog": {"color": "#c06040", "density": 0.03}, "post": {"outline": 0.8}},
            "post": {"ao": 0.5},
            "lights": [{"id": "l1", "type": "point", "color": "#fff4e0"}, {"id": "l2", "type": "point", "color": "#00ff00"}],
            "objects": []
        }))
        .unwrap();
        assert_eq!(v["lights"][0]["id"], "style_sun");
        assert_eq!(v["lights"][1]["color"], "#ff9a50", "a lamp left at the blueprint's colour takes the palette's lamp");
        assert_eq!(v["lights"][2]["color"], "#00ff00", "a lamp the scene chose keeps its colour");
        assert_eq!(v["background"]["sky_top"], "#1a1030");
        assert_eq!(v["fog"]["density"], 0.03);
        assert_eq!((v["post"]["ao"].as_f64(), v["post"]["outline"].as_f64()), (Some(0.5), Some(0.8)));
        let own = run(json!({"style": {"sky": {"top": "#000000"}}, "background": {"color": "#123456"}, "objects": []})).unwrap();
        assert_eq!(own["background"], json!({"color": "#123456"}), "the scene's own sky wins");
    }

    #[test]
    fn a_bad_palette_says_how_to_write_it() {
        let e = run(json!({"style": {"palette": {"Wood": "#5a3d2b", "metal": "@wood", "stone": 3}, "colour": {}}, "objects": []})).unwrap_err();
        let all = e.join("\n");
        assert!(all.contains("style.colour: unknown field") || all.contains("style.colour"), "{all}");
        assert!(all.contains("style.palette.Wood") && all.contains("style.palette.metal") && all.contains("style.palette.stone"), "{all}");
    }

    #[test]
    fn colour_distance_is_zero_for_the_same_colour_and_large_for_opposites() {
        assert_eq!(color_distance("#7a5230", "#7a5230"), Some(0.0));
        assert!(color_distance("#000000", "#ffffff").unwrap() > 700.0);
        assert!(color_distance("#7a5230", "#7b5331").unwrap() < 5.0);
    }
}
