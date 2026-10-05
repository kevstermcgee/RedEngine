//! A small deterministic software 3D renderer: the "3D element" of a hybrid game.
//!
//! It exists so a 2D game can use 3D where 3D adds something (a boss that is a rotating model, a spinning item in the HUD, a 3D world behind sprites, a whole game seen
//! in perspective with a 2D minimap on top) and so that game still runs everywhere the 2D runtime does: natively, headless, and in a browser, with no GPU. It is not the
//! wgpu engine and does not try to be: flat or toon shading, one directional light, a depth buffer, meshes built from a few primitives, sprites as camera-facing billboards.
//!
//! Determinism: only `+ - * /`, `sqrt` and `libm` trigonometry (the same bits natively and in WebAssembly), no fused multiply-add, no sorting by floats with ties left to chance.
//! Coordinates: right-handed, +Y up; a triangle is front-facing when its counter-clockwise winding seen from outside gives a normal pointing at the viewer.

use crate::game::Color;

/// A point or direction.
pub type V3 = [f32; 3];

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn mul(a: V3, k: f32) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
/// Dot product.
pub fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn len(a: V3) -> f32 {
    dot(a, a).sqrt()
}
fn norm(a: V3) -> V3 {
    let l = len(a);
    if l < 1e-9 {
        [0.0, 1.0, 0.0]
    } else {
        mul(a, 1.0 / l)
    }
}

/// One coloured triangle (counter-clockwise seen from outside).
#[derive(Debug, Clone, Copy)]
pub struct Tri {
    /// First corner.
    pub a: V3,
    /// Second corner.
    pub b: V3,
    /// Third corner.
    pub c: V3,
    /// Colour.
    pub color: [u8; 3],
}

impl Tri {
    /// The outward normal (not normalised).
    pub fn normal(&self) -> V3 {
        cross(sub(self.b, self.a), sub(self.c, self.a))
    }
}

/// A model: triangles around its own origin.
#[derive(Debug, Clone, Default)]
pub struct Mesh {
    /// The triangles.
    pub tris: Vec<Tri>,
}

impl Mesh {
    /// The radius of the smallest sphere about the origin that holds every corner.
    pub fn radius(&self) -> f32 {
        let mut r2 = 0.0f32;
        for t in &self.tris {
            for p in [t.a, t.b, t.c] {
                r2 = r2.max(dot(p, p));
            }
        }
        r2.sqrt()
    }
}

/// A rotation by yaw (about Y), pitch (about X) and roll (about Z), in degrees, applied roll first then pitch then yaw.
pub fn rotation(yaw: f32, pitch: f32, roll: f32) -> [[f32; 3]; 3] {
    let (y, p, r) = (yaw.to_radians(), pitch.to_radians(), roll.to_radians());
    let (sy, cy, sp, cp, sr, cr) = (libm::sinf(y), libm::cosf(y), libm::sinf(p), libm::cosf(p), libm::sinf(r), libm::cosf(r));
    let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let rx = [[1.0, 0.0, 0.0], [0.0, cp, -sp], [0.0, sp, cp]];
    let rz = [[cr, -sr, 0.0], [sr, cr, 0.0], [0.0, 0.0, 1.0]];
    let m = |a: [[f32; 3]; 3], b: [[f32; 3]; 3]| {
        let mut o = [[0.0f32; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                o[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
            }
        }
        o
    };
    m(ry, m(rx, rz))
}

fn apply(m: &[[f32; 3]; 3], v: V3) -> V3 {
    [m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2], m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2], m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2]]
}

// ---- primitives ---------------------------------------------------------------------------------------------------------------------------------

/// A shape a model is built from.
#[derive(Debug, Clone)]
pub enum Shape {
    /// A box of this size, centred.
    Box(V3),
    /// A UV sphere.
    Sphere {
        /// Radius.
        radius: f32,
        /// Segments around (rings are half as many).
        seg: u32,
    },
    /// A cylinder along Y, centred.
    Cylinder {
        /// Radius.
        radius: f32,
        /// Height.
        height: f32,
        /// Segments around.
        seg: u32,
    },
    /// A cone along Y, centred (apex up).
    Cone {
        /// Base radius.
        radius: f32,
        /// Height.
        height: f32,
        /// Segments around.
        seg: u32,
    },
    /// A four-sided pyramid (apex up).
    Pyramid {
        /// Side of the square base.
        base: f32,
        /// Height.
        height: f32,
    },
    /// A torus lying in the XZ plane.
    Torus {
        /// Distance from the centre to the tube's middle.
        major: f32,
        /// Tube radius.
        minor: f32,
        /// Segments around the ring and around the tube.
        seg: u32,
    },
    /// A flat rectangle in the XZ plane facing up.
    Plane([f32; 2]),
}

fn quad(a: V3, b: V3, c: V3, d: V3, color: [u8; 3], out: &mut Vec<Tri>) {
    out.push(Tri { a, b, c, color });
    out.push(Tri { a, b: c, c: d, color });
}

/// The triangles of a primitive.
pub fn build(shape: &Shape, color: [u8; 3]) -> Vec<Tri> {
    let mut t = Vec::new();
    match shape {
        Shape::Box(s) => {
            let (x, y, z) = (s[0] * 0.5, s[1] * 0.5, s[2] * 0.5);
            let p = |sx: f32, sy: f32, sz: f32| [sx * x, sy * y, sz * z];
            quad(p(1.0, -1.0, -1.0), p(1.0, 1.0, -1.0), p(1.0, 1.0, 1.0), p(1.0, -1.0, 1.0), color, &mut t); // +X
            quad(p(-1.0, -1.0, 1.0), p(-1.0, 1.0, 1.0), p(-1.0, 1.0, -1.0), p(-1.0, -1.0, -1.0), color, &mut t); // -X
            quad(p(-1.0, 1.0, -1.0), p(-1.0, 1.0, 1.0), p(1.0, 1.0, 1.0), p(1.0, 1.0, -1.0), color, &mut t); // +Y
            quad(p(-1.0, -1.0, 1.0), p(-1.0, -1.0, -1.0), p(1.0, -1.0, -1.0), p(1.0, -1.0, 1.0), color, &mut t); // -Y
            quad(p(-1.0, -1.0, 1.0), p(1.0, -1.0, 1.0), p(1.0, 1.0, 1.0), p(-1.0, 1.0, 1.0), color, &mut t); // +Z
            quad(p(1.0, -1.0, -1.0), p(-1.0, -1.0, -1.0), p(-1.0, 1.0, -1.0), p(1.0, 1.0, -1.0), color, &mut t);
            // -Z
        }
        Shape::Sphere { radius, seg } => {
            let (n, rings) = ((*seg).clamp(4, 48), ((*seg).clamp(4, 48) / 2).max(2));
            let pt = |i: u32, j: u32| {
                let (th, ph) = (core::f32::consts::PI * j as f32 / rings as f32, 2.0 * core::f32::consts::PI * i as f32 / n as f32);
                [radius * libm::sinf(th) * libm::cosf(ph), radius * libm::cosf(th), radius * libm::sinf(th) * libm::sinf(ph)]
            };
            for j in 0..rings {
                for i in 0..n {
                    let (a, b, c, d) = (pt(i, j), pt(i + 1, j), pt(i + 1, j + 1), pt(i, j + 1));
                    if j == 0 {
                        t.push(Tri { a, b: c, c: d, color });
                    } else if j == rings - 1 {
                        t.push(Tri { a, b, c: d, color });
                    } else {
                        t.push(Tri { a, b: c, c: d, color });
                        t.push(Tri { a, b, c, color });
                    }
                }
            }
        }
        Shape::Cylinder { radius, height, seg } => {
            let n = (*seg).clamp(3, 48);
            let h = height * 0.5;
            let ring = |i: u32, y: f32| {
                let a = 2.0 * core::f32::consts::PI * i as f32 / n as f32;
                [radius * libm::cosf(a), y, radius * libm::sinf(a)]
            };
            for i in 0..n {
                let (b0, b1, t0, t1) = (ring(i, -h), ring(i + 1, -h), ring(i, h), ring(i + 1, h));
                quad(b1, b0, t0, t1, color, &mut t);
                t.push(Tri { a: [0.0, h, 0.0], b: t1, c: t0, color });
                t.push(Tri { a: [0.0, -h, 0.0], b: b0, c: b1, color });
            }
        }
        Shape::Cone { radius, height, seg } => {
            let n = (*seg).clamp(3, 48);
            let h = height * 0.5;
            let ring = |i: u32| {
                let a = 2.0 * core::f32::consts::PI * i as f32 / n as f32;
                [radius * libm::cosf(a), -h, radius * libm::sinf(a)]
            };
            for i in 0..n {
                t.push(Tri { a: ring(i + 1), b: ring(i), c: [0.0, h, 0.0], color });
                t.push(Tri { a: [0.0, -h, 0.0], b: ring(i), c: ring(i + 1), color });
            }
        }
        Shape::Pyramid { base, height } => {
            let (b, h) = (base * 0.5, height * 0.5);
            let c = [[-b, -h, -b], [b, -h, -b], [b, -h, b], [-b, -h, b]];
            let apex = [0.0, h, 0.0];
            for i in 0..4 {
                t.push(Tri { a: c[(i + 1) % 4], b: c[i], c: apex, color });
            }
            quad(c[0], c[1], c[2], c[3], color, &mut t); // counter-clockwise seen from below: its normal points -Y
        }
        Shape::Torus { major, minor, seg } => {
            let n = (*seg).clamp(4, 40);
            let p = |i: u32, j: u32| {
                let (u, v) = (2.0 * core::f32::consts::PI * i as f32 / n as f32, 2.0 * core::f32::consts::PI * j as f32 / n as f32);
                let r = major + minor * libm::cosf(v);
                [r * libm::cosf(u), minor * libm::sinf(v), r * libm::sinf(u)]
            };
            for i in 0..n {
                for j in 0..n {
                    let (a, b, c, d) = (p(i, j), p(i + 1, j), p(i + 1, j + 1), p(i, j + 1));
                    t.push(Tri { a, b, c, color });
                    t.push(Tri { a, b: c, c: d, color });
                }
            }
            // orientation is checked by the outward-normal test; flip if this winding faces inward
            if let Some(f) = t.first() {
                let centroid = mul(add(add(f.a, f.b), f.c), 1.0 / 3.0);
                let ring = {
                    let l = (centroid[0] * centroid[0] + centroid[2] * centroid[2]).sqrt().max(1e-6);
                    [centroid[0] / l * major, 0.0, centroid[2] / l * major]
                };
                if dot(f.normal(), sub(centroid, ring)) < 0.0 {
                    for tri in &mut t {
                        std::mem::swap(&mut tri.b, &mut tri.c);
                    }
                }
            }
        }
        Shape::Plane(s) => {
            let (x, z) = (s[0] * 0.5, s[1] * 0.5);
            quad([-x, 0.0, z], [x, 0.0, z], [x, 0.0, -z], [-x, 0.0, -z], color, &mut t);
        }
    }
    t
}

/// A sprite pressed into cubes: every opaque pixel becomes a cube of side `cell`, `depth` cubes thick, only the faces that can be seen. Centred on the origin.
pub fn voxels(pixels: &[Color], w: usize, h: usize, depth: u32, cell: f32) -> Vec<Tri> {
    let mut t = Vec::new();
    let solid = |x: i32, y: i32| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && pixels[y as usize * w + x as usize][3] > 0;
    let (ox, oy) = (w as f32 * cell * 0.5, h as f32 * cell * 0.5);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            if !solid(x, y) {
                continue;
            }
            let px = pixels[y as usize * w + x as usize];
            let col = [px[0], px[1], px[2]];
            let c = [x as f32 * cell - ox + cell * 0.5, oy - y as f32 * cell - cell * 0.5, 0.0];
            let mut cube = build(&Shape::Box([cell, cell, depth as f32 * cell]), col);
            // keep only faces whose neighbour is empty: +X, -X, +Y, -Y are tris (0,1), (2,3), (4,5), (6,7); +Z (8,9) and -Z (10,11) always show
            let keep = [(0usize, solid(x + 1, y)), (2, solid(x - 1, y)), (4, solid(x, y - 1)), (6, solid(x, y + 1))];
            let mut drop = [false; 12];
            for (i, hidden) in keep {
                if hidden {
                    drop[i] = true;
                    drop[i + 1] = true;
                }
            }
            for (i, tri) in cube.iter_mut().enumerate() {
                if drop[i] {
                    continue;
                }
                tri.a = add(tri.a, c);
                tri.b = add(tri.b, c);
                tri.c = add(tri.c, c);
                t.push(*tri);
            }
        }
    }
    t
}

/// A model assembled from parts: each part is a shape, a position, a rotation (yaw, pitch, roll in degrees) and a colour.
pub fn assemble(parts: &[(Vec<Tri>, V3, V3)]) -> Mesh {
    let mut tris = Vec::new();
    for (shape_tris, at, rot) in parts {
        let m = rotation(rot[0], rot[1], rot[2]);
        for t in shape_tris {
            tris.push(Tri { a: add(apply(&m, t.a), *at), b: add(apply(&m, t.b), *at), c: add(apply(&m, t.c), *at), color: t.color });
        }
    }
    Mesh { tris }
}

// ---- the renderer -------------------------------------------------------------------------------------------------------------------------------

/// Where the viewer is and how it sees.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    /// Position.
    pub eye: V3,
    /// The point looked at.
    pub target: V3,
    /// Which way is up on the screen (not parallel to the view direction).
    pub up: V3,
    /// Vertical field of view in degrees (perspective).
    pub fov: f32,
    /// If set, an orthographic view whose half-height in world units is this.
    pub ortho: Option<f32>,
}

/// One directional light and an ambient floor.
#[derive(Debug, Clone, Copy)]
pub struct Light {
    /// The direction the light travels.
    pub dir: V3,
    /// Brightness of unlit faces, 0 to 1.
    pub ambient: f32,
    /// Three flat steps of brightness instead of a smooth fall-off.
    pub toon: bool,
}

impl Default for Light {
    fn default() -> Self {
        Light { dir: [-0.4, -0.8, -0.45], ambient: 0.38, toon: false }
    }
}

/// A mesh placed in the scene.
#[derive(Debug, Clone)]
pub struct Item<'a> {
    /// The model.
    pub mesh: &'a Mesh,
    /// Position of its origin.
    pub pos: V3,
    /// Yaw, pitch, roll in degrees.
    pub rot: V3,
    /// Uniform scale.
    pub scale: f32,
    /// Multiplies every colour (None = as built).
    pub tint: Option<[u8; 3]>,
}

/// A camera-facing picture at a point: a sprite, or a coloured rectangle.
#[derive(Debug, Clone)]
pub struct Billboard<'a> {
    /// Bottom-centre in the scene.
    pub pos: V3,
    /// Width and height in world units.
    pub size: [f32; 2],
    /// Pixels (alpha 0 = see-through), row-major.
    pub pixels: &'a [Color],
    /// Pixel width.
    pub pw: usize,
    /// Pixel height.
    pub ph: usize,
}

/// A colour and depth buffer.
#[derive(Debug, Clone)]
pub struct Target {
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
    /// RGBA; alpha 0 where nothing was drawn.
    pub rgba: Vec<u8>,
    depth: Vec<f32>,
}

impl Target {
    /// A transparent target.
    pub fn new(w: u32, h: u32) -> Target {
        Target { w, h, rgba: vec![0; (w * h * 4) as usize], depth: vec![f32::INFINITY; (w * h) as usize] }
    }
    /// Fills the colour with `c` (the depth stays infinite).
    pub fn clear(&mut self, c: Color) {
        for p in self.rgba.as_chunks_mut::<4>().0 {
            p.copy_from_slice(&c);
        }
    }
    /// Pixels that were drawn on (alpha above 0 and not the clear colour is the caller's business; this counts alpha).
    pub fn covered(&self) -> usize {
        self.rgba.as_chunks::<4>().0.iter().filter(|p| p[3] > 0).count()
    }
}

struct Basis {
    eye: V3,
    right: V3,
    up: V3,
    fwd: V3,
}

fn basis(cam: &Camera) -> Basis {
    let fwd = norm(sub(cam.target, cam.eye));
    let mut right = cross(fwd, cam.up);
    if len(right) < 1e-6 {
        right = cross(fwd, [0.0, 0.0, -1.0]);
        if len(right) < 1e-6 {
            right = [1.0, 0.0, 0.0];
        }
    }
    let right = norm(right);
    let up = cross(right, fwd);
    Basis { eye: cam.eye, right, up, fwd }
}

struct Proj {
    cx: f32,
    cy: f32,
    f: f32,
    ortho: Option<f32>,
}

impl Proj {
    fn new(cam: &Camera, w: u32, h: u32) -> Proj {
        let f = match cam.ortho {
            Some(half) => h as f32 * 0.5 / half.max(1e-3),
            None => h as f32 * 0.5 / libm::tanf((cam.fov.clamp(5.0, 150.0) * 0.5).to_radians()),
        };
        Proj { cx: w as f32 * 0.5, cy: h as f32 * 0.5, f, ortho: cam.ortho }
    }
    /// View-space point (x right, y up, z forward) to the screen.
    fn screen(&self, p: V3) -> [f32; 2] {
        if self.ortho.is_some() {
            [self.cx + p[0] * self.f, self.cy - p[1] * self.f]
        } else {
            [self.cx + p[0] / p[2] * self.f, self.cy - p[1] / p[2] * self.f]
        }
    }
}

fn to_view(b: &Basis, p: V3) -> V3 {
    let d = sub(p, b.eye);
    [dot(d, b.right), dot(d, b.up), dot(d, b.fwd)]
}

fn shade(color: [u8; 3], n: V3, light: &Light, tint: Option<[u8; 3]>) -> [u8; 3] {
    let l = norm(mul(light.dir, -1.0));
    let d = dot(norm(n), l).max(0.0);
    let k = if light.toon {
        let s = light.ambient + (1.0 - light.ambient) * d;
        if s > 0.8 {
            1.0
        } else if s > 0.55 {
            0.78
        } else {
            0.55
        }
    } else {
        light.ambient + (1.0 - light.ambient) * d
    };
    let mut o = [0u8; 3];
    for i in 0..3 {
        let base = match tint {
            Some(t) => (color[i] as u32 * t[i] as u32 / 255) as f32,
            None => color[i] as f32,
        };
        o[i] = (base * k).clamp(0.0, 255.0) as u8;
    }
    o
}

const NEAR: f32 = 0.05;

/// Draws a triangle given in view space.
fn draw_tri(t: &mut Target, pr: &Proj, v: [V3; 3], color: [u8; 3], far: f32) {
    // clip against the near plane (perspective only; an orthographic view has no behind)
    let mut poly: Vec<V3> = Vec::with_capacity(4);
    if pr.ortho.is_some() {
        poly.extend_from_slice(&v);
    } else {
        for i in 0..3 {
            let (a, b) = (v[i], v[(i + 1) % 3]);
            let (ina, inb) = (a[2] >= NEAR, b[2] >= NEAR);
            if ina {
                poly.push(a);
            }
            if ina != inb {
                let k = (NEAR - a[2]) / (b[2] - a[2]);
                poly.push([a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, NEAR]);
            }
        }
    }
    if poly.len() < 3 {
        return;
    }
    for i in 1..poly.len() - 1 {
        raster(t, pr, [poly[0], poly[i], poly[i + 1]], color, far);
    }
}

fn raster(t: &mut Target, pr: &Proj, v: [V3; 3], color: [u8; 3], far: f32) {
    let s = [pr.screen(v[0]), pr.screen(v[1]), pr.screen(v[2])];
    let area = (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[1][1] - s[0][1]) * (s[2][0] - s[0][0]);
    if area.abs() < 1e-9 {
        return;
    }
    let (minx, maxx) = (s[0][0].min(s[1][0]).min(s[2][0]).floor().max(0.0) as i32, (s[0][0].max(s[1][0]).max(s[2][0]).ceil() as i32).min(t.w as i32 - 1));
    let (miny, maxy) = (s[0][1].min(s[1][1]).min(s[2][1]).floor().max(0.0) as i32, (s[0][1].max(s[1][1]).max(s[2][1]).ceil() as i32).min(t.h as i32 - 1));
    let inv = 1.0 / area;
    let persp = pr.ortho.is_none();
    let iz = [1.0 / v[0][2], 1.0 / v[1][2], 1.0 / v[2][2]];
    for y in miny..=maxy {
        for x in minx..=maxx {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((s[1][0] - px) * (s[2][1] - py) - (s[1][1] - py) * (s[2][0] - px)) * inv;
            let w1 = ((s[2][0] - px) * (s[0][1] - py) - (s[2][1] - py) * (s[0][0] - px)) * inv;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let z = if persp { 1.0 / (w0 * iz[0] + w1 * iz[1] + w2 * iz[2]) } else { w0 * v[0][2] + w1 * v[1][2] + w2 * v[2][2] };
            if z > far {
                continue;
            }
            let i = (y as u32 * t.w + x as u32) as usize;
            if z < t.depth[i] {
                t.depth[i] = z;
                t.rgba[i * 4..i * 4 + 4].copy_from_slice(&[color[0], color[1], color[2], 255]);
            }
        }
    }
}

/// Draws meshes and billboards into `t` (which keeps what is already there and its depth).
pub fn render(t: &mut Target, cam: &Camera, light: &Light, items: &[Item], boards: &[Billboard]) {
    let b = basis(cam);
    let pr = Proj::new(cam, t.w, t.h);
    let far = 1.0e6;
    for it in items {
        let m = rotation(it.rot[0], it.rot[1], it.rot[2]);
        for tri in &it.mesh.tris {
            let w = |p: V3| add(mul(apply(&m, p), it.scale), it.pos);
            let (a, bb, c) = (w(tri.a), w(tri.b), w(tri.c));
            let n = cross(sub(bb, a), sub(c, a));
            // back-face culling: the face must look at the viewer
            let toward = if pr.ortho.is_some() { mul(b.fwd, -1.0) } else { sub(b.eye, a) };
            if dot(n, toward) <= 0.0 {
                continue;
            }
            draw_tri(t, &pr, [to_view(&b, a), to_view(&b, bb), to_view(&b, c)], shade(tri.color, n, light, it.tint), far);
        }
    }
    for bd in boards {
        let c = to_view(&b, bd.pos);
        if pr.ortho.is_none() && c[2] < NEAR {
            continue;
        }
        let z = c[2];
        let bottom = pr.screen(c);
        let scale = if pr.ortho.is_some() { pr.f } else { pr.f / z };
        let (sw, sh) = (bd.size[0] * scale, bd.size[1] * scale);
        if sw < 0.5 || sh < 0.5 {
            continue;
        }
        let (x0, y0) = (bottom[0] - sw * 0.5, bottom[1] - sh);
        for y in (y0.floor().max(0.0) as i32)..(((y0 + sh).ceil() as i32).min(t.h as i32)) {
            for x in (x0.floor().max(0.0) as i32)..(((x0 + sw).ceil() as i32).min(t.w as i32)) {
                let (u, v) = (((x as f32 + 0.5 - x0) / sw * bd.pw as f32) as usize, ((y as f32 + 0.5 - y0) / sh * bd.ph as f32) as usize);
                if u >= bd.pw || v >= bd.ph {
                    continue;
                }
                let px = bd.pixels[v * bd.pw + u];
                if px[3] == 0 {
                    continue;
                }
                let i = (y as u32 * t.w + x as u32) as usize;
                if z < t.depth[i] {
                    t.depth[i] = z;
                    t.rgba[i * 4..i * 4 + 4].copy_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
        }
    }
}

/// Where a point in the scene lands on a `w x h` screen, and how far it is in front of the camera (None if it is behind).
pub fn project(cam: &Camera, w: u32, h: u32, p: V3) -> Option<([f32; 2], f32)> {
    let b = basis(cam);
    let pr = Proj::new(cam, w, h);
    let v = to_view(&b, p);
    if pr.ortho.is_none() && v[2] < NEAR {
        return None;
    }
    Some((pr.screen(v), v[2]))
}

fn ray(b: &Basis, pr: &Proj, sx: f32, sy: f32) -> (V3, V3) {
    let (vx, vy) = ((sx - pr.cx) / pr.f, (pr.cy - sy) / pr.f);
    if pr.ortho.is_some() {
        (add(add(b.eye, mul(b.right, vx)), mul(b.up, vy)), b.fwd)
    } else {
        (b.eye, norm(add(add(b.fwd, mul(b.right, vx)), mul(b.up, vy))))
    }
}

fn hit_plane(o: V3, d: V3, origin: V3, n: V3) -> Option<V3> {
    let denom = dot(n, d);
    if denom.abs() < 1e-6 {
        return None;
    }
    let k = dot(n, sub(origin, o)) / denom;
    (k >= 0.0).then(|| add(o, mul(d, k)))
}

/// The point where the ray through screen pixel `(sx, sy)` meets the plane through `origin` with normal `n` (None if it never does, or points away).
pub fn unproject_to_plane(cam: &Camera, w: u32, h: u32, sx: f32, sy: f32, origin: V3, n: V3) -> Option<V3> {
    let (b, pr) = (basis(cam), Proj::new(cam, w, h));
    let (o, d) = ray(&b, &pr, sx, sy);
    hit_plane(o, d, origin, n)
}

/// Paints each pixel whose view ray meets the plane through `origin` (normal `n`) with `color(point)`; `None` leaves the pixel alone. The depth is not touched, so everything
/// drawn afterwards covers it: this is how a floor is made without triangles.
pub fn paint_plane(t: &mut Target, cam: &Camera, origin: V3, n: V3, color: impl Fn(V3) -> Option<Color>) {
    let (b, pr) = (basis(cam), Proj::new(cam, t.w, t.h));
    for y in 0..t.h {
        for x in 0..t.w {
            let (o, d) = ray(&b, &pr, x as f32 + 0.5, y as f32 + 0.5);
            if let Some(c) = hit_plane(o, d, origin, n).and_then(&color) {
                let i = ((y * t.w + x) * 4) as usize;
                t.rgba[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> Camera {
        Camera { eye: [0.0, 0.0, -6.0], target: [0.0, 0.0, 0.0], up: [0.0, 1.0, 0.0], fov: 50.0, ortho: None }
    }

    fn mesh_of(shape: Shape) -> Mesh {
        Mesh { tris: build(&shape, [200, 100, 50]) }
    }

    #[test]
    fn every_convex_primitive_has_its_triangles_facing_outward() {
        for (name, shape) in [
            ("box", Shape::Box([2.0, 3.0, 1.0])),
            ("sphere", Shape::Sphere { radius: 1.5, seg: 12 }),
            ("cylinder", Shape::Cylinder { radius: 1.0, height: 2.0, seg: 10 }),
            ("cone", Shape::Cone { radius: 1.0, height: 2.0, seg: 10 }),
            ("pyramid", Shape::Pyramid { base: 2.0, height: 2.0 }),
        ] {
            let m = mesh_of(shape);
            assert!(!m.tris.is_empty(), "{name}");
            for (i, t) in m.tris.iter().enumerate() {
                let centroid = mul(add(add(t.a, t.b), t.c), 1.0 / 3.0);
                assert!(dot(t.normal(), centroid) > -1e-4, "{name}: triangle {i} faces inward ({:?})", t.normal());
            }
        }
        // A torus faces away from the ring's core line.
        let m = mesh_of(Shape::Torus { major: 2.0, minor: 0.5, seg: 12 });
        for t in &m.tris {
            let c = mul(add(add(t.a, t.b), t.c), 1.0 / 3.0);
            let l = (c[0] * c[0] + c[2] * c[2]).sqrt();
            let core = [c[0] / l * 2.0, 0.0, c[2] / l * 2.0];
            assert!(dot(t.normal(), sub(c, core)) > -1e-4);
        }
    }

    #[test]
    fn a_cube_in_front_of_the_camera_is_drawn_in_the_middle_with_flat_shaded_faces() {
        let cube = mesh_of(Shape::Box([2.0, 2.0, 2.0]));
        let mut t = Target::new(64, 64);
        // Turn it so three faces show.
        render(
            &mut t,
            &cam(),
            &Light { dir: [0.6, -0.7, 0.5], ambient: 0.1, toon: false },
            &[Item { mesh: &cube, pos: [0.0; 3], rot: [35.0, 30.0, 0.0], scale: 1.0, tint: None }],
            &[],
        );
        assert!(t.covered() > 600 && t.covered() < 2200, "{}", t.covered());
        assert_eq!(t.rgba[(32 * 64 + 32) * 4 + 3], 255, "the middle is drawn");
        assert_eq!(t.rgba[3], 0, "the corner is empty");
        let shades: std::collections::BTreeSet<[u8; 3]> = t.rgba.as_chunks::<4>().0.iter().filter(|p| p[3] > 0).map(|p| [p[0], p[1], p[2]]).collect();
        assert!((2..=3).contains(&shades.len()), "visible faces are flat-shaded, one colour each: {shades:?}");
    }

    #[test]
    fn nearer_things_hide_farther_ones_whatever_the_draw_order() {
        let ball = mesh_of(Shape::Sphere { radius: 1.0, seg: 12 });
        let red = Mesh { tris: build(&Shape::Sphere { radius: 1.0, seg: 12 }, [255, 0, 0]) };
        let blue = Mesh { tris: build(&Shape::Sphere { radius: 1.0, seg: 12 }, [0, 0, 255]) };
        let _ = ball;
        for order in [[0, 1], [1, 0]] {
            let items = [
                Item { mesh: &red, pos: [0.0, 0.0, -1.0], rot: [0.0; 3], scale: 1.0, tint: None },
                Item { mesh: &blue, pos: [0.0, 0.0, 1.5], rot: [0.0; 3], scale: 1.0, tint: None },
            ];
            let mut t = Target::new(64, 64);
            render(&mut t, &cam(), &Light::default(), &[items[order[0]].clone(), items[order[1]].clone()], &[]);
            let mid = &t.rgba[(32 * 64 + 32) * 4..][..3];
            assert!(mid[0] > mid[2], "the nearer red ball is in front in order {order:?}: {mid:?}");
        }
    }

    #[test]
    fn light_makes_faces_toward_it_brighter_and_toon_has_three_steps() {
        let plane = Mesh { tris: build(&Shape::Plane([4.0, 4.0]), [200, 200, 200]) };
        let top = Camera { eye: [0.0, 6.0, 0.0], target: [0.0; 3], up: [0.0, 0.0, -1.0], fov: 40.0, ortho: None };
        let lit = |dir: V3| {
            let mut t = Target::new(32, 32);
            render(
                &mut t,
                &top,
                &Light { dir, ambient: 0.2, toon: false },
                &[Item { mesh: &plane, pos: [0.0; 3], rot: [0.0; 3], scale: 1.0, tint: None }],
                &[],
            );
            t.rgba[(16 * 32 + 16) * 4]
        };
        assert!(lit([0.0, -1.0, 0.0]) > lit([1.0, -0.2, 0.0]), "a face turned to the light is brighter");
        assert_eq!(lit([0.0, 1.0, 0.0]), (200.0f32 * 0.2) as u8, "a face turned away keeps only the ambient light");
        let toon: std::collections::BTreeSet<u8> = (0..20)
            .map(|i| {
                let mut t = Target::new(8, 8);
                let a = i as f32 / 20.0;
                render(
                    &mut t,
                    &top,
                    &Light { dir: [a, -1.0 + a * 0.5, 0.0], ambient: 0.2, toon: true },
                    &[Item { mesh: &plane, pos: [0.0; 3], rot: [0.0; 3], scale: 1.0, tint: None }],
                    &[],
                );
                t.rgba[(4 * 8 + 4) * 4]
            })
            .collect();
        assert!(toon.len() <= 3, "{toon:?}");
    }

    #[test]
    fn something_behind_the_camera_or_crossing_the_near_plane_does_not_crash_or_draw_behind() {
        let big = mesh_of(Shape::Box([2.0, 2.0, 2.0]));
        let mut t = Target::new(32, 32);
        render(&mut t, &cam(), &Light::default(), &[Item { mesh: &big, pos: [0.0, 0.0, -20.0], rot: [0.0; 3], scale: 1.0, tint: None }], &[]);
        assert_eq!(t.covered(), 0, "behind the camera");
        // A floor that runs from behind the camera to the horizon: the part in front of the near plane is drawn, the rest is clipped away.
        let floor = Mesh { tris: build(&Shape::Plane([40.0, 40.0]), [90, 160, 90]) };
        let low = Camera { eye: [0.0, 2.0, -6.0], target: [0.0, 2.0, 0.0], up: [0.0, 1.0, 0.0], fov: 60.0, ortho: None };
        let mut t = Target::new(32, 32);
        render(&mut t, &low, &Light::default(), &[Item { mesh: &floor, pos: [0.0; 3], rot: [0.0; 3], scale: 1.0, tint: None }], &[]);
        assert!(t.covered() > 100, "the floor in front is drawn: {}", t.covered());
        assert_eq!(t.rgba[(2 * 32 + 16) * 4 + 3], 0, "the sky above the horizon is empty");
        assert_eq!(t.rgba[(31 * 32 + 16) * 4 + 3], 255, "the floor under the camera is drawn");
    }

    #[test]
    fn an_orthographic_view_keeps_size_with_distance_and_a_perspective_one_shrinks() {
        let cube = mesh_of(Shape::Box([2.0, 2.0, 2.0]));
        let width = |c: &Camera, z: f32| {
            let mut t = Target::new(64, 64);
            render(&mut t, c, &Light::default(), &[Item { mesh: &cube, pos: [0.0, 0.0, z], rot: [0.0; 3], scale: 1.0, tint: None }], &[]);
            t.covered()
        };
        let mut o = cam();
        o.ortho = Some(4.0);
        assert_eq!(width(&o, 0.0), width(&o, 5.0));
        assert!(width(&cam(), 5.0) < width(&cam(), 0.0) / 2);
    }

    #[test]
    fn a_sprite_becomes_voxels_and_a_billboard_faces_the_camera() {
        let px = [[255, 0, 0, 255], [0, 0, 0, 0], [0, 255, 0, 255], [0, 0, 255, 255]];
        let v = voxels(&px, 2, 2, 2, 1.0);
        assert!(v.len() >= 3 * 2 * 4, "three cubes, their exposed faces: {}", v.len());
        let mesh = Mesh { tris: v };
        let mut t = Target::new(48, 48);
        render(&mut t, &cam(), &Light::default(), &[Item { mesh: &mesh, pos: [0.0; 3], rot: [0.0; 3], scale: 1.0, tint: None }], &[]);
        assert!(t.covered() > 100);
        let board = Billboard { pos: [0.0, -1.0, 0.0], size: [2.0, 2.0], pixels: &px, pw: 2, ph: 2 };
        let mut t2 = Target::new(48, 48);
        render(&mut t2, &cam(), &Light::default(), &[], &[board]);
        assert!(t2.covered() > 100 && t2.covered() < t.covered() * 2);
    }

    #[test]
    fn projection_and_unprojection_agree() {
        let c = Camera { eye: [3.0, 8.0, 5.0], target: [3.0, 0.0, 1.0], up: [0.0, 0.0, -1.0], fov: 45.0, ortho: None };
        let p = [4.5, 0.0, 2.0];
        let (s, depth) = project(&c, 160, 90, p).unwrap();
        assert!(depth > 0.0);
        let back = unproject_to_plane(&c, 160, 90, s[0], s[1], [0.0; 3], [0.0, 1.0, 0.0]).unwrap();
        assert!((back[0] - p[0]).abs() < 1e-3 && (back[2] - p[2]).abs() < 1e-3, "{back:?}");
        assert!(project(&c, 160, 90, [3.0, 20.0, 5.0]).is_none(), "behind the camera");
        assert!(unproject_to_plane(&c, 160, 90, 80.0, 45.0, [0.0, 50.0, 0.0], [0.0, 1.0, 0.0]).is_none(), "a plane above the camera is never hit");
    }

    #[test]
    fn rendering_twice_gives_the_same_bytes() {
        let m = assemble(&[
            (build(&Shape::Sphere { radius: 1.0, seg: 14 }, [20, 200, 90]), [0.0, 1.0, 0.0], [0.0; 3]),
            (build(&Shape::Torus { major: 1.6, minor: 0.3, seg: 14 }, [200, 200, 20]), [0.0; 3], [20.0, 10.0, 0.0]),
        ]);
        let run = || {
            let mut t = Target::new(80, 60);
            render(&mut t, &cam(), &Light::default(), &[Item { mesh: &m, pos: [0.0; 3], rot: [33.0, 12.0, 4.0], scale: 1.0, tint: None }], &[]);
            t.rgba
        };
        assert_eq!(run(), run());
    }
}
