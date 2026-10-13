//! The scene's `world` block: how far the walkable world reaches. A map may loop along one axis (an endless coast, a road, a corridor
//! that never ends) and may declare the limits of the other axis (where the wading has to stop).
//!
//! ```json
//! "world": { "wrap": { "axis": "z", "min": -120, "max": 120 }, "bounds": { "x": [-34, 60] } }
//! ```
//!
//! **Wrap** turns the axis into a circle of `max - min` metres: walk past `max` and you are at `min`, with your speed, look and
//! everything you carry intact. The map is authored as *one period*; the engine makes the seam invisible: the renderer draws the
//! neighbouring copies, static collision and ground near the seam exist on both sides (`collide`), remote players and rule zones measure
//! distance the short way round ([`Wrap::delta`]), and the player step wraps the position in the one place all of the single-player
//! game, the authoritative server and client prediction share (`sim::player`). **Bounds** clamp the position on the other axes so a
//! player cannot leave the map: the invisible wall at the edge of an ocean or a ridge, deterministic everywhere the step runs.
//!
//! Pure data and arithmetic: no window, no GPU (the headless server depends on it).

use crate::strict::check_keys;
use glam::Vec2;
use serde_json::{Map, Value};

/// `world` keys.
pub const WORLD_KEYS: &[&str] = &["wrap", "bounds"];
/// `world.wrap` keys.
pub const WRAP_KEYS: &[&str] = &["axis", "min", "max"];
/// `world.bounds` keys.
pub const BOUNDS_KEYS: &[&str] = &["x", "z"];
/// The shortest allowed period, m: shorter than this and a player sees themselves from behind (and the renderer's three copies no
/// longer reach the horizon).
pub const MIN_PERIOD: f32 = 60.0;

/// Which horizontal axis loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// World X.
    X,
    /// World Z.
    Z,
}

impl Axis {
    /// The component of a planar position (x, z) on this axis.
    pub fn of(self, p: Vec2) -> f32 {
        match self {
            Axis::X => p.x,
            Axis::Z => p.y,
        }
    }

    /// `p` with this axis's component replaced.
    pub fn with(self, mut p: Vec2, value: f32) -> Vec2 {
        match self {
            Axis::X => p.x = value,
            Axis::Z => p.y = value,
        }
        p
    }

    /// A planar offset of `amount` along this axis.
    pub fn offset(self, amount: f32) -> Vec2 {
        match self {
            Axis::X => Vec2::new(amount, 0.0),
            Axis::Z => Vec2::new(0.0, amount),
        }
    }

    /// `x` or `z`.
    pub fn name(self) -> &'static str {
        match self {
            Axis::X => "x",
            Axis::Z => "z",
        }
    }
}

/// One looping axis: positions live in `[min, max)` and `max` is `min`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wrap {
    /// The axis that loops.
    pub axis: Axis,
    /// The low end of the period, m.
    pub min: f32,
    /// The high end, m (exclusive).
    pub max: f32,
}

impl Wrap {
    /// The length of one loop, m.
    pub fn period(&self) -> f32 {
        self.max - self.min
    }

    /// `v` brought into `[min, max)`.
    pub fn wrap_value(&self, v: f32) -> f32 {
        let w = self.min + (v - self.min).rem_euclid(self.period());
        // rem_euclid of a tiny negative can round up to exactly the period: keep the half-open range honest.
        if w >= self.max {
            self.min
        } else {
            w
        }
    }

    /// The planar position `p` with its looping coordinate brought into the period.
    pub fn wrap_pos(&self, p: Vec2) -> Vec2 {
        self.axis.with(p, self.wrap_value(self.axis.of(p)))
    }

    /// The shortest signed way from `a` to `b` along the looping axis (the other axis is the plain difference): what distance, direction
    /// and interpolation mean on a world that loops.
    pub fn delta(&self, a: Vec2, b: Vec2) -> Vec2 {
        let mut d = b - a;
        let p = self.period();
        let along = self.axis.of(d);
        let short = (along + p * 0.5).rem_euclid(p) - p * 0.5;
        d = self.axis.with(d, short);
        d
    }

    /// The copy of `b` that is nearest to `a` (`a + delta(a, b)`), for measuring or drawing across the seam.
    pub fn nearest_image(&self, a: Vec2, b: Vec2) -> Vec2 {
        a + self.delta(a, b)
    }
}

/// The limits of the walkable world on the axes that do not loop.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Bounds {
    /// Allowed `x` range `(low, high)`, if limited.
    pub x: Option<(f32, f32)>,
    /// Allowed `z` range `(low, high)`, if limited.
    pub z: Option<(f32, f32)>,
}

/// How far the world reaches: an optional looping axis and optional walkable limits.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Expanse {
    /// The looping axis, if the world is endless along one.
    pub wrap: Option<Wrap>,
    /// The walkable limits.
    pub bounds: Bounds,
}

impl Expanse {
    /// True when the world has neither a loop nor limits (every classic map).
    pub fn is_plain(&self) -> bool {
        self.wrap.is_none() && self.bounds.x.is_none() && self.bounds.z.is_none()
    }

    /// `p` clamped into the walkable limits; the second value is which axes were clamped (`[x, z]`), so a caller can stop that
    /// velocity component.
    pub fn clamp_bounds(&self, p: Vec2) -> (Vec2, [bool; 2]) {
        let mut out = p;
        let mut hit = [false; 2];
        if let Some((lo, hi)) = self.bounds.x {
            let c = p.x.clamp(lo, hi);
            hit[0] = c != p.x;
            out.x = c;
        }
        if let Some((lo, hi)) = self.bounds.z {
            let c = p.y.clamp(lo, hi);
            hit[1] = c != p.y;
            out.y = c;
        }
        (out, hit)
    }

    /// `p` wrapped into the period (unchanged without a loop).
    pub fn wrap_pos(&self, p: Vec2) -> Vec2 {
        self.wrap.map_or(p, |w| w.wrap_pos(p))
    }

    /// The shortest vector from `a` to `b` (the plain difference without a loop).
    pub fn delta(&self, a: Vec2, b: Vec2) -> Vec2 {
        self.wrap.map_or(b - a, |w| w.delta(a, b))
    }

    /// The whole confinement in one call: clamp to the limits, then wrap.
    pub fn confine(&self, p: Vec2) -> Vec2 {
        self.wrap_pos(self.clamp_bounds(p).0)
    }
}

fn range(errors: &mut Vec<String>, obj: &Map<String, Value>, key: &str) -> Option<(f32, f32)> {
    let raw = obj.get(key)?;
    let pair = raw.as_array().filter(|a| a.len() == 2).and_then(|a| Some((a[0].as_f64()? as f32, a[1].as_f64()? as f32)));
    match pair {
        Some((lo, hi)) if lo < hi && lo.is_finite() && hi.is_finite() => Some((lo, hi)),
        Some(_) => {
            errors.push(format!("world.bounds.{key}: [low, high] needs low < high"));
            None
        }
        None => {
            errors.push(format!("world.bounds.{key}: must be [low, high] in metres"));
            None
        }
    }
}

/// Parses the scene's `world` block (a scene without one is [`Expanse::default`]).
pub fn parse_world(root: &Map<String, Value>) -> Result<Expanse, Vec<String>> {
    let Some(raw) = root.get("world") else { return Ok(Expanse::default()) };
    let Some(obj) = raw.as_object() else {
        return Err(vec!["world: must be an object like {\"wrap\": {\"axis\": \"z\", \"min\": -120, \"max\": 120}}".to_string()]);
    };
    let mut errors = Vec::new();
    check_keys(&mut errors, "world", obj, WORLD_KEYS);
    let mut expanse = Expanse::default();
    if let Some(w) = obj.get("wrap") {
        match w.as_object() {
            None => errors.push("world.wrap: must be an object like {\"axis\": \"z\", \"min\": -120, \"max\": 120}".to_string()),
            Some(w) => {
                check_keys(&mut errors, "world.wrap", w, WRAP_KEYS);
                let axis = match w.get("axis").and_then(Value::as_str) {
                    Some("x") | Some("X") => Some(Axis::X),
                    Some("z") | Some("Z") => Some(Axis::Z),
                    _ => {
                        errors.push("world.wrap.axis: must be \"x\" or \"z\"".to_string());
                        None
                    }
                };
                let min = w.get("min").and_then(Value::as_f64).map(|v| v as f32);
                let max = w.get("max").and_then(Value::as_f64).map(|v| v as f32);
                match (axis, min, max) {
                    (Some(axis), Some(min), Some(max)) if max - min >= MIN_PERIOD && min.is_finite() && max.is_finite() => {
                        expanse.wrap = Some(Wrap { axis, min, max })
                    }
                    (Some(_), Some(min), Some(max)) => {
                        errors.push(format!("world.wrap: max - min = {} m is too short a loop (at least {MIN_PERIOD} m, or you would see the seam)", max - min))
                    }
                    (Some(_), _, _) => errors.push("world.wrap: needs numbers `min` and `max` (metres along the axis)".to_string()),
                    _ => {}
                }
            }
        }
    }
    if let Some(b) = obj.get("bounds") {
        match b.as_object() {
            None => errors.push("world.bounds: must be an object like {\"x\": [-30, 60]}".to_string()),
            Some(b) => {
                check_keys(&mut errors, "world.bounds", b, BOUNDS_KEYS);
                expanse.bounds.x = range(&mut errors, b, "x");
                expanse.bounds.z = range(&mut errors, b, "z");
            }
        }
    }
    if let Some(w) = expanse.wrap {
        let limited = match w.axis {
            Axis::X => expanse.bounds.x,
            Axis::Z => expanse.bounds.z,
        };
        if limited.is_some() {
            errors.push(format!("world.bounds.{}: the axis that loops has no edge (remove it, or stop wrapping {})", w.axis.name(), w.axis.name()));
        }
    }
    if errors.is_empty() {
        Ok(expanse)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn z_wrap() -> Wrap {
        Wrap { axis: Axis::Z, min: -100.0, max: 100.0 }
    }

    #[test]
    fn positions_wrap_into_a_half_open_period() {
        let w = z_wrap();
        assert_eq!(w.wrap_value(100.0), -100.0);
        assert_eq!(w.wrap_value(-100.0), -100.0);
        assert!((w.wrap_value(101.5) - -98.5).abs() < 1e-4);
        assert!((w.wrap_value(-100.5) - 99.5).abs() < 1e-4);
        assert!((w.wrap_value(1e6 + 3.0) - w.wrap_value(3.0)).abs() < 0.2, "far away stays inside the period");
        for v in [-1e-9f32, 99.999_99, -100.0, 300.0] {
            let r = w.wrap_value(v);
            assert!((-100.0..100.0).contains(&r), "{v} -> {r}");
        }
    }

    #[test]
    fn distance_and_direction_go_the_short_way_round() {
        let w = z_wrap();
        let d = w.delta(Vec2::new(0.0, 99.0), Vec2::new(4.0, -99.0));
        assert!((d.x - 4.0).abs() < 1e-4 && (d.y - 2.0).abs() < 1e-3, "{d}");
        let back = w.delta(Vec2::new(0.0, -99.0), Vec2::new(0.0, 99.0));
        assert!((back.y - -2.0).abs() < 1e-3, "{back}");
        // The un-looped axis is a plain difference.
        assert_eq!(w.delta(Vec2::new(-3.0, 0.0), Vec2::new(9.0, 0.0)).x, 12.0);
    }

    #[test]
    fn bounds_clamp_and_report_which_axis_hit() {
        let e = Expanse { wrap: Some(z_wrap()), bounds: Bounds { x: Some((-30.0, 50.0)), z: None } };
        let (p, hit) = e.clamp_bounds(Vec2::new(-31.0, 5.0));
        assert_eq!((p, hit), (Vec2::new(-30.0, 5.0), [true, false]));
        assert_eq!(e.confine(Vec2::new(10.0, 101.0)), Vec2::new(10.0, -99.0));
        assert!(Expanse::default().is_plain() && !e.is_plain());
    }

    #[test]
    fn the_block_parses_and_bad_ones_say_why() {
        let ok = parse_world(json!({"world": {"wrap": {"axis": "z", "min": -120, "max": 120}, "bounds": {"x": [-34, 60]}}}).as_object().unwrap()).unwrap();
        assert_eq!(ok.wrap.unwrap().period(), 240.0);
        assert_eq!(ok.bounds.x, Some((-34.0, 60.0)));
        assert_eq!(parse_world(json!({}).as_object().unwrap()).unwrap(), Expanse::default());
        for (bad, needle) in [
            (json!({"world": {"wrap": {"axis": "y", "min": 0, "max": 100}}}), "axis"),
            (json!({"world": {"wrap": {"axis": "z", "min": 0, "max": 10}}}), "too short"),
            (json!({"world": {"bounds": {"x": [5, 1]}}}), "low < high"),
            (json!({"world": {"wrap": {"axis": "z", "min": 0, "max": 100}, "bounds": {"z": [0, 5]}}}), "no edge"),
            (json!({"world": {"wrapp": {}}}), "wrapp"),
        ] {
            let errs = parse_world(bad.as_object().unwrap()).unwrap_err();
            assert!(errs.iter().any(|e| e.contains(needle)), "{needle}: {errs:?}");
        }
    }
}
