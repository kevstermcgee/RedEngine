//! A section's meshes: every face of a block that can be seen (its neighbour does not hide it), with ambient occlusion and smooth light in each
//! corner, and the plants and torches as small models of boxes.
//!
//! The vertex colour carries the game's data for its `surface` shader (the object's `material.z` is 1): red is the texture number / 255, green the
//! sky light (0..1) and blue the block light (0..1), both already darkened by the corner's occlusion. Positions are relative to the section's
//! corner, so every number stays small.

use crate::block::{tex, Block, Pass, Plant, Shape, DIRS};
use crate::chunk::{ChunkData, H, SECTION, W};
use glam::{Mat4, Vec3};
use red_engine2::mesh::{Mesh, Vertex};

/// A chunk and its eight neighbours, read as one block of cells (local x and z from -16 to 31).
pub struct Hood<'a> {
    /// Indexed `(dx + 1) + 3 * (dz + 1)`.
    pub chunks: [&'a ChunkData; 9],
}

impl Hood<'_> {
    #[inline]
    fn chunk(&self, x: i32, z: i32) -> (&ChunkData, i32, i32) {
        let (cx, cz) = (x.div_euclid(W), z.div_euclid(W));
        (self.chunks[((cx + 1) + 3 * (cz + 1)) as usize], x.rem_euclid(W), z.rem_euclid(W))
    }

    #[inline]
    pub fn block(&self, x: i32, y: i32, z: i32) -> Block {
        let (c, lx, lz) = self.chunk(x, z);
        c.get(lx, y, lz)
    }

    /// (sky, block) light, 0..15 each.
    #[inline]
    pub fn light(&self, x: i32, y: i32, z: i32) -> (u8, u8) {
        let (c, lx, lz) = self.chunk(x, z);
        (c.sky(lx, y, lz), c.glow(lx, y, lz))
    }
}

/// One section's geometry, by pass.
#[derive(Default)]
pub struct SectionMesh {
    pub solid: Mesh,
    pub water: Mesh,
    pub glass: Mesh,
}

impl SectionMesh {
    pub fn is_empty(&self) -> bool {
        self.solid.indices.is_empty() && self.water.indices.is_empty() && self.glass.indices.is_empty()
    }

    pub fn triangles(&self) -> usize {
        (self.solid.indices.len() + self.water.indices.len() + self.glass.indices.len()) / 3
    }
}

/// The four corners of each face (unit cube), counter-clockwise seen from outside, in [`DIRS`] order.
pub const FACES: [[[f32; 3]; 4]; 6] = [
    [[1.0, 0.0, 1.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [1.0, 1.0, 1.0]],
    [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 1.0], [0.0, 1.0, 0.0]],
    [[0.0, 1.0, 1.0], [1.0, 1.0, 1.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
    [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 0.0, 1.0]],
    [[0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [1.0, 1.0, 1.0], [0.0, 1.0, 1.0]],
    [[1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]],
];

/// How much each count of open neighbours lights a corner (0: tucked into a corner).
const AO: [f32; 4] = [0.42, 0.62, 0.8, 1.0];

/// Whether `b`'s face toward `n` is drawn.
fn face_shows(b: Block, n: Block) -> bool {
    if n.cull() {
        return false;
    }
    match b.info().shape {
        Shape::Liquid => !(b.is_water() && (n.is_water() || n == Block::ICE)) && n != b,
        _ => match b.info().pass {
            Pass::Solid => true,
            _ => n != b,
        },
    }
}

fn push_quad(m: &mut Mesh, corners: [[f32; 3]; 4], normal: [f32; 3], colors: [[f32; 3]; 4], sway: [f32; 4], flip: bool) {
    let base = m.vertices.len() as u32;
    for i in 0..4 {
        m.vertices.push(Vertex { pos: corners[i], normal, color: colors[i], sway: sway[i] });
    }
    if flip {
        m.indices.extend_from_slice(&[base + 1, base + 2, base + 3, base + 1, base + 3, base]);
    } else {
        m.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// The data colour of a vertex.
#[inline]
fn data(tex: u8, sky: f32, glow: f32) -> [f32; 3] {
    [tex as f32 / 255.0, sky, glow]
}

/// Faces waiting to be merged: for each pass and direction, a key per cell (0: no face there). Only faces whose four corners carry the same
/// light are kept here (merging others would smear their shading); the shader takes texture coordinates from the position, so a merged quad
/// tiles its texture as the separate faces would have.
struct Greedy {
    keys: Vec<u64>,
}

const CELLS16: usize = 16 * 16 * 16;

impl Greedy {
    fn new() -> Greedy {
        Greedy { keys: vec![0; 3 * 6 * CELLS16] }
    }

    fn at(pass: usize, d: usize, x: i32, y: i32, z: i32) -> usize {
        ((pass * 6 + d) * 16 + y as usize) * 256 + z as usize * 16 + x as usize
    }

    /// Merges each direction's faces slice by slice into rectangles and writes the quads.
    fn emit(&self, out: &mut SectionMesh) {
        for pass in 0..3 {
            for (d, n) in DIRS.iter().enumerate() {
                let axis = if n[0] != 0 {
                    0
                } else if n[1] != 0 {
                    1
                } else {
                    2
                };
                let (ua, va) = match axis {
                    0 => (2, 1),
                    1 => (0, 2),
                    _ => (0, 1),
                };
                for slice in 0..16 {
                    // The slice's keys, indexed [v][u].
                    let mut m = [[0u64; 16]; 16];
                    for v in 0..16 {
                        for u in 0..16 {
                            let mut c = [0i32; 3];
                            c[axis] = slice;
                            c[ua] = u;
                            c[va] = v;
                            m[v as usize][u as usize] = self.keys[Greedy::at(pass, d, c[0], c[1], c[2])];
                        }
                    }
                    for v in 0..16 {
                        let mut u = 0;
                        while u < 16 {
                            let k = m[v][u];
                            if k == 0 {
                                u += 1;
                                continue;
                            }
                            let mut w = 1;
                            while u + w < 16 && m[v][u + w] == k {
                                w += 1;
                            }
                            let mut h = 1;
                            'grow: while v + h < 16 {
                                for i in 0..w {
                                    if m[v + h][u + i] != k {
                                        break 'grow;
                                    }
                                }
                                h += 1;
                            }
                            for row in m.iter_mut().skip(v).take(h) {
                                for cell in row.iter_mut().skip(u).take(w) {
                                    *cell = 0;
                                }
                            }
                            let mesh = match pass {
                                0 => &mut out.solid,
                                1 => &mut out.water,
                                _ => &mut out.glass,
                            };
                            quad_of(mesh, d, axis, ua, va, slice, u as i32, v as i32, w as i32, h as i32, k);
                            u += w;
                        }
                    }
                }
            }
        }
    }
}

/// A key: texture (8 bits), sky and glow (8 bits each, as 0..255), whether a liquid's top sits low (1 bit).
fn face_key(tex: u8, sky: f32, glow: f32, low: bool) -> u64 {
    1 | (tex as u64) << 1 | ((sky * 255.0).round() as u64) << 9 | ((glow * 255.0).round() as u64) << 17 | (low as u64) << 25
}

#[allow(clippy::too_many_arguments)]
fn quad_of(m: &mut Mesh, d: usize, axis: usize, ua: usize, va: usize, slice: i32, u: i32, v: i32, w: i32, h: i32, k: u64) {
    let tex = ((k >> 1) & 255) as u8;
    let sky = ((k >> 9) & 255) as f32 / 255.0;
    let glow = ((k >> 17) & 255) as f32 / 255.0;
    let low = (k >> 25) & 1 == 1;
    let n = DIRS[d];
    let mut corners = FACES[d];
    for c in corners.iter_mut() {
        let mut p = [0.0f32; 3];
        p[axis] = slice as f32 + c[axis];
        p[ua] = if c[ua] > 0.5 { (u + w) as f32 } else { u as f32 };
        p[va] = if c[va] > 0.5 { (v + h) as f32 } else { v as f32 };
        if low && c[1] > 0.5 {
            // A liquid's top (or the top edge of its side) sits a little low.
            p[1] -= 2.0 / 16.0;
        }
        *c = p;
    }
    let col = data(tex, sky, glow);
    push_quad(m, corners, [n[0] as f32, n[1] as f32, n[2] as f32], [col; 4], [0.0; 4], false);
}

/// Meshes section `s` (y from 16 s) of the middle chunk of `hood`.
pub fn mesh_section(hood: &Hood, s: usize) -> SectionMesh {
    let mut out = SectionMesh::default();
    let mut greedy = Greedy::new();
    let y0 = s as i32 * SECTION;
    for y in y0..y0 + SECTION {
        for z in 0..W {
            for x in 0..W {
                let b = hood.block(x, y, z);
                if b.is_air() {
                    continue;
                }
                let local = [x as f32, (y - y0) as f32, z as f32];
                match b.info().shape {
                    Shape::Air => {}
                    Shape::Cube | Shape::Liquid | Shape::Cactus => cube_faces(hood, &mut out, &mut greedy, b, x, y, z, local),
                    Shape::Plant(p) => plant(hood, &mut out.solid, p, x, y, z, local),
                    Shape::Torch(facing) => torch(hood, &mut out.solid, facing, x, y, z, local),
                }
            }
        }
    }
    greedy.emit(&mut out);
    out
}

#[allow(clippy::too_many_arguments)]
fn cube_faces(hood: &Hood, out: &mut SectionMesh, greedy: &mut Greedy, b: Block, x: i32, y: i32, z: i32, local: [f32; 3]) {
    let info = b.info();
    let liquid = info.shape == Shape::Liquid;
    let cactus = info.shape == Shape::Cactus;
    // A liquid with none of its kind (or ice) above sits a little lower.
    let above = hood.block(x, y + 1, z);
    // A liquid with none of its kind (or ice) above sits lower, the lower the farther it has flowed.
    let covered = above == b || (b.is_water() && (above.is_water() || above == Block::ICE));
    let top = if !liquid || covered { 1.0 } else { (14.0 - 1.6 * b.water_level().unwrap_or(0) as f32) / 16.0 };
    let pass = match info.pass {
        Pass::Solid => 0,
        Pass::Water => 1,
        Pass::Glass => 2,
    };
    for (d, dir) in DIRS.iter().enumerate() {
        let (nx, ny, nz) = (x + dir[0], y + dir[1], z + dir[2]);
        if ny < 0 {
            continue;
        }
        let n = if ny >= H { Block::AIR } else { hood.block(nx, ny, nz) };
        let show = if cactus {
            // The sides are drawn a sixteenth in, so always; the ends meet other cacti.
            !(2..=3).contains(&d) || n != b && !n.cull()
        } else {
            face_shows(b, n) || (liquid && d == 2 && top < 1.0 && !n.cull())
        };
        if !show {
            continue;
        }
        let texture = b.face_tex(d);
        let normal = [dir[0] as f32, dir[1] as f32, dir[2] as f32];
        let mut corners = FACES[d];
        let mut colors = [[0.0; 3]; 4];
        let mut occl = [3usize; 4];
        for (i, c) in corners.iter_mut().enumerate() {
            if c[1] > 0.5 {
                c[1] = top;
            }
            if cactus && d != 2 && d != 3 {
                // Sides in by a sixteenth.
                let axis = if d < 2 { 0 } else { 2 };
                c[axis] = if c[axis] > 0.5 { 15.0 / 16.0 } else { 1.0 / 16.0 };
            }
            let (sky, glow, ao) = if liquid || info.pass != Pass::Solid {
                let (s, g) = if ny >= H { (15, 0) } else { hood.light(nx, ny, nz) };
                (s as f32 / 15.0, g as f32 / 15.0, 3)
            } else {
                corner_light(hood, x, y, z, d, FACES[d][i])
            };
            occl[i] = ao;
            let k = AO[ao];
            colors[i] = data(texture, sky * k, glow * k);
            *c = [c[0] + local[0], c[1] + local[1], c[2] + local[2]];
        }
        // A face lit alike at all four corners waits to be merged with its like.
        // (A flowing liquid's top sits at its own height: those are not merged.)
        let uniform = !cactus && colors.iter().all(|c| *c == colors[0]) && (top == 1.0 || top == 14.0 / 16.0);
        if uniform {
            let ly = y - (y.div_euclid(SECTION) * SECTION);
            greedy.keys[Greedy::at(pass, d, x, ly, z)] = face_key(texture, colors[0][1], colors[0][2], liquid && top < 1.0);
            continue;
        }
        let mesh = match pass {
            0 => &mut out.solid,
            1 => &mut out.water,
            _ => &mut out.glass,
        };
        // Split the quad along the diagonal that keeps the darker corners apart (otherwise the occlusion shows a crease).
        let flip = occl[0] + occl[2] < occl[1] + occl[3];
        push_quad(mesh, corners, normal, colors, [0.0; 4], flip);
    }
}

/// A face corner's light: the cells in front of the face around that corner, averaged over the open ones, and how many of the three cells beside the
/// corner are open (0..3) for the occlusion.
fn corner_light(hood: &Hood, x: i32, y: i32, z: i32, d: usize, corner: [f32; 3]) -> (f32, f32, usize) {
    let n = DIRS[d];
    let (fx, fy, fz) = (x + n[0], y + n[1], z + n[2]);
    // The face's two in-plane axes, stepping toward the corner.
    let axis = if n[0] != 0 {
        0
    } else if n[1] != 0 {
        1
    } else {
        2
    };
    let (ua, va) = match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let step = |a: usize| if corner[a] > 0.5 { 1 } else { -1 };
    let mut u = [0i32; 3];
    u[ua] = step(ua);
    let mut v = [0i32; 3];
    v[va] = step(va);
    let cell = |o: [i32; 3]| {
        let (cx, cy, cz) = (fx + o[0], fy + o[1], fz + o[2]);
        if cy >= H {
            return (false, 15u8, 0u8);
        }
        if cy < 0 {
            return (true, 0, 0);
        }
        let b = hood.block(cx, cy, cz);
        let (s, g) = hood.light(cx, cy, cz);
        (b.cull(), s, g)
    };
    let face = cell([0, 0, 0]);
    let side1 = cell(u);
    let side2 = cell(v);
    let corner_cell = cell([u[0] + v[0], u[1] + v[1], u[2] + v[2]]);
    let ao = if side1.0 && side2.0 { 0 } else { 3 - (side1.0 as usize + side2.0 as usize + corner_cell.0 as usize) };
    let (mut sky, mut glow, mut count) = (face.1 as f32, face.2 as f32, 1.0);
    for c in [side1, side2] {
        if !c.0 {
            sky += c.1 as f32;
            glow += c.2 as f32;
            count += 1.0;
        }
    }
    if !corner_cell.0 && !(side1.0 && side2.0) {
        sky += corner_cell.1 as f32;
        glow += corner_cell.2 as f32;
        count += 1.0;
    }
    (sky / count / 15.0, glow / count / 15.0, ao)
}

/// Boxes in sixteenths of a block: min, max, texture.
type Part = ([u8; 3], [u8; 3], u8);

const DANDELION: &[Part] = &[
    ([7, 0, 7], [8, 6, 8], tex::STEM),
    ([5, 1, 7], [7, 2, 8], tex::LEAF),
    ([8, 2, 7], [10, 3, 8], tex::LEAF),
    ([6, 6, 6], [10, 8, 10], tex::YELLOW),
    ([7, 8, 7], [9, 9, 9], tex::YELLOW_DARK),
];

const POPPY: &[Part] = &[
    ([7, 0, 7], [8, 7, 8], tex::STEM),
    ([5, 1, 7], [7, 2, 8], tex::LEAF),
    ([8, 3, 7], [10, 4, 8], tex::LEAF),
    ([6, 7, 6], [10, 9, 10], tex::RED),
    ([5, 8, 7], [11, 9, 9], tex::RED),
    ([7, 9, 7], [9, 10, 9], tex::PETAL_DARK),
];

const CORNFLOWER: &[Part] = &[
    ([7, 0, 7], [8, 8, 8], tex::STEM),
    ([6, 2, 7], [7, 3, 8], tex::LEAF),
    ([8, 4, 7], [9, 5, 8], tex::LEAF),
    ([6, 8, 6], [10, 10, 10], tex::BLUE),
    ([7, 10, 7], [9, 11, 9], tex::BLUE_DARK),
    ([5, 9, 7], [6, 10, 9], tex::BLUE),
    ([10, 9, 7], [11, 10, 9], tex::BLUE),
];

const TALL_GRASS: &[Part] = &[
    ([2, 0, 3], [3, 9, 4], tex::BLADE),
    ([5, 0, 10], [6, 12, 11], tex::BLADE_LIGHT),
    ([8, 0, 5], [9, 10, 6], tex::BLADE),
    ([11, 0, 9], [12, 8, 10], tex::BLADE_LIGHT),
    ([13, 0, 2], [14, 11, 3], tex::BLADE),
    ([4, 0, 13], [5, 7, 14], tex::BLADE_LIGHT),
    ([10, 0, 12], [11, 13, 13], tex::BLADE),
    ([7, 0, 8], [8, 6, 9], tex::BLADE_LIGHT),
    ([12, 0, 6], [13, 9, 7], tex::BLADE_LIGHT),
    ([3, 0, 7], [4, 11, 8], tex::BLADE),
];

const DEAD_BUSH: &[Part] = &[
    ([7, 0, 7], [8, 5, 8], tex::DEAD),
    ([5, 4, 7], [7, 5, 8], tex::DEAD),
    ([4, 5, 7], [5, 8, 8], tex::DEAD),
    ([8, 3, 7], [10, 4, 8], tex::DEAD),
    ([10, 4, 7], [11, 7, 8], tex::DEAD),
    ([7, 5, 5], [8, 6, 7], tex::DEAD),
    ([7, 6, 4], [8, 9, 5], tex::DEAD),
    ([7, 4, 8], [8, 5, 10], tex::DEAD),
    ([7, 5, 10], [8, 7, 11], tex::DEAD),
];

const TORCH: &[Part] = &[([7, 0, 7], [9, 10, 9], tex::TORCH_WOOD), ([7, 10, 7], [9, 12, 9], tex::FLAME), ([7, 12, 7], [9, 13, 9], tex::FLAME_CORE)];

/// The model of a plant.
pub fn plant_parts(p: Plant) -> &'static [Part] {
    match p {
        Plant::Dandelion => DANDELION,
        Plant::Poppy => POPPY,
        Plant::Cornflower => CORNFLOWER,
        Plant::TallGrass => TALL_GRASS,
        Plant::DeadBush => DEAD_BUSH,
    }
}

/// The torch's boxes.
pub fn torch_parts() -> &'static [Part] {
    TORCH
}

/// Writes boxes (in sixteenths, placed by `place`, a transform of the unit cell) lit by (sky, glow); `sway` moves their tops in the wind.
pub fn boxes(m: &mut Mesh, parts: &[Part], place: Mat4, sky: f32, glow: f32, sway: f32) {
    for (lo, hi, t) in parts {
        let lo = Vec3::new(lo[0] as f32, lo[1] as f32, lo[2] as f32) / 16.0;
        let hi = Vec3::new(hi[0] as f32, hi[1] as f32, hi[2] as f32) / 16.0;
        for (d, dir) in DIRS.iter().enumerate() {
            if d == 3 && lo.y == 0.0 {
                continue; // a bottom on the ground is never seen
            }
            let normal = place.transform_vector3(Vec3::new(dir[0] as f32, dir[1] as f32, dir[2] as f32)).normalize();
            let mut corners = [[0.0; 3]; 4];
            let mut colors = [[0.0; 3]; 4];
            let mut sways = [0.0; 4];
            for (i, c) in FACES[d].iter().enumerate() {
                let p = Vec3::new(lo.x + c[0] * (hi.x - lo.x), lo.y + c[1] * (hi.y - lo.y), lo.z + c[2] * (hi.z - lo.z));
                // A little darker at the foot.
                let k = 0.7 + 0.3 * p.y.min(0.6) / 0.6;
                colors[i] = data(*t, sky * k, glow * k);
                sways[i] = sway * p.y;
                corners[i] = place.transform_point3(p).to_array();
            }
            push_quad(m, corners, normal.to_array(), colors, sways, false);
        }
    }
}

fn cell_light(hood: &Hood, x: i32, y: i32, z: i32) -> (f32, f32) {
    let (s, g) = hood.light(x, y, z);
    (s as f32 / 15.0, g as f32 / 15.0)
}

/// A tuft of grass: blades of different heights, each a tapered strip seen from both sides, leaning its own way from a foot near the middle of
/// the cell; their tips sway in the wind.
fn grass_tuft(m: &mut Mesh, h: u32, local: [f32; 3], sky: f32, glow: f32) {
    let mut r = h | 1;
    let mut roll = || {
        r ^= r << 13;
        r ^= r >> 17;
        r ^= r << 5;
        (r >> 8) as f32 / (1u32 << 24) as f32
    };
    let blades = 9 + (h % 4) as usize;
    let base = Vec3::from(local);
    for k in 0..blades {
        let foot = base + Vec3::new(0.18 + roll() * 0.64, 0.0, 0.18 + roll() * 0.64);
        let tall = 0.3 + roll() * 0.48;
        let turn = roll() * std::f32::consts::TAU;
        let lean = 0.1 + roll() * 0.3;
        let w = 0.035 + roll() * 0.025;
        let across = Vec3::new(turn.cos(), 0.0, turn.sin());
        let out = Vec3::new(-turn.sin(), 0.0, turn.cos());
        let tip = foot + Vec3::Y * tall + out * (lean * tall);
        let t = if k % 3 == 0 { tex::BLADE_LIGHT } else { tex::BLADE };
        let (lo, hi) = (data(t, sky * 0.72, glow * 0.72), data(t, sky, glow));
        let n = out.cross(across).normalize();
        let quad = [foot - across * w, foot + across * w, tip + across * (w * 0.25), tip - across * (w * 0.25)];
        let sway = 0.1 * tall;
        for side in [1.0f32, -1.0] {
            let b = m.vertices.len() as u32;
            for (i, p) in quad.iter().enumerate() {
                let top = i >= 2;
                m.vertices.push(Vertex {
                    pos: p.to_array(),
                    normal: (n * side).to_array(),
                    color: if top { hi } else { lo },
                    sway: if top { sway } else { 0.0 },
                });
            }
            if side > 0.0 {
                m.indices.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
            } else {
                m.indices.extend_from_slice(&[b, b + 2, b + 1, b, b + 3, b + 2]);
            }
        }
    }
}

fn plant(hood: &Hood, m: &mut Mesh, p: Plant, x: i32, y: i32, z: i32, local: [f32; 3]) {
    let (sky, glow) = cell_light(hood, x, y, z);
    if p == Plant::TallGrass {
        grass_tuft(m, crate::noise::hash3(0x6a55, x, y, z), local, sky, glow);
        return;
    }
    // Each plant turned and nudged its own way, so a field does not look stamped.
    let h = crate::noise::hash3(0x9a17, x, y, z);
    let turn = (h % 4) as f32 * std::f32::consts::FRAC_PI_2 + ((h >> 4) % 5) as f32 * 0.17;
    let nudge = Vec3::new(((h >> 8) % 5) as f32 / 16.0 - 0.125, 0.0, ((h >> 12) % 5) as f32 / 16.0 - 0.125);
    let place = Mat4::from_translation(Vec3::from(local) + Vec3::new(0.5, 0.0, 0.5) + nudge)
        * Mat4::from_rotation_y(turn)
        * Mat4::from_translation(Vec3::new(-0.5, 0.0, -0.5));
    let sway = if p == Plant::DeadBush { 0.0 } else { 0.12 };
    boxes(m, plant_parts(p), place, sky, glow, sway);
}

/// Where a torch's boxes go in its cell: standing, or leaning out from the wall behind it.
pub fn torch_place(facing: Option<u8>) -> Mat4 {
    match facing {
        None => Mat4::IDENTITY,
        Some(f) => {
            // `facing` is the way it leans: its foot is against the wall on the other side.
            let yaw = match f {
                0 => 0.0,
                1 => -std::f32::consts::FRAC_PI_2,
                2 => std::f32::consts::PI,
                _ => std::f32::consts::FRAC_PI_2,
            };
            Mat4::from_translation(Vec3::new(0.5, 0.2, 0.5))
                * Mat4::from_rotation_y(yaw)
                * Mat4::from_translation(Vec3::new(0.0, 0.0, 0.32))
                * Mat4::from_rotation_x(-0.38)
                * Mat4::from_translation(Vec3::new(-0.5, 0.0, -0.5))
        }
    }
}

fn torch(hood: &Hood, m: &mut Mesh, facing: Option<u8>, x: i32, y: i32, z: i32, local: [f32; 3]) {
    let (sky, glow) = cell_light(hood, x, y, z);
    boxes(m, TORCH, Mat4::from_translation(Vec3::from(local)) * torch_place(facing), sky, glow, 0.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_face_winds_counter_clockwise_seen_from_outside() {
        for (d, f) in FACES.iter().enumerate() {
            let p = f.map(Vec3::from);
            let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
            let want = Vec3::new(DIRS[d][0] as f32, DIRS[d][1] as f32, DIRS[d][2] as f32);
            assert!((n - want).length() < 1e-5, "face {d}: {n} vs {want}");
        }
    }

    fn hood_with(f: impl Fn(&mut ChunkData)) -> Vec<ChunkData> {
        let mut v: Vec<ChunkData> = (0..9).map(|_| ChunkData::default()).collect();
        f(&mut v[4]);
        for c in v.iter_mut() {
            c.light.fill(15 << 4);
        }
        v
    }

    #[test]
    fn a_lone_block_shows_six_faces_and_two_touching_hide_the_two_between() {
        let v = hood_with(|c| c.set(5, 20, 5, Block::STONE));
        let hood = Hood { chunks: std::array::from_fn(|i| &v[i]) };
        let m = mesh_section(&hood, 1);
        assert_eq!(m.solid.indices.len(), 6 * 6);
        let v = hood_with(|c| {
            c.set(5, 20, 5, Block::STONE);
            c.set(6, 20, 5, Block::DIRT);
        });
        let hood = Hood { chunks: std::array::from_fn(|i| &v[i]) };
        let m = mesh_section(&hood, 1);
        assert_eq!(m.solid.indices.len(), 10 * 6);
        // The dirt's top carries its texture and full sky light.
        let top = m.solid.vertices.iter().find(|v| v.normal == [0.0, 1.0, 0.0] && v.pos[0] > 6.5).unwrap();
        assert_eq!((top.color[0] * 255.0).round() as u8, tex::DIRT);
        assert!(top.color[1] > 0.99);
    }

    #[test]
    fn a_corner_tucked_against_walls_is_darker() {
        // A floor with a wall along x = 4: the floor cell beside the wall has two dark corners.
        let v = hood_with(|c| {
            for z in 0..16 {
                for x in 0..16 {
                    c.set(x, 20, z, Block::STONE);
                }
                c.set(4, 21, z, Block::STONE);
            }
        });
        let hood = Hood { chunks: std::array::from_fn(|i| &v[i]) };
        let m = mesh_section(&hood, 1);
        let tops: Vec<&Vertex> = m.solid.vertices.iter().filter(|v| v.normal == [0.0, 1.0, 0.0] && (v.pos[1] - 5.0).abs() < 1e-4).collect();
        let beside = tops.iter().filter(|v| v.pos[0] == 5.0 && v.pos[2] == 8.0).map(|v| v.color[1]).fold(1.0f32, f32::min);
        let open = tops.iter().map(|v| v.color[1]).fold(0.0f32, f32::max);
        assert!(beside < open - 0.15, "against the wall {beside} vs open {open}");
        // The open floor was merged into a few large quads.
        assert!(tops.len() < 4 * 60, "{} top corners", tops.len());
    }

    #[test]
    fn water_shows_its_surface_low_and_hides_against_itself_and_glass_goes_to_its_own_pass() {
        let v = hood_with(|c| {
            for x in 0..3 {
                c.set(x, 20, 0, Block::WATER);
            }
            c.set(8, 20, 8, Block::GLASS);
        });
        let hood = Hood { chunks: std::array::from_fn(|i| &v[i]) };
        let m = mesh_section(&hood, 1);
        let top = m.water.vertices.iter().find(|v| v.normal == [0.0, 1.0, 0.0]).unwrap();
        assert!((top.pos[1] - (4.0 + 14.0 / 16.0)).abs() < 1e-5);
        // 3 cells in a row, lit alike: one top, one bottom, one side each way, two ends (merged).
        assert_eq!(m.water.indices.len() / 6, 1 + 1 + 2 + 2);
        assert_eq!(m.glass.indices.len() / 6, 6);
    }

    #[test]
    fn plants_and_torches_are_small_models() {
        let v = hood_with(|c| {
            c.set(2, 20, 2, Block::POPPY);
            c.set(9, 20, 9, Block::TORCH);
        });
        let hood = Hood { chunks: std::array::from_fn(|i| &v[i]) };
        let m = mesh_section(&hood, 1);
        assert!(m.solid.vertices.iter().any(|v| v.sway > 0.0), "the poppy sways");
        let flame = m.solid.vertices.iter().find(|v| (v.color[0] * 255.0).round() as u8 == tex::FLAME).unwrap();
        assert!(flame.pos[1] > 4.6 && flame.pos[1] < 4.9);
    }
}
