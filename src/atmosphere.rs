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
//! Presentation only: nothing here changes what the simulation does.

use crate::color::parse_hex_to_linear;
use crate::strict::check_keys;
use glam::Vec3;
use serde_json::{Map, Value};

/// `sky` keys.
pub const SKY_KEYS: &[&str] = &["zenith", "horizon", "gradient_power", "sun"];
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
}
