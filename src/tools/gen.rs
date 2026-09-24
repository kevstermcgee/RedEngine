//! Generators that write *concrete* objects into a scene: `scatter` (seeded random placement of
//! props inside a region, avoiding walls/furniture/each other) and `line` (evenly spaced props
//! along a segment).
//!
//! The output is plain `prop` objects, not a hidden procedural rule — the map stays fully
//! inspectable and hand-editable, and the seed makes a scatter reproducible if you re-run it
//! (`rm --match 'tree_*'` then scatter again with a different seed to re-roll a garden).

use super::edit::num;
use super::world::MapWorld;
use crate::props::{collision_box, local_bounds, PropKind};
use crate::viewer::collider_blocks_at;
use glam::Vec2;
use serde_json::{json, Value};

/// SplitMix64: tiny, fast, and good enough for placement jitter.
pub struct Rng(u64);

impl Rng {
    /// A deterministic RNG seeded with `seed` (same seed, same layout).
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_add(0x9E37_79B9_7F4A_7C15))
    }
    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in `[0, 1)`.
    pub fn f(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    /// A uniform float in `lo..hi`.
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f()
    }
    /// A uniformly chosen element of `items`.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[(self.next_u64() % items.len() as u64) as usize]
    }
}

/// Parameters of `scatter`: kinds, count, allowed/excluded rects, seed, clearance and spacing.
pub struct ScatterParams {
    pub kinds: Vec<PropKind>,
    pub count: usize,
    pub rects: Vec<(Vec2, Vec2)>,
    pub excludes: Vec<(Vec2, Vec2)>,
    pub seed: u64,
    pub id_prefix: String,
    pub colors: Vec<String>,
    pub scale: (f32, f32),
    /// Extra empty space kept between scattered items (on top of their own radii).
    pub min_gap: f32,
    /// Distance kept from existing walls/furniture, beyond the item's own radius.
    pub clearance: f32,
    /// Height of the surface being planted on.
    pub y: f32,
    /// Random yaw (default true); off keeps everything facing +Z.
    pub random_yaw: bool,
    /// Lint checks to silence on every generated object (`"lint_ignore"`), e.g. `unreachable`
    /// for a decorative tree line outside the fence.
    pub lint_ignore: Vec<String>,
}

fn horiz_extent(min: glam::Vec3, max: glam::Vec3) -> f32 {
    ((max.x - min.x).max(max.z - min.z)) * 0.5
}

/// Radius of the part of the prop that blocks the player (the trunk for a tree).
fn body_radius(kind: PropKind, scale: f32) -> f32 {
    let (mn, mx) = collision_box(kind).unwrap_or_else(|| local_bounds(kind));
    horiz_extent(mn, mx) * scale
}

/// Radius used to keep scattered things from crowding each other visually (a tree's canopy).
fn spacing_radius(kind: PropKind, scale: f32) -> f32 {
    let (mn, mx) = local_bounds(kind);
    let r = horiz_extent(mn, mx) * scale;
    match kind {
        PropKind::TreeOak | PropKind::TreePine => r * 0.7,
        _ => r,
    }
}

fn default_color(kind: PropKind) -> &'static str {
    match kind {
        PropKind::TreeOak => "#3f7a34",
        PropKind::TreePine => "#1f5a3a",
        PropKind::Bush => "#3d7d3a",
        PropKind::Hedge => "#2f6a30",
        PropKind::FlowerPatch => "#e0587a",
        PropKind::Boulder => "#8a8a84",
        _ => "#b0b0b0",
    }
}

/// Multiplies each channel of `#rrggbb` by a random factor in `1 ± amount`.
fn jitter_hex(hex: &str, amount: f32, rng: &mut Rng) -> String {
    let h = hex.trim_start_matches('#');
    if h.len() < 6 {
        return hex.to_string();
    }
    let k = 1.0 + rng.range(-amount, amount);
    let c = |i: usize| (u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(128) as f32 * k).clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", c(0), c(2), c(4))
}

fn prop_object(id: String, kind: PropKind, pos: [f32; 3], yaw: f32, scale: f32, color: &str, lint_ignore: &[String]) -> Value {
    let mut o = json!({
        "id": id,
        "type": "prop",
        "prop": kind.name(),
        "position": [num(pos[0] as f64), num(pos[1] as f64), num(pos[2] as f64)],
        "rotation": [0, num(yaw as f64), 0],
        "material": { "color": color, "roughness": 0.85 },
    });
    if (scale - 1.0).abs() > 0.005 {
        o["scale"] = num(scale as f64);
    }
    if !lint_ignore.is_empty() {
        o["lint_ignore"] = json!(lint_ignore);
    }
    o
}

/// Seeded random placement that avoids walls, props and other placed items; returns the new object JSON (nothing is written here).
pub fn scatter(world: &MapWorld, p: &ScatterParams) -> Result<Vec<Value>, String> {
    if p.kinds.is_empty() {
        return Err("no prop kinds given".to_string());
    }
    if p.rects.is_empty() {
        return Err("give a region with --rect x0,z0,x1,z1 or --zone <zone id>".to_string());
    }
    let mut rng = Rng::new(p.seed);
    let total_area: f32 = p.rects.iter().map(|(a, b)| (b.x - a.x).max(0.0) * (b.y - a.y).max(0.0)).sum();
    if total_area <= 0.0 {
        return Err("the region has no area".to_string());
    }
    let mut placed: Vec<(Vec2, f32)> = Vec::new();
    let mut out = Vec::new();
    let mut attempts = 0;
    while out.len() < p.count && attempts < p.count * 400 + 200 {
        attempts += 1;
        // Area-weighted rect choice, then a uniform point in it.
        let mut t = rng.f() * total_area;
        let mut rect = p.rects[0];
        for r in &p.rects {
            let a = (r.1.x - r.0.x).max(0.0) * (r.1.y - r.0.y).max(0.0);
            if t <= a {
                rect = *r;
                break;
            }
            t -= a;
        }
        let pos = Vec2::new(rng.range(rect.0.x, rect.1.x), rng.range(rect.0.y, rect.1.y));
        if p.excludes.iter().any(|(a, b)| pos.x >= a.x && pos.x <= b.x && pos.y >= a.y && pos.y <= b.y) {
            continue;
        }
        let kind = *rng.pick(&p.kinds);
        let scale = rng.range(p.scale.0, p.scale.1);
        let (body, spacing) = (body_radius(kind, scale), spacing_radius(kind, scale));
        // Keep clear of walls/furniture at this height.
        let blocked = world.colliders.iter().filter(|c| collider_blocks_at(c, p.y)).any(|c| {
            let d = pos - pos.clamp(c.min, c.max);
            d.length() < body + p.clearance
        });
        if blocked {
            continue;
        }
        if placed.iter().any(|(q, r)| (*q - pos).length() < spacing + r + p.min_gap) {
            continue;
        }
        placed.push((pos, spacing));
        let yaw = if p.random_yaw { rng.range(0.0, 360.0) } else { 0.0 };
        let color = if p.colors.is_empty() { jitter_hex(default_color(kind), 0.14, &mut rng) } else { rng.pick(&p.colors).clone() };
        out.push(prop_object(format!("{}_{}", p.id_prefix, out.len() + 1), kind, [pos.x, p.y, pos.y], yaw, scale, &color, &p.lint_ignore));
    }
    if out.len() < p.count {
        eprintln!("note: only placed {} of {} (the region is crowded — widen it or lower --min-gap/--clearance)", out.len(), p.count);
    }
    Ok(out)
}

/// Parameters of `line`: prop kind, endpoints and spacing.
pub struct LineParams {
    pub kind: PropKind,
    pub from: Vec2,
    pub to: Vec2,
    /// Center-to-center distance between items. Default: the prop's own length (touching).
    pub spacing: f32,
    pub id_prefix: String,
    pub color: Option<String>,
    pub y: f32,
    pub scale: f32,
    pub lint_ignore: Vec<String>,
}

/// Evenly spaced copies along `from -> to`, each turned to run along the line (props with a
/// long axis, like `hedge`/`fence_section`, are authored along local `+X`).
pub fn line(p: &LineParams) -> Result<Vec<Value>, String> {
    let d = p.to - p.from;
    let len = d.length();
    if len < 0.1 {
        return Err("--from and --to are the same point".to_string());
    }
    let n = ((len / p.spacing).round() as usize).max(1);
    let step = len / n as f32;
    let yaw = (-d.y).atan2(d.x).to_degrees();
    let mut rng = Rng::new(7);
    let color = p.color.clone().unwrap_or_else(|| default_color(p.kind).to_string());
    Ok((0..n)
        .map(|i| {
            let c = p.from + d / len * (step * (i as f32 + 0.5));
            let col = if p.color.is_some() { color.clone() } else { jitter_hex(&color, 0.08, &mut rng) };
            prop_object(format!("{}_{}", p.id_prefix, i + 1), p.kind, [c.x, p.y, c.y], yaw, p.scale, &col, &p.lint_ignore)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn world() -> MapWorld {
        MapWorld::from_text(
            r##"{"camera":{},"objects":[{"id":"wall","type":"box","size":[10,2,0.4],"position":[0,1,0]}]}"##,
            Path::new("t.json"),
        )
        .unwrap()
    }

    fn params(seed: u64) -> ScatterParams {
        ScatterParams {
            kinds: vec![PropKind::Bush, PropKind::TreeOak],
            count: 12,
            rects: vec![(Vec2::new(-10.0, -6.0), Vec2::new(10.0, 6.0))],
            excludes: vec![(Vec2::new(-1.0, -1.0), Vec2::new(1.0, 1.0))],
            seed,
            id_prefix: "g".into(),
            colors: vec![],
            scale: (0.9, 1.2),
            min_gap: 0.4,
            clearance: 0.6,
            y: 0.0,
            random_yaw: true,
            lint_ignore: vec![],
        }
    }

    #[test]
    fn scatter_is_deterministic_and_respects_constraints() {
        let w = world();
        let a = scatter(&w, &params(3)).unwrap();
        let b = scatter(&w, &params(3)).unwrap();
        assert_eq!(a, b, "same seed, same layout");
        assert_ne!(a, scatter(&w, &params(4)).unwrap(), "different seed, different layout");
        assert_eq!(a.len(), 12);
        for o in &a {
            let p = o["position"].as_array().unwrap();
            let (x, z) = (p[0].as_f64().unwrap() as f32, p[2].as_f64().unwrap() as f32);
            assert!(!(x.abs() <= 1.0 && z.abs() <= 1.0), "must stay out of the exclude rect");
            assert!(!(x.abs() <= 5.3 && z.abs() <= 0.8), "must keep clear of the wall ({x}, {z})");
        }
    }

    #[test]
    fn line_places_evenly_and_faces_along_the_line() {
        let objs = line(&LineParams { kind: PropKind::Hedge, from: Vec2::new(0.0, 0.0), to: Vec2::new(0.0, 5.4), spacing: 1.8, id_prefix: "h".into(), color: None, y: 0.0, scale: 1.0, lint_ignore: vec![] }).unwrap();
        assert_eq!(objs.len(), 3);
        // Running along +Z means the prop's local +X must map to +Z: yaw -90.
        assert!((objs[0]["rotation"][1].as_f64().unwrap() - -90.0).abs() < 1e-3);
        let z1 = objs[0]["position"][2].as_f64().unwrap();
        let z2 = objs[1]["position"][2].as_f64().unwrap();
        assert!((z2 - z1 - 1.8).abs() < 1e-3);
    }
}
