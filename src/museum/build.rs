//! Turning a furnished room, a doorway or a world inside a painting into things to draw and things to bump into.
//!
//! A [`Model`] is plain data (meshes as [`Geo`], transforms, materials, colliders, lamps) in its own space, placed in the world at `origin`; the renderer
//! uploads it once and moves only its animated pieces ([`Anim`]). Nothing here touches a GPU, so the headless tests can build every room.

use super::exhibit::{partition_height, partition_length, Contents, Exhibit, FrameStyle, Ground, Kind, Nested, Pocket, Stand, PARTITION_T};
use super::layout::{EdgeId, Floor, Plan, Room, Side, CELL, DOOR_H, DOOR_W, HALL_H, HALL_W, WALL};
use super::sculpt::{build as build_sculpture, Family, Part};
use super::shape::{aabox, ball, canvas, lathe, obox, quad, tube_path};
use super::spec::{Collection, Rgb};
use super::text::Draw;
use crate::collide::Collider2D;
use crate::procgen::geo::{mix, shade, Geo};
use glam::{Mat3, Mat4, Vec2, Vec3};
use std::sync::Arc;

/// How a piece of a model is shaded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: Vec3,
    /// A painted surface's numbers (`x`: 1 canvas, 2 solid; `y`: style or material; `z`: seed; `w`: canvas aspect or solid glow); zero: not painted.
    pub surface: [f32; 4],
    pub palette: [[f32; 4]; 4],
}

impl Look {
    pub const MATTE: Look = Look { metallic: 0.0, roughness: 0.82, emissive: Vec3::ZERO, surface: [0.0; 4], palette: [[0.0; 4]; 4] };
    pub const METAL: Look = Look { metallic: 0.85, roughness: 0.32, emissive: Vec3::ZERO, surface: [0.0; 4], palette: [[0.0; 4]; 4] };

    fn glow(c: Rgb, k: f32) -> Look {
        Look { emissive: Vec3::from(c) * k, ..Look::MATTE }
    }
}

/// How a piece moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Anim {
    Still,
    /// Part `part` of the sculpture of exhibit `exhibit` (see [`Model::rigs`]).
    Part {
        exhibit: u8,
        part: u16,
    },
    /// A door leaf: it turns about the vertical line through `hinge` (model space) by up to `swing` radians as the door opens.
    Leaf {
        hinge: Vec3,
        swing: f32,
    },
    /// The key: it turns and bobs.
    Key,
    /// A painting floating in a world: it drifts.
    Drift {
        phase: f32,
    },
}

/// One drawable thing.
#[derive(Debug, Clone)]
pub struct Piece {
    pub geo: Arc<Geo>,
    /// Model-space transform (before any animation).
    pub local: Mat4,
    pub look: Look,
    pub anim: Anim,
}

/// A lamp (world space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lamp {
    pub at: Vec3,
    pub color: Rgb,
    pub intensity: f32,
    pub range: f32,
}

/// The moving parts of one sculpture, without their meshes (those are in the pieces).
#[derive(Debug, Clone)]
pub struct Rig {
    /// Where the sculpture stands (model space): its base, turned to face its way.
    pub base: Mat4,
    pub parts: Vec<Part>,
}

/// Something to draw and bump into, placed at `origin`.
#[derive(Clone, Default)]
pub struct Model {
    pub origin: Vec3,
    pub pieces: Vec<Piece>,
    /// World space.
    pub colliders: Vec<Collider2D>,
    pub lamps: Vec<Lamp>,
    /// Sculptures by exhibit number.
    pub rigs: Vec<(u8, Rig)>,
}

impl Model {
    fn add(&mut self, geo: Geo, local: Mat4, look: Look, anim: Anim) {
        if !geo.idx.is_empty() {
            self.pieces.push(Piece { geo: Arc::new(geo), local, look, anim });
        }
    }

    /// A box collider from model-space corners.
    fn collide(&mut self, min: Vec3, max: Vec3) {
        let (a, b) = (min + self.origin, max + self.origin);
        self.colliders.push(Collider2D {
            min: Vec2::new(a.x.min(b.x), a.z.min(b.z)),
            max: Vec2::new(a.x.max(b.x), a.z.max(b.z)),
            min_y: a.y.min(b.y),
            max_y: a.y.max(b.y),
        });
    }

    /// The rig of exhibit `e`.
    pub fn rig(&self, e: u8) -> Option<&Rig> {
        self.rigs.iter().find(|(i, _)| *i == e).map(|(_, r)| r)
    }

    /// Triangles in all pieces.
    pub fn tris(&self) -> usize {
        self.pieces.iter().map(|p| p.geo.tris()).sum()
    }
}

/// The unit canvas every painting is drawn with (a tapestry has its own folds).
pub fn unit_canvas() -> Geo {
    canvas(1, 1, |_, _| 0.0)
}

fn rgb4(c: Rgb, w: f32) -> [f32; 4] {
    [c[0], c[1], c[2], w]
}

/// A painted canvas's look. `uv` is (offset, scale) along x (a triptych's panels share one picture); `alive` how much it glows and moves; `glint` 1 for a
/// painting on the way to a key (its picture keeps a faint golden shimmer).
pub fn canvas_look(art: &super::exhibit::Art, aspect: f32, uv: (f32, f32), alive: f32, glint: f32) -> Look {
    Look {
        metallic: 0.0,
        roughness: 0.7,
        emissive: Vec3::ZERO,
        surface: [1.0, art.style as f32, art.seed, aspect],
        palette: [rgb4(art.palette[0], uv.0), rgb4(art.palette[1], uv.1), rgb4(art.palette[2], alive), rgb4(art.palette[3], glint)],
    }
}

/// A sculpture's painted material.
fn solid_look(e: &Exhibit) -> Look {
    let a = &e.art;
    Look {
        metallic: 0.0,
        roughness: 0.5,
        emissive: Vec3::ZERO,
        surface: [2.0, a.style as f32, a.seed, 0.0],
        palette: [rgb4(a.palette[0], 0.0), rgb4(a.palette[1], 0.0), rgb4(a.palette[2], 0.0), rgb4(a.palette[3], 0.0)],
    }
}

/// A local frame for something facing `facing`: columns right, up, facing.
fn facing_basis(facing: Vec3) -> Mat3 {
    let f = Vec3::new(facing.x, 0.0, facing.z).normalize_or(Vec3::Z);
    let right = Vec3::Y.cross(f).normalize();
    Mat3::from_cols(right, Vec3::Y, f)
}

/// The frame around a canvas centred at `c` (model space), facing `f`, `size` across; matte parts go to `matte`, gilt and metal to `metal`.
pub fn frame(matte: &mut Geo, metal: &mut Geo, c: Vec3, f: Vec3, size: Vec2, style: FrameStyle, seed: u32) {
    let m = facing_basis(f);
    let (r, u, n) = (m.x_axis, m.y_axis, m.z_axis);
    let b = style.border(size);
    let hw = size.x * 0.5;
    let hh = size.y * 0.5;
    let mut d = Draw::new(&[seed as i64, 3]);
    // A rail: one side of the frame, from `inner` (the canvas edge) out by `width`, standing out `depth` from the wall plane behind the canvas.
    let rail = |g: &mut Geo, width: f32, depth: f32, inset: f32, col: Rgb| {
        let back = -n * 0.04;
        // Top and bottom run the full width; the sides between them.
        let w_out = hw + inset + width;
        let h_out = hh + inset + width;
        for s in [-1.0f32, 1.0] {
            obox(g, c + back + u * s * (hh + inset + width * 0.5) + n * depth * 0.5, [r, u, n], Vec3::new(w_out, width * 0.5, depth * 0.5 + 0.02), col);
            obox(g, c + back + r * s * (hw + inset + width * 0.5) + n * depth * 0.5, [r, u, n], Vec3::new(width * 0.5, h_out - width, depth * 0.5 + 0.02), col);
        }
    };
    let gold = [0.83, 0.6, 0.24];
    match style {
        FrameStyle::Gilded => {
            rail(metal, b, 0.07, 0.0, gold);
            rail(metal, b * 0.25, 0.1, b * 0.15, shade(gold, 1.15));
            rail(matte, b * 0.12, 0.03, -0.01, [0.32, 0.22, 0.12]);
        }
        FrameStyle::Baroque => {
            rail(metal, b, 0.09, 0.0, shade(gold, 0.95));
            rail(metal, b * 0.35, 0.13, b * 0.3, shade(gold, 1.2));
            rail(metal, b * 0.12, 0.05, -0.01, shade(gold, 0.7));
            // Carved bosses at the corners and the middles.
            for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0), (0.0, 1.0), (0.0, -1.0), (-1.0, 0.0), (1.0, 0.0)] {
                let p = c + r * x * (hw + b * 0.5) + u * y * (hh + b * 0.5) + n * 0.11;
                metal.blob(p, Vec3::new(b * 0.55, b * 0.55, b * 0.3), 1, d.below(99) as u32, 0.25, shade(gold, 0.9), shade(gold, 1.25));
            }
        }
        FrameStyle::Ebony => rail(matte, b, 0.05, 0.0, [0.02, 0.018, 0.016]),
        FrameStyle::Oak => rail(matte, b, 0.06, 0.0, [0.55, 0.38, 0.2]),
        FrameStyle::Gallery => {
            rail(matte, b, 0.012, 0.0, [0.92, 0.9, 0.86]);
            rail(matte, 0.025, 0.05, b, [0.03, 0.03, 0.03]);
        }
        FrameStyle::Driftwood => {
            let grey = [0.42, 0.4, 0.36];
            for s in [-1.0f32, 1.0] {
                let wob = d.range(-0.04, 0.04);
                obox(
                    matte,
                    c + u * s * (hh + b * 0.5) + r * wob + n * 0.03,
                    [r, u, n],
                    Vec3::new(hw + b * d.range(1.2, 1.9), b * 0.5, 0.04),
                    shade(grey, d.range(0.85, 1.1)),
                );
                obox(
                    matte,
                    c + r * s * (hw + b * 0.5) + u * wob + n * 0.035,
                    [r, u, n],
                    Vec3::new(b * 0.5, hh + b * d.range(1.1, 1.8), 0.04),
                    shade(grey, d.range(0.85, 1.1)),
                );
            }
        }
        FrameStyle::Floating => {
            // A dark shadow gap behind the canvas edge.
            obox(matte, c - n * 0.03, [r, u, n], Vec3::new(hw - 0.03, hh - 0.03, 0.03), [0.03, 0.03, 0.03]);
        }
        FrameStyle::Copper => {
            rail(metal, b, 0.05, 0.0, [0.75, 0.42, 0.25]);
            for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let p = c + r * x * (hw + b * 0.5) + u * y * (hh + b * 0.5) + n * 0.05;
                obox(matte, p, [r, u, n], Vec3::new(b * 0.6, b * 0.6, 0.012), [0.25, 0.55, 0.45]);
            }
        }
        FrameStyle::Velvet => {
            rail(matte, b, 0.16, 0.0, [0.18, 0.04, 0.07]);
            rail(metal, 0.03, 0.18, b, [0.7, 0.55, 0.3]);
        }
        FrameStyle::Silver => {
            rail(metal, b, 0.06, 0.0, [0.78, 0.8, 0.84]);
            rail(metal, b * 0.2, 0.08, b * 0.4, [0.9, 0.92, 0.95]);
        }
    }
    // Canvases (except floating ones) are set back a hair into the frame; the back board.
    if style != FrameStyle::Floating {
        obox(matte, c - n * 0.035, [r, u, n], Vec3::new(hw + 0.01, hh + 0.01, 0.01), [0.08, 0.07, 0.06]);
    }
}

/// The transform that turns the unit canvas into one `size` across at `c`, facing `f`.
pub fn canvas_transform(c: Vec3, f: Vec3, size: Vec2) -> Mat4 {
    let m = facing_basis(f);
    Mat4::from_cols((m.x_axis * size.x).extend(0.0), (m.y_axis * size.y).extend(0.0), m.z_axis.extend(0.0), c.extend(1.0))
}

/// A wall box from `a` to `b` along the floor (model space), `t` thick, between heights `y0` and `y1`.
fn wall(g: &mut Geo, min: Vec3, max: Vec3, col: Rgb) {
    aabox(g, min, max, col);
}

/// A room, its halls and everything in it.
pub fn room_model(c: &Collection, k: &Contents) -> Model {
    let room = &k.room;
    let theme = &c.themes[room.theme];
    let mut m = Model { origin: room.id.centre(), ..Default::default() };
    let mut matte = Geo::default();
    let mut metal = Geo::default();
    let mut glow = Geo::default();
    let (hx, hz) = (room.size.x * 0.5, room.size.y * 0.5);
    let h = room.height;
    let mut d = Draw::new(&[room.id.x as i64, room.id.z as i64, 61]);
    let wall_c = theme.wall;
    let wains = shade(mix(theme.wall, theme.floor, 0.35), 0.8);
    let trim = theme.trim;

    floor(&mut matte, room, theme.floor, &mut d);

    // ---- Walls, with openings for the halls ----
    for side in Side::ALL {
        let door = room.has_door(side);
        let (len, along_x) = (room.wall_length(side), side.runs_along_x());
        let out = side.out();
        let face = if along_x { hz } else { hx };
        // Pieces along the wall (u from -len/2 to len/2), as (u0, u1, y0, y1).
        let mut spans = vec![];
        if door {
            spans.push((-len * 0.5 - WALL, -HALL_W * 0.5, 0.0, h));
            spans.push((HALL_W * 0.5, len * 0.5 + WALL, 0.0, h));
            spans.push((-HALL_W * 0.5, HALL_W * 0.5, HALL_H, h));
        } else {
            spans.push((-len * 0.5 - WALL, len * 0.5 + WALL, 0.0, h));
        }
        let to3 = |u: f32, y: f32, depth: f32| -> Vec3 {
            if along_x {
                Vec3::new(u, y, out.z * (face + depth))
            } else {
                Vec3::new(out.x * (face + depth), y, u)
            }
        };
        for (u0, u1, y0, y1) in spans.iter().copied() {
            let (a, b) = (to3(u0, y0, 0.0), to3(u1, y1, WALL));
            wall(&mut matte, a, b, wall_c);
            m.collide(a.min(b), a.max(b));
            // Wainscot, chair rail and skirting on the inside face (not above the hall opening).
            if y0 == 0.0 {
                let (u0, u1) = (u0.max(-len * 0.5), u1.min(len * 0.5));
                let (a, b) = (to3(u0, 0.0, -0.025), to3(u1, 1.0, 0.0));
                aabox(&mut matte, a.min(b), a.max(b), wains);
                let (a, b) = (to3(u0, 0.98, -0.05), to3(u1, 1.06, 0.0));
                aabox(&mut metal, a.min(b), a.max(b), shade(trim, 0.9));
                let (a, b) = (to3(u0, 0.0, -0.04), to3(u1, 0.16, 0.0));
                aabox(&mut matte, a.min(b), a.max(b), shade(theme.floor, 0.6));
            }
            // A moulding under the ceiling.
            let (u0c, u1c) = (u0.max(-len * 0.5), u1.min(len * 0.5));
            let (a, b) = (to3(u0c, h - 0.22, -0.08), to3(u1c, h, 0.0));
            aabox(&mut matte, a.min(b), a.max(b), shade(trim, 0.75));
        }
        if door {
            // The hall: floor, walls and ceiling from the room's wall to the cell border, and an architrave round the opening.
            let half_cell = CELL * 0.5;
            let hall = |u: f32, y: f32, depth: f32| -> Vec3 {
                if along_x {
                    Vec3::new(u, y, out.z * depth)
                } else {
                    Vec3::new(out.x * depth, y, u)
                }
            };
            let (d0, d1) = (face, half_cell);
            let fa = hall(-HALL_W * 0.5, -0.02, d0);
            let fb = hall(HALL_W * 0.5, 0.0, d1);
            aabox(&mut matte, fa.min(fb), fa.max(fb), shade(theme.floor, 0.75));
            // Side walls of the hall.
            for s in [-1.0f32, 1.0] {
                let a = hall(s * HALL_W * 0.5, 0.0, d0 + WALL);
                let b = hall(s * (HALL_W * 0.5 + WALL), HALL_H, d1);
                wall(&mut matte, a.min(b), a.max(b), shade(wall_c, 0.85));
                m.collide(a.min(b), a.max(b));
            }
            let a = hall(-HALL_W * 0.5 - WALL, HALL_H, d0);
            let b = hall(HALL_W * 0.5 + WALL, HALL_H + 0.3, d1);
            aabox(&mut matte, a.min(b), a.max(b), shade(theme.ceiling, 0.8));
            // A lamp in the hall's ceiling.
            let lamp = hall(0.0, HALL_H - 0.05, (d0 + d1) * 0.5);
            ball(&mut glow, lamp, Vec3::new(0.35, 0.05, 0.35), 1, theme.light);
            // Architrave on the room side.
            for s in [-1.0f32, 1.0] {
                let a = hall(s * HALL_W * 0.5, 0.0, face - 0.06);
                let b = hall(s * (HALL_W * 0.5 + 0.22), HALL_H + 0.22, face);
                aabox(&mut metal, a.min(b), a.max(b), trim);
            }
            let a = hall(-HALL_W * 0.5 - 0.22, HALL_H, face - 0.06);
            let b = hall(HALL_W * 0.5 + 0.22, HALL_H + 0.24, face);
            aabox(&mut metal, a.min(b), a.max(b), trim);
        }
    }

    // ---- Ceiling ----
    ceiling(&mut matte, &mut glow, room, theme.ceiling, theme.light);

    // ---- Plan features ----
    match room.plan {
        Plan::Plain => {}
        Plan::Pillars => {
            for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let p = Vec3::new(sx * (hx - 1.6), 0.0, sz * (hz - 1.6));
                let r = 0.34;
                let profile =
                    [(r * 1.5, 0.0), (r * 1.5, 0.25), (r * 1.15, 0.32), (r, 0.5), (r * 0.92, h - 0.7), (r * 1.2, h - 0.45), (r * 1.6, h - 0.25), (r * 1.6, h)];
                lathe(&mut matte, p, &profile, 20, |t| mix(shade(wall_c, 1.15), shade(trim, 0.9), (t - 0.9).max(0.0) * 10.0));
                m.collide(p - Vec3::new(r * 1.1, 0.0, r * 1.1), p + Vec3::new(r * 1.1, h, r * 1.1));
            }
        }
        Plan::Partition { along_x } => {
            let long = if along_x { room.size.x } else { room.size.y };
            let len = partition_length(long);
            let ph = partition_height(h);
            let half = if along_x { Vec3::new(len * 0.5, ph * 0.5, PARTITION_T * 0.5) } else { Vec3::new(PARTITION_T * 0.5, ph * 0.5, len * 0.5) };
            let centre = Vec3::Y * ph * 0.5;
            aabox(&mut matte, centre - half, centre + half, shade(wall_c, 1.05));
            let cap = half + Vec3::new(0.05, 0.0, 0.05);
            aabox(&mut metal, Vec3::new(-cap.x, ph, -cap.z), Vec3::new(cap.x, ph + 0.08, cap.z), trim);
            aabox(&mut matte, Vec3::new(-cap.x, 0.0, -cap.z), Vec3::new(cap.x, 0.16, cap.z), shade(theme.floor, 0.6));
            m.collide(centre - half, centre + half);
        }
    }

    // ---- Benches ----
    for b in &k.benches {
        let at = b.at - room.id.centre();
        let mm = facing_basis(b.facing);
        let (r, f) = (mm.x_axis, mm.z_axis);
        let wood = [0.33, 0.2, 0.12];
        obox(&mut matte, at + Vec3::Y * 0.44, [r, Vec3::Y, f], Vec3::new(b.length * 0.5, 0.04, 0.24), wood);
        obox(&mut matte, at + Vec3::Y * 0.47, [r, Vec3::Y, f], Vec3::new(b.length * 0.5 - 0.04, 0.012, 0.22), [0.18, 0.08, 0.07]);
        for s in [-1.0, 1.0] {
            obox(&mut metal, at + r * s * (b.length * 0.5 - 0.15) + Vec3::Y * 0.21, [r, Vec3::Y, f], Vec3::new(0.03, 0.21, 0.2), [0.15, 0.14, 0.13]);
        }
        let ext = (r * b.length * 0.5).abs() + (f * 0.26).abs();
        m.collide(at - Vec3::new(ext.x, 0.0, ext.z), at + Vec3::new(ext.x, 0.5, ext.z));
    }

    // ---- The curator's note ----
    {
        let at = k.note.at - room.id.centre();
        let mm = facing_basis(k.note.facing);
        let (r, u, n) = (mm.x_axis, mm.y_axis, mm.z_axis);
        obox(&mut metal, at, [r, u, n], Vec3::new(0.3, 0.38, 0.015), shade(trim, 0.8));
        obox(&mut matte, at + n * 0.016, [r, u, n], Vec3::new(0.26, 0.34, 0.004), [0.86, 0.82, 0.72]);
        // A few lines of "text" in ink.
        for i in 0..7 {
            let w = if i == 0 { 0.16 } else { d.range(0.12, 0.22) };
            let y = 0.24 - i as f32 * 0.07 - if i > 0 { 0.04 } else { 0.0 };
            obox(&mut matte, at + n * 0.021 + u * y + r * (w - 0.22), [r, u, n], Vec3::new(w, if i == 0 { 0.012 } else { 0.006 }, 0.001), [0.12, 0.1, 0.09]);
        }
    }

    // ---- The pieces ----
    for e in &k.exhibits {
        exhibit_model(&mut m, &mut matte, &mut metal, &mut glow, c, room, e, &mut d);
    }

    // ---- Light ----
    // One lamp high in the middle and one over each quarter, where the walls with the pictures are.
    m.lamps.push(Lamp {
        at: room.id.centre() + Vec3::Y * (h - 0.8),
        color: theme.light,
        intensity: 30.0 + room.size.x * room.size.y * 0.04,
        range: room.size.length() * 0.9 + 4.0,
    });
    for (sx, sz) in [(-1.0, -1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, 1.0)] {
        m.lamps.push(Lamp {
            at: room.id.centre() + Vec3::new(sx * hx * 0.5, (h * 0.62).min(h - 1.4), sz * hz * 0.5),
            color: theme.light,
            intensity: 18.0,
            range: room.size.length() * 0.55 + 3.0,
        });
    }

    m.add(matte, Mat4::IDENTITY, Look::MATTE, Anim::Still);
    m.add(metal, Mat4::IDENTITY, Look::METAL, Anim::Still);
    m.add(glow, Mat4::IDENTITY, Look::glow(theme.light, 0.85), Anim::Still);
    m
}

fn floor(g: &mut Geo, room: &Room, base: Rgb, d: &mut Draw) {
    let (hx, hz) = (room.size.x * 0.5, room.size.y * 0.5);
    let up = Vec3::Y;
    let light = shade(base, 1.55);
    let dark = shade(base, 0.7);
    match room.floor {
        Floor::Checker => {
            let n = 1.0;
            let (nx, nz) = ((room.size.x / n) as i32, (room.size.y / n) as i32);
            for i in 0..nx {
                for j in 0..nz {
                    let (x0, z0) = (-hx + i as f32 * n, -hz + j as f32 * n);
                    let col = if (i + j) % 2 == 0 { light } else { dark };
                    quad(
                        g,
                        Vec3::new(x0, 0.0, z0),
                        Vec3::new(x0, 0.0, z0 + n),
                        Vec3::new(x0 + n, 0.0, z0 + n),
                        Vec3::new(x0 + n, 0.0, z0),
                        up,
                        shade(col, d.range(0.96, 1.04)),
                    );
                }
            }
        }
        Floor::Boards => {
            let w = 0.22;
            let n = (room.size.y / w).ceil() as i32;
            for j in 0..n {
                let z0 = -hz + j as f32 * w;
                let z1 = (z0 + w).min(hz);
                // Boards of random lengths, end to end.
                let mut x = -hx;
                while x < hx {
                    let len = d.range(1.4, 3.2).min(hx - x);
                    let col = shade(base, d.range(0.82, 1.18));
                    quad(
                        g,
                        Vec3::new(x, 0.0, z0),
                        Vec3::new(x, 0.0, z1 - 0.008),
                        Vec3::new(x + len - 0.008, 0.0, z1 - 0.008),
                        Vec3::new(x + len - 0.008, 0.0, z0),
                        up,
                        col,
                    );
                    x += len;
                }
            }
            quad(g, Vec3::new(-hx, -0.001, -hz), Vec3::new(-hx, -0.001, hz), Vec3::new(hx, -0.001, hz), Vec3::new(hx, -0.001, -hz), up, shade(base, 0.35));
        }
        Floor::Slabs => {
            let border = 0.8;
            quad(g, Vec3::new(-hx, 0.0, -hz), Vec3::new(-hx, 0.0, hz), Vec3::new(hx, 0.0, hz), Vec3::new(hx, 0.0, -hz), up, dark);
            let s = 2.0;
            let (ix, iz) = (hx - border, hz - border);
            let (nx, nz) = ((ix * 2.0 / s).round().max(1.0) as i32, (iz * 2.0 / s).round().max(1.0) as i32);
            let (sx, sz) = (ix * 2.0 / nx as f32, iz * 2.0 / nz as f32);
            for i in 0..nx {
                for j in 0..nz {
                    let (x0, z0) = (-ix + i as f32 * sx + 0.01, -iz + j as f32 * sz + 0.01);
                    let col = shade(light, d.range(0.9, 1.06));
                    quad(
                        g,
                        Vec3::new(x0, 0.002, z0),
                        Vec3::new(x0, 0.002, z0 + sz - 0.02),
                        Vec3::new(x0 + sx - 0.02, 0.002, z0 + sz - 0.02),
                        Vec3::new(x0 + sx - 0.02, 0.002, z0),
                        up,
                        col,
                    );
                }
            }
        }
        Floor::Parquet => {
            // Basket weave: squares of three planks, turned every other square.
            let s = 0.66;
            let (nx, nz) = ((room.size.x / s).ceil() as i32, (room.size.y / s).ceil() as i32);
            for i in 0..nx {
                for j in 0..nz {
                    let (x0, z0) = (-hx + i as f32 * s, -hz + j as f32 * s);
                    let (x1, z1) = ((x0 + s).min(hx), (z0 + s).min(hz));
                    for k in 0..3 {
                        let col = shade(base, d.range(0.8, 1.2));
                        let t0 = k as f32 / 3.0;
                        let t1 = (k + 1) as f32 / 3.0;
                        let (a, b) = if (i + j) % 2 == 0 {
                            (Vec3::new(x0, 0.0, z0 + (z1 - z0) * t0), Vec3::new(x1 - 0.006, 0.0, z0 + (z1 - z0) * t1 - 0.006))
                        } else {
                            (Vec3::new(x0 + (x1 - x0) * t0, 0.0, z0), Vec3::new(x0 + (x1 - x0) * t1 - 0.006, 0.0, z1 - 0.006))
                        };
                        quad(g, a, Vec3::new(a.x, 0.0, b.z), b, Vec3::new(b.x, 0.0, a.z), up, col);
                    }
                }
            }
            quad(g, Vec3::new(-hx, -0.001, -hz), Vec3::new(-hx, -0.001, hz), Vec3::new(hx, -0.001, hz), Vec3::new(hx, -0.001, -hz), up, shade(base, 0.3));
        }
    }
}

fn ceiling(g: &mut Geo, glow: &mut Geo, room: &Room, col: Rgb, light: Rgb) {
    let (hx, hz) = (room.size.x * 0.5, room.size.y * 0.5);
    let h = room.height;
    let down = -Vec3::Y;
    let slab = |g: &mut Geo, x0: f32, z0: f32, x1: f32, z1: f32| {
        quad(g, Vec3::new(x0, h, z0), Vec3::new(x1, h, z0), Vec3::new(x1, h, z1), Vec3::new(x0, h, z1), down, col);
        aabox(g, Vec3::new(x0 - WALL, h + 0.01, z0 - WALL), Vec3::new(x1 + WALL, h + 0.4, z1 + WALL), shade(col, 0.6));
    };
    if room.skylight {
        // A lantern: an opening with a deep well; the sky (the game's own) shows through it.
        let (sx, sz) = (hx * 0.45, hz * 0.45);
        slab(g, -hx, -hz, hx, -sz);
        slab(g, -hx, sz, hx, hz);
        slab(g, -hx, -sz, -sx, sz);
        slab(g, sx, -sz, hx, sz);
        let well = 2.2;
        for (a, b) in [
            (Vec3::new(-sx - 0.2, h, -sz - 0.2), Vec3::new(sx + 0.2, h + well, -sz)),
            (Vec3::new(-sx - 0.2, h, sz), Vec3::new(sx + 0.2, h + well, sz + 0.2)),
            (Vec3::new(-sx - 0.2, h, -sz), Vec3::new(-sx, h + well, sz)),
            (Vec3::new(sx, h, -sz), Vec3::new(sx + 0.2, h + well, sz)),
        ] {
            aabox(g, a, b, shade(col, 1.1));
        }
        // Glazing bars across the opening.
        for i in 1..4 {
            let x = -sx + 2.0 * sx * i as f32 / 4.0;
            aabox(g, Vec3::new(x - 0.04, h + well - 0.1, -sz), Vec3::new(x + 0.04, h + well, sz), [0.08, 0.08, 0.09]);
        }
        for i in 1..3 {
            let z = -sz + 2.0 * sz * i as f32 / 3.0;
            aabox(g, Vec3::new(-sx, h + well - 0.1, z - 0.04), Vec3::new(sx, h + well, z + 0.04), [0.08, 0.08, 0.09]);
        }
    } else {
        slab(g, -hx, -hz, hx, hz);
        // A ring of lamps or a glowing laylight in the middle.
        let (lx, lz) = (hx * 0.3, hz * 0.3);
        quad(glow, Vec3::new(-lx, h - 0.02, -lz), Vec3::new(lx, h - 0.02, -lz), Vec3::new(lx, h - 0.02, lz), Vec3::new(-lx, h - 0.02, lz), down, light);
        aabox(g, Vec3::new(-lx - 0.12, h - 0.1, -lz - 0.12), Vec3::new(lx + 0.12, h, -lz), shade(col, 0.7));
        aabox(g, Vec3::new(-lx - 0.12, h - 0.1, lz), Vec3::new(lx + 0.12, h, lz + 0.12), shade(col, 0.7));
        aabox(g, Vec3::new(-lx - 0.12, h - 0.1, -lz), Vec3::new(-lx, h, lz), shade(col, 0.7));
        aabox(g, Vec3::new(lx, h - 0.1, -lz), Vec3::new(lx + 0.12, h, lz), shade(col, 0.7));
    }
    if room.beams {
        let n = ((room.size.x / 4.0) as i32).max(2);
        for i in 1..n {
            let x = -hx + room.size.x * i as f32 / n as f32;
            if x.abs() < hx * 0.32 {
                continue;
            }
            aabox(g, Vec3::new(x - 0.18, h - 0.45, -hz), Vec3::new(x + 0.18, h, hz), shade(col, 0.55));
        }
    }
}

/// Adds one exhibit: canvas, frame, plinth, sculpture parts.
#[allow(clippy::too_many_arguments)]
fn exhibit_model(m: &mut Model, matte: &mut Geo, metal: &mut Geo, glow: &mut Geo, c: &Collection, room: &Room, e: &Exhibit, d: &mut Draw) {
    let at = e.at - room.id.centre();
    let theme = &c.themes[e.theme.min(c.themes.len() - 1)];
    match e.kind {
        Kind::Painting => {
            frame(matte, metal, at, e.facing, e.size, e.frame, e.index as u32 + d.below(1000) as u32);
            m.add(unit_canvas(), canvas_transform(at, e.facing, e.size), canvas_look(&e.art, e.size.x / e.size.y, (0.0, 1.0), 1.0, 0.0), Anim::Still);
            picture_light(metal, glow, at, e.facing, e.size, e.frame, theme.light);
        }
        Kind::Triptych => {
            let gap = 0.12;
            let side = (e.size.x - 2.0 * gap) * 0.28;
            let mid = e.size.x - 2.0 * gap - 2.0 * side;
            let r = e.right();
            let panels = [(-(mid * 0.5 + gap + side * 0.5), side, 0.0, 0.28), (0.0, mid, 0.31, 0.38), (mid * 0.5 + gap + side * 0.5, side, 0.72, 0.28)];
            for (k, (u, w, uv0, uvs)) in panels.into_iter().enumerate() {
                let size = Vec2::new(w, if k == 1 { e.size.y } else { e.size.y * 0.86 });
                let p = at + r * u;
                frame(matte, metal, p, e.facing, size, e.frame, e.index as u32 * 7 + k as u32);
                m.add(unit_canvas(), canvas_transform(p, e.facing, size), canvas_look(&e.art, e.size.x / e.size.y, (uv0, uvs), 1.0, 0.0), Anim::Still);
            }
            picture_light(metal, glow, at, e.facing, e.size, e.frame, theme.light);
        }
        Kind::Tapestry => {
            let mut dd = Draw::new(&[e.art.seed as i64, 5]);
            let (f1, f2, ph) = (dd.range(9.0, 15.0), dd.range(0.012, 0.03), dd.range(0.0, 6.0));
            let mut g = canvas(24, 32, move |x, y| (x * f1 + ph).sin() * f2 * (0.6 + 0.4 * (0.5 - y)));
            // The bottom stirs in the air (the engine's wind moves vertices by their sway).
            for (i, p) in g.pos.iter().enumerate() {
                let below = (0.5 - p[1]).clamp(0.0, 1.0);
                g.sway[i] = 0.05 * below * below;
            }
            m.add(g, canvas_transform(at, e.facing, e.size), canvas_look(&e.art, e.size.x / e.size.y, (0.0, 1.0), 0.8, 0.0), Anim::Still);
            // The rod and its finials.
            let r = e.right();
            let top = at + Vec3::Y * (e.size.y * 0.5 + 0.03) + e.facing * 0.02;
            tube_path(metal, &[top - r * (e.size.x * 0.5 + 0.18), top + r * (e.size.x * 0.5 + 0.18)], &[0.025, 0.025], 8, |_| [0.6, 0.45, 0.2]);
            for s in [-1.0, 1.0] {
                ball(metal, top + r * s * (e.size.x * 0.5 + 0.2), Vec3::splat(0.05), 1, [0.7, 0.52, 0.22]);
            }
        }
        Kind::Sculpture | Kind::Installation => {
            let Some(fam) = e.family else { return };
            if e.kind == Kind::Sculpture && e.plinth > 0.05 {
                plinth(matte, metal, at - Vec3::Y * e.plinth, e.size.x * plinth_share(fam), e.plinth, theme.trim, d);
            }
            if e.kind == Kind::Installation {
                // A rose on the ceiling where it hangs.
                lathe(metal, at - Vec3::Y * 0.12, &[(0.05, 0.0), (0.22, 0.08), (0.28, 0.12)], 16, |_| shade(theme.trim, 0.9));
            }
            let light = mix(e.art.palette[3], theme.light, 0.35);
            let parts = build_sculpture(fam, e.size.x, e.size.y, e.art.seed, light);
            let base = Mat4::from_translation(at) * Mat4::from_mat3(facing_basis(e.facing));
            let mut rig = Rig { base, parts: Vec::new() };
            for (i, mut p) in parts.into_iter().enumerate() {
                let geo = std::mem::take(&mut p.geo);
                let look = if p.painted { solid_look(e) } else { Look::glow(light, p.glow.max(0.3)) };
                m.add(geo, base, look, Anim::Part { exhibit: e.index, part: i as u16 });
                rig.parts.push(p);
            }
            m.rigs.push((e.index, rig));
            // Floor pieces are solid to walk into.
            if e.kind == Kind::Sculpture {
                let r = (e.size.x * plinth_share(fam) * 1.1 + 0.12).max(if fam == Family::Monolith || fam == Family::Wave { e.size.x * 1.3 } else { 0.0 });
                let top = e.plinth + e.size.y;
                m.collide(at - Vec3::new(r, e.plinth, r), at + Vec3::new(r, top - e.plinth, r));
            }
            // A small brass label on the plinth's front or the floor.
            let label = at - Vec3::Y * e.plinth + e.facing * (e.size.x * 1.2 + 0.08) + Vec3::Y * (e.plinth.max(0.4) * 0.7);
            if e.kind == Kind::Sculpture {
                let mm = facing_basis(e.facing);
                obox(metal, label, [mm.x_axis, mm.y_axis, mm.z_axis], Vec3::new(0.12, 0.05, 0.01), [0.75, 0.6, 0.3]);
            }
        }
    }
}

/// A brass picture light above a painting, with a glowing underside.
fn picture_light(metal: &mut Geo, glow: &mut Geo, at: Vec3, facing: Vec3, size: Vec2, style: FrameStyle, light: Rgb) {
    if size.x < 1.0 || style == FrameStyle::Floating {
        return;
    }
    let mm = facing_basis(facing);
    let (r, n) = (mm.x_axis, mm.z_axis);
    let top = at + Vec3::Y * (size.y * 0.5 + style.border(size) + 0.12);
    let w = (size.x * 0.32).clamp(0.35, 1.2);
    tube_path(metal, &[top - n * 0.03, top + n * 0.22 + Vec3::Y * 0.08], &[0.012, 0.012], 5, |_| [0.7, 0.55, 0.25]);
    let bar = top + n * 0.24 + Vec3::Y * 0.08;
    obox(metal, bar, [r, Vec3::Y, n], Vec3::new(w, 0.03, 0.05), [0.72, 0.56, 0.26]);
    obox(glow, bar - Vec3::Y * 0.031, [r, Vec3::Y, n], Vec3::new(w - 0.03, 0.002, 0.035), light);
}

/// How much of a piece's reach its plinth covers: an orbit's moons swing far beyond its stem, a cage stands on all of its base.
pub fn plinth_share(f: Family) -> f32 {
    match f {
        Family::Orbit | Family::Bloom => 0.5,
        Family::Knot | Family::Hand => 0.6,
        Family::Vessel | Family::Heart => 0.8,
        _ => 1.0,
    }
}

fn plinth(matte: &mut Geo, metal: &mut Geo, base: Vec3, r: f32, h: f32, trim: Rgb, d: &mut Draw) {
    let w = r * 1.1 + 0.1;
    let stone = shade([0.86, 0.84, 0.8], d.range(0.85, 1.05));
    if d.chance(0.5) {
        aabox(matte, base - Vec3::new(w, 0.0, w), base + Vec3::new(w, h - 0.045, w), stone);
        aabox(metal, base + Vec3::new(-w - 0.02, h - 0.045, -w - 0.02), base + Vec3::new(w + 0.02, h, w + 0.02), shade(trim, 0.8));
    } else {
        lathe(matte, base, &[(w * 1.1, 0.0), (w * 1.1, 0.08), (w * 0.92, 0.14), (w * 0.88, h - 0.1), (w * 1.05, h - 0.04), (w * 1.05, h)], 24, |_| stone);
    }
}

/// The doorway on a border: the portal wall with its opening, the trim, and the two door leaves (animated).
pub fn edge_model(e: EdgeId) -> Model {
    let mut m = Model { origin: e.centre(), ..Default::default() };
    let mut matte = Geo::default();
    let mut metal = Geo::default();
    let along = if e.east { Vec3::Z } else { Vec3::X };
    let thru = e.through();
    let stone = [0.62, 0.6, 0.56];
    let span = |u0: f32, u1: f32, y0: f32, y1: f32, t: f32| -> (Vec3, Vec3) {
        let a = along * u0 + Vec3::Y * y0 - thru * t;
        let b = along * u1 + Vec3::Y * y1 + thru * t;
        (a.min(b), a.max(b))
    };
    // The portal wall across the hall, with its opening.
    for (u0, u1) in [(-HALL_W * 0.5 - WALL, -DOOR_W * 0.5), (DOOR_W * 0.5, HALL_W * 0.5 + WALL)] {
        let (a, b) = span(u0, u1, 0.0, HALL_H, 0.25);
        aabox(&mut matte, a, b, stone);
        m.collide(a, b);
    }
    let (a, b) = span(-DOOR_W * 0.5, DOOR_W * 0.5, DOOR_H, HALL_H, 0.25);
    aabox(&mut matte, a, b, stone);
    // A carved surround on both faces.
    for s in [-1.0f32, 1.0] {
        for (u0, u1, y0, y1) in [
            (-DOOR_W * 0.5 - 0.25, -DOOR_W * 0.5, 0.0, DOOR_H + 0.25),
            (DOOR_W * 0.5, DOOR_W * 0.5 + 0.25, 0.0, DOOR_H + 0.25),
            (-DOOR_W * 0.5 - 0.25, DOOR_W * 0.5 + 0.25, DOOR_H, DOOR_H + 0.3),
        ] {
            let a = along * u0 + Vec3::Y * y0 + thru * s * 0.25;
            let b = along * u1 + Vec3::Y * y1 + thru * s * 0.31;
            aabox(&mut metal, a.min(b), a.max(b), [0.72, 0.6, 0.38]);
        }
        // A keystone.
        let k = along * 0.0 + Vec3::Y * (DOOR_H + 0.15) + thru * s * 0.33;
        obox(&mut metal, k, [along, Vec3::Y, thru], Vec3::new(0.16, 0.22, 0.03), [0.8, 0.66, 0.4]);
    }
    // Threshold.
    let (a, b) = span(-DOOR_W * 0.5, DOOR_W * 0.5, 0.0, 0.03, 0.26);
    aabox(&mut matte, a, b, [0.3, 0.28, 0.26]);
    m.add(matte, Mat4::IDENTITY, Look::MATTE, Anim::Still);
    m.add(metal, Mat4::IDENTITY, Look::METAL, Anim::Still);
    // The leaves: each hinged at its side of the opening, swinging toward the second room.
    for s in [-1.0f32, 1.0] {
        let hinge = along * s * DOOR_W * 0.5;
        let mut leaf = Geo::default();
        let w = DOOR_W * 0.5 - 0.02;
        let centre = hinge - along * s * (w * 0.5 + 0.01) + Vec3::Y * (DOOR_H - 0.02) * 0.5;
        let wood = [0.24, 0.13, 0.07];
        obox(&mut leaf, centre, [along, Vec3::Y, thru], Vec3::new(w * 0.5, (DOOR_H - 0.04) * 0.5, 0.045), wood);
        // Raised panels on both faces.
        for f in [-1.0f32, 1.0] {
            for (py, ph) in [(0.85, 0.65), (2.1, 1.1)] {
                obox(
                    &mut leaf,
                    hinge - along * s * (w * 0.5) + Vec3::Y * py + thru * f * 0.05,
                    [along, Vec3::Y, thru],
                    Vec3::new(w * 0.32, ph * 0.5, 0.012),
                    shade(wood, 1.3),
                );
            }
            // A brass handle near the meeting edge.
            let handle = hinge - along * s * (w - 0.12) + Vec3::Y * 1.05 + thru * f * 0.075;
            obox(&mut leaf, handle, [along, Vec3::Y, thru], Vec3::new(0.015, 0.14, 0.02), [0.85, 0.66, 0.3]);
        }
        let swing = s * -1.65 * if e.east { -1.0 } else { 1.0 };
        m.add(leaf, Mat4::IDENTITY, Look { roughness: 0.6, ..Look::MATTE }, Anim::Leaf { hinge, swing });
    }
    m
}

/// The collider that fills a closed doorway (world space).
pub fn door_collider(e: EdgeId) -> Collider2D {
    let c = e.centre();
    let (hx, hz) = if e.east { (0.12, DOOR_W * 0.5) } else { (DOOR_W * 0.5, 0.12) };
    Collider2D { min: Vec2::new(c.x - hx, c.z - hz), max: Vec2::new(c.x + hx, c.z + hz), min_y: 0.0, max_y: DOOR_H }
}

/// The world inside a painting (its own space: the centre at the origin).
pub fn pocket_model(c: &Collection, p: &Pocket, origin: Vec3) -> Model {
    let mut m = Model { origin, ..Default::default() };
    let theme = &c.themes[p.theme.min(c.themes.len() - 1)];
    let mut matte = Geo::default();
    let mut metal = Geo::default();
    let glow = Geo::default();
    let mut d = Draw::new(&[p.art.seed as i64, p.path.depth() as i64, 808]);
    let r = p.radius;
    // ---- Ground ----
    match p.ground {
        Ground::Mirror => {
            // Still water: the game's surface shader mirrors the painting's world in it (`surface.x` 3).
            let disc = disc_geo(r * 3.0, 64, [1.0, 1.0, 1.0]);
            let mut look = canvas_look(&p.art, 1.0, (0.0, 1.0), 1.0, 0.0);
            look.surface[0] = 3.0;
            look.roughness = 0.08;
            m.add(disc, Mat4::IDENTITY, look, Anim::Still);
        }
        Ground::Painted => {
            let g = canvas(48, 48, |_, _| 0.0);
            let flat = Mat4::from_cols(Vec3::X.extend(0.0) * r * 3.2, (-Vec3::Z * r * 3.2).extend(0.0), Vec3::Y.extend(0.0), Vec3::ZERO.extend(1.0));
            let mut look = canvas_look(&p.art, 1.0, (0.0, 1.0), 0.45, 0.0);
            look.roughness = 0.5;
            m.add(g, flat, look, Anim::Still);
        }
        Ground::Dune => {
            let mut g = Geo::default();
            let n = 40;
            let size = r * 3.0;
            let hgt = |x: f32, z: f32| 0.25 * ((x * 0.31).sin() * (z * 0.23).cos()) + 0.12 * ((x * 0.7 + z * 0.4).sin()) - 0.1;
            let col = mix(p.art.palette[2], [0.86, 0.76, 0.6], 0.6);
            for j in 0..=n {
                for i in 0..=n {
                    let (x, z) = (-size * 0.5 + size * i as f32 / n as f32, -size * 0.5 + size * j as f32 / n as f32);
                    let e = 0.2;
                    let nrm = Vec3::new(hgt(x - e, z) - hgt(x + e, z), 2.0 * e, hgt(x, z - e) - hgt(x, z + e)).normalize();
                    // Flat where people walk.
                    let flatten = ((Vec2::new(x, z).length() - r * 0.9) / 3.0).clamp(0.0, 1.0);
                    g.vert(Vec3::new(x, hgt(x, z) * flatten.max(0.15) - 0.02, z), nrm, shade(col, d.range(0.92, 1.05)));
                }
            }
            for j in 0..n {
                for i in 0..n {
                    let a = j * (n + 1) + i;
                    let b = a + n + 1;
                    g.idx.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
                }
            }
            m.add(g, Mat4::IDENTITY, Look::MATTE, Anim::Still);
        }
        Ground::Stones => {
            let mut g = Geo::default();
            let count = 90;
            for _ in 0..count {
                let a = d.range(0.0, std::f32::consts::TAU);
                let rr = r * d.range(0.0, 1.1).sqrt();
                let at = Vec3::new(a.cos() * rr, -0.18, a.sin() * rr);
                let s = d.range(0.6, 1.6);
                g.blob(at, Vec3::new(s, 0.18, s * d.range(0.7, 1.0)), 1, d.below(999) as u32, 0.08, shade(p.art.palette[1], 0.5), shade(p.art.palette[2], 0.9));
            }
            // A faint glowing mist plane far below, where the stones float.
            let mist = disc_geo(r * 3.0, 48, p.art.palette[1]);
            m.add(mist, Mat4::from_translation(-Vec3::Y * 6.0), Look::glow(p.art.palette[1], 0.6), Anim::Still);
            m.add(g, Mat4::IDENTITY, Look::MATTE, Anim::Still);
        }
    }
    // ---- The way back: a frame of light behind the arrival point ----
    {
        let size = Vec2::new(1.6, 2.4);
        let at = p.exit + Vec3::Y * 1.3;
        frame(&mut matte, &mut metal, at, -Vec3::Z, size, FrameStyle::Gilded, 7);
        let mut veil = unit_canvas();
        for c in veil.col.iter_mut() {
            *c = [0.95, 0.92, 0.85];
        }
        m.add(veil, canvas_transform(at, -Vec3::Z, size), Look::glow([1.0, 0.94, 0.82], 1.8), Anim::Still);
        tube_path(&mut metal, &[p.exit + Vec3::new(-0.6, 0.0, 0.3), at - Vec3::Y * 1.2], &[0.03, 0.03], 6, |_| [0.6, 0.45, 0.2]);
        tube_path(&mut metal, &[p.exit + Vec3::new(0.6, 0.0, 0.3), at - Vec3::Y * 1.2], &[0.03, 0.03], 6, |_| [0.6, 0.45, 0.2]);
    }
    // ---- The paintings in this world ----
    for f in &p.frames {
        nested_model(&mut m, &mut matte, &mut metal, f);
    }
    // ---- The key ----
    if let Some(k) = p.key {
        let g = key_geo();
        m.add(g, Mat4::from_translation(k), Look { emissive: Vec3::new(0.9, 0.62, 0.18) * 0.6, ..Look::METAL }, Anim::Key);
        // A pedestal of light under it.
        let mut beam = Geo::default();
        lathe(&mut beam, Vec3::new(k.x, 0.0, k.z), &[(0.45, 0.0), (0.4, 0.05), (0.25, 0.1)], 24, |_| [1.0, 0.85, 0.5]);
        m.add(beam, Mat4::IDENTITY, Look::glow([1.0, 0.8, 0.45], 1.4), Anim::Still);
        m.lamps.push(Lamp { at: origin + k + Vec3::Y * 0.8, color: [1.0, 0.82, 0.5], intensity: 16.0, range: 9.0 });
    }
    m.lamps.push(Lamp { at: origin + Vec3::Y * 9.0, color: mix(p.art.palette[3], theme.light, 0.5), intensity: 30.0, range: r * 2.2 });
    // ---- An invisible ring at the edge (boxes that overlap, so there is no gap to slip through) ----
    let n = 56;
    for i in 0..n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        let c = Vec3::new(a.cos(), 0.0, a.sin()) * (r + 0.6);
        m.collide(c - Vec3::new(1.2, 0.0, 1.2), c + Vec3::new(1.2, 3.0, 1.2));
    }
    m.add(matte, Mat4::IDENTITY, Look::MATTE, Anim::Still);
    m.add(metal, Mat4::IDENTITY, Look::METAL, Anim::Still);
    m.add(glow, Mat4::IDENTITY, Look::glow(theme.light, 1.5), Anim::Still);
    m
}

fn nested_model(m: &mut Model, matte: &mut Geo, metal: &mut Geo, f: &Nested) {
    let mut fm = Geo::default();
    let mut fmetal = Geo::default();
    // The frame and canvas are built about the origin, so a floating one can drift as a whole.
    frame(&mut fm, &mut fmetal, Vec3::ZERO, f.facing, f.size, f.frame, f.index as u32 + 11);
    let place = Mat4::from_translation(f.at);
    let anim = if f.stand == Stand::Floating { Anim::Drift { phase: f.index as f32 * 1.7 } } else { Anim::Still };
    m.add(fm, place, Look::MATTE, anim);
    let shimmer = if f.on_route { Look { emissive: Vec3::new(0.5, 0.36, 0.1), ..Look::METAL } } else { Look::METAL };
    m.add(fmetal, place, shimmer, anim);
    m.add(
        unit_canvas(),
        place * canvas_transform(Vec3::ZERO, f.facing, f.size),
        canvas_look(&f.art, f.size.x / f.size.y, (0.0, 1.0), 1.0, if f.on_route { 1.0 } else { 0.0 }),
        anim,
    );
    let base = Vec3::new(f.at.x, 0.0, f.at.z);
    let mm = facing_basis(f.facing);
    match f.stand {
        Stand::Easel => {
            let bottom = f.at.y - f.size.y * 0.5;
            let wood = [0.42, 0.28, 0.15];
            for s in [-1.0f32, 1.0] {
                let foot = base + mm.x_axis * s * (f.size.x * 0.35) + mm.z_axis * 0.25;
                tube_path(matte, &[foot, base + Vec3::Y * (f.at.y + f.size.y * 0.55) - mm.z_axis * 0.08], &[0.03, 0.025], 5, |_| wood);
            }
            tube_path(matte, &[base - mm.z_axis * 0.9, base + Vec3::Y * (f.at.y + f.size.y * 0.5) - mm.z_axis * 0.1], &[0.03, 0.025], 5, |_| wood);
            obox(matte, base + Vec3::Y * (bottom - 0.03) + mm.z_axis * 0.06, [mm.x_axis, Vec3::Y, mm.z_axis], Vec3::new(f.size.x * 0.45, 0.025, 0.06), wood);
            m.collide(base - Vec3::new(0.6, 0.0, 0.6), base + Vec3::new(0.6, 2.0, 0.6));
        }
        Stand::Pillar => {
            let top = f.at.y - f.size.y * 0.5 - 0.05;
            lathe(matte, base, &[(0.32, 0.0), (0.3, 0.1), (0.2, 0.16), (0.18, top - 0.12), (0.26, top - 0.05), (0.26, top)], 20, |_| [0.85, 0.83, 0.8]);
            obox(metal, base + Vec3::Y * (top + 0.02), [mm.x_axis, Vec3::Y, mm.z_axis], Vec3::new(f.size.x * 0.4, 0.02, 0.08), [0.7, 0.55, 0.3]);
            m.collide(base - Vec3::new(0.35, 0.0, 0.35), base + Vec3::new(0.35, top, 0.35));
        }
        Stand::Floating => {}
    }
}

fn disc_geo(r: f32, n: u32, c: Rgb) -> Geo {
    let mut g = Geo::default();
    let centre = g.vert(Vec3::ZERO, Vec3::Y, c);
    let rim: Vec<u32> = (0..n)
        .map(|k| {
            let a = k as f32 / n as f32 * std::f32::consts::TAU;
            g.vert(Vec3::new(a.cos() * r, 0.0, a.sin() * r), Vec3::Y, shade(c, 0.6))
        })
        .collect();
    for k in 0..n as usize {
        g.idx.extend_from_slice(&[centre, rim[(k + 1) % n as usize], rim[k]]);
    }
    g
}

/// An old key: a ring bow with a quatrefoil, a shaft and a bit with wards. About 40 cm long, centred on its middle.
pub fn key_geo() -> Geo {
    let mut g = Geo::default();
    let gold = [0.95, 0.72, 0.3];
    let bow: Vec<Vec3> = (0..=40)
        .map(|k| {
            let a = k as f32 / 40.0 * std::f32::consts::TAU;
            Vec3::new(a.cos() * 0.075, 0.14 + a.sin() * 0.075, 0.0)
        })
        .collect();
    tube_path(&mut g, &bow, &vec![0.017; bow.len()], 8, |_| gold);
    for k in 0..4 {
        let a = k as f32 * std::f32::consts::FRAC_PI_2 + std::f32::consts::FRAC_PI_4;
        ball(&mut g, Vec3::new(a.cos() * 0.035, 0.14 + a.sin() * 0.035, 0.0), Vec3::new(0.025, 0.025, 0.012), 1, gold);
    }
    tube_path(&mut g, &[Vec3::new(0.0, 0.065, 0.0), Vec3::new(0.0, -0.2, 0.0)], &[0.019, 0.016], 10, |_| gold);
    ball(&mut g, Vec3::new(0.0, 0.055, 0.0), Vec3::new(0.03, 0.02, 0.03), 1, gold);
    obox(&mut g, Vec3::new(0.035, -0.165, 0.0), [Vec3::X, Vec3::Y, Vec3::Z], Vec3::new(0.035, 0.03, 0.01), gold);
    obox(&mut g, Vec3::new(0.06, -0.12, 0.0), [Vec3::X, Vec3::Y, Vec3::Z], Vec3::new(0.012, 0.02, 0.01), gold);
    g
}

/// Every style of the collection on one dark wall, in rows of `cols`, each in its theme's palette (for `red_engine2 museum --styles`): the canvases face +z
/// and the wall's middle is at the origin, 1.6 m up.
pub fn style_sheet(c: &Collection, cols: usize, seed: f32) -> Model {
    let mut m = Model::default();
    let mut matte = Geo::default();
    let mut metal = Geo::default();
    let n = c.styles.len();
    let rows = n.div_ceil(cols);
    let (w, h, gap) = (1.6, 1.1, 0.35);
    let width = cols as f32 * (w + gap);
    let height = rows as f32 * (h + gap);
    aabox(
        &mut matte,
        Vec3::new(-width * 0.5 - 1.0, 1.6 - height * 0.5 - 1.0, -0.2),
        Vec3::new(width * 0.5 + 1.0, 1.6 + height * 0.5 + 1.0, -0.1),
        [0.06, 0.06, 0.07],
    );
    for i in 0..n {
        let (cx, cy) = ((i % cols) as f32, (i / cols) as f32);
        let at = Vec3::new(-width * 0.5 + (cx + 0.5) * (w + gap), 1.6 + height * 0.5 - (cy + 0.5) * (h + gap), 0.0);
        let theme = c.themes.iter().find(|t| t.styles.contains(&i)).unwrap_or(&c.themes[0]);
        let art = super::exhibit::Art { style: i as u32 + 1, seed: seed + i as f32 * 7.0, palette: theme.palettes[0] };
        frame(&mut matte, &mut metal, at, Vec3::Z, Vec2::new(w, h), FrameStyle::Ebony, i as u32);
        m.add(unit_canvas(), canvas_transform(at, Vec3::Z, Vec2::new(w, h)), canvas_look(&art, w / h, (0.0, 1.0), 1.0, 0.0), Anim::Still);
    }
    m.lamps.push(Lamp { at: Vec3::new(0.0, 4.0, 6.0), color: [1.0, 0.96, 0.9], intensity: 40.0, range: 30.0 });
    m.add(matte, Mat4::IDENTITY, Look::MATTE, Anim::Still);
    m.add(metal, Mat4::IDENTITY, Look::METAL, Anim::Still);
    m
}

/// How far a door has to be open before one can walk through.
pub const DOOR_PASSABLE: f32 = 0.55;

#[cfg(test)]
mod tests {
    use super::super::exhibit::{furnish, pocket, Homes, PocketPath};
    use super::super::layout::{Layout, RoomId};
    use super::*;

    fn collection() -> Collection {
        let v: serde_json::Value = serde_json::from_str(include_str!("../../examples/frameless/collection.json")).unwrap();
        super::super::spec::parse_collection(&v, None).expect("the fixture collection parses")
    }

    #[test]
    fn rooms_build_within_a_triangle_budget_and_their_doorways_are_open() {
        let c = collection();
        let l = Layout::new(21, c.themes.len(), None);
        let homes = Homes::place(&l, &c);
        let mut worst = 0;
        for x in -3..3 {
            for z in -3..3 {
                let room = l.room(RoomId::new(x, z));
                let k = furnish(&l, &c, &homes, &room);
                let m = room_model(&c, &k);
                worst = worst.max(m.tris());
                // No collider stands in a doorway, its hall, or the first metre and a half of the room inside it.
                for s in Side::ALL.into_iter().filter(|s| room.has_door(*s)) {
                    let wall = room.wall_length(s.opposite()) * 0.0 + if s.runs_along_x() { room.size.y } else { room.size.x } * 0.5;
                    for depth in [wall - 1.5, wall - 0.5, wall + 0.5, CELL * 0.5 - 0.6] {
                        let p = room.id.centre() + s.out() * depth;
                        let blocked = m
                            .colliders
                            .iter()
                            .any(|c| p.x > c.min.x - 0.3 && p.x < c.max.x + 0.3 && p.z > c.min.y - 0.3 && p.z < c.max.y + 0.3 && c.min_y < 1.5);
                        assert!(!blocked, "({x},{z}) {s:?} doorway blocked {depth} m out");
                    }
                }
                assert!(m.lamps.len() <= 5);
                for (e, rig) in &m.rigs {
                    assert!(rig.parts.iter().all(|p| p.geo.idx.is_empty()), "exhibit {e}: rig parts keep no meshes");
                }
            }
        }
        assert!(worst < 220_000, "the heaviest room has {worst} triangles");
    }

    #[test]
    fn a_pocket_builds_with_its_key_and_its_way_back() {
        let c = collection();
        let l = Layout::new(4, c.themes.len(), None);
        let homes = Homes::place(&l, &c);
        let k = furnish(&l, &c, &homes, &l.room(RoomId::ORIGIN));
        let p = pocket(&l, &c, &k, &PocketPath { room: RoomId::ORIGIN, exhibit: k.key.exhibit, nest: vec![] }).unwrap();
        let m = pocket_model(&c, &p, Vec3::ZERO);
        assert!(m.pieces.iter().any(|p| p.anim == Anim::Key));
        assert!(m.tris() > 100);
        let e = edge_model(EdgeId::of(RoomId::ORIGIN, Side::East));
        assert_eq!(e.pieces.iter().filter(|p| matches!(p.anim, Anim::Leaf { .. })).count(), 2);
    }
}
