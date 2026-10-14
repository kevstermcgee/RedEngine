//! Geometry helpers for the museum on top of [`red_engine2::procgen::geo::Geo`]: boxes, lathes, tubes along paths, rippled cloth and canvases.

use glam::{Mat3, Vec3};
use red_engine2::procgen::geo::{basis, Geo, Rgb};
use std::f32::consts::TAU;

/// A box given by its centre, three orthonormal axes and half sizes along them; flat faces, one colour per face (top, sides, bottom shading).
pub fn obox(g: &mut Geo, centre: Vec3, axes: [Vec3; 3], half: Vec3, c: Rgb) {
    let [ax, ay, az] = axes;
    let corner = |sx: f32, sy: f32, sz: f32| centre + ax * half.x * sx + ay * half.y * sy + az * half.z * sz;
    // Each face: the outward normal axis and two in-plane axes ordered so the quad winds counter-clockwise from outside.
    type Face = (Vec3, f32, [(f32, f32, f32); 4]);
    let faces: [Face; 6] = [
        (ax, 0.92, [(1.0, -1.0, -1.0), (1.0, 1.0, -1.0), (1.0, 1.0, 1.0), (1.0, -1.0, 1.0)]),
        (-ax, 0.92, [(-1.0, -1.0, 1.0), (-1.0, 1.0, 1.0), (-1.0, 1.0, -1.0), (-1.0, -1.0, -1.0)]),
        (ay, 1.0, [(-1.0, 1.0, -1.0), (-1.0, 1.0, 1.0), (1.0, 1.0, 1.0), (1.0, 1.0, -1.0)]),
        (-ay, 0.7, [(-1.0, -1.0, -1.0), (1.0, -1.0, -1.0), (1.0, -1.0, 1.0), (-1.0, -1.0, 1.0)]),
        (az, 0.96, [(-1.0, -1.0, 1.0), (1.0, -1.0, 1.0), (1.0, 1.0, 1.0), (-1.0, 1.0, 1.0)]),
        (-az, 0.96, [(1.0, -1.0, -1.0), (-1.0, -1.0, -1.0), (-1.0, 1.0, -1.0), (1.0, 1.0, -1.0)]),
    ];
    for (n, shade, q) in faces {
        let col = [c[0] * shade, c[1] * shade, c[2] * shade];
        let p: Vec<Vec3> = q.iter().map(|(x, y, z)| corner(*x, *y, *z)).collect();
        quad(g, p[0], p[1], p[2], p[3], n, col);
    }
}

/// An axis-aligned box between two corners.
pub fn aabox(g: &mut Geo, min: Vec3, max: Vec3, c: Rgb) {
    obox(g, (min + max) * 0.5, [Vec3::X, Vec3::Y, Vec3::Z], (max - min).abs() * 0.5, c);
}

/// A flat quad `a b c d` (counter-clockwise seen from the side `n` points to).
pub fn quad(g: &mut Geo, a: Vec3, b: Vec3, c: Vec3, d: Vec3, n: Vec3, col: Rgb) {
    let i = [g.vert(a, n, col), g.vert(b, n, col), g.vert(c, n, col), g.vert(d, n, col)];
    // Orient by the actual winding so a caller's corner order cannot flip a face inside out.
    if (b - a).cross(c - a).dot(n) >= 0.0 {
        g.idx.extend_from_slice(&[i[0], i[1], i[2], i[0], i[2], i[3]]);
    } else {
        g.idx.extend_from_slice(&[i[0], i[2], i[1], i[0], i[3], i[2]]);
    }
}

/// A quad with a colour per corner.
pub fn quad4(g: &mut Geo, p: [Vec3; 4], n: Vec3, c: [Rgb; 4]) {
    let i = [g.vert(p[0], n, c[0]), g.vert(p[1], n, c[1]), g.vert(p[2], n, c[2]), g.vert(p[3], n, c[3])];
    if (p[1] - p[0]).cross(p[2] - p[0]).dot(n) >= 0.0 {
        g.idx.extend_from_slice(&[i[0], i[1], i[2], i[0], i[2], i[3]]);
    } else {
        g.idx.extend_from_slice(&[i[0], i[2], i[1], i[0], i[3], i[2]]);
    }
}

/// A solid of revolution about +Y through `base`: `profile` is (radius, height) from the bottom up; smooth normals; a cap closes each end whose radius
/// is not zero. `col(t)` colours by height fraction.
pub fn lathe(g: &mut Geo, base: Vec3, profile: &[(f32, f32)], segments: u32, col: impl Fn(f32) -> Rgb) {
    let n = profile.len();
    if n < 2 {
        return;
    }
    let top = profile[n - 1].1.max(1e-4);
    let first = g.pos.len() as u32;
    for (i, &(r, y)) in profile.iter().enumerate() {
        // The profile's slope gives the normal's lean.
        let (a, b) = (profile[i.saturating_sub(1)], profile[(i + 1).min(n - 1)]);
        let (dr, dy) = (b.0 - a.0, b.1 - a.1);
        for k in 0..=segments {
            let ang = k as f32 / segments as f32 * TAU;
            let (s, c) = ang.sin_cos();
            let p = base + Vec3::new(c * r, y, s * r);
            let nrm = Vec3::new(c * dy, -dr, s * dy).normalize_or(Vec3::new(c, 0.0, s));
            g.vert(p, nrm, col(y / top));
        }
    }
    let ring = segments + 1;
    for i in 0..n as u32 - 1 {
        for k in 0..segments {
            let a = first + i * ring + k;
            let b = a + ring;
            // Rings go from +x toward +z: (a, a+1, b+1) faces outward when viewed from outside.
            g.idx.extend_from_slice(&[a, b + 1, a + 1, a, b, b + 1]);
        }
    }
    for (end, up) in [(0usize, false), (n - 1, true)] {
        let (r, y) = profile[end];
        if r <= 1e-4 {
            continue;
        }
        let nrm = if up { Vec3::Y } else { -Vec3::Y };
        let c = col(y / top);
        let centre = g.vert(base + Vec3::Y * y, nrm, c);
        let rim: Vec<u32> = (0..segments)
            .map(|k| {
                let ang = k as f32 / segments as f32 * TAU;
                g.vert(base + Vec3::new(ang.cos() * r, y, ang.sin() * r), nrm, c)
            })
            .collect();
        for k in 0..segments as usize {
            let (a, b) = (rim[k], rim[(k + 1) % segments as usize]);
            if up {
                g.idx.extend_from_slice(&[centre, b, a]);
            } else {
                g.idx.extend_from_slice(&[centre, a, b]);
            }
        }
    }
}

/// A tube along a path with a radius per point (smooth normals, open ends unless the radius closes to 0).
pub fn tube_path(g: &mut Geo, pts: &[Vec3], radii: &[f32], sides: u32, col: impl Fn(f32) -> Rgb) {
    let n = pts.len();
    if n < 2 {
        return;
    }
    let first = g.pos.len() as u32;
    // A frame carried along the path (parallel transport) so the tube does not twist.
    let mut t0 = (pts[1] - pts[0]).normalize_or(Vec3::Y);
    let (mut u, _) = basis(t0);
    for i in 0..n {
        let t = if i == 0 {
            t0
        } else if i == n - 1 {
            (pts[i] - pts[i - 1]).normalize_or(t0)
        } else {
            (pts[i + 1] - pts[i - 1]).normalize_or(t0)
        };
        // Rotate u from the previous tangent to this one.
        let axis = t0.cross(t);
        if axis.length_squared() > 1e-10 {
            let ang = t0.dot(t).clamp(-1.0, 1.0).acos();
            u = Mat3::from_axis_angle(axis.normalize(), ang) * u;
        }
        u = (u - t * u.dot(t)).normalize_or(basis(t).0);
        let v = t.cross(u);
        t0 = t;
        let c = col(i as f32 / (n - 1) as f32);
        for k in 0..=sides {
            let ang = k as f32 / sides as f32 * TAU;
            let dir = u * ang.cos() + v * ang.sin();
            g.vert(pts[i] + dir * radii[i], dir, c);
        }
    }
    let ring = sides + 1;
    for i in 0..n as u32 - 1 {
        for k in 0..sides {
            let a = first + i * ring + k;
            let b = a + ring;
            g.idx.extend_from_slice(&[a, a + 1, b + 1, a, b + 1, b]);
        }
    }
}

/// A smooth ellipsoid (an icosphere, `subdiv` 1 or 2) with one colour, optionally lumpy.
pub fn ball(g: &mut Geo, centre: Vec3, radii: Vec3, subdiv: u32, c: Rgb) {
    g.blob(centre, radii, subdiv, 1, 0.0, c, c);
}

/// A canvas: the unit square in x, y (-0.5..0.5) facing +z, cut into `nx` by `ny` cells; `bulge(x, y)` pushes points along z (a tapestry's folds).
pub fn canvas(nx: u32, ny: u32, bulge: impl Fn(f32, f32) -> f32) -> Geo {
    let mut g = Geo::default();
    let at = |i: u32, j: u32| {
        let (x, y) = (i as f32 / nx as f32 - 0.5, j as f32 / ny as f32 - 0.5);
        Vec3::new(x, y, bulge(x, y))
    };
    for j in 0..=ny {
        for i in 0..=nx {
            let p = at(i, j);
            // The normal from neighbouring points (the folds shade).
            let dx = at((i + 1).min(nx), j) - at(i.saturating_sub(1), j);
            let dy = at(i, (j + 1).min(ny)) - at(i, j.saturating_sub(1));
            let n = dx.cross(dy).normalize_or(Vec3::Z);
            g.vert(p, n, [1.0, 1.0, 1.0]);
        }
    }
    let row = nx + 1;
    for j in 0..ny {
        for i in 0..nx {
            let a = j * row + i;
            g.idx.extend_from_slice(&[a, a + 1, a + row + 1, a, a + row + 1, a + row]);
        }
    }
    g
}

/// Moves and turns every vertex of `g` from index `from` on: `m` is a rotation (columns are the new axes), then `at` is added.
pub fn place(g: &mut Geo, from: usize, m: Mat3, at: Vec3) {
    for i in from..g.pos.len() {
        g.pos[i] = (m * Vec3::from(g.pos[i]) + at).to_array();
        g.nrm[i] = (m * Vec3::from(g.nrm[i])).normalize_or_zero().to_array();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use red_engine2::procgen::geo::assert_wound_outward;

    #[test]
    fn boxes_lathes_and_tubes_face_outward() {
        let mut g = Geo::default();
        obox(&mut g, Vec3::new(1.0, 2.0, 3.0), [Vec3::X, Vec3::Y, Vec3::Z], Vec3::new(0.5, 1.0, 0.25), [1.0; 3]);
        assert_wound_outward(&g, "box");
        let mut g = Geo::default();
        lathe(&mut g, Vec3::ZERO, &[(0.3, 0.0), (0.5, 0.4), (0.2, 1.0), (0.25, 1.2)], 16, |_| [1.0; 3]);
        assert_wound_outward(&g, "lathe");
        let mut g = Geo::default();
        let pts: Vec<Vec3> = (0..12).map(|i| Vec3::new((i as f32 * 0.5).cos(), i as f32 * 0.1, (i as f32 * 0.5).sin())).collect();
        tube_path(&mut g, &pts, &[0.1; 12], 8, |_| [1.0; 3]);
        assert!(g.tris() > 100);
    }
}
