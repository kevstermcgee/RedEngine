//! A tiny low-poly geometry builder for plants: tubes, lumpy blobs, cones and ribbons, with vertex colours.
//!
//! The look is stylised and soft: foliage is made of lumpy ellipsoids with smooth normals (they shade like clouds of leaves, not like crystals),
//! trunks and stems are tapered tubes, and leaves, petals and blades are thin ribbons drawn from both sides (the scene pipeline culls back
//! faces). Colour is per vertex (linear RGB, multiplied by the object's material), with a darker underside on every blob, which does the work of
//! ambient occlusion for free. Everything is plain data (positions, normals, colours, indices) so the headless server and the tests can build it.

use super::noise::Rng;
use glam::{Mat3, Vec3};

/// A triangle mesh with a colour per vertex; counter-clockwise triangles face outward.
#[derive(Debug, Clone, Default)]
pub struct Geo {
    /// Vertex positions.
    pub pos: Vec<[f32; 3]>,
    /// Unit vertex normals.
    pub nrm: Vec<[f32; 3]>,
    /// Vertex colours (linear RGB).
    pub col: Vec<[f32; 3]>,
    /// Triangle indices.
    pub idx: Vec<u32>,
}

/// A colour as three linear channels.
pub type Rgb = [f32; 3];

/// Linear interpolation between two colours.
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// A colour scaled in brightness.
pub fn shade(c: Rgb, k: f32) -> Rgb {
    [c[0] * k, c[1] * k, c[2] * k]
}

/// An `#rrggbb` colour as linear RGB (magenta if it is malformed, so a typo is loud).
pub fn hex(s: &str) -> Rgb {
    crate::color::parse_hex_to_linear(s).map(|v| v.to_array()).unwrap_or([1.0, 0.0, 1.0])
}

impl Geo {
    /// Number of triangles.
    pub fn tris(&self) -> usize {
        self.idx.len() / 3
    }

    fn vert(&mut self, p: Vec3, n: Vec3, c: Rgb) -> u32 {
        self.pos.push(p.to_array());
        self.nrm.push(n.normalize_or_zero().to_array());
        self.col.push(c);
        (self.pos.len() - 1) as u32
    }

    /// One flat triangle, counter-clockwise from the side it faces.
    pub fn tri(&mut self, a: Vec3, b: Vec3, c: Vec3, ca: Rgb, cb: Rgb, cc: Rgb) {
        let n = (b - a).cross(c - a);
        let (ia, ib, ic) = (self.vert(a, n, ca), self.vert(b, n, cb), self.vert(c, n, cc));
        self.idx.extend_from_slice(&[ia, ib, ic]);
    }

    /// One flat triangle visible from both sides.
    pub fn tri2(&mut self, a: Vec3, b: Vec3, c: Vec3, ca: Rgb, cb: Rgb, cc: Rgb) {
        self.tri(a, b, c, ca, cb, cc);
        self.tri(a, c, b, ca, cc, cb);
    }

    /// A tapered tube from `a` to `b` with `sides` facets and smooth normals. `cap` closes the far end.
    #[allow(clippy::too_many_arguments)]
    pub fn tube(&mut self, a: Vec3, b: Vec3, r0: f32, r1: f32, sides: u32, c0: Rgb, c1: Rgb, cap: bool) {
        let axis = (b - a).normalize_or_zero();
        let (u, v) = basis(axis);
        let base = self.pos.len() as u32;
        for k in 0..sides {
            let ang = k as f32 / sides as f32 * std::f32::consts::TAU;
            let r = u * ang.cos() + v * ang.sin();
            let slope = (r0 - r1) / (b - a).length().max(1e-4);
            let n = r + axis * slope;
            self.vert(a + r * r0, n, c0);
            self.vert(b + r * r1, n, c1);
        }
        for k in 0..sides {
            let (a0, a1) = (base + 2 * k, base + 2 * ((k + 1) % sides));
            // (a_k, a_k+1, b_k+1) and (a_k, b_k+1, b_k): outward for a ring that turns from u toward v.
            self.idx.extend_from_slice(&[a0, a1, a1 + 1, a0, a1 + 1, a0 + 1]);
        }
        if cap && r1 > 0.0 {
            let apex = self.vert(b, axis, c1);
            let ring: Vec<u32> = (0..sides)
                .map(|k| {
                    let ang = k as f32 / sides as f32 * std::f32::consts::TAU;
                    self.vert(b + (u * ang.cos() + v * ang.sin()) * r1, axis, c1)
                })
                .collect();
            for k in 0..sides as usize {
                self.idx.extend_from_slice(&[apex, ring[k], ring[(k + 1) % sides as usize]]);
            }
        }
    }

    /// A cone standing on `base`: smooth sides and a flat underside. `turn` rotates the ring.
    #[allow(clippy::too_many_arguments)]
    pub fn cone(&mut self, base: Vec3, r: f32, h: f32, sides: u32, turn: f32, c_base: Rgb, c_tip: Rgb) {
        let tip = base + Vec3::Y * h;
        let ring: Vec<Vec3> = (0..sides)
            .map(|k| {
                base + Vec3::new(
                    (turn + k as f32 / sides as f32 * std::f32::consts::TAU).cos(),
                    0.0,
                    (turn + k as f32 / sides as f32 * std::f32::consts::TAU).sin(),
                ) * r
            })
            .collect();
        let slope = r / h.max(1e-4);
        let first = self.pos.len() as u32;
        for k in 0..sides as usize {
            let dir = (ring[k] - base).normalize_or_zero();
            self.vert(ring[k], dir + Vec3::Y * slope, c_base);
            self.vert(tip, dir + Vec3::Y * slope, c_tip);
        }
        for k in 0..sides {
            let (a0, a1) = (first + 2 * k, first + 2 * ((k + 1) % sides));
            // Looking down the cone from above, k runs clockwise (x toward z); the outward face is (ring k+1, ring k, tip).
            self.idx.extend_from_slice(&[a1, a0, a0 + 1]);
        }
        let under = shade(c_base, 0.55);
        let centre = self.vert(base, -Vec3::Y, under);
        let rim: Vec<u32> = ring.iter().map(|p| self.vert(*p, -Vec3::Y, under)).collect();
        for k in 0..sides as usize {
            self.idx.extend_from_slice(&[centre, rim[k], rim[(k + 1) % sides as usize]]);
        }
    }

    /// A lumpy ellipsoid with smooth normals: `subdiv` 0 is 20 triangles, 1 is 80, 2 is 320; `lump` is the random bulge (0.15 is leafy).
    #[allow(clippy::too_many_arguments)]
    pub fn blob(&mut self, centre: Vec3, radii: Vec3, subdiv: u32, seed: u32, lump: f32, c_bottom: Rgb, c_top: Rgb) {
        let (dirs, faces) = icosphere(subdiv);
        let mut rng = Rng::at(seed, 17, 31);
        let verts: Vec<Vec3> = dirs.iter().map(|d| centre + *d * radii * (1.0 + lump * (rng.white() * 2.0 - 1.0))).collect();
        let mut normals = vec![Vec3::ZERO; verts.len()];
        for f in &faces {
            let n = (verts[f[1] as usize] - verts[f[0] as usize]).cross(verts[f[2] as usize] - verts[f[0] as usize]);
            for i in f {
                normals[*i as usize] += n;
            }
        }
        let first = self.pos.len() as u32;
        for (i, p) in verts.iter().enumerate() {
            // Brighter on top, darker underneath, with a little shot colour from vertex to vertex.
            let up = (dirs[i].y * 0.5 + 0.5).clamp(0.0, 1.0);
            let jitter = 0.94 + 0.12 * rng.white();
            self.vert(*p, normals[i], shade(mix(c_bottom, c_top, up), jitter));
        }
        for f in &faces {
            self.idx.extend_from_slice(&[first + f[0], first + f[1], first + f[2]]);
        }
    }

    /// A ribbon along a polyline: `widths[i]` is the full width at `pts[i]`, spread along `side`. Seen from both sides.
    pub fn ribbon(&mut self, pts: &[Vec3], widths: &[f32], side: Vec3, c0: Rgb, c1: Rgb) {
        let n = pts.len();
        for i in 0..n - 1 {
            let (t0, t1) = (i as f32 / (n - 1) as f32, (i + 1) as f32 / (n - 1) as f32);
            let (ca, cb) = (mix(c0, c1, t0), mix(c0, c1, t1));
            let (a, b) = (pts[i], pts[i + 1]);
            let (wa, wb) = (widths[i] * 0.5, widths[i + 1] * 0.5);
            let (a_l, a_r, b_l, b_r) = (a - side * wa, a + side * wa, b - side * wb, b + side * wb);
            // Front: counter-clockwise seen from the side the (side x tangent) normal points to.
            self.tri2(a_l, a_r, b_r, ca, ca, cb);
            if wb > 1e-5 || i + 2 < n {
                self.tri2(a_l, b_r, b_l, ca, cb, cb);
            }
        }
    }

    /// Adds another mesh, scaled, turned about the vertical axis and moved.
    pub fn add(&mut self, other: &Geo, at: Vec3, yaw: f32, scale: f32) {
        let rot = Mat3::from_rotation_y(yaw);
        let first = self.pos.len() as u32;
        for i in 0..other.pos.len() {
            let p = rot * (Vec3::from(other.pos[i]) * scale) + at;
            let n = rot * Vec3::from(other.nrm[i]);
            self.pos.push(p.to_array());
            self.nrm.push(n.to_array());
            self.col.push(other.col[i]);
        }
        self.idx.extend(other.idx.iter().map(|i| first + i));
    }

    /// The smallest and largest corner of the mesh.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        self.pos.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(Vec3::from(*p)), hi.max(Vec3::from(*p))))
    }
}

/// Two unit vectors perpendicular to `axis` and to each other, right-handed (`u x v = axis`).
pub fn basis(axis: Vec3) -> (Vec3, Vec3) {
    let helper = if axis.y.abs() < 0.95 { Vec3::Y } else { Vec3::X };
    let u = helper.cross(axis).normalize_or_zero();
    let v = axis.cross(u);
    (u, v)
}

/// A unit icosphere: the vertex directions and outward-facing triangles.
pub fn icosphere(subdiv: u32) -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let t = (1.0 + 5.0f32.sqrt()) / 2.0;
    let mut v: Vec<Vec3> = [
        (-1.0, t, 0.0),
        (1.0, t, 0.0),
        (-1.0, -t, 0.0),
        (1.0, -t, 0.0),
        (0.0, -1.0, t),
        (0.0, 1.0, t),
        (0.0, -1.0, -t),
        (0.0, 1.0, -t),
        (t, 0.0, -1.0),
        (t, 0.0, 1.0),
        (-t, 0.0, -1.0),
        (-t, 0.0, 1.0),
    ]
    .iter()
    .map(|p| Vec3::new(p.0, p.1, p.2).normalize())
    .collect();
    let mut f: Vec<[u32; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    for _ in 0..subdiv {
        let mut cache = std::collections::HashMap::new();
        let mut mid = |a: u32, b: u32, v: &mut Vec<Vec3>| -> u32 {
            let key = (a.min(b), a.max(b));
            *cache.entry(key).or_insert_with(|| {
                v.push(((v[a as usize] + v[b as usize]) * 0.5).normalize());
                (v.len() - 1) as u32
            })
        };
        let mut next = Vec::with_capacity(f.len() * 4);
        for tri in &f {
            let (a, b, c) = (mid(tri[0], tri[1], &mut v), mid(tri[1], tri[2], &mut v), mid(tri[2], tri[0], &mut v));
            next.extend_from_slice(&[[tri[0], a, c], [tri[1], b, a], [tri[2], c, b], [a, b, c]]);
        }
        f = next;
    }
    // Make every face counter-clockwise seen from outside, whatever the table above says.
    for tri in &mut f {
        let (a, b, c) = (v[tri[0] as usize], v[tri[1] as usize], v[tri[2] as usize]);
        if (b - a).cross(c - a).dot(a + b + c) < 0.0 {
            tri.swap(1, 2);
        }
    }
    (v, f)
}

#[cfg(test)]
/// Every triangle's winding agrees with its vertex normals (so back-face culling shows the outside).
pub fn assert_wound_outward(g: &Geo, what: &str) {
    for t in g.idx.as_chunks::<3>().0 {
        let (a, b, c) = (Vec3::from(g.pos[t[0] as usize]), Vec3::from(g.pos[t[1] as usize]), Vec3::from(g.pos[t[2] as usize]));
        let face = (b - a).cross(c - a);
        if face.length() < 1e-9 {
            continue;
        }
        let n = (Vec3::from(g.nrm[t[0] as usize]) + Vec3::from(g.nrm[t[1] as usize]) + Vec3::from(g.nrm[t[2] as usize])).normalize_or_zero();
        assert!(face.normalize().dot(n) > 0.0, "{what}: a triangle faces against its normals at {a:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_icosphere_is_closed_and_outward() {
        for s in 0..3 {
            let (v, f) = icosphere(s);
            assert_eq!(f.len(), 20 * 4usize.pow(s));
            assert!(v.iter().all(|p| (p.length() - 1.0).abs() < 1e-5));
            for t in &f {
                let (a, b, c) = (v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]);
                assert!((b - a).cross(c - a).dot(a + b + c) > 0.0);
            }
        }
    }

    #[test]
    fn tubes_cones_blobs_and_ribbons_face_outward() {
        let mut g = Geo::default();
        g.tube(Vec3::ZERO, Vec3::new(0.3, 2.0, 0.1), 0.2, 0.1, 7, [0.3; 3], [0.2; 3], true);
        assert_wound_outward(&g, "tube");
        let mut g = Geo::default();
        g.cone(Vec3::ZERO, 1.0, 2.0, 9, 0.3, [0.1; 3], [0.3; 3]);
        assert_wound_outward(&g, "cone");
        let mut g = Geo::default();
        g.blob(Vec3::Y, Vec3::new(1.0, 0.7, 1.0), 1, 4, 0.15, [0.1; 3], [0.4; 3]);
        assert_wound_outward(&g, "blob");
        let mut g = Geo::default();
        g.ribbon(&[Vec3::ZERO, Vec3::new(0.0, 1.0, 0.2), Vec3::new(0.0, 1.8, 0.7)], &[0.2, 0.15, 0.0], Vec3::X, [0.1; 3], [0.3; 3]);
        assert_wound_outward(&g, "ribbon");
    }

    #[test]
    fn a_tube_wall_points_away_from_its_axis() {
        let mut g = Geo::default();
        g.tube(Vec3::ZERO, Vec3::Y, 0.5, 0.5, 8, [1.0; 3], [1.0; 3], false);
        for t in g.idx.as_chunks::<3>().0 {
            let (a, b, c) = (Vec3::from(g.pos[t[0] as usize]), Vec3::from(g.pos[t[1] as usize]), Vec3::from(g.pos[t[2] as usize]));
            let n = (b - a).cross(c - a);
            let out = Vec3::new((a.x + b.x + c.x) / 3.0, 0.0, (a.z + b.z + c.z) / 3.0);
            assert!(n.dot(out) > 0.0, "tube wall faces inward");
        }
    }

    #[test]
    fn a_blob_has_the_size_and_smoothness_asked_for() {
        let mut g = Geo::default();
        g.blob(Vec3::new(0.0, 5.0, 0.0), Vec3::new(2.0, 1.0, 2.0), 2, 9, 0.1, [0.1; 3], [0.5; 3]);
        let (lo, hi) = g.bounds();
        assert!(lo.x > -2.4 && hi.x < 2.4 && lo.y > 3.8 && hi.y < 6.2, "{lo:?} {hi:?}");
        assert_eq!(g.tris(), 320);
        assert!(g.nrm.iter().all(|n| (Vec3::from(*n).length() - 1.0).abs() < 1e-3));
    }

    #[test]
    fn meshes_can_be_merged_and_moved() {
        let mut a = Geo::default();
        a.cone(Vec3::ZERO, 1.0, 1.0, 6, 0.0, [0.1; 3], [0.2; 3]);
        let mut b = Geo::default();
        b.add(&a, Vec3::new(10.0, 0.0, 0.0), 1.0, 2.0);
        b.add(&a, Vec3::ZERO, 0.0, 1.0);
        assert_eq!(b.tris(), a.tris() * 2);
        assert!(b.idx.iter().all(|i| (*i as usize) < b.pos.len()));
        let (lo, hi) = b.bounds();
        assert!(hi.x > 10.5 && lo.x < 0.0);
    }
}
