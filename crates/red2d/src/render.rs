//! The 2D renderer: a CPU rasteriser that draws the virtual screen into an RGBA buffer.
//!
//! The same code makes the pictures in native tests, in `frame` (PNG) and in the browser (the buffer goes to a canvas), so a screenshot taken headless is what the player sees, to the pixel.
//! There is no GPU, no window and no floating-point image resampling: sprites are drawn at integer scales, circles by pixel-centre distance, text from a bitmap font. Scaling the virtual screen
//! to a window of another shape is [`layout`] and [`present`] (nearest neighbour, letterboxed), and [`window_to_view`] maps a pointer back, so resolution independence has one definition.

use crate::font;
use crate::game::*;
use crate::sim::Sim;
use crate::{game3d, raster3d};

/// An RGBA picture.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// Pixels wide.
    pub w: u32,
    /// Pixels high.
    pub h: u32,
    /// `w * h * 4` bytes, row-major.
    pub rgba: Vec<u8>,
}

/// What a picture contains, for "is it blank?" checks.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameStats {
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
    /// Different colours present.
    pub distinct_colors: usize,
    /// Fraction of pixels that are not the background colour.
    pub covered: f32,
    /// A hash of the pixels, 16 hex digits.
    pub hash: String,
}

impl Frame {
    /// A picture filled with `bg`.
    pub fn new(w: u32, h: u32, bg: Color) -> Frame {
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..w * h {
            rgba.extend_from_slice(&bg);
        }
        Frame { w, h, rgba }
    }

    /// Draws one pixel with alpha blending; outside the picture is ignored.
    pub fn blend(&mut self, x: i32, y: i32, c: Color) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 || c[3] == 0 {
            return;
        }
        let i = ((y as u32 * self.w + x as u32) * 4) as usize;
        if c[3] == 255 {
            self.rgba[i..i + 4].copy_from_slice(&c);
            return;
        }
        let a = c[3] as u32;
        for (k, &channel) in c.iter().take(3).enumerate() {
            self.rgba[i + k] = ((channel as u32 * a + self.rgba[i + k] as u32 * (255 - a) + 127) / 255) as u8;
        }
        self.rgba[i + 3] = 255;
    }

    /// A filled rectangle.
    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Color) {
        for yy in y.max(0)..(y + h).min(self.h as i32) {
            for xx in x.max(0)..(x + w).min(self.w as i32) {
                self.blend(xx, yy, c);
            }
        }
    }

    /// A filled disc of diameter `d` whose bounding box has its top-left at `(x, y)`.
    pub fn disc(&mut self, x: i32, y: i32, d: i32, c: Color) {
        let r = d as f32 * 0.5;
        let (cx, cy) = (x as f32 + r, y as f32 + r);
        for yy in y..y + d {
            for xx in x..x + d {
                let (dx, dy) = (xx as f32 + 0.5 - cx, yy as f32 + 0.5 - cy);
                if dx * dx + dy * dy <= r * r {
                    self.blend(xx, yy, c);
                }
            }
        }
    }

    /// Text with its top-left at `(x, y)`.
    pub fn text(&mut self, x: i32, y: i32, text: &str, scale: i32, c: Color) {
        let mut px = Vec::new();
        font::for_each_pixel(text, x, y, scale, |a, b| px.push((a, b)));
        for (a, b) in px {
            self.blend(a, b, c);
        }
    }

    /// Counts what is in the picture.
    pub fn stats(&self, bg: Color) -> FrameStats {
        let mut seen = std::collections::BTreeSet::new();
        let mut covered = 0usize;
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for px in self.rgba.as_chunks::<4>().0 {
            seen.insert([px[0], px[1], px[2]]);
            if px[..3] != bg[..3] {
                covered += 1;
            }
            for b in px {
                h = (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3);
            }
        }
        FrameStats {
            width: self.w,
            height: self.h,
            distinct_colors: seen.len(),
            covered: covered as f32 / (self.w * self.h).max(1) as f32,
            hash: format!("{h:016x}"),
        }
    }
}

fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Replaces `{var}`, `{var:3}` (zero-padded to 3 characters) and `{var:.1}` (one decimal) with the variables' values.
pub fn fill_template(text: &str, sim: &Sim) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let spec = &after[..close];
        let (name, fmt) = spec.split_once(':').unwrap_or((spec, ""));
        let (width, decimals) = match fmt.strip_prefix('.') {
            Some(d) => (0, d.parse::<usize>().ok()),
            None => (fmt.parse::<usize>().unwrap_or(0), None),
        };
        let v = sim.var(name).map_or_else(
            || "?".to_string(),
            |x| match decimals {
                Some(d) => format!("{x:.d$}"),
                None => fmt_num(x),
            },
        );
        if width > 0 {
            out.push_str(&format!("{v:0>width$}"));
        } else {
            out.push_str(&v);
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

fn lighten(c: Color, by: u8) -> Color {
    [c[0].saturating_add(by), c[1].saturating_add(by), c[2].saturating_add(by), c[3]]
}

/// What is drawn in the flat world, in order.
enum Draw {
    Entity(usize),
    Layer(usize),
}

fn sprite_frame(sim: &Sim, sp: &Sprite) -> usize {
    if sp.frames.len() > 1 {
        ((sim.tick as f32 * sp.fps / 60.0) as usize) % sp.frames.len()
    } else {
        0
    }
}

/// Puts a 3D picture on the screen at `(x0, y0)`; pixels nothing was drawn on stay as they were.
fn composite(f: &mut Frame, t: &raster3d::Target, x0: i32, y0: i32) {
    for y in 0..t.h as i32 {
        for x in 0..t.w as i32 {
            let i = ((y as u32 * t.w + x as u32) * 4) as usize;
            f.blend(x0 + x, y0 + y, [t.rgba[i], t.rgba[i + 1], t.rgba[i + 2], t.rgba[i + 3]]);
        }
    }
}

fn items_of<'a>(def: &'a GameDef, vars: &[f64], list: &[game3d::ModelItem]) -> Vec<raster3d::Item<'a>> {
    list.iter()
        .map(|m| raster3d::Item {
            mesh: &def.models[m.model].mesh,
            pos: m.at,
            rot: [m.yaw.eval(vars) as f32, m.pitch.eval(vars) as f32, m.roll.eval(vars) as f32],
            scale: m.scale,
            tint: m.tint.map(|c| [c[0], c[1], c[2]]),
        })
        .collect()
}

/// A small 3D scene in a rectangle of the screen.
fn draw_view3d(f: &mut Frame, sim: &Sim, v: &game3d::View3d) {
    let (w, h) = (v.size[0].round().max(1.0) as u32, v.size[1].round().max(1.0) as u32);
    let mut t = raster3d::Target::new(w, h);
    t.clear(v.background);
    let cam = raster3d::Camera { eye: v.eye, target: v.target, up: [0.0, 1.0, 0.0], fov: v.fov, ortho: v.ortho };
    raster3d::render(&mut t, &cam, &v.light.light(), &items_of(&sim.def, &sim.vars, &v.items), &[]);
    composite(f, &t, v.at[0].round() as i32, v.at[1].round() as i32);
}

/// A prefab drawn as a 3D model inside its box, as seen from a fixed camera that looks down by `elevation` degrees.
fn draw_model_entity(f: &mut Frame, sim: &Sim, ms: &game3d::ModelShape, size: [f32; 2], c: [f32; 2]) {
    let m = &sim.def.models[ms.model];
    let (w, h) = (size[0].round().max(1.0) as u32, size[1].round().max(1.0) as u32);
    let reach = (m.radius * ms.scale).max(0.001);
    let el = ms.elevation.to_radians();
    let d = reach * 4.0;
    let cam = raster3d::Camera {
        eye: [0.0, libm::sinf(el) * d, libm::cosf(el) * d],
        target: [0.0; 3],
        up: [0.0, 1.0, 0.0],
        fov: 30.0,
        ortho: Some(reach * h as f32 / w.min(h) as f32),
    };
    let item = raster3d::Item {
        mesh: &m.mesh,
        pos: [0.0; 3],
        rot: [ms.yaw.eval(&sim.vars) as f32, ms.pitch.eval(&sim.vars) as f32, ms.roll.eval(&sim.vars) as f32],
        scale: ms.scale,
        tint: ms.tint.map(|c| [c[0], c[1], c[2]]),
    };
    let mut t = raster3d::Target::new(w, h);
    raster3d::render(&mut t, &cam, &ms.light.light(), &[item], &[]);
    composite(f, &t, (c[0] - w as f32 * 0.5).round() as i32, (c[1] - h as f32 * 0.5).round() as i32);
}

fn average_color(px: &[Color]) -> [u8; 3] {
    let (mut sum, mut n) = ([0u32; 3], 0u32);
    for p in px.iter().filter(|p| p[3] > 0) {
        for k in 0..3 {
            sum[k] += p[k] as u32;
        }
        n += 1;
    }
    let n = n.max(1);
    [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8]
}

fn disc_pixels(color: Color) -> Vec<Color> {
    const N: usize = 16;
    (0..N * N)
        .map(|k| {
            let (dx, dy) = ((k % N) as f32 + 0.5 - N as f32 * 0.5, (k / N) as f32 + 0.5 - N as f32 * 0.5);
            if dx * dx + dy * dy <= (N * N) as f32 * 0.25 {
                color
            } else {
                [0; 4]
            }
        })
        .collect()
}

/// What an entity is in the 3D world.
enum Body3 {
    Mesh { mesh: raster3d::Mesh, pos: raster3d::V3 },
    Model { model: usize, pos: raster3d::V3, rot: raster3d::V3, scale: f32, tint: Option<[u8; 3]> },
    Board { pos: raster3d::V3, size: [f32; 2], pixels: Vec<Color>, pw: usize, ph: usize },
}

fn body_of(sim: &Sim, w: &game3d::World3d, e: &crate::sim::Entity, p: &Prefab) -> Option<Body3> {
    let def = &*sim.def;
    let [sx, sy] = p.size;
    let (x, y) = (e.x, e.y);
    // A thing with `height3d` is a box on its footprint (on a wall the number is how thick it is).
    let boxed = |color: [u8; 3], h: f32| {
        let (dims, pos) = match w.plane {
            game3d::Plane::Ground => ([sx, h, sy], w.plane.to3(x, y, h * 0.5)),
            game3d::Plane::Wall => ([sx, sy, h], w.plane.to3(x, y, 0.0)),
        };
        Body3::Mesh { mesh: raster3d::Mesh { tris: raster3d::build(&raster3d::Shape::Box(dims), color) }, pos }
    };
    // A flat thing stands up facing the camera; its bottom edge is on the ground (on a wall, `y` is its middle).
    let board = |pixels: Vec<Color>, pw: usize, ph: usize| {
        let pos = match w.plane {
            game3d::Plane::Ground => w.plane.to3(x, y, 0.0),
            game3d::Plane::Wall => w.plane.to3(x, y + sy * 0.5, 0.0),
        };
        Body3::Board { pos, size: [sx, sy], pixels, pw, ph }
    };
    match &p.shape {
        Shape::None | Shape::Text { .. } => None,
        Shape::Rect { color } => Some(match p.height3d {
            Some(h) => boxed([color[0], color[1], color[2]], h),
            None => board(vec![*color], 1, 1),
        }),
        Shape::Circle { color } => Some(match p.height3d {
            Some(h) => boxed([color[0], color[1], color[2]], h),
            None => board(disc_pixels(*color), 16, 16),
        }),
        Shape::Sprite { sprite, flip, .. } => {
            let sp = &def.sprites[*sprite];
            let px = &sp.frames[sprite_frame(sim, sp)];
            Some(match p.height3d {
                Some(h) => boxed(average_color(px), h),
                None => {
                    let mirror = *flip == Flip::X || (*flip == Flip::Auto && e.face_left);
                    let pixels = if mirror { (0..px.len()).map(|k| px[(k / sp.w) * sp.w + (sp.w - 1 - k % sp.w)]).collect() } else { px.clone() };
                    board(pixels, sp.w, sp.h)
                }
            })
        }
        Shape::Model(ms) => {
            let m = &def.models[ms.model];
            let s = (sx.min(sy) * 0.5 / m.radius.max(0.001)) * ms.scale;
            let lift = m.radius * s;
            Some(Body3::Model {
                model: ms.model,
                pos: match w.plane {
                    game3d::Plane::Ground => w.plane.to3(x, y, lift),
                    game3d::Plane::Wall => w.plane.to3(x, y, 0.0),
                },
                rot: [ms.yaw.eval(&sim.vars) as f32, ms.pitch.eval(&sim.vars) as f32, ms.roll.eval(&sim.vars) as f32],
                scale: s,
                tint: ms.tint.map(|c| [c[0], c[1], c[2]]),
            })
        }
    }
}

/// The whole world in perspective: the floor, then every thing as a box, a model or a camera-facing picture, then particles and text projected on top.
fn draw_world3d(f: &mut Frame, sim: &Sim, w: &game3d::World3d, off: [f32; 2]) {
    let def = &*sim.def;
    let (vw, vh) = (def.view.width, def.view.height);
    let center = [off[0] + vw as f32 * 0.5, off[1] + vh as f32 * 0.5];
    let cam = game3d::world_camera(w, &sim.vars, center);
    let mut t = raster3d::Target::new(vw, vh);
    t.clear(w.sky);
    if let Some(g) = &w.ground {
        let (origin, n) = w.plane.surface();
        let world = def.view.world;
        raster3d::paint_plane(&mut t, &cam, origin, n, |p| {
            let [x, y] = w.plane.from3(p);
            if x < 0.0 || y < 0.0 || x > world.0 || y > world.1 {
                return None;
            }
            let odd = (libm::floorf(x / g.tile) as i64 + libm::floorf(y / g.tile) as i64).rem_euclid(2) == 1;
            Some(if odd { g.alt.unwrap_or(g.color) } else { g.color })
        });
    }
    let bodies: Vec<Body3> =
        sim.entities.iter().filter(|e| e.alive && !def.prefabs[e.prefab].hidden).filter_map(|e| body_of(sim, w, e, &def.prefabs[e.prefab])).collect();
    let mut items = Vec::new();
    let mut boards = Vec::new();
    for b in &bodies {
        match b {
            Body3::Mesh { mesh, pos } => items.push(raster3d::Item { mesh, pos: *pos, rot: [0.0; 3], scale: 1.0, tint: None }),
            Body3::Model { model, pos, rot, scale, tint } => {
                items.push(raster3d::Item { mesh: &def.models[*model].mesh, pos: *pos, rot: *rot, scale: *scale, tint: *tint })
            }
            Body3::Board { pos, size, pixels, pw, ph } => boards.push(raster3d::Billboard { pos: *pos, size: *size, pixels, pw: *pw, ph: *ph }),
        }
    }
    raster3d::render(&mut t, &cam, &w.light.light(), &items, &boards);
    composite(f, &t, 0, 0);
    let flat = |x: f32, y: f32, lift: f32| raster3d::project(&cam, vw, vh, w.plane.to3(x, y, lift));
    for p in &sim.particles {
        let Some((s, z)) = flat(p.x, p.y, 0.0) else { continue };
        let mut c = p.color;
        c[3] = (c[3] as f32 * (p.life / p.max_life).clamp(0.0, 1.0)) as u8;
        let k = (w.distance / z.max(1.0)).clamp(0.2, 4.0);
        let side = (p.size.max(1.0) * k).round().max(1.0) as i32;
        f.rect(s[0].round() as i32 - side / 2, s[1].round() as i32 - side / 2, side, side, c);
    }
    for e in sim.entities.iter().filter(|e| e.alive) {
        let p = &def.prefabs[e.prefab];
        if let (Shape::Text { text, color, scale }, false) = (&p.shape, p.hidden) {
            let Some((s, _)) = flat(e.x, e.y, 0.0) else { continue };
            let t = fill_template(text, sim);
            let tw = font::text_width(&t, *scale as i32);
            f.text(s[0].round() as i32 - tw / 2, s[1].round() as i32 - (font::GLYPH_H * *scale as i32) / 2, &t, *scale as i32, *color);
        }
    }
}

/// A flat map of the world.
fn draw_minimap(f: &mut Frame, sim: &Sim, at: [f32; 2], size: [f32; 2], colors: &[(String, Color)], back: Color, border: Color, dot: f32, viewport: bool) {
    let def = &*sim.def;
    let (x0, y0, w, h) = (at[0] as i32, at[1] as i32, size[0] as i32, size[1] as i32);
    f.rect(x0, y0, w, h, back);
    let (ww, wh) = (def.view.world.0.max(1.0), def.view.world.1.max(1.0));
    let to = |x: f32, y: f32| [x0 as f32 + (x / ww).clamp(0.0, 1.0) * size[0], y0 as f32 + (y / wh).clamp(0.0, 1.0) * size[1]];
    let d = dot.round().max(1.0) as i32;
    for e in sim.entities.iter().filter(|e| e.alive) {
        let p = &def.prefabs[e.prefab];
        if let Some((_, c)) = colors.iter().find(|(tag, _)| p.tags.iter().any(|t| t == tag)) {
            let s = to(e.x, e.y);
            f.rect(s[0].round() as i32 - d / 2, s[1].round() as i32 - d / 2, d, d, *c);
        }
    }
    if viewport {
        let (a, b) = (to(sim.cam[0], sim.cam[1]), to(sim.cam[0] + def.view.width as f32, sim.cam[1] + def.view.height as f32));
        let (rx, ry, rw, rh) = (a[0].round() as i32, a[1].round() as i32, (b[0] - a[0]).round() as i32, (b[1] - a[1]).round() as i32);
        let c = [255, 255, 255, 200];
        f.rect(rx, ry, rw, 1, c);
        f.rect(rx, ry + rh - 1, rw, 1, c);
        f.rect(rx, ry, 1, rh, c);
        f.rect(rx + rw - 1, ry, 1, rh, c);
    }
    f.rect(x0, y0, w, 1, border);
    f.rect(x0, y0 + h - 1, w, 1, border);
    f.rect(x0, y0, 1, h, border);
    f.rect(x0 + w - 1, y0, 1, h, border);
}

/// Draws the current state of the game at its virtual resolution.
pub fn render(sim: &Sim) -> Frame {
    let def = &*sim.def;
    let mut f = Frame::new(def.view.width, def.view.height, def.view.background);
    let off = [sim.cam[0] - sim.shake_off[0], sim.cam[1] - sim.shake_off[1]];
    if let Some(w) = &def.world3d {
        draw_world3d(&mut f, sim, w, off);
    } else {
        let mut order: Vec<(i32, u8, u32, Draw)> = (0..sim.entities.len())
            .filter(|&i| sim.entities[i].alive && !def.prefabs[sim.entities[i].prefab].hidden)
            .map(|i| (def.prefabs[sim.entities[i].prefab].layer, 1, sim.entities[i].id, Draw::Entity(i)))
            .collect();
        order.extend(def.layers3d.iter().enumerate().map(|(k, l)| (l.layer, 0, k as u32, Draw::Layer(k))));
        order.sort_by_key(|o| (o.0, o.1, o.2));
        for (_, _, _, d) in order {
            match d {
                Draw::Layer(k) => draw_view3d(&mut f, sim, &def.layers3d[k].view),
                Draw::Entity(i) => draw_entity(&mut f, sim, &sim.entities[i], off),
            }
        }
        for p in &sim.particles {
            let mut c = p.color;
            let k = (p.life / p.max_life).clamp(0.0, 1.0);
            c[3] = (c[3] as f32 * k) as u8;
            let s = p.size.max(1.0).round() as i32;
            f.rect((p.x - off[0]).round() as i32 - s / 2, (p.y - off[1]).round() as i32 - s / 2, s, s, c);
        }
    }
    let hover = |at: &[f32; 2], size: &[f32; 2]| {
        let ptr = sim.pointer_screen();
        ptr[0] >= at[0] && ptr[0] < at[0] + size[0] && ptr[1] >= at[1] && ptr[1] < at[1] + size[1]
    };
    for w in &def.ui {
        if !sim.widget_shown(w) {
            continue;
        }
        match &w.kind {
            WidgetKind::Text { text, at, color, scale, align } => {
                let t = fill_template(text, sim);
                let tw = font::text_width(&t, *scale as i32);
                let x = match align {
                    Align::Left => at[0] as i32,
                    Align::Center => at[0] as i32 - tw / 2,
                    Align::Right => at[0] as i32 - tw,
                };
                f.text(x, at[1] as i32, &t, *scale as i32, *color);
            }
            WidgetKind::Bar { var, max, at, size, color, back } => {
                f.rect(at[0] as i32, at[1] as i32, size[0] as i32, size[1] as i32, *back);
                let m = max.eval(&sim.vars).max(1e-9);
                let k = (sim.vars[*var] / m).clamp(0.0, 1.0);
                f.rect(at[0] as i32, at[1] as i32, (size[0] as f64 * k).round() as i32, size[1] as i32, *color);
            }
            WidgetKind::Panel { at, size, color } => f.rect(at[0] as i32, at[1] as i32, size[0] as i32, size[1] as i32, *color),
            WidgetKind::View3d(v) => draw_view3d(&mut f, sim, v),
            WidgetKind::Minimap { at, size, colors, back, border, dot, viewport } => {
                draw_minimap(&mut f, sim, *at, *size, colors, *back, *border, *dot, *viewport)
            }
            WidgetKind::Button { label, at, size, color, .. } => {
                let c = if hover(at, size) { lighten(*color, 30) } else { *color };
                f.rect(at[0] as i32, at[1] as i32, size[0] as i32, size[1] as i32, lighten(c, 50));
                f.rect(at[0] as i32 + 1, at[1] as i32 + 1, size[0] as i32 - 2, size[1] as i32 - 2, c);
                let tw = font::text_width(label, 1);
                f.text(at[0] as i32 + (size[0] as i32 - tw) / 2, at[1] as i32 + (size[1] as i32 - font::GLYPH_H) / 2, label, 1, [255, 255, 255, 255]);
            }
        }
    }
    f
}

fn draw_entity(f: &mut Frame, sim: &Sim, e: &crate::sim::Entity, off: [f32; 2]) {
    let def = &*sim.def;
    let p = &def.prefabs[e.prefab];
    let (cx, cy) = (e.x - off[0], e.y - off[1]);
    match &p.shape {
        Shape::None => {}
        Shape::Rect { color } => {
            f.rect((cx - p.size[0] * 0.5).round() as i32, (cy - p.size[1] * 0.5).round() as i32, p.size[0].round() as i32, p.size[1].round() as i32, *color)
        }
        Shape::Circle { color } => f.disc((cx - p.size[0] * 0.5).round() as i32, (cy - p.size[1] * 0.5).round() as i32, p.size[0].round() as i32, *color),
        Shape::Model(ms) => draw_model_entity(f, sim, ms, p.size, [cx, cy]),
        Shape::Sprite { sprite, scale, flip } => {
            let sp = &def.sprites[*sprite];
            let s = *scale as i32;
            let frame = sprite_frame(sim, sp);
            let (x0, y0) = ((cx - (sp.w as i32 * s) as f32 * 0.5).round() as i32, (cy - (sp.h as i32 * s) as f32 * 0.5).round() as i32);
            for (k, px) in sp.frames[frame].iter().enumerate() {
                if px[3] == 0 {
                    continue;
                }
                let (mut sx, sy) = ((k % sp.w) as i32, (k / sp.w) as i32);
                if *flip == Flip::X || (*flip == Flip::Auto && e.face_left) {
                    sx = sp.w as i32 - 1 - sx;
                }
                f.rect(x0 + sx * s, y0 + sy * s, s, s, *px);
            }
        }
        Shape::Text { text, color, scale } => {
            let t = fill_template(text, sim);
            let w = font::text_width(&t, *scale as i32);
            f.text((cx - w as f32 * 0.5).round() as i32, (cy - (font::GLYPH_H * *scale as i32) as f32 * 0.5).round() as i32, &t, *scale as i32, *color);
        }
    }
}

/// Where the virtual screen lands in a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// Left edge in window px.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width on screen.
    pub w: u32,
    /// Height on screen.
    pub h: u32,
}

/// The largest placement of a `vw x vh` screen in a `ww x wh` window that keeps its aspect ratio (whole-number scale for `Scale::Integer`, when at least 1x fits).
pub fn layout(vw: u32, vh: u32, ww: u32, wh: u32, scale: Scale) -> Layout {
    let (ww, wh) = (ww.max(1), wh.max(1));
    let (w, h) = match scale {
        Scale::Integer if ww >= vw && wh >= vh => {
            let k = (ww / vw).min(wh / vh).max(1);
            (vw * k, vh * k)
        }
        _ => {
            // Compare ww/vw with wh/vh exactly in integers.
            if (ww as u64) * (vh as u64) <= (wh as u64) * (vw as u64) {
                (ww, ((ww as u64 * vh as u64 + vw as u64 / 2) / vw as u64).max(1) as u32)
            } else {
                (((wh as u64 * vw as u64 + vh as u64 / 2) / vh as u64).max(1) as u32, wh)
            }
        }
    };
    Layout { x: ((ww as i64 - w as i64) / 2) as i32, y: ((wh as i64 - h as i64) / 2) as i32, w, h }
}

/// A window position to a virtual-screen position (`None` in the bars).
pub fn window_to_view(l: Layout, vw: u32, vh: u32, wx: f32, wy: f32) -> Option<[f32; 2]> {
    let (x, y) = (wx - l.x as f32, wy - l.y as f32);
    if x < 0.0 || y < 0.0 || x >= l.w as f32 || y >= l.h as f32 {
        return None;
    }
    Some([x * vw as f32 / l.w as f32, y * vh as f32 / l.h as f32])
}

/// The picture scaled (nearest neighbour) into a window of `ww x wh` with black bars.
pub fn present(frame: &Frame, ww: u32, wh: u32, scale: Scale) -> Frame {
    let l = layout(frame.w, frame.h, ww, wh, scale);
    let mut out = Frame::new(ww.max(1), wh.max(1), [0, 0, 0, 255]);
    for y in 0..l.h {
        let sy = (y as u64 * frame.h as u64 / l.h as u64) as u32;
        for x in 0..l.w {
            let sx = (x as u64 * frame.w as u64 / l.w as u64) as u32;
            let (si, di) = (((sy * frame.w + sx) * 4) as usize, (((y as i32 + l.y) as u32 * out.w + (x as i32 + l.x) as u32) * 4) as usize);
            out.rgba[di..di + 4].copy_from_slice(&frame.rgba[si..si + 4]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::tests::game;
    use std::sync::Arc;

    #[test]
    fn layout_keeps_the_aspect_ratio_and_centres_with_bars() {
        // 16:9 screen in a 4:3 window: bars above and below.
        let l = layout(320, 180, 800, 600, Scale::Fit);
        assert_eq!((l.w, l.h, l.x, l.y), (800, 450, 0, 75));
        // In a very wide window: bars left and right.
        let l = layout(320, 180, 1600, 400, Scale::Fit);
        assert_eq!((l.h, l.w), (400, 711));
        assert_eq!(l.x, (1600 - 711) / 2);
        // Integer scale never blurs: 800x600 holds 2x of 320x180 (640x360), not 2.5x.
        let l = layout(320, 180, 800, 600, Scale::Integer);
        assert_eq!((l.w, l.h, l.x, l.y), (640, 360, 80, 120));
        // A window smaller than the screen falls back to fitting it.
        let l = layout(320, 180, 160, 90, Scale::Integer);
        assert_eq!((l.w, l.h), (160, 90));
        // A tiny sliver of a window still has a drawable area.
        let l = layout(320, 180, 1, 1, Scale::Fit);
        assert!(l.w >= 1 && l.h >= 1);
    }

    #[test]
    fn a_pointer_in_the_window_maps_back_to_the_virtual_screen_and_bars_map_to_nothing() {
        let l = layout(320, 180, 800, 600, Scale::Fit);
        assert_eq!(window_to_view(l, 320, 180, 400.0, 300.0), Some([160.0, 90.0]));
        assert_eq!(window_to_view(l, 320, 180, 0.0, 0.0), None, "the top bar");
        assert_eq!(window_to_view(l, 320, 180, 0.0, 75.0), Some([0.0, 0.0]));
        assert_eq!(window_to_view(l, 320, 180, 799.9, 524.9).map(|p| p[0] > 319.0 && p[1] > 179.0), Some(true));
        assert_eq!(window_to_view(l, 320, 180, 400.0, 530.0), None, "the bottom bar");
    }

    #[test]
    fn presenting_scales_pixels_without_blur_and_letterboxes() {
        let mut f = Frame::new(2, 2, [0, 0, 0, 255]);
        f.blend(0, 0, [255, 0, 0, 255]);
        f.blend(1, 1, [0, 0, 255, 255]);
        let p = present(&f, 8, 6, Scale::Integer); // 2x2 at 3x = 6x6, centred in 8x6
        assert_eq!((p.w, p.h), (8, 6));
        let px = |x: u32, y: u32| p.rgba[((y * 8 + x) * 4) as usize..][..3].to_vec();
        assert_eq!(px(1, 0), vec![255, 0, 0], "red block starts after the 1px bar");
        assert_eq!(px(3, 2), vec![255, 0, 0]);
        assert_eq!(px(4, 3), vec![0, 0, 255]);
        assert_eq!(px(0, 0), vec![0, 0, 0], "bar");
        let colors: std::collections::BTreeSet<Vec<u8>> = (0..8).flat_map(|x| (0..6).map(move |y| (x, y))).map(|(x, y)| px(x, y)).collect();
        assert_eq!(colors.len(), 3, "only the source colours: no blending");
    }

    #[test]
    fn templates_fill_variables_pad_and_mark_unknowns() {
        let s = Sim::new(game("", r##""c":{"shape":{"circle":2}}"##, r#"{"prefab":"c","at":[5,5]}"#), 1);
        assert_eq!(fill_template("SCORE {score}", &s), "SCORE 0");
        assert_eq!(fill_template("{score:4}!", &s), "0000!");
        assert_eq!(fill_template("{nope}", &s), "?");
        assert_eq!(fill_template("{score:.2}", &s), "0.00");
        assert_eq!(fmt_num(2.5), "2.5");
        assert_eq!(fmt_num(3.0), "3");
        assert_eq!(fmt_num(1.0 / 3.0), "0.33");
    }

    #[test]
    fn a_sprite_is_mirrored_always_or_while_it_last_moved_left() {
        let text = |flip: &str, vx: i32| {
            format!(
                r##"{{"game2d":1,"id":"t","title":"T","description":"d","capabilities":{{"presentation":"2d","platforms":["windows","linux"],"networking":"offline","input":[],"persistence":[]}},
                "view":{{"width":64,"height":64,"background":"#000000"}},"sprites":{{"two":{{"palette":{{"a":"#ff0000","b":"#0000ff"}},"rows":["ab"]}}}},
                "prefabs":{{"s":{{"shape":{{"sprite":"two","scale":2{flip}}},"move":{{"drift":[{vx},0]}}}}}},"scene":[{{"prefab":"s","at":[32,32]}}],"rules":[]}}"##
            )
        };
        let px = |s: &Sim| {
            let f = render(s);
            // The sprite is 4x2 px at scale 2; find its first lit pixel's colour.
            let i = f.rgba.chunks(4).position(|p| p[0] != 0 || p[2] != 0).unwrap();
            f.rgba[i * 4..i * 4 + 3].to_vec()
        };
        let run = |flip: &str, vx: i32| {
            let mut s = Sim::new(Arc::new(parse(&text(flip, vx)).unwrap_or_else(|e| panic!("{e:?}"))), 1);
            s.run_ticks(10);
            px(&s)
        };
        assert_eq!(run("", 0), vec![255, 0, 0], "as drawn: red on the left");
        assert_eq!(run(r#","flip":"x""#, 0), vec![0, 0, 255], "always mirrored");
        assert_eq!(run(r#","flip":"auto""#, 0), vec![255, 0, 0], "auto: not moving left, as drawn");
        assert_eq!(run(r#","flip":"auto""#, 30), vec![255, 0, 0], "auto: moving right, as drawn");
        assert_eq!(run(r#","flip":"auto""#, -30), vec![0, 0, 255], "auto: moving left, mirrored");
        let bad = parse(&text(r#","flip":"y""#, 0)).unwrap_err().join(" ");
        assert!(bad.contains("flip") && bad.contains("\"auto\""), "{bad}");
    }

    #[test]
    fn a_game_renders_its_things_in_layer_order_with_text_and_a_hud_and_is_not_blank() {
        let prefabs = r##""bg":{"shape":{"rect":[160,90],"color":"#204060"},"layer":-1},"dot":{"shape":{"circle":15,"color":"#ff0000"}},"sq":{"shape":{"rect":[20,20],"color":"#00ff00"},"layer":1},
            "label":{"shape":{"text":"HI {score}","color":"#ffffff","scale":1}}"##;
        let scene = r#"{"prefab":"bg","at":[80,45]},{"prefab":"sq","at":[80,45]},{"prefab":"dot","at":[80,45]},{"prefab":"label","at":[80,20]}"#;
        let s = Sim::new(game("", prefabs, scene), 1);
        let f = render(&s);
        assert_eq!((f.w, f.h), (160, 90));
        let px = |x: usize, y: usize| f.rgba[(y * 160 + x) * 4..][..3].to_vec();
        assert_eq!(px(80, 45), vec![0, 255, 0], "the layer-1 square is over the red disc");
        assert_eq!(px(5, 85), vec![0x20, 0x40, 0x60], "the background shape");
        let st = f.stats([16, 20, 28, 255]);
        assert!(st.distinct_colors >= 4 && st.covered > 0.5, "{st:?}");
        // Deterministic.
        assert_eq!(render(&s), f);
    }

    fn hybrid(view_extra: &str, prefabs: &str, scene: &str, ui: &str) -> Sim {
        let text = format!(
            r##"{{"game2d":1,"id":"h","title":"H","description":"d",
            "capabilities":{{"presentation":"hybrid","platforms":["windows","linux"],"networking":"offline","input":["keyboard"],"persistence":[]}},
            "view":{{"width":160,"height":90,"background":"#000000"{view_extra}}},
            "models":{{"m":{{"parts":[{{"shape":"box","size":[2,2,2],"color":"#ff0000"}}]}}}},
            "prefabs":{{{prefabs}}},"scene":[{scene}],"ui":[{ui}]}}"##
        );
        Sim::new(Arc::new(parse(&text).unwrap_or_else(|e| panic!("{e:?}\n{text}"))), 1)
    }

    fn px(f: &Frame, x: usize, y: usize) -> Vec<u8> {
        f.rgba[(y * f.w as usize + x) * 4..][..3].to_vec()
    }

    #[test]
    fn a_model_is_drawn_inside_its_box_and_nowhere_else() {
        let s = hybrid("", r##""boss":{"shape":{"model":"m","fit":[40,40],"yaw":30,"elevation":20}}"##, r#"{"prefab":"boss","at":[80,45]}"#, "");
        let f = render(&s);
        let c = px(&f, 80, 45);
        assert!(c[0] > 100 && c[1] == 0 && c[2] == 0, "a red model in the middle: {c:?}");
        assert_eq!(px(&f, 10, 10), vec![0, 0, 0], "the corner is the background");
        assert_eq!(render(&s), f, "deterministic");
        let st = f.stats([0, 0, 0, 255]);
        assert!(st.covered > 0.05 && st.covered < 0.3, "{st:?}");
    }

    #[test]
    fn a_viewport_in_the_hud_and_a_minimap_draw_where_they_are_put() {
        let ui = r##"{"view3d":{"at":[4,4],"size":[40,40],"background":"#0000ff","camera":{"eye":[0,0,6],"target":[0,0,0]},"items":[{"model":"m"}]}},
            {"minimap":{"at":[100,4],"size":[50,30],"colors":{"dot":"#00ff00"},"dot":4,"background":"#202020","border":"#ffffff"}}"##;
        let s = hybrid(r#","world":[320,180]"#, r##""dot":{"tag":"dot","shape":{"rect":[4,4],"color":"#888888"}}"##, r#"{"prefab":"dot","at":[160,90]}"#, ui);
        let f = render(&s);
        let c = px(&f, 24, 24);
        assert!(c[0] > 100 && c[2] == 0, "the model is in the middle of the viewport: {c:?}");
        assert_eq!(px(&f, 6, 6), vec![0, 0, 255], "the viewport's own background");
        assert_eq!(px(&f, 125, 19), vec![0, 255, 0], "a thing in the middle of the world is a dot in the middle of the map");
        assert_eq!(px(&f, 100, 10), vec![255, 255, 255], "border");
        assert_eq!(px(&f, 110, 10), vec![0x20, 0x20, 0x20], "map background");
    }

    #[test]
    fn a_world_view_draws_the_ground_boxes_and_pictures_and_clicks_land_on_the_ground() {
        let prefabs = r##""hedge":{"tag":"wall","shape":{"rect":[20,20],"color":"#00ff00"},"height3d":20,"body":{"type":"static"}},
            "me":{"tag":"me","shape":{"rect":[10,10],"color":"#ff00ff"},"clamp":true}"##;
        let view = r##","world":[320,320],"camera":{"follow":"me","lerp":1},"world3d":{"pitch":60,"distance":160,"sky":"#101010","ground":{"color":"#404040","alt":"#505050","tile":20}}"##;
        let s = hybrid(view, prefabs, r#"{"prefab":"me","at":[160,160],"id":"me"},{"prefab":"hedge","at":[100,160]}"#, "");
        let f = render(&s);
        let st = f.stats([0, 0, 0, 255]);
        assert!(st.distinct_colors >= 5 && st.covered > 0.6, "sky, two ground tones, a box and a picture: {st:?}");
        assert_eq!(render(&s), f, "deterministic");
        assert!((0..90).any(|y| (0..160).any(|x| px(&f, x, y) == vec![255, 0, 255])), "the picture of `me` is drawn");
        assert!((0..90).any(|y| (0..160).any(|x| px(&f, x, y)[1] > 0 && px(&f, x, y)[0] == 0)), "the hedge box is drawn");
        // Looking at the thing in the middle of the screen: the pointer there is over its feet.
        let mut s = s;
        s.set_pointer(80.0, 45.0);
        let w = s.pointer_world();
        assert!((w[0] - 160.0).abs() < 25.0 && (w[1] - 160.0).abs() < 25.0, "the centre of the screen is the point looked at: {w:?}");
        // Higher on the screen is farther from the camera: smaller y in the world.
        s.set_pointer(80.0, 20.0);
        assert!(s.pointer_world()[1] < w[1] - 20.0, "{:?} vs {w:?}", s.pointer_world());
    }
}
