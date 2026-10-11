//! The `sky` and `ocean` scene blocks: a sky dome with a sun at infinity, and an endless water plane that runs to the horizon.
//!
//! ```json
//! "sky":   { "zenith": "#1a0826", "horizon": "#ff9a5a", "gradient_power": 0.55,
//!            "sun": { "direction": [-1, 0.045, 0.1], "size_deg": 2.6, "color": "#fff1c8", "glow": 0.8 } },
//! "ocean": { "y": 0.0, "color_deep": "#0d2b3a", "color_shallow": "#2a7f8f", "foam_color": "#ffffff",
//!            "wave_amplitude": 0.05, "wave_frequency": 0.8, "wave_speed": 1.0, "roughness": 0.12, "depth_scale": 4.0, "haze_distance": 260 }
//! ```
//!
//! **Sky.** Without a `sky` block the background is a gradient in *screen* space (it sits still on the screen whatever you look at).
//! With one it is shaded by the view direction: `horizon` at eye level rising to `zenith` overhead, and a sun disc with a glow drawn at
//! infinity in a fixed direction. Because it has no position it cannot be walked past, dipped under or clipped by the ground: it rises
//! and sets only behind whatever occludes the horizon (the ocean, a ridge).
//!
//! **Ocean.** An analytic plane at height `y`, drawn from the eye to the horizon (there is no edge to walk to and no z-fighting), with
//! animated wave normals, a Fresnel reflection of the sky and the sun, a sun glitter path, and colour and opacity that follow the
//! *water depth* over the scene's first `terrain`: shallow and translucent over a shelving shore, deep offshore, with foam lapping at the
//! waterline. It fades into the sky's `horizon` colour with distance, so sea and sky meet without a seam.
//!
//! **A game's own sky.** `"sky": {"shader": "sky/aurora.wgsl", "seed": "launch"}` draws the background with the game's WGSL instead of the engine's dome: the
//! file defines `fn sky(dir: vec3<f32>, px: f32) -> vec3<f32>` (the linear colour seen along the unit world direction `dir`; `px` is the angle one pixel spans,
//! for antialiasing) and may call the engine's helpers: `sky_time()` (scene seconds), `sky_seed()`, `hash13`, `hash33`, `vnoise`, `fbm`, and read `globals`.
//! It is composed into the background pass ([`background_source`]) and checked with naga when the scene is validated ([`check_sky_shader`]): parse, validate,
//! no derivative lookups in loops, HLSL translation, so a sky that would fail on Windows fails `validate` instead. The ocean still reflects the engine's sky.
//!
//! Presentation only: nothing here changes what the simulation does.

use crate::color::parse_hex_to_linear;
use crate::strict::check_keys;
use glam::Vec3;
use serde_json::{Map, Value};

/// `sky` keys.
pub const SKY_KEYS: &[&str] = &["zenith", "horizon", "gradient_power", "sun", "shader", "seed"];
/// `sky.sun` keys.
pub const SUN_KEYS: &[&str] = &["direction", "size_deg", "color", "glow"];
/// `ocean` keys.
pub const OCEAN_KEYS: &[&str] =
    &["y", "color_deep", "color_shallow", "foam_color", "wave_amplitude", "wave_frequency", "wave_speed", "roughness", "depth_scale", "haze_distance"];

/// The sun: a direction at infinity and how it looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sun {
    /// Unit vector from the viewer toward the sun.
    pub direction: Vec3,
    /// Angular radius of the disc, degrees.
    pub radius_deg: f32,
    /// Disc and glow colour (linear).
    pub color: Vec3,
    /// Strength of the glow around the disc and of its glitter on water, 0 .. 2.
    pub glow: f32,
}

/// A sky dome (see the module docs). Colours are linear RGB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sky {
    /// Colour straight overhead.
    pub zenith: Vec3,
    /// Colour at eye level, and what distant water fades into.
    pub horizon: Vec3,
    /// Exponent shaping the climb from horizon to zenith (below 1 keeps the warm colour high; above 1 hugs the horizon).
    pub gradient_power: f32,
    /// The sun, if the sky has one.
    pub sun: Option<Sun>,
}

/// A game's own sky (`sky.shader`, see the module docs): the WGSL that colours every background pixel.
#[derive(Debug, Clone, PartialEq)]
pub struct SkyShader {
    /// The file, as the scene named it (for messages).
    pub file: String,
    /// Its text.
    pub source: String,
    /// What `sky_seed()` returns.
    pub seed: LaunchSeed,
}

/// A seed a scene gives as a number or as `"launch"` (`sky.seed`, `audio.journey.seed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchSeed {
    /// The same every run (`"seed": 7`; the default is 0).
    Fixed(u32),
    /// A new one each launch of the live client (`"seed": "launch"`); offline frames, checks and tools use 0, so they repeat.
    Launch,
}

impl LaunchSeed {
    /// Parses `"seed"` at `path`: a whole number below 2^24 (exact as a shader float) or `"launch"`; absent is 0.
    pub fn parse(v: Option<&Value>, path: &str) -> Result<LaunchSeed, String> {
        match v {
            None => Ok(LaunchSeed::Fixed(0)),
            Some(Value::String(s)) if s == "launch" => Ok(LaunchSeed::Launch),
            Some(v) => match v.as_u64() {
                Some(n) if n < (1 << 24) => Ok(LaunchSeed::Fixed(n as u32)),
                _ => Err(format!("{path}: a whole number from 0 to 16777215, or \"launch\" (a new one each launch)")),
            },
        }
    }

    /// The seed for a run: the fixed one, or for `"launch"` the `launch` value the client drew (`None` offline: 0).
    pub fn value(self, launch: Option<u32>) -> u32 {
        match self {
            LaunchSeed::Fixed(n) => n,
            LaunchSeed::Launch => launch.unwrap_or(0) & 0x00ff_ffff,
        }
    }
}

impl SkyShader {
    /// The seed for a run: the fixed one, or for `"launch"` the `launch` value the client drew (kept below 2^24 so the shader's float holds it exactly).
    pub fn seed_for(&self, launch: Option<u32>) -> u32 {
        self.seed.value(launch)
    }
}

/// The engine's background module: the shared declarations, the sky dome and the background pass.
const BACKGROUND_WGSL: &str = concat!(include_str!("shaders/common.wgsl"), include_str!("shaders/sky.wgsl"), include_str!("shaders/background.wgsl"));

/// The function in `background.wgsl` that a game's sky replaces.
const SKY_HOOK: &str = "fn sky_of(dir: vec3<f32>, px: f32) -> vec3<f32> {\n    return sky_color_px(dir, px);\n}\n";

/// The WGSL of the background pass: the engine's own, or with a game's sky (`custom`) in place of the engine's dome. A game's painted surfaces (`art`,
/// `museum.art`) come first, so its sky may call the same functions (a painting's world drawn around the player).
pub fn background_source(custom: Option<&str>, art: Option<&str>) -> std::borrow::Cow<'static, str> {
    match custom {
        None => std::borrow::Cow::Borrowed(BACKGROUND_WGSL),
        Some(code) => {
            debug_assert!(BACKGROUND_WGSL.contains(SKY_HOOK), "background.wgsl lost its sky_of hook");
            let art = art.unwrap_or("");
            let hook = format!("{art}\n{code}\nfn sky_of(dir: vec3<f32>, px: f32) -> vec3<f32> {{\n    return sky(dir, px);\n}}\n");
            std::borrow::Cow::Owned(BACKGROUND_WGSL.replacen(SKY_HOOK, &hook, 1))
        }
    }
}

/// The engine's scene module: the shared declarations and the lit object shader.
const SCENE_WGSL: &str = concat!(include_str!("shaders/common.wgsl"), include_str!("shaders/scene.wgsl"));

/// The function in `scene.wgsl` that a game's painted surfaces replace.
const SURFACE_HOOK: &str = "fn surface_of(s: SurfaceIn) -> vec4<f32> {\n    return vec4<f32>(1.0, 1.0, 1.0, 0.0);\n}\n";

/// The WGSL of the scene (object) pass: the engine's own, or with a game's `fn surface(s: SurfaceIn) -> vec4<f32>` (`museum.art`) painting every object whose
/// `surface.x` is above 0.5.
pub fn scene_source(art: Option<&str>) -> std::borrow::Cow<'static, str> {
    match art {
        None => std::borrow::Cow::Borrowed(SCENE_WGSL),
        Some(code) => {
            debug_assert!(SCENE_WGSL.contains(SURFACE_HOOK), "scene.wgsl lost its surface_of hook");
            let hook = format!("{code}\nfn surface_of(s: SurfaceIn) -> vec4<f32> {{\n    return surface(s);\n}}\n");
            std::borrow::Cow::Owned(SCENE_WGSL.replacen(SURFACE_HOOK, &hook, 1))
        }
    }
}

/// Whether a game's painted surfaces (`museum.art`) compile everywhere the engine runs: the file must define `fn surface(s: SurfaceIn) -> vec4<f32>`;
/// it is checked inside the scene module, like the engine's own shaders. Errors name the game's file and its own line numbers.
pub fn check_art_shader(file: &str, code: &str) -> Result<(), String> {
    let has = code.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("fn surface(") || l.starts_with("fn surface (")
    });
    if !has {
        return Err(format!(
            "museum.art: `{file}` has no `fn surface(s: SurfaceIn) -> vec4<f32>` (the colour and glow of a painted point; the engine calls it for every pixel of a painting or sculpture)"
        ));
    }
    art_compile_check(file, code)
}

#[cfg(feature = "render")]
fn art_compile_check(file: &str, code: &str) -> Result<(), String> {
    let source = scene_source(Some(code));
    let before = SCENE_WGSL[..SCENE_WGSL.find(SURFACE_HOOK).unwrap_or(0)].lines().count();
    let user_lines = code.lines().count();
    crate::shader_check::check("museum.art", &source).map_err(|e| {
        let mut out = String::new();
        for line in e.lines() {
            out.push_str(&relocate_line(line, before, user_lines, file));
            out.push('\n');
        }
        format!("museum.art: `{file}` does not compile for every GPU backend:\n{}", out.trim_end())
    })
}

#[cfg(not(feature = "render"))]
fn art_compile_check(_file: &str, _code: &str) -> Result<(), String> {
    Ok(())
}

/// Whether a game's sky compiles everywhere the engine runs (see [`crate::shader_check`]). Errors name the game's file and its own line numbers.
pub fn check_sky_shader(file: &str, code: &str) -> Result<(), String> {
    check_sky_shader_with(file, code, None)
}

/// [`check_sky_shader`] for a sky that may call a game's painted-surface functions (`art`, `museum.art`), composed in front of it as the renderer does.
pub fn check_sky_shader_with(file: &str, code: &str, art: Option<&str>) -> Result<(), String> {
    let has_sky = code.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("fn sky(") || l.starts_with("fn sky (")
    });
    if !has_sky {
        return Err(format!(
            "sky.shader: `{file}` has no `fn sky(dir: vec3<f32>, px: f32) -> vec3<f32>` (the colour seen along `dir`; the engine calls it for every background pixel)"
        ));
    }
    compile_check(file, code, art)
}

/// The naga half of [`check_sky_shader`], in a build that can draw (`render`); the headless server, which never draws a sky, only checks the shape above.
#[cfg(feature = "render")]
fn compile_check(file: &str, code: &str, art: Option<&str>) -> Result<(), String> {
    let source = background_source(Some(code), art);
    // naga reports lines of the whole module: move them back to the game's file (a game's surfaces come first: their lines count as before the sky's).
    let before = BACKGROUND_WGSL[..BACKGROUND_WGSL.find(SKY_HOOK).unwrap_or(0)].lines().count() + art.map_or(0, |a| a.lines().count() + 1);
    let user_lines = code.lines().count();
    crate::shader_check::check("sky.shader", &source).map_err(|e| {
        let mut out = String::new();
        for line in e.lines() {
            out.push_str(&relocate_line(line, before, user_lines, file));
            out.push('\n');
        }
        format!("sky.shader: `{file}` does not compile for every GPU backend:\n{}", out.trim_end())
    })
}

#[cfg(not(feature = "render"))]
fn compile_check(_file: &str, _code: &str, _art: Option<&str>) -> Result<(), String> {
    Ok(())
}

/// Rewrites naga's `:LINE:` / `(line LINE)` / `LINE |` references into the game's file where they fall inside it.
#[cfg(feature = "render")]
fn relocate_line(line: &str, before: usize, user_lines: usize, file: &str) -> String {
    let map = |n: usize| -> Option<usize> { (n > before && n <= before + user_lines).then(|| n - before) };
    let t = line.trim_start();
    // A gutter line: "123 │     code" (naga draws the bar as U+2502).
    if let Some(bar) = t.find(['│', '|']) {
        if let Ok(n) = t[..bar].trim().parse::<usize>() {
            let rest = &t[bar..];
            return match map(n) {
                Some(m) => format!("{m:>3} {rest}"),
                None => format!("(engine) {rest}"),
            };
        }
    }
    // "wgsl:123:45" locations.
    if let Some(i) = line.find("wgsl:") {
        let tail = &line[i + 5..];
        let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse::<usize>() {
            if let Some(m) = map(n) {
                return format!("{}{file}:{m}{}", &line[..i], &tail[digits.len()..]);
            }
        }
    }
    // "(line 123)" from the derivative check.
    if let Some(i) = line.find("(line ") {
        let tail = &line[i + 6..];
        let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse::<usize>() {
            if let Some(m) = map(n) {
                return format!("{}({file} line {m}{}", &line[..i], &tail[digits.len()..]);
            }
        }
    }
    line.to_string()
}

/// Parses `sky.shader` and `sky.seed`, reading the shader file next to the scene (`base`; `None`, a scene with no folder, only checks the keys) and checking it.
pub fn parse_sky_shader(root: &Map<String, Value>, base: Option<&std::path::Path>) -> Result<Option<SkyShader>, Vec<String>> {
    let Some(obj) = root.get("sky").and_then(Value::as_object) else { return Ok(None) };
    let seed = LaunchSeed::parse(obj.get("seed"), "sky.seed").map_err(|e| vec![e])?;
    let file = match obj.get("shader") {
        None => {
            return if obj.contains_key("seed") {
                Err(vec!["sky.seed: only means something with `sky.shader` (a game's own sky)".to_string()])
            } else {
                Ok(None)
            };
        }
        Some(Value::String(f)) if f.ends_with(".wgsl") => f.clone(),
        Some(_) => return Err(vec!["sky.shader: the path of a .wgsl file next to the scene, like \"sky/aurora.wgsl\"".to_string()]),
    };
    let Some(base) = base else { return Ok(Some(SkyShader { file, source: String::new(), seed })) };
    let path = base.join(&file);
    let source = std::fs::read_to_string(&path)
        .map_err(|e| vec![format!("sky.shader: `{file}` cannot be read (looked for {}): {e}; a game published without it draws no sky", path.display())])?;
    // A museum's painted surfaces are composed in front of the sky (it may call them): check the sky with them. Their own errors are `museum.art`'s.
    let art = root.get("museum").and_then(|m| m.get("art")).and_then(Value::as_str).and_then(|f| std::fs::read_to_string(base.join(f)).ok());
    check_sky_shader_with(&file, &source, art.as_deref()).map_err(|e| vec![e])?;
    Ok(Some(SkyShader { file, source, seed }))
}

/// An endless water plane (see the module docs). Colours are linear RGB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ocean {
    /// Water level, world Y.
    pub y: f32,
    /// Colour of deep water.
    pub deep: Vec3,
    /// Colour of shallow water.
    pub shallow: Vec3,
    /// Foam colour.
    pub foam: Vec3,
    /// Wave height, m (0.02 calm .. 0.4 rough).
    pub wave_amplitude: f32,
    /// Wave frequency, radians per metre (0.8 is a swell about 8 m long).
    pub wave_frequency: f32,
    /// Animation speed multiplier.
    pub wave_speed: f32,
    /// 0 (mirror) .. 1 (choppy, dull): how much the wave normals lean and how sharp the sun glitter is.
    pub roughness: f32,
    /// Depth over which shallow turns to deep and the water goes from clear to opaque, m.
    pub depth_scale: f32,
    /// Distance at which the water has faded into the horizon colour, m.
    pub haze_distance: f32,
}

fn hex(errors: &mut Vec<String>, obj: &Map<String, Value>, key: &str, path: &str, default: Vec3) -> Vec3 {
    match obj.get(key) {
        None => default,
        Some(v) => match v.as_str().map(parse_hex_to_linear) {
            Some(Ok(c)) => c,
            _ => {
                errors.push(format!("{path}.{key}: must be a '#rrggbb' hex string"));
                default
            }
        },
    }
}

fn num(errors: &mut Vec<String>, obj: &Map<String, Value>, key: &str, path: &str, default: f32, lo: f32, hi: f32) -> f32 {
    match obj.get(key) {
        None => default,
        Some(v) => match v.as_f64() {
            Some(n) if (lo as f64..=hi as f64).contains(&n) => n as f32,
            _ => {
                errors.push(format!("{path}.{key}: must be a number from {lo} to {hi}"));
                default
            }
        },
    }
}

/// Parses the `sky` block. `zenith` and `horizon` default to the scene's `background` `sky_top` / `sky_bottom`.
pub fn parse_sky(root: &Map<String, Value>, default_zenith: Vec3, default_horizon: Vec3) -> Result<Option<Sky>, Vec<String>> {
    let Some(raw) = root.get("sky") else { return Ok(None) };
    let Some(obj) = raw.as_object() else { return Err(vec!["sky: must be an object like {\"sun\": {\"direction\": [-1, 0.05, 0]}}".to_string()]) };
    let mut errors = Vec::new();
    check_keys(&mut errors, "sky", obj, SKY_KEYS);
    let zenith = hex(&mut errors, obj, "zenith", "sky", default_zenith);
    let horizon = hex(&mut errors, obj, "horizon", "sky", default_horizon);
    let gradient_power = num(&mut errors, obj, "gradient_power", "sky", 0.6, 0.1, 4.0);
    let sun = match obj.get("sun") {
        None => None,
        Some(Value::Object(s)) => {
            check_keys(&mut errors, "sky.sun", s, SUN_KEYS);
            let dir = match s.get("direction").and_then(Value::as_array).filter(|a| a.len() == 3) {
                Some(a) => match (a[0].as_f64(), a[1].as_f64(), a[2].as_f64()) {
                    (Some(x), Some(y), Some(z)) if (x * x + y * y + z * z) > 1e-9 => Some(Vec3::new(x as f32, y as f32, z as f32).normalize()),
                    _ => None,
                },
                None => None,
            };
            if dir.is_none() {
                errors.push("sky.sun.direction: required, [x, y, z] pointing from the viewer toward the sun (not all zero)".to_string());
            }
            Some(Sun {
                direction: dir.unwrap_or(Vec3::X),
                radius_deg: num(&mut errors, s, "size_deg", "sky.sun", 2.5, 0.2, 30.0),
                color: hex(&mut errors, s, "color", "sky.sun", Vec3::new(1.0, 0.9, 0.7)),
                glow: num(&mut errors, s, "glow", "sky.sun", 0.7, 0.0, 2.0),
            })
        }
        Some(_) => {
            errors.push("sky.sun: must be an object like {\"direction\": [-1, 0.05, 0], \"size_deg\": 2.5}".to_string());
            None
        }
    };
    if errors.is_empty() {
        Ok(Some(Sky { zenith, horizon, gradient_power, sun }))
    } else {
        Err(errors)
    }
}

/// Parses the `ocean` block.
pub fn parse_ocean(root: &Map<String, Value>) -> Result<Option<Ocean>, Vec<String>> {
    let Some(raw) = root.get("ocean") else { return Ok(None) };
    let Some(obj) = raw.as_object() else { return Err(vec!["ocean: must be an object like {\"y\": 0, \"color_deep\": \"#0d2b3a\"}".to_string()]) };
    let mut errors = Vec::new();
    check_keys(&mut errors, "ocean", obj, OCEAN_KEYS);
    let ocean = Ocean {
        y: num(&mut errors, obj, "y", "ocean", 0.0, -1000.0, 1000.0),
        deep: hex(&mut errors, obj, "color_deep", "ocean", parse_hex_to_linear("#0d2b3a").unwrap_or(Vec3::ZERO)),
        shallow: hex(&mut errors, obj, "color_shallow", "ocean", parse_hex_to_linear("#2a7f8f").unwrap_or(Vec3::ZERO)),
        foam: hex(&mut errors, obj, "foam_color", "ocean", Vec3::ONE),
        wave_amplitude: num(&mut errors, obj, "wave_amplitude", "ocean", 0.05, 0.0, 2.0),
        wave_frequency: num(&mut errors, obj, "wave_frequency", "ocean", 0.8, 0.05, 6.0),
        wave_speed: num(&mut errors, obj, "wave_speed", "ocean", 1.0, 0.0, 8.0),
        roughness: num(&mut errors, obj, "roughness", "ocean", 0.12, 0.0, 1.0),
        depth_scale: num(&mut errors, obj, "depth_scale", "ocean", 4.0, 0.2, 100.0),
        haze_distance: num(&mut errors, obj, "haze_distance", "ocean", 260.0, 20.0, 5000.0),
    };
    if errors.is_empty() {
        Ok(Some(ocean))
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_sky_and_an_ocean_parse_with_defaults_and_say_what_is_wrong() {
        let root = json!({"sky": {"sun": {"direction": [-2, 0, 0]}}, "ocean": {"y": -0.5}});
        let root = root.as_object().unwrap();
        let sky = parse_sky(root, Vec3::ZERO, Vec3::ONE).unwrap().unwrap();
        let sun = sky.sun.unwrap();
        assert!((sun.direction - Vec3::NEG_X).length() < 1e-6, "normalised");
        assert_eq!((sky.zenith, sky.horizon), (Vec3::ZERO, Vec3::ONE), "defaults come from the background");
        let ocean = parse_ocean(root).unwrap().unwrap();
        assert_eq!((ocean.y, ocean.wave_frequency), (-0.5, 0.8));
        assert!(parse_sky(json!({}).as_object().unwrap(), Vec3::ZERO, Vec3::ONE).unwrap().is_none());
        for (bad, needle) in [
            (json!({"sky": {"sun": {}}}), "direction"),
            (json!({"sky": {"sun": {"direction": [0, 1, 0], "size_deg": 90}}}), "size_deg"),
            (json!({"sky": {"sunn": {}}}), "sunn"),
        ] {
            let e = parse_sky(bad.as_object().unwrap(), Vec3::ZERO, Vec3::ONE).unwrap_err();
            assert!(e.iter().any(|m| m.contains(needle)), "{needle}: {e:?}");
        }
        let e = parse_ocean(json!({"ocean": {"roughness": 3}}).as_object().unwrap()).unwrap_err();
        assert!(e[0].contains("ocean.roughness"), "{e:?}");
    }

    #[cfg(feature = "render")]
    const AURORA: &str = "fn sky(dir: vec3<f32>, px: f32) -> vec3<f32> {\n    let t = sky_time() + sky_seed();\n    return vec3<f32>(0.1, 0.4 + 0.3 * fbm(dir * 3.0 + t), 0.6) * max(dir.y, 0.0);\n}\n";

    #[cfg(feature = "render")]
    fn sky_dir(tag: &str, file: &str, code: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("re2-sky-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sky")).unwrap();
        std::fs::write(dir.join("sky").join(file), code).unwrap();
        dir
    }

    #[cfg(feature = "render")]
    #[test]
    fn a_game_sky_replaces_the_engines_and_is_checked_for_every_backend() {
        // The engine's own module is untouched without one, and a game's sky sits in place of the hook with the engine's helpers before it.
        assert_eq!(background_source(None, None), BACKGROUND_WGSL);
        let with = background_source(Some(AURORA), None);
        assert!(with.contains(AURORA) && with.contains("return sky(dir, px);") && !with.contains("return sky_color_px(dir, px);"));
        assert_eq!(crate::shader_check::check("aurora", &with), Ok(()));
        // Parsed from a scene: the file is read next to it and the seed kept.
        let dir = sky_dir("ok", "aurora.wgsl", AURORA);
        let root = json!({"sky": {"shader": "sky/aurora.wgsl", "seed": "launch"}});
        let sky = parse_sky_shader(root.as_object().unwrap(), Some(&dir)).unwrap().unwrap();
        assert_eq!((sky.file.as_str(), sky.seed, sky.seed_for(Some(0x1234_5678)), sky.seed_for(None)), ("sky/aurora.wgsl", LaunchSeed::Launch, 0x34_5678, 0));
        let fixed = json!({"sky": {"shader": "sky/aurora.wgsl", "seed": 7}});
        assert_eq!(parse_sky_shader(fixed.as_object().unwrap(), Some(&dir)).unwrap().unwrap().seed_for(Some(99)), 7);
        assert!(parse_sky_shader(json!({"sky": {"zenith": "#000000"}}).as_object().unwrap(), Some(&dir)).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "render")]
    #[test]
    fn a_broken_game_sky_is_refused_naming_its_own_file_and_line() {
        // No `sky` function at all.
        let e = check_sky_shader("sky/x.wgsl", "fn skyy(d: vec3<f32>) -> vec3<f32> { return d; }\n").unwrap_err();
        assert!(e.contains("no `fn sky(") && e.contains("sky/x.wgsl"), "{e}");
        // A type error on line 3 of the game's file is reported as line 3 of that file, not of the composed module.
        let bad = "// a comment\nfn sky(dir: vec3<f32>, px: f32) -> vec3<f32> {\n    return dir.x;\n}\n";
        let e = check_sky_shader("sky/bad.wgsl", bad).unwrap_err();
        assert!(e.contains("sky/bad.wgsl") && e.contains("does not compile"), "{e}");
        assert!(e.lines().any(|l| l.trim_start().starts_with("3 │")), "the gutter shows the game's line 3:\n{e}");
        assert!(e.contains("sky/bad.wgsl:2:1"), "the location is the game's file and line:\n{e}");
        // A derivative inside a loop is what Direct3D refuses: refused here too.
        let in_loop = "fn sky(dir: vec3<f32>, px: f32) -> vec3<f32> {\n    var c = vec3<f32>(0.0);\n    for (var i = 0; i < 3; i = i + 1) {\n        c = c + fwidth(dir);\n    }\n    return c;\n}\n";
        let e = check_sky_shader("sky/loop.wgsl", in_loop).unwrap_err();
        assert!(e.contains("inside a loop"), "{e}");
        // Scene-level mistakes.
        let dir = sky_dir("bad", "ok.wgsl", AURORA);
        for (root, needle) in [
            (json!({"sky": {"shader": "sky/missing.wgsl"}}), "cannot be read"),
            (json!({"sky": {"shader": "sky/ok.glsl"}}), ".wgsl file"),
            (json!({"sky": {"shader": "sky/ok.wgsl", "seed": -1}}), "sky.seed"),
            (json!({"sky": {"shader": "sky/ok.wgsl", "seed": "random"}}), "sky.seed"),
            (json!({"sky": {"seed": 3}}), "only means something with `sky.shader`"),
        ] {
            let e = parse_sky_shader(root.as_object().unwrap(), Some(&dir)).unwrap_err();
            assert!(e.iter().any(|m| m.contains(needle)), "{needle}: {e:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
