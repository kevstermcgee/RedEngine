//! `red_engine2 plan`: a top-down floor plan of one level of a map, as a labelled PNG (or ASCII).
//!
//! The plan shows what the player would run into at that floor height: walls and furniture
//! (labelled with their ids), the stairs (with an arrow pointing up-slope), floor coverage
//! (holes in a slab show as void — e.g. a stairwell opening), the area the player can actually
//! walk to (green tint, from [`super::reach`]), lights, the spawn, and any `lint` findings.
//! Coordinates: `+X` is right, `+Z` is down the image, matching the world axes.

use super::font::{draw_text, draw_text_centered, text_width};
use super::lint::{Finding, Severity};
use super::reach::Reach;
use super::world::{Item, ItemKind, MapWorld};
use crate::schema::LightKind;
use glam::{Vec2, Vec3};
use image::{Rgb, RgbImage};
use std::collections::HashMap;

/// Which labels a plan draws: automatic (declutter), all, or none.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Labels {
    Auto,
    All,
    None,
}

/// Options for a plan render: floor height, scale, world window and labels.
pub struct PlanOptions {
    /// Floor height to draw (the foot height of the player on that level).
    pub y: f32,
    /// Pixels per meter (auto-reduced to keep the image under ~2400 px).
    pub scale: f32,
    /// World XZ window `(min, max)`; default = the map's solid bounds plus a margin.
    pub bounds: Option<(Vec2, Vec2)>,
    pub labels: Labels,
    pub show_reach: bool,
    pub show_findings: bool,
}

impl Default for PlanOptions {
    fn default() -> Self {
        PlanOptions { y: 0.0, scale: 40.0, bounds: None, labels: Labels::Auto, show_reach: true, show_findings: true }
    }
}

const MARGIN: i32 = 30;

struct View {
    min: Vec2,
    scale: f32,
}

impl View {
    fn px(&self, p: Vec2) -> (f32, f32) {
        ((p.x - self.min.x) * self.scale + MARGIN as f32, (p.y - self.min.y) * self.scale + MARGIN as f32)
    }
    fn world(&self, x: f32, y: f32) -> Vec2 {
        Vec2::new((x - MARGIN as f32) / self.scale + self.min.x, (y - MARGIN as f32) / self.scale + self.min.y)
    }
}

fn rgb(c: Vec3) -> Rgb<u8> {
    Rgb([(c.x.clamp(0.0, 1.0) * 255.0) as u8, (c.y.clamp(0.0, 1.0) * 255.0) as u8, (c.z.clamp(0.0, 1.0) * 255.0) as u8])
}

fn blend(img: &mut RgbImage, x: i32, y: i32, c: Rgb<u8>, a: f32) {
    if x < 0 || y < 0 || x as u32 >= img.width() || y as u32 >= img.height() {
        return;
    }
    let p = img.get_pixel_mut(x as u32, y as u32);
    for i in 0..3 {
        p.0[i] = (p.0[i] as f32 * (1.0 - a) + c.0[i] as f32 * a) as u8;
    }
}

fn fill_rect(img: &mut RgbImage, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgb<u8>, a: f32) {
    for y in y0.floor() as i32..=y1.ceil() as i32 {
        for x in x0.floor() as i32..=x1.ceil() as i32 {
            blend(img, x, y, c, a);
        }
    }
}

fn line(img: &mut RgbImage, a: (f32, f32), b: (f32, f32), c: Rgb<u8>, thick: i32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let n = dx.abs().max(dy.abs()).ceil().max(1.0) as i32;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let (x, y) = (a.0 + dx * t, a.1 + dy * t);
        for ox in 0..thick {
            for oy in 0..thick {
                blend(img, x as i32 + ox - thick / 2, y as i32 + oy - thick / 2, c, 1.0);
            }
        }
    }
}

/// Fills a convex quad given in pixel space; `shade` may recolor each pixel from its position.
fn fill_quad(img: &mut RgbImage, q: [(f32, f32); 4], a: f32, mut shade: impl FnMut(i32, i32) -> Rgb<u8>) {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in q {
        x0 = x0.min(p.0);
        y0 = y0.min(p.1);
        x1 = x1.max(p.0);
        y1 = y1.max(p.1);
    }
    for y in y0.floor() as i32..=y1.ceil() as i32 {
        for x in x0.floor() as i32..=x1.ceil() as i32 {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut pos = 0;
            let mut neg = 0;
            for i in 0..4 {
                let (p, r) = (q[i], q[(i + 1) % 4]);
                let cr = (r.0 - p.0) * (fy - p.1) - (r.1 - p.1) * (fx - p.0);
                if cr > 0.0 {
                    pos += 1;
                } else if cr < 0.0 {
                    neg += 1;
                }
            }
            if pos == 0 || neg == 0 {
                let c = shade(x, y);
                blend(img, x, y, c, a);
            }
        }
    }
}

fn outline_quad(img: &mut RgbImage, q: [(f32, f32); 4], c: Rgb<u8>) {
    for i in 0..4 {
        line(img, q[i], q[(i + 1) % 4], c, 1);
    }
}

/// True if the item physically occupies the player's body band when standing at `h`.
fn in_band(it: &Item, h: f32) -> bool {
    it.volumes.iter().any(|(mn, mx)| mx.y >= h + 0.05 && mn.y <= h + 2.0)
}

fn is_floor_fill(it: &Item, h: f32) -> bool {
    match it.kind {
        ItemKind::Plane => (it.max.y - h).abs() <= 0.15,
        ItemKind::Box => (it.max.y - it.min.y) <= 0.6 && (it.max.y - h).abs() <= 0.06,
        _ => false,
    }
}

fn is_wall_like(it: &Item) -> bool {
    matches!(it.kind, ItemKind::Box) && (it.max.y - it.min.y) >= 1.0 && {
        let (w, d) = (it.max.x - it.min.x, it.max.z - it.min.z);
        w.min(d) <= 0.5
    }
}

fn frame_bounds(world: &MapWorld, opts: &PlanOptions) -> (Vec2, Vec2) {
    if let Some(b) = opts.bounds {
        return b;
    }
    let (min, max) = world.solid_bounds();
    (min - Vec2::splat(1.5), max + Vec2::splat(1.5))
}

/// Renders a labelled top-down plan of one floor as an image.
pub fn render_png(world: &MapWorld, reach: Option<&Reach>, findings: &[Finding], opts: &PlanOptions) -> RgbImage {
    let (bmin, bmax) = frame_bounds(world, opts);
    let extent = (bmax - bmin).max(Vec2::splat(1.0));
    let scale = opts.scale.min(2400.0 / extent.x.max(extent.y)).max(4.0);
    let view = View { min: bmin, scale };
    let w = (extent.x * scale) as i32 + MARGIN * 2;
    let h = (extent.y * scale) as i32 + MARGIN * 2;
    let ground_floor = opts.y < 0.2;

    let void_c = Rgb([26, 30, 40]);
    let mut img = RgbImage::from_pixel(w as u32, h as u32, void_c);
    let (tl, br) = (view.px(bmin), view.px(bmax));
    fill_rect(&mut img, tl.0, tl.1, br.0, br.1, if ground_floor { Rgb([58, 66, 60]) } else { void_c }, 1.0);

    // Floor coverage (planes and slab tops at this height).
    for it in world.items.iter().filter(|i| is_floor_fill(i, opts.y)) {
        if let Some(fp) = it.footprint {
            let q = fp.corners().map(|c| view.px(c));
            let col = rgb(it.color * 0.85 + Vec3::splat(0.08));
            fill_quad(&mut img, q, 1.0, |_, _| col);
        }
    }

    // Grid: 1 m minor, 5 m major, labelled every 2 m along the top/left.
    let (gx0, gx1) = (bmin.x.ceil() as i32, bmax.x.floor() as i32);
    let (gz0, gz1) = (bmin.y.ceil() as i32, bmax.y.floor() as i32);
    for gx in gx0..=gx1 {
        let x = view.px(Vec2::new(gx as f32, 0.0)).0;
        let major = gx % 5 == 0;
        for y in tl.1 as i32..=br.1 as i32 {
            blend(&mut img, x as i32, y, Rgb([255, 255, 255]), if major { 0.22 } else { 0.07 });
        }
        if gx % 2 == 0 {
            draw_text_centered(&mut img, x as i32, MARGIN / 2, &gx.to_string(), 1, Rgb([200, 210, 225]), None);
        }
    }
    for gz in gz0..=gz1 {
        let y = view.px(Vec2::new(0.0, gz as f32)).1;
        let major = gz % 5 == 0;
        for x in tl.0 as i32..=br.0 as i32 {
            blend(&mut img, x, y as i32, Rgb([255, 255, 255]), if major { 0.22 } else { 0.07 });
        }
        if gz % 2 == 0 {
            let s = gz.to_string();
            draw_text(&mut img, MARGIN - 4 - text_width(&s, 1), y as i32 - 3, &s, 1, Rgb([200, 210, 225]), None);
        }
    }

    // Zones.
    for z in &world.zones {
        if (z.y - opts.y).abs() > 0.6 {
            continue;
        }
        let (a, b) = (view.px(z.min), view.px(z.max));
        fill_rect(&mut img, a.0, a.1, b.0, b.1, Rgb([90, 140, 220]), 0.10);
        line(&mut img, (a.0, a.1), (b.0, a.1), Rgb([120, 170, 240]), 1);
        line(&mut img, (b.0, a.1), (b.0, b.1), Rgb([120, 170, 240]), 1);
        line(&mut img, (b.0, b.1), (a.0, b.1), Rgb([120, 170, 240]), 1);
        line(&mut img, (a.0, b.1), (a.0, a.1), Rgb([120, 170, 240]), 1);
        draw_text(&mut img, a.0 as i32 + 3, a.1 as i32 + 3, &z.id, 2, Rgb([170, 205, 255]), Some(Rgb([20, 30, 50])));
    }

    // Reachable area.
    if let (true, Some(r)) = (opts.show_reach, reach) {
        let half = r.cell * 0.5 * scale;
        for (i, lv) in r.levels.iter().enumerate() {
            if lv.iter().any(|l| (l - opts.y).abs() <= 0.35) {
                let p = view.px(r.cell_center(i));
                fill_rect(&mut img, p.0 - half, p.1 - half, p.0 + half, p.1 + half, Rgb([40, 215, 235]), 0.28);
            }
        }
    }

    // Solids at this floor.
    let wall_c = Rgb([52, 56, 66]);
    for it in world.items.iter().filter(|i| i.kind != ItemKind::Stairs && i.is_solid() && in_band(i, opts.y)) {
        let Some(fp) = it.footprint else { continue };
        let q = fp.corners().map(|c| view.px(c));
        if is_wall_like(it) {
            fill_quad(&mut img, q, 1.0, |_, _| wall_c);
        } else {
            let col = rgb(it.color * 0.9 + Vec3::splat(0.05));
            fill_quad(&mut img, q, 1.0, |_, _| col);
            outline_quad(&mut img, q, Rgb([10, 12, 16]));
        }
    }

    // Stairs: shaded low -> high with an arrow pointing up-slope.
    for it in world.items.iter().filter(|i| i.stairs.is_some()) {
        let s = it.stairs.unwrap();
        let base = s.base_y();
        if opts.y < base - 0.7 || opts.y > base + s.rise + 0.7 {
            continue;
        }
        let Some(fp) = it.footprint else { continue };
        let q = fp.corners().map(|c| view.px(c));
        let inv = s.world.inverse();
        fill_quad(&mut img, q, 1.0, |x, y| {
            let wp = view.world(x as f32 + 0.5, y as f32 + 0.5);
            let local = inv.transform_point3(Vec3::new(wp.x, 0.0, wp.y));
            let t = ((local.z + s.run * 0.5) / s.run).clamp(0.0, 1.0);
            let v = 0.85 - 0.5 * t;
            Rgb([(220.0 * v) as u8, (170.0 * v) as u8, (90.0 * v) as u8])
        });
        outline_quad(&mut img, q, Rgb([250, 235, 190]));
        let (a, b) = (view.px(s.point(0.3, 0.0)), view.px(s.point(s.run - 0.3, 0.0)));
        line(&mut img, a, b, Rgb([255, 255, 255]), 2);
        // Arrow head at the top end.
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        let (ux, uy) = (dx / len, dy / len);
        for sgn in [-1.0f32, 1.0] {
            let tip = (b.0 - ux * 9.0 - uy * 6.0 * sgn, b.1 - uy * 9.0 + ux * 6.0 * sgn);
            line(&mut img, b, tip, Rgb([255, 255, 255]), 2);
        }
        let mid = ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
        draw_text_centered(&mut img, mid.0 as i32, mid.1 as i32 + 10, &format!("UP TO {:.1}", base + s.rise), 1, Rgb([255, 255, 255]), Some(Rgb([40, 25, 10])));
    }

    // Lights and the spawn.
    for l in &world.scene.lights {
        if let LightKind::Point { position, .. } = &l.kind {
            let p = position.sample(0.0);
            if p.y >= opts.y && p.y <= opts.y + 3.4 {
                let (x, y) = view.px(Vec2::new(p.x, p.z));
                for r in 0..=4 {
                    line(&mut img, (x - r as f32, y), (x + r as f32, y), Rgb([255, 225, 120]), 1);
                }
                draw_text(&mut img, x as i32 + 6, y as i32 - 3, &l.id, 1, Rgb([255, 235, 150]), Some(Rgb([30, 25, 5])));
            }
        }
    }
    if opts.y < 0.5 {
        let (x, y) = view.px(world.spawn);
        line(&mut img, (x - 7.0, y), (x + 7.0, y), Rgb([80, 160, 255]), 2);
        line(&mut img, (x, y - 7.0), (x, y + 7.0), Rgb([80, 160, 255]), 2);
        draw_text(&mut img, x as i32 + 9, y as i32 + 4, "SPAWN", 1, Rgb([150, 200, 255]), Some(Rgb([10, 20, 40])));
    }

    // Labels: one per top-level object, at its largest visible piece.
    if opts.labels != Labels::None {
        let mut best: HashMap<&str, (f32, Vec2, bool)> = HashMap::new();
        for it in world.items.iter().filter(|i| (i.is_solid() && in_band(i, opts.y)) || i.stairs.is_some()) {
            let Some(fp) = it.footprint else { continue };
            let area = fp.half.x * fp.half.y * 4.0;
            let e = best.entry(it.top_id.as_str()).or_insert((0.0, fp.center, is_wall_like(it)));
            if area > e.0 {
                *e = (area, fp.center, is_wall_like(it));
            }
        }
        let mut labels: Vec<(&str, f32, Vec2, bool)> = best.into_iter().map(|(k, v)| (k, v.0, v.1, v.2)).collect();
        labels.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(b.0)));
        let mut placed: Vec<(i32, i32, i32, i32)> = Vec::new();
        for (id, area, center, wall) in labels {
            if opts.labels == Labels::Auto && !wall && area < 0.35 {
                continue;
            }
            let (x, y) = view.px(center);
            let tw = text_width(id, 1);
            let rect = (x as i32 - tw / 2 - 1, y as i32 - 5, x as i32 + tw / 2 + 1, y as i32 + 5);
            if placed.iter().any(|p| rect.0 < p.2 && rect.2 > p.0 && rect.1 < p.3 && rect.3 > p.1) {
                continue;
            }
            placed.push(rect);
            let col = if wall { Rgb([255, 214, 130]) } else { Rgb([255, 255, 255]) };
            draw_text_centered(&mut img, x as i32, y as i32, id, 1, col, Some(Rgb([10, 12, 16])));
        }
    }

    // Lint findings.
    if opts.show_findings {
        for (n, f) in findings.iter().filter(|f| f.sev >= Severity::Warn).enumerate() {
            let Some(at) = f.at else { continue };
            if at.y < opts.y - 0.7 || at.y > opts.y + 3.5 {
                continue;
            }
            let (x, y) = view.px(Vec2::new(at.x, at.z));
            let col = if f.sev == Severity::Error { Rgb([255, 70, 70]) } else { Rgb([255, 170, 40]) };
            for a in 0..64 {
                let ang = a as f32 / 64.0 * std::f32::consts::TAU;
                for r in [9.0f32, 10.0] {
                    blend(&mut img, (x + ang.cos() * r) as i32, (y + ang.sin() * r) as i32, col, 1.0);
                }
            }
            draw_text(&mut img, x as i32 + 12, y as i32 - 4, &format!("{}:{}", n + 1, f.code), 1, col, Some(Rgb([10, 5, 5])));
        }
    }

    // Title.
    let title = format!("PLAN y={:.1}   {:.0}px/m   +X right  +Z down   cyan=walkable", opts.y, scale);
    draw_text(&mut img, MARGIN, 4, &title, 1, Rgb([235, 240, 250]), None);
    img
}

/// Text rendering of the plan at `cell` meters per character. Legend: `#` wall, letters = props
/// (first letter of the kind), `o` other box, `^` stairs, `.` walkable, `,` floor the player
/// can't reach, blank = void/outside, `!` lint finding, `@` spawn.
pub fn render_ascii(world: &MapWorld, reach: Option<&Reach>, findings: &[Finding], opts: &PlanOptions, cell: f32) -> String {
    let (bmin, bmax) = frame_bounds(world, opts);
    let nx = ((bmax.x - bmin.x) / cell).ceil() as usize;
    let nz = ((bmax.y - bmin.y) / cell).ceil() as usize;
    let mut grid = vec![vec![' '; nx]; nz];
    let ground_floor = opts.y < 0.2;
    let mut legend: HashMap<char, String> = HashMap::new();
    for (iz, row) in grid.iter_mut().enumerate() {
        for (ix, ch) in row.iter_mut().enumerate() {
            let p = Vec2::new(bmin.x + (ix as f32 + 0.5) * cell, bmin.y + (iz as f32 + 0.5) * cell);
            let floor = ground_floor || world.items.iter().any(|i| is_floor_fill(i, opts.y) && i.footprint.is_some_and(|f| f.contains(p)));
            let walk = reach.is_some_and(|r| r.reachable(p, opts.y, 0.35));
            *ch = if walk { '.' } else if floor { ',' } else { ' ' };
            // Glyph priority: wall > stairs > prop > other box, so a baseboard strip never hides
            // the wall it runs along and a rug/trim never hides furniture.
            let mut rank = 0;
            for it in world.items.iter().filter(|i| i.is_solid() && in_band(i, opts.y)) {
                let Some(fp) = it.footprint else { continue };
                // A wall thinner than a character cell can fall between cell centers; give
                // wall-like pieces a half-cell margin so they always register.
                let margin = if is_wall_like(it) { cell * 0.5 } else { 0.0 };
                let d = p - fp.center;
                let (u, v) = (d.dot(fp.axis), d.dot(Vec2::new(-fp.axis.y, fp.axis.x)));
                if u.abs() > fp.half.x + margin || v.abs() > fp.half.y + margin {
                    continue;
                }
                let (glyph, r) = match it.kind {
                    ItemKind::Stairs => ('^', 3),
                    _ if is_wall_like(it) => ('#', 4),
                    ItemKind::Prop(k) => {
                        let c = k.name().chars().next().unwrap().to_ascii_uppercase();
                        legend.entry(c).or_insert_with(|| k.name().to_string());
                        (c, 2)
                    }
                    _ => ('o', 1),
                };
                if r >= rank {
                    *ch = glyph;
                    rank = r;
                }
            }
            for it in world.items.iter().filter(|i| i.stairs.is_some()) {
                let s = it.stairs.unwrap();
                if opts.y >= s.base_y() - 0.7 && opts.y <= s.base_y() + s.rise + 0.7 && it.footprint.is_some_and(|f| f.contains(p)) && rank < 4 {
                    *ch = '^';
                }
            }
        }
    }
    let mut mark = |p: Vec2, c: char| {
        let ix = ((p.x - bmin.x) / cell) as isize;
        let iz = ((p.y - bmin.y) / cell) as isize;
        if ix >= 0 && iz >= 0 && (ix as usize) < nx && (iz as usize) < nz {
            grid[iz as usize][ix as usize] = c;
        }
    };
    if opts.show_findings {
        for f in findings.iter().filter(|f| f.sev >= Severity::Warn) {
            if let Some(at) = f.at {
                if at.y >= opts.y - 0.7 && at.y <= opts.y + 3.5 {
                    mark(Vec2::new(at.x, at.z), '!');
                }
            }
        }
    }
    if opts.y < 0.5 {
        mark(world.spawn, '@');
    }

    let mut out = String::new();
    out.push_str(&format!(
        "plan y={:.1}  {}x{} cells of {}m  origin=({:.1},{:.1})  (+X right, +Z down)\n",
        opts.y, nx, nz, cell, bmin.x, bmin.y
    ));
    // Column ruler: a tick every 2 m, labelled with the whole-meter x coordinate.
    let mut ruler = vec![' '; nx + 6];
    let mut x = bmin.x.ceil() as i32;
    while (x as f32) < bmax.x {
        let col = ((x as f32 - bmin.x) / cell) as usize + 6;
        if x % 2 == 0 {
            for (k, c) in x.to_string().chars().enumerate() {
                if col + k < ruler.len() {
                    ruler[col + k] = c;
                }
            }
        }
        x += 1;
    }
    out.push_str(&ruler.iter().collect::<String>());
    out.push('\n');
    for (iz, row) in grid.iter().enumerate() {
        let z = bmin.y + iz as f32 * cell;
        let label = if (z - z.round()).abs() < cell * 0.5 && (z.round() as i32) % 2 == 0 { format!("{:>4} ", z.round() as i32) } else { "     ".to_string() };
        out.push_str(&label);
        out.push_str(row.iter().collect::<String>().trim_end());
        out.push('\n');
    }
    let mut keys: Vec<_> = legend.into_iter().collect();
    keys.sort();
    out.push_str("legend: # wall  o box  ^ stairs  . walkable  , unreachable floor  @ spawn  ! finding  ");
    for (c, name) in keys {
        out.push_str(&format!("{c}={name} "));
    }
    out.push('\n');
    out
}
