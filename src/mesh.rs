use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::f32::consts::PI;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
}

impl Vertex {
    pub const ATTRS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRS,
        }
    }
}

#[derive(Default, Clone)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    fn quad(a: Vec3, b: Vec3, c: Vec3, d: Vec3, normal: Vec3, out: &mut Mesh) {
        let base = out.vertices.len() as u32;
        for p in [a, b, c, d] {
            out.vertices.push(Vertex { pos: p.to_array(), normal: normal.to_array() });
        }
        out.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// Axis-aligned box centered at the local origin, `size` = full width/height/depth.
    pub fn cuboid(size: Vec3) -> Mesh {
        let h = size * 0.5;
        let mut m = Mesh::default();
        // +Y top, -Y bottom, +X right, -X left, +Z front, -Z back
        Mesh::quad(
            Vec3::new(-h.x, h.y, -h.z), Vec3::new(-h.x, h.y, h.z),
            Vec3::new(h.x, h.y, h.z), Vec3::new(h.x, h.y, -h.z),
            Vec3::Y, &mut m,
        );
        Mesh::quad(
            Vec3::new(-h.x, -h.y, h.z), Vec3::new(-h.x, -h.y, -h.z),
            Vec3::new(h.x, -h.y, -h.z), Vec3::new(h.x, -h.y, h.z),
            -Vec3::Y, &mut m,
        );
        Mesh::quad(
            Vec3::new(h.x, -h.y, h.z), Vec3::new(h.x, -h.y, -h.z),
            Vec3::new(h.x, h.y, -h.z), Vec3::new(h.x, h.y, h.z),
            Vec3::X, &mut m,
        );
        Mesh::quad(
            Vec3::new(-h.x, -h.y, -h.z), Vec3::new(-h.x, -h.y, h.z),
            Vec3::new(-h.x, h.y, h.z), Vec3::new(-h.x, h.y, -h.z),
            -Vec3::X, &mut m,
        );
        Mesh::quad(
            Vec3::new(h.x, -h.y, h.z), Vec3::new(h.x, h.y, h.z),
            Vec3::new(-h.x, h.y, h.z), Vec3::new(-h.x, -h.y, h.z),
            Vec3::Z, &mut m,
        );
        Mesh::quad(
            Vec3::new(-h.x, -h.y, -h.z), Vec3::new(-h.x, h.y, -h.z),
            Vec3::new(h.x, h.y, -h.z), Vec3::new(h.x, -h.y, -h.z),
            -Vec3::Z, &mut m,
        );
        m
    }

    /// UV sphere, smooth-shaded, centered at the local origin.
    pub fn uv_sphere(radius: f32, rings: u32, segments: u32) -> Mesh {
        let mut m = Mesh::default();
        for r in 0..=rings {
            let v = r as f32 / rings as f32;
            let phi = v * PI; // 0 (top) .. PI (bottom)
            let y = phi.cos();
            let ring_r = phi.sin();
            for s in 0..=segments {
                let u = s as f32 / segments as f32;
                let theta = u * 2.0 * PI;
                let x = ring_r * theta.cos();
                let z = ring_r * theta.sin();
                let n = Vec3::new(x, y, z);
                m.vertices.push(Vertex { pos: (n * radius).to_array(), normal: n.to_array() });
            }
        }
        let stride = segments + 1;
        for r in 0..rings {
            for s in 0..segments {
                let a = r * stride + s;
                let b = a + stride;
                m.indices.extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
            }
        }
        m
    }

    /// Cylinder centered at the local origin, extending `-height/2..height/2` along Y.
    pub fn cylinder(radius: f32, height: f32, segments: u32) -> Mesh {
        let mut m = Mesh::default();
        let hh = height * 0.5;
        // Sides (smooth radial normals)
        let side_base = 0u32;
        for s in 0..=segments {
            let theta = s as f32 / segments as f32 * 2.0 * PI;
            let n = Vec3::new(theta.cos(), 0.0, theta.sin());
            let top = Vec3::new(n.x * radius, hh, n.z * radius);
            let bot = Vec3::new(n.x * radius, -hh, n.z * radius);
            m.vertices.push(Vertex { pos: top.to_array(), normal: n.to_array() });
            m.vertices.push(Vertex { pos: bot.to_array(), normal: n.to_array() });
        }
        for s in 0..segments {
            let a = side_base + s * 2;
            let b = a + 2;
            // a=top(s) a+1=bot(s) b=top(s+1) b+1=bot(s+1)
            m.indices.extend_from_slice(&[a, b, a + 1, b, b + 1, a + 1]);
        }
        // Caps (flat normals, fan from center)
        Mesh::disc_cap(&mut m, radius, hh, segments, Vec3::Y, true);
        Mesh::disc_cap(&mut m, radius, -hh, segments, -Vec3::Y, false);
        m
    }

    fn disc_cap(m: &mut Mesh, radius: f32, y: f32, segments: u32, normal: Vec3, winding_ccw_from_above: bool) {
        let center_idx = m.vertices.len() as u32;
        m.vertices.push(Vertex { pos: [0.0, y, 0.0], normal: normal.to_array() });
        let ring_base = m.vertices.len() as u32;
        for s in 0..=segments {
            let theta = s as f32 / segments as f32 * 2.0 * PI;
            let p = Vec3::new(theta.cos() * radius, y, theta.sin() * radius);
            m.vertices.push(Vertex { pos: p.to_array(), normal: normal.to_array() });
        }
        for s in 0..segments {
            let a = ring_base + s;
            let b = ring_base + s + 1;
            if winding_ccw_from_above {
                m.indices.extend_from_slice(&[center_idx, b, a]);
            } else {
                m.indices.extend_from_slice(&[center_idx, a, b]);
            }
        }
    }

    /// Cone centered at the local origin: apex at `+height/2`, base circle at `-height/2`.
    pub fn cone(radius: f32, height: f32, segments: u32) -> Mesh {
        let mut m = Mesh::default();
        let hh = height * 0.5;
        let apex = Vec3::new(0.0, hh, 0.0);
        let slant = (radius * radius + height * height).sqrt().max(1e-6);
        let ny = radius / slant;
        let nr = height / slant;
        for s in 0..segments {
            let t0 = s as f32 / segments as f32 * 2.0 * PI;
            let t1 = (s + 1) as f32 / segments as f32 * 2.0 * PI;
            let p0 = Vec3::new(t0.cos() * radius, -hh, t0.sin() * radius);
            let p1 = Vec3::new(t1.cos() * radius, -hh, t1.sin() * radius);
            let n_mid_theta = (t0 + t1) * 0.5;
            let n0 = Vec3::new(t0.cos() * nr, ny, t0.sin() * nr);
            let n1 = Vec3::new(t1.cos() * nr, ny, t1.sin() * nr);
            let n_apex = Vec3::new(n_mid_theta.cos() * nr, ny, n_mid_theta.sin() * nr);
            let base = m.vertices.len() as u32;
            m.vertices.push(Vertex { pos: apex.to_array(), normal: n_apex.to_array() });
            m.vertices.push(Vertex { pos: p0.to_array(), normal: n0.to_array() });
            m.vertices.push(Vertex { pos: p1.to_array(), normal: n1.to_array() });
            m.indices.extend_from_slice(&[base, base + 2, base + 1]);
        }
        Mesh::disc_cap(&mut m, radius, -hh, segments, -Vec3::Y, false);
        m
    }

    /// Capsule centered at the local origin, total extent `height` along Y (radius included).
    /// The straight cylindrical section has length `(height - 2*radius).max(0)`.
    pub fn capsule(radius: f32, height: f32, segments: u32, rings: u32) -> Mesh {
        let mut m = Mesh::default();
        let half_cyl = ((height - 2.0 * radius).max(0.0)) * 0.5;
        let cap_rings = rings.max(2);

        // Single monotonically-increasing-Y profile: bottom pole -> bottom equator (meets the
        // cylinder) -> top equator -> top pole. Each entry is (y, ring_radius, normal_y).
        let mut profile: Vec<(f32, f32, f32)> = Vec::with_capacity(cap_rings as usize * 2 + 1);
        for i in 0..=cap_rings {
            let phi = PI - (i as f32 / cap_rings as f32) * (PI / 2.0); // PI (pole) -> PI/2 (equator)
            profile.push((-half_cyl + phi.cos() * radius, phi.sin() * radius, phi.cos()));
        }
        for i in 1..=cap_rings {
            let phi = PI / 2.0 - (i as f32 / cap_rings as f32) * (PI / 2.0); // PI/2 (equator) -> 0 (pole)
            profile.push((half_cyl + phi.cos() * radius, phi.sin() * radius, phi.cos()));
        }

        let stride = segments + 1;
        for &(y, r, ny) in &profile {
            for s in 0..=segments {
                let theta = s as f32 / segments as f32 * 2.0 * PI;
                let nxz = (1.0 - ny * ny).max(0.0).sqrt();
                let n = Vec3::new(theta.cos() * nxz, ny, theta.sin() * nxz);
                let p = Vec3::new(theta.cos() * r, y, theta.sin() * r);
                m.vertices.push(Vertex { pos: p.to_array(), normal: n.to_array() });
            }
        }
        for ring in 0..(profile.len() as u32 - 1) {
            for s in 0..segments {
                let a = ring * stride + s;
                let b = a + stride;
                m.indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        m
    }

    /// Flat quad in the XZ plane, facing `+Y`, centered at the local origin.
    pub fn plane(width: f32, depth: f32) -> Mesh {
        let mut m = Mesh::default();
        let hw = width * 0.5;
        let hd = depth * 0.5;
        Mesh::quad(
            Vec3::new(-hw, 0.0, hd), Vec3::new(hw, 0.0, hd),
            Vec3::new(hw, 0.0, -hd), Vec3::new(-hw, 0.0, -hd),
            Vec3::Y, &mut m,
        );
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_valid(m: &Mesh) {
        assert!(!m.vertices.is_empty());
        assert!(!m.indices.is_empty());
        assert_eq!(m.indices.len() % 3, 0);
        for &i in &m.indices {
            assert!((i as usize) < m.vertices.len());
        }
        for v in &m.vertices {
            let n = Vec3::from_array(v.normal);
            assert!((n.length() - 1.0).abs() < 1e-3, "normal not unit length: {n:?}");
        }
    }

    #[test]
    fn all_primitives_produce_valid_meshes() {
        assert_valid(&Mesh::cuboid(Vec3::new(1.0, 2.0, 0.5)));
        assert_valid(&Mesh::uv_sphere(0.5, 12, 16));
        assert_valid(&Mesh::cylinder(0.4, 1.2, 16));
        assert_valid(&Mesh::cone(0.4, 1.0, 16));
        assert_valid(&Mesh::capsule(0.3, 1.0, 16, 6));
        assert_valid(&Mesh::capsule(0.5, 0.4, 16, 6)); // height < 2*radius
        assert_valid(&Mesh::plane(4.0, 4.0));
    }

    /// The live viewer's main/shadow pipelines cull backfaces (`front_face: Ccw`), so every
    /// triangle must be wound CCW as seen from outside the mesh — equivalently, its geometric
    /// winding normal (`cross(v1-v0, v2-v0)`, computed purely from vertex order/position) must
    /// point the same general direction as its stored shading normals (computed independently,
    /// from each generator's parametric surface formula). A generator that got this backwards
    /// would render as invisible (or inside-out) the moment culling is on, so this is checked
    /// computationally rather than relying on a visual check catching it.
    fn assert_ccw_front_facing(m: &Mesh, name: &str) {
        for tri in m.indices.chunks(3) {
            let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            let pa = Vec3::from_array(m.vertices[a].pos);
            let pb = Vec3::from_array(m.vertices[b].pos);
            let pc = Vec3::from_array(m.vertices[c].pos);
            let winding_normal = (pb - pa).cross(pc - pa);
            // Pole rings (uv_sphere, capsule caps, cone's apex fan) legitimately produce
            // zero-area triangles (two of the three vertices coincide at the pole/apex) — those
            // render as nothing regardless of winding, so they're not a meaningful check.
            if winding_normal.length_squared() < 1e-12 {
                continue;
            }
            let avg_shading_normal = (Vec3::from_array(m.vertices[a].normal)
                + Vec3::from_array(m.vertices[b].normal)
                + Vec3::from_array(m.vertices[c].normal))
                / 3.0;
            assert!(
                winding_normal.dot(avg_shading_normal) > 0.0,
                "{name}: triangle ({a},{b},{c}) wound opposite its stored normals — backface culling would hide it"
            );
        }
    }

    #[test]
    fn all_primitives_are_ccw_front_facing() {
        assert_ccw_front_facing(&Mesh::cuboid(Vec3::new(1.0, 2.0, 0.5)), "cuboid");
        assert_ccw_front_facing(&Mesh::uv_sphere(0.5, 12, 16), "uv_sphere");
        assert_ccw_front_facing(&Mesh::cylinder(0.4, 1.2, 16), "cylinder");
        assert_ccw_front_facing(&Mesh::cone(0.4, 1.0, 16), "cone");
        assert_ccw_front_facing(&Mesh::capsule(0.3, 1.0, 16, 6), "capsule");
        assert_ccw_front_facing(&Mesh::capsule(0.5, 0.4, 16, 6), "capsule (height < 2*radius)");
        assert_ccw_front_facing(&Mesh::plane(4.0, 4.0), "plane");
    }
}
