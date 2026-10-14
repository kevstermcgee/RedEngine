//! Heightfield terrain: the `terrain` object type. A rolling surface (dunes, a shore, a headland) that is drawn, walked on and lit as
//! one coherent thing, instead of a scatter of scaled spheres that read as geometric clumps and cannot be walked over.
//!
//! ```json
//! { "id": "shore", "type": "terrain", "position": [10, 0, 0], "size": [120, 240], "cell": 1.0,
//!   "generator": { "seed": 7,
//!                  "profile": [[-50, -2.5], [-20, -0.6], [-4, 0.1], [12, 0.9], [40, 4.0]],
//!                  "noise": { "amplitude": 0.7, "wavelength": [20, 40], "octaves": 3, "fade_x": [-12, 4] } },
//!   "palette": [[-2.5, "#5b6f78"], [-0.1, "#a89474"], [0.3, "#e6d3a8"], [3.0, "#8a9a5b"]], "grain": 0.05,
//!   "material": { "roughness": 0.95 } }
//! ```
//!
//! A terrain gets its heights from exactly one of `heights` (rows of numbers, z-major), `heightmap` (a grey PNG, white =
//! `elevation_scale` metres above `position.y`) or `generator` (a piecewise-smooth cross-section `profile` along x plus fractal value
//! `noise`). Everything is deterministic integer-hashed arithmetic, so the server and every client build bit-identical ground.
//!
//! `position` is the centre of the footprint (x, z) and the height that zero maps to (y); `rotation` and `scale` are refused (the
//! heights are world heights). On a world that loops (`world.wrap`) a terrain must span exactly one period along that axis; it then tiles
//! seamlessly (the generator's noise is periodic on that axis and lookups wrap).
//!
//! Walking: where a terrain exists it *is* the ground (`collide::ground_height_at`), boxes and stairs may stand on it, and there is
//! no invisible floor at y = 0 beneath it, so a seabed below sea level is walkable. Slopes steeper than [`MAX_WALK_SLOPE`] are a lint
//! error (`terrain-slope`): a player climbs any slope instantly, so a cliff would read as a ramp.
//!
//! Pure data and arithmetic (the triangle list [`Terrain::build_mesh`] returns is plain data: the renderer and the physics world each turn it into
//! what they need); nothing here touches a GPU, so the headless server builds it.

use crate::color::parse_hex_to_linear;
use crate::expanse::{Axis, Wrap};
use crate::strict::check_keys;
use glam::{Vec2, Vec3};
use serde_json::{Map, Value};
use std::path::Path;

/// Keys of a `terrain` object beyond the common ones.
pub const TERRAIN_KEYS: &[&str] = &["size", "cell", "resolution", "heights", "heightmap", "elevation_scale", "generator", "palette", "grain", "material"];
/// `generator` keys.
pub const GENERATOR_KEYS: &[&str] = &["seed", "profile", "noise"];
/// `generator.noise` keys.
pub const NOISE_KEYS: &[&str] = &["amplitude", "wavelength", "octaves", "seed", "fade_x"];
/// The steepest slope (rise over run) a terrain may have where players walk; `lint` reports steeper ground (`terrain-slope`).
pub const MAX_WALK_SLOPE: f32 = 1.0;
/// The most samples one terrain may have (a 1 m cell over 1 km x 1 km).
pub const MAX_SAMPLES: usize = 1_000_000;

/// One vertex of a terrain's triangle list.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainVertex {
    /// Position in the object's local space.
    pub pos: [f32; 3],
    /// Unit normal.
    pub normal: [f32; 3],
    /// Colour multiplier (linear RGB).
    pub color: [f32; 3],
}

/// A terrain's triangles: counter-clockwise from above.
#[derive(Debug, Clone, Default)]
pub struct TerrainMesh {
    /// The vertices, row-major over the samples.
    pub vertices: Vec<TerrainVertex>,
    /// Three indices per triangle.
    pub indices: Vec<u32>,
}

/// A heightfield: `nx` x `nz` samples of world height, `cell` metres apart, from `origin` (the low corner).
#[derive(Debug, Clone)]
pub struct Terrain {
    /// World (x, z) of sample (0, 0).
    pub origin: Vec2,
    /// Metres between samples along x and z.
    pub cell: Vec2,
    /// Samples along x.
    pub nx: usize,
    /// Samples along z.
    pub nz: usize,
    /// Which axis (if any) is periodic: that axis has `n` cells for `n` samples (the last neighbours the first) instead of `n - 1`.
    pub periodic: Option<Axis>,
    /// World height of each sample, row-major (`iz * nx + ix`).
    pub heights: Vec<f32>,
    /// Height-banded colours, ascending (empty = the material colour alone).
    pub palette: Vec<(f32, Vec3)>,
    /// Brightness variation of the colour, 0 (flat) .. 0.3.
    pub grain: f32,
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn hash(ix: i32, iz: i32, seed: u32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(0x9E37_79B1) ^ (iz as u32).wrapping_mul(0x85EB_CA6B) ^ seed.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h >> 8) as f32 / 8_388_608.0 - 1.0 // [-1, 1)
}

/// Smooth value noise at lattice coordinates `(u, v)`, in `[-1, 1]`; an axis with `Some(n)` repeats every `n` lattice cells.
fn value_noise(u: f32, v: f32, per_u: Option<i32>, per_v: Option<i32>, seed: u32) -> f32 {
    let (fu, fv) = (u.floor(), v.floor());
    let (iu, iv) = (fu as i32, fv as i32);
    let (tu, tv) = (smooth(u - fu), smooth(v - fv));
    let wrap = |i: i32, per: Option<i32>| per.map_or(i, |p| i.rem_euclid(p));
    let c = |du: i32, dv: i32| hash(wrap(iu + du, per_u), wrap(iv + dv, per_v), seed);
    let a = c(0, 0) + (c(1, 0) - c(0, 0)) * tu;
    let b = c(0, 1) + (c(1, 1) - c(0, 1)) * tu;
    a + (b - a) * tv
}

/// Where the parsed generator puts noise, and how loud.
#[derive(Debug, Clone)]
struct Noise {
    amplitude: f32,
    wavelength: Vec2,
    octaves: u32,
    seed: u32,
    fade_x: Option<(f32, f32)>,
}

/// Piecewise-smooth interpolation of `profile` (`(x, height)` knots, ascending in x) at `x`; flat beyond the ends.
fn profile_at(profile: &[(f32, f32)], x: f32) -> f32 {
    match profile {
        [] => 0.0,
        [only] => only.1,
        _ => {
            if x <= profile[0].0 {
                return profile[0].1;
            }
            for pair in profile.windows(2) {
                let ((x0, h0), (x1, h1)) = (pair[0], pair[1]);
                if x <= x1 {
                    return h0 + (h1 - h0) * smooth((x - x0) / (x1 - x0).max(1e-4));
                }
            }
            profile[profile.len() - 1].1
        }
    }
}

impl Terrain {
    /// The number of cells along x and z (samples, minus one on a non-periodic axis).
    fn cells(&self) -> (usize, usize) {
        (if self.periodic == Some(Axis::X) { self.nx } else { self.nx - 1 }, if self.periodic == Some(Axis::Z) { self.nz } else { self.nz - 1 })
    }

    /// The footprint's size in metres.
    pub fn size(&self) -> Vec2 {
        let (cx, cz) = self.cells();
        Vec2::new(cx as f32 * self.cell.x, cz as f32 * self.cell.y)
    }

    fn at(&self, ix: usize, iz: usize) -> f32 {
        self.heights[iz * self.nx + ix]
    }

    /// The footprint's low and high corner (x, z).
    pub fn footprint(&self) -> (Vec2, Vec2) {
        (self.origin, self.origin + self.size())
    }

    /// World height at `(x, z)` by bilinear interpolation, or `None` outside the footprint (a periodic axis has no outside).
    pub fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        let size = self.size();
        let (mut u, mut v) = ((x - self.origin.x) / self.cell.x, (z - self.origin.y) / self.cell.y);
        let (cx, cz) = self.cells();
        match self.periodic {
            Some(Axis::X) => u = u.rem_euclid(cx as f32),
            Some(Axis::Z) => v = v.rem_euclid(cz as f32),
            None => {}
        }
        // A whisker of tolerance so a query exactly on the far edge (floating-point wrap or a spawn on the boundary) still lands.
        let eps = 1e-3;
        if u < -eps || v < -eps || u > cx as f32 + eps || v > cz as f32 + eps || size.x <= 0.0 || size.y <= 0.0 {
            return None;
        }
        let (u, v) = (u.clamp(0.0, cx as f32), v.clamp(0.0, cz as f32));
        let (iu, iv) = ((u.floor() as usize).min(cx.saturating_sub(1)), (v.floor() as usize).min(cz.saturating_sub(1)));
        let (fu, fv) = (u - iu as f32, v - iv as f32);
        let ix1 = if iu + 1 >= self.nx { 0 } else { iu + 1 };
        let iz1 = if iv + 1 >= self.nz { 0 } else { iv + 1 };
        let (h00, h10, h01, h11) = (self.at(iu, iv), self.at(ix1, iv), self.at(iu, iz1), self.at(ix1, iz1));
        let a = h00 + (h10 - h00) * fu;
        let b = h01 + (h11 - h01) * fu;
        Some(a + (b - a) * fv)
    }

    /// The lowest and highest sample.
    pub fn height_range(&self) -> (f32, f32) {
        self.heights.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &h| (lo.min(h), hi.max(h)))
    }

    /// The steepest slope (rise over run) between neighbouring samples whose cell centre lies inside `region` (low and high corner in x, z),
    /// or everywhere when `None`; also returns where it is.
    pub fn steepest_slope(&self, region: Option<(Vec2, Vec2)>) -> (f32, Vec2) {
        let (cx, cz) = self.cells();
        let mut best = (0.0f32, self.origin);
        for iz in 0..cz {
            for ix in 0..cx {
                let centre = self.origin + Vec2::new((ix as f32 + 0.5) * self.cell.x, (iz as f32 + 0.5) * self.cell.y);
                if let Some((lo, hi)) = region {
                    if centre.x < lo.x || centre.x > hi.x || centre.y < lo.y || centre.y > hi.y {
                        continue;
                    }
                }
                let ix1 = (ix + 1) % self.nx;
                let iz1 = (iz + 1) % self.nz;
                let h = self.at(ix, iz);
                let sx = (self.at(ix1, iz) - h).abs() / self.cell.x;
                let sz = (self.at(ix, iz1) - h).abs() / self.cell.y;
                let s = sx.max(sz);
                if s > best.0 {
                    best = (s, centre);
                }
            }
        }
        best
    }

    /// The surface colour at height `h` (linear RGB): the palette blended by height, with the grain applied by the caller.
    pub fn colour_at(&self, h: f32) -> Vec3 {
        match self.palette.as_slice() {
            [] => Vec3::ONE,
            [(_, c)] => *c,
            p => {
                if h <= p[0].0 {
                    return p[0].1;
                }
                for pair in p.windows(2) {
                    if h <= pair[1].0 {
                        let t = (h - pair[0].0) / (pair[1].0 - pair[0].0).max(1e-4);
                        return pair[0].1.lerp(pair[1].1, smooth(t));
                    }
                }
                p[p.len() - 1].1
            }
        }
    }

    /// The triangle mesh, in the object's local space (x, z relative to the footprint centre, y relative to `base_y`): one vertex per
    /// sample (plus a closing row on a periodic axis), smooth normals, vertex colours from the palette and grain.
    pub fn build_mesh(&self, centre: Vec2, base_y: f32) -> TerrainMesh {
        let (cx, cz) = self.cells();
        let (vx, vz) = (cx + 1, cz + 1);
        let mut mesh = TerrainMesh::default();
        mesh.vertices.reserve(vx * vz);
        let h = |ix: i64, iz: i64| -> f32 {
            let ix = if self.periodic == Some(Axis::X) { ix.rem_euclid(self.nx as i64) } else { ix.clamp(0, self.nx as i64 - 1) };
            let iz = if self.periodic == Some(Axis::Z) { iz.rem_euclid(self.nz as i64) } else { iz.clamp(0, self.nz as i64 - 1) };
            self.at(ix as usize, iz as usize)
        };
        for iz in 0..vz {
            for ix in 0..vx {
                let (i, j) = (ix as i64, iz as i64);
                let y = h(i, j);
                let dx = (h(i + 1, j) - h(i - 1, j)) / (2.0 * self.cell.x);
                let dz = (h(i, j + 1) - h(i, j - 1)) / (2.0 * self.cell.y);
                let normal = Vec3::new(-dx, 1.0, -dz).normalize();
                let world = self.origin + Vec2::new(ix as f32 * self.cell.x, iz as f32 * self.cell.y);
                let mut colour = self.colour_at(y);
                if self.grain > 0.0 {
                    let n = hash(ix as i32 % self.nx.max(1) as i32, iz as i32 % self.nz.max(1) as i32, 0x51ED) * self.grain;
                    colour *= 1.0 + n;
                }
                mesh.vertices.push(TerrainVertex {
                    pos: [world.x - centre.x, y - base_y, world.y - centre.y],
                    normal: normal.to_array(),
                    color: colour.to_array(),
                });
            }
        }
        for iz in 0..cz {
            for ix in 0..cx {
                let a = (iz * vx + ix) as u32;
                let (b, c, d) = (a + 1, a + vx as u32, a + vx as u32 + 1);
                // CCW seen from above (+Y): (x, z) -> (x, z + 1) -> (x + 1, z + 1).
                mesh.indices.extend_from_slice(&[a, c, d, a, d, b]);
            }
        }
        mesh
    }
}

fn number(obj: &Map<String, Value>, key: &str, path: &str, errors: &mut Vec<String>) -> Option<f32> {
    match obj.get(key) {
        None => None,
        Some(v) => match v.as_f64() {
            Some(n) if n.is_finite() => Some(n as f32),
            _ => {
                errors.push(format!("{path}.{key}: must be a number"));
                None
            }
        },
    }
}

fn pair(v: &Value) -> Option<(f32, f32)> {
    let a = v.as_array().filter(|a| a.len() == 2)?;
    Some((a[0].as_f64()? as f32, a[1].as_f64()? as f32))
}

fn parse_noise(raw: &Value, path: &str, errors: &mut Vec<String>) -> Option<Noise> {
    let Some(obj) = raw.as_object() else {
        errors.push(format!("{path}: must be an object like {{\"amplitude\": 0.7, \"wavelength\": 20}}"));
        return None;
    };
    check_keys(errors, path, obj, NOISE_KEYS);
    let amplitude = number(obj, "amplitude", path, errors).unwrap_or(0.5);
    let wavelength = match obj.get("wavelength") {
        None => Vec2::splat(16.0),
        Some(v) => match (v.as_f64(), pair(v)) {
            (Some(w), _) => Vec2::splat(w as f32),
            (_, Some((wx, wz))) => Vec2::new(wx, wz),
            _ => {
                errors.push(format!("{path}.wavelength: must be a number or [along x, along z] in metres"));
                Vec2::splat(16.0)
            }
        },
    };
    if wavelength.x < 1.0 || wavelength.y < 1.0 {
        errors.push(format!("{path}.wavelength: at least 1 m (the terrain is sampled about every metre)"));
    }
    let octaves = obj.get("octaves").and_then(Value::as_u64).unwrap_or(3);
    if !(1..=6).contains(&octaves) {
        errors.push(format!("{path}.octaves: 1 to 6"));
    }
    let seed = obj.get("seed").and_then(Value::as_u64).unwrap_or(1) as u32;
    let fade_x = match obj.get("fade_x") {
        None => None,
        Some(v) => match pair(v) {
            Some(p) => Some(p),
            None => {
                errors.push(format!("{path}.fade_x: must be [x where the noise is silent, x where it is full]"));
                None
            }
        },
    };
    Some(Noise { amplitude, wavelength, octaves: octaves.clamp(1, 6) as u32, seed, fade_x })
}

/// Everything a `terrain` object needs from its surroundings.
pub struct TerrainContext<'a> {
    /// The scene's looping axis, if any.
    pub wrap: Option<Wrap>,
    /// The folder a `heightmap` path is relative to (the scene file's), if known.
    pub base_dir: Option<&'a Path>,
}

/// Builds the heightfield for one `terrain` object. `centre` is its `position`; `id` prefixes every message.
pub fn parse_terrain(obj: &Map<String, Value>, id: &str, centre: Vec3, ctx: &TerrainContext) -> Result<Terrain, Vec<String>> {
    let mut errors = Vec::new();
    // (The object's own keys are checked by the caller against the common keys plus `TERRAIN_KEYS`.)
    for refused in ["rotation", "scale"] {
        if obj.contains_key(refused) {
            errors.push(format!("{id}.{refused}: a terrain has world heights and cannot be rotated or scaled (change `size`, `cell` or the heights)"));
        }
    }
    let size = match obj.get("size").and_then(pair) {
        Some((sx, sz)) if sx >= 4.0 && sz >= 4.0 => Vec2::new(sx, sz),
        _ => {
            errors.push(format!("{id}.size: required, [width along x, depth along z] in metres, at least 4 x 4"));
            return Err(errors);
        }
    };
    let cell = number(obj, "cell", id, &mut errors).unwrap_or(1.0);
    if !(0.1..=32.0).contains(&cell) {
        errors.push(format!("{id}.cell: metres between samples, 0.1 to 32"));
        return Err(errors);
    }
    let periodic = ctx.wrap.map(|w| w.axis);
    // The samples per axis: `resolution` says it outright, `cell` derives it. A periodic axis has one cell per sample.
    let (mut nx, mut nz) = match obj.get("resolution").and_then(pair) {
        Some((rx, rz)) if rx >= 2.0 && rz >= 2.0 => (rx as usize, rz as usize),
        _ if obj.contains_key("resolution") => {
            errors.push(format!("{id}.resolution: [samples along x, samples along z], each at least 2"));
            return Err(errors);
        }
        _ => ((size.x / cell).round() as usize + 1, (size.y / cell).round() as usize + 1),
    };
    if periodic == Some(Axis::X) {
        nx -= 1;
    }
    if periodic == Some(Axis::Z) {
        nz -= 1;
    }
    if nx < 2 || nz < 2 || nx.saturating_mul(nz) > MAX_SAMPLES {
        errors.push(format!("{id}: {nx} x {nz} samples is out of range (2 up to {MAX_SAMPLES} in all); raise `cell`"));
        return Err(errors);
    }
    let cells = (if periodic == Some(Axis::X) { nx } else { nx - 1 }, if periodic == Some(Axis::Z) { nz } else { nz - 1 });
    let cellv = Vec2::new(size.x / cells.0 as f32, size.y / cells.1 as f32);
    let origin = Vec2::new(centre.x - size.x * 0.5, centre.z - size.y * 0.5);
    if let Some(w) = ctx.wrap {
        let (lo, span) = match w.axis {
            Axis::X => (origin.x, size.x),
            Axis::Z => (origin.y, size.y),
        };
        if (span - w.period()).abs() > 0.01 || (lo - w.min).abs() > 0.01 {
            errors.push(format!(
                "{id}: the world loops on {} from {} to {}, so a terrain must span exactly that ({} m from {}): set `size` and `position` to match",
                w.axis.name(),
                w.min,
                w.max,
                w.period(),
                w.min
            ));
            return Err(errors);
        }
    }

    let sources = ["heights", "heightmap", "generator"].iter().filter(|k| obj.contains_key(**k)).count();
    if sources != 1 {
        errors.push(format!("{id}: give exactly one of `heights`, `heightmap` or `generator` (found {sources})"));
        return Err(errors);
    }
    let mut heights = vec![0.0f32; nx * nz];
    if let Some(rows) = obj.get("heights") {
        let grid: Option<Vec<Vec<f32>>> = rows.as_array().map(|r| {
            r.iter().map(|row| row.as_array().map(|c| c.iter().filter_map(|v| v.as_f64().map(|n| n as f32)).collect::<Vec<_>>()).unwrap_or_default()).collect()
        });
        match grid {
            Some(g) if g.len() == nz && g.iter().all(|r| r.len() == nx) => {
                for (iz, row) in g.iter().enumerate() {
                    for (ix, h) in row.iter().enumerate() {
                        heights[iz * nx + ix] = centre.y + h;
                    }
                }
            }
            _ => errors.push(format!("{id}.heights: must be {nz} rows (along z) of {nx} numbers (along x); this size and `cell` need exactly that many")),
        }
    } else if let Some(path) = obj.get("heightmap").and_then(Value::as_str) {
        let scale = number(obj, "elevation_scale", id, &mut errors).unwrap_or(1.0);
        match load_heightmap(path, ctx.base_dir) {
            Ok(img) => {
                for iz in 0..nz {
                    for ix in 0..nx {
                        let (u, v) = (ix as f32 / cells.0 as f32, iz as f32 / cells.1 as f32);
                        heights[iz * nx + ix] = centre.y + img.sample(u, v, periodic) * scale;
                    }
                }
            }
            Err(e) => errors.push(format!("{id}.heightmap: {e}")),
        }
    } else if let Some(g) = obj.get("generator") {
        let gpath = format!("{id}.generator");
        match g.as_object() {
            None => errors.push(format!("{gpath}: must be an object like {{\"profile\": [[-20, -1], [10, 1]], \"noise\": {{\"amplitude\": 0.5}}}}")),
            Some(g) => {
                check_keys(&mut errors, &gpath, g, GENERATOR_KEYS);
                let mut profile: Vec<(f32, f32)> = Vec::new();
                match g.get("profile") {
                    None => {}
                    Some(p) => match p.as_array().map(|a| a.iter().map(pair).collect::<Option<Vec<_>>>()) {
                        Some(Some(knots)) if knots.windows(2).all(|w| w[0].0 < w[1].0) => profile = knots,
                        _ => errors.push(format!("{gpath}.profile: must be [[x, height], ...] with x ascending (the cross-section along x)")),
                    },
                }
                let noise = g.get("noise").and_then(|n| parse_noise(n, &format!("{gpath}.noise"), &mut errors));
                let seed = g.get("seed").and_then(Value::as_u64).unwrap_or(1) as u32;
                for iz in 0..nz {
                    for ix in 0..nx {
                        let (x, z) = (origin.x + ix as f32 * cellv.x, origin.y + iz as f32 * cellv.y);
                        let mut h = profile_at(&profile, x);
                        if let Some(n) = &noise {
                            h += fractal(n, seed, Vec2::new(x, z), origin, size, periodic);
                        }
                        heights[iz * nx + ix] = centre.y + h;
                    }
                }
            }
        }
    }

    let mut palette = Vec::new();
    if let Some(p) = obj.get("palette") {
        match p.as_array() {
            None => errors.push(format!("{id}.palette: must be [[height, \"#rrggbb\"], ...]")),
            Some(stops) => {
                for (i, stop) in stops.iter().enumerate() {
                    let parsed =
                        stop.as_array().filter(|a| a.len() == 2).and_then(|a| Some((a[0].as_f64()? as f32, parse_hex_to_linear(a[1].as_str()?).ok()?)));
                    match parsed {
                        Some(s) => palette.push(s),
                        None => errors.push(format!("{id}.palette[{i}]: must be [height, \"#rrggbb\"]")),
                    }
                }
                if palette.windows(2).any(|w| w[0].0 >= w[1].0) {
                    errors.push(format!("{id}.palette: heights must ascend"));
                }
            }
        }
    }
    let grain = number(obj, "grain", id, &mut errors).unwrap_or(0.0);
    if !(0.0..=0.3).contains(&grain) {
        errors.push(format!("{id}.grain: 0 to 0.3"));
    }
    if errors.is_empty() {
        Ok(Terrain { origin, cell: cellv, nx, nz, periodic, heights, palette, grain })
    } else {
        Err(errors)
    }
}

/// The fractal noise of one generator at world `(x, z)`: octaves of value noise, amplitude ramped by `fade_x`.
fn fractal(n: &Noise, base_seed: u32, p: Vec2, origin: Vec2, size: Vec2, periodic: Option<Axis>) -> f32 {
    let mut fade = 1.0;
    if let Some((x0, x1)) = n.fade_x {
        fade = smooth((p.x - x0) / (x1 - x0));
    }
    if fade <= 0.0 {
        return 0.0;
    }
    let (mut sum, mut amp, mut norm) = (0.0f32, 1.0f32, 0.0f32);
    for octave in 0..n.octaves {
        let k = (1u32 << octave) as f32;
        // Lattice cells per axis: a periodic axis holds a whole number of them so the noise tiles; the other keeps the wavelength.
        let (u, per_u) = lattice(p.x - origin.x, size.x, n.wavelength.x / k, periodic == Some(Axis::X));
        let (v, per_v) = lattice(p.y - origin.y, size.y, n.wavelength.y / k, periodic == Some(Axis::Z));
        sum += value_noise(u, v, per_u, per_v, base_seed ^ n.seed.wrapping_mul(0x9E37) ^ octave.wrapping_mul(0x1F35)) * amp;
        norm += amp;
        amp *= 0.5;
    }
    n.amplitude * fade * sum / norm
}

fn lattice(offset: f32, span: f32, wavelength: f32, periodic: bool) -> (f32, Option<i32>) {
    if periodic {
        let cells = (span / wavelength).round().max(1.0);
        (offset / span * cells, Some(cells as i32))
    } else {
        (offset / wavelength, None)
    }
}

/// A grey image as heights in `[0, 1]`.
struct HeightImage {
    w: usize,
    h: usize,
    px: Vec<f32>,
}

impl HeightImage {
    fn sample(&self, u: f32, v: f32, periodic: Option<Axis>) -> f32 {
        let (fx, fy) = (u * (self.w - 1) as f32, v * (self.h - 1) as f32);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (x1, y1) = (if periodic == Some(Axis::X) && x0 + 1 >= self.w { 0 } else { x1 }, if periodic == Some(Axis::Z) && y0 + 1 >= self.h { 0 } else { y1 });
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let g = |x: usize, y: usize| self.px[y.min(self.h - 1) * self.w + x.min(self.w - 1)];
        let a = g(x0, y0) + (g(x1, y0) - g(x0, y0)) * tx;
        let b = g(x0, y1) + (g(x1, y1) - g(x0, y1)) * tx;
        a + (b - a) * ty
    }
}

fn load_heightmap(path: &str, base: Option<&Path>) -> Result<HeightImage, String> {
    let full = match base {
        Some(b) if Path::new(path).is_relative() => b.join(path),
        _ => Path::new(path).to_path_buf(),
    };
    let img = image::open(&full).map_err(|e| format!("cannot read {}: {e} (paths are relative to the scene file)", full.display()))?;
    let luma = img.to_luma16();
    let (w, h) = (luma.width() as usize, luma.height() as usize);
    if w < 2 || h < 2 {
        return Err("the image must be at least 2 x 2 pixels".to_string());
    }
    Ok(HeightImage { w, h, px: luma.pixels().map(|p| p.0[0] as f32 / 65535.0).collect() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn build(v: Value, wrap: Option<Wrap>) -> Result<Terrain, Vec<String>> {
        parse_terrain(v.as_object().unwrap(), "t", Vec3::new(0.0, 0.5, 0.0), &TerrainContext { wrap, base_dir: None })
    }

    fn beach() -> Value {
        json!({"size": [60, 120], "cell": 1.0, "generator": {"seed": 3,
            "profile": [[-30, -2.0], [-8, -0.3], [4, 0.6], [30, 3.0]],
            "noise": {"amplitude": 0.6, "wavelength": [12, 30], "octaves": 3, "fade_x": [-10, 4]}},
            "palette": [[-2.0, "#445566"], [0.0, "#e6d3a8"], [3.0, "#8a9a5b"]], "grain": 0.05})
    }

    #[test]
    fn the_generator_follows_its_profile_and_is_deterministic() {
        let a = build(beach(), None).unwrap();
        let b = build(beach(), None).unwrap();
        assert_eq!(a.heights, b.heights, "same recipe, same ground, bit for bit");
        assert_eq!((a.nx, a.nz), (61, 121));
        // Far on the water side the noise has faded out, so the seabed is exactly the profile (plus position.y = 0.5).
        let h = a.height_at(-30.0, 0.0).unwrap();
        assert!((h - (-2.0 + 0.5)).abs() < 1e-3, "{h}");
        let shore = a.height_at(-8.0, 10.0).unwrap();
        assert!((shore - (-0.3 + 0.5)).abs() < 0.05, "no noise below x = -10: {shore}");
        let dune = a.height_at(20.0, 10.0).unwrap();
        let ridge = a.height_at(20.0, 40.0).unwrap();
        assert!(dune != ridge, "the noise varies along the coast");
        assert!(a.height_at(31.0, 0.0).is_none() && a.height_at(0.0, 61.0).is_none(), "outside the footprint");
    }

    #[test]
    fn a_wrapped_terrain_tiles_without_a_seam() {
        let wrap = Wrap { axis: Axis::Z, min: -60.0, max: 60.0 };
        let t = build(beach(), Some(wrap)).unwrap();
        assert_eq!((t.nx, t.nz), (61, 120), "one cell per sample on the looping axis");
        for x in [-6.0f32, 5.0, 12.0, 25.0] {
            let below = t.height_at(x, 59.999).unwrap();
            let above = t.height_at(x, -60.0).unwrap();
            assert!((below - above).abs() < 0.02, "x = {x}: {below} vs {above}");
            assert!((t.height_at(x, 61.0).unwrap() - t.height_at(x, -59.0).unwrap()).abs() < 1e-4, "lookups wrap");
        }
        // The mesh closes the loop with a duplicate row whose heights match the first.
        let m = t.build_mesh(Vec2::ZERO, 0.0);
        assert_eq!(m.vertices.len(), 61 * 121);
        assert!((m.vertices[0].pos[1] - m.vertices[120 * 61].pos[1]).abs() < 1e-5);
    }

    #[test]
    fn a_terrain_must_span_the_loop_and_says_how() {
        let wrap = Wrap { axis: Axis::Z, min: -100.0, max: 100.0 };
        let errs = build(beach(), Some(wrap)).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("must span exactly")), "{errs:?}");
    }

    #[test]
    fn meshes_face_up_and_carry_the_palette() {
        let t = build(
            json!({"size": [8, 8], "cell": 2.0, "heights": [[0,0,0,0,0],[0,0,0,0,0],[0,0,0,0,0],[0,0,0,0,0],[0,0,0,0,0]],
                             "palette": [[0.0, "#ff0000"]]}),
            None,
        )
        .unwrap();
        let m = t.build_mesh(Vec2::ZERO, 0.5);
        for tri in m.indices.chunks(3) {
            let p: Vec<Vec3> = tri.iter().map(|&i| Vec3::from_array(m.vertices[i as usize].pos)).collect();
            assert!((p[1] - p[0]).cross(p[2] - p[0]).y > 0.0, "counter-clockwise from above");
        }
        assert!(m.vertices.iter().all(|v| v.normal[1] > 0.99 && v.color[0] > 0.9 && v.color[1] < 0.01));
    }

    #[test]
    fn slopes_are_measured_and_bad_input_is_explained() {
        let t = build(json!({"size": [8, 8], "cell": 2.0, "heights": [[0,0,0,0,0],[0,0,0,0,0],[0,0,4,0,0],[0,0,0,0,0],[0,0,0,0,0]]}), None).unwrap();
        let (slope, at) = t.steepest_slope(None);
        assert!((slope - 2.0).abs() < 1e-4, "4 m over one 2 m cell: {slope}");
        assert!(at.x.abs() <= 4.0);
        assert!(t.steepest_slope(Some((Vec2::new(-4.0, -4.0), Vec2::new(-3.0, -3.0)))).0 < 1e-4, "the region excludes the spike");
        for (bad, needle) in [
            (json!({"size": [8, 8]}), "exactly one of"),
            (json!({"size": [8, 8], "heights": [[0]]}), "must be 9 rows"),
            (json!({"size": [8, 8], "generator": {"profile": [[3, 0], [1, 1]]}}), "ascend"),
            (json!({"size": [8, 8], "rotation": [0, 45, 0], "generator": {}}), "cannot be rotated"),
            (json!({"size": [8, 8], "generator": {"nois": {}}}), "nois"),
            (json!({"size": [8, 8], "heightmap": "nowhere.png"}), "cannot read"),
        ] {
            let errs = build(bad, None).unwrap_err();
            assert!(errs.iter().any(|e| e.contains(needle)), "{needle}: {errs:?}");
        }
    }
}
