//! The plan of the museum as far as the player has explored it: drawn into an RGBA image for the pause menu's map and `frameless museum --map`.
//!
//! Rooms walked into are filled in their wing's colours with their gallery number; rooms only glimpsed through a doorway are outlined; a key marks a room
//! whose key was found; the halls show whether their doors are open; wing names float over the rooms of each wing; an arrow shows where the player is.

use super::layout::{EdgeId, RoomId, Side, CELL, HALL_W};
use super::state::Museum;
use glam::{Vec2, Vec3};
use red_engine2::ui::serif::{Face, Image, Serif};
use std::collections::BTreeMap;

/// What part of the plan to show.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapView {
    /// The middle of the picture, in rooms (x east, y south).
    pub centre: Vec2,
    /// Pixels per room.
    pub zoom: f32,
}

impl MapView {
    /// Centred on the player, at a comfortable scale for a `h`-pixel-tall picture.
    pub fn around(player: Vec3, h: u32) -> MapView {
        MapView { centre: Vec2::new(player.x / CELL, player.z / CELL), zoom: (h as f32 / 6.0).clamp(50.0, 180.0) }
    }
}

fn srgb(c: [f32; 3], k: f32, a: u8) -> [u8; 4] {
    let f = |x: f32| ((x * k).clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8;
    [f(c[0]), f(c[1]), f(c[2]), a]
}

/// Paints the plan. `player` is the feet and the camera yaw; `time` drives the gentle pulse of the marker.
pub fn paint(m: &Museum, serif: &Serif, img: &mut Image<'_>, view: &MapView, player: (Vec3, f32), time: f32) {
    let (w, h) = (img.w as f32, img.h as f32);
    // Night-blue paper with a faint grid of the cells.
    for y in 0..img.h as i32 {
        let k = y as f32 / h;
        let c = [(10.0 + 8.0 * k) as u8, (12.0 + 8.0 * k) as u8, (22.0 + 10.0 * k) as u8, 240];
        img.rect(0, y, img.w as i32, y + 1, c);
    }
    let to_screen = |x: f32, z: f32| -> (f32, f32) { (w * 0.5 + (x / CELL - view.centre.x) * view.zoom, h * 0.5 + (z / CELL - view.centre.y) * view.zoom) };
    let seen = m.seen();
    let visible = |r: &RoomId| {
        let (sx, sy) = to_screen(r.x as f32 * CELL, r.z as f32 * CELL);
        sx > -view.zoom && sx < w + view.zoom && sy > -view.zoom && sy < h + view.zoom
    };
    let themes = &m.spec.collection.themes;
    // ---- Halls between seen rooms ----
    for r in seen.iter().filter(|r| visible(r)) {
        for s in [Side::East, Side::South] {
            let e = EdgeId::of(*r, s);
            if !m.layout.has_door(e) {
                continue;
            }
            let (a, b) = e.rooms();
            if !(m.progress.visited.contains(&a) || m.progress.visited.contains(&b)) {
                continue;
            }
            let (ax, ay) = to_screen(a.x as f32 * CELL, a.z as f32 * CELL);
            let (bx, by) = to_screen(b.x as f32 * CELL, b.z as f32 * CELL);
            let open = m.door_unlocked(e);
            let col = if open { [214, 186, 128, 220] } else { [90, 90, 110, 200] };
            img.line((ax, ay), (bx, by), (HALL_W / CELL * view.zoom).max(3.0), col);
            if !open {
                // A shut door: a bar across the hall at the border.
                let (cx, cy) = ((ax + bx) * 0.5, (ay + by) * 0.5);
                let half = view.zoom * 0.09;
                if e.east {
                    img.line((cx, cy - half), (cx, cy + half), 3.0, [150, 60, 50, 255]);
                } else {
                    img.line((cx - half, cy), (cx + half, cy), 3.0, [150, 60, 50, 255]);
                }
            }
        }
    }
    // ---- Rooms ----
    let mut wings: BTreeMap<usize, (Vec2, u32)> = BTreeMap::new();
    for r in seen.iter().filter(|r| visible(r)) {
        let room = m.layout.room(*r);
        let theme = &themes[room.theme];
        let (cx, cy) = to_screen(r.x as f32 * CELL, r.z as f32 * CELL);
        let (hw, hh) = (room.size.x / CELL * view.zoom * 0.5, room.size.y / CELL * view.zoom * 0.5);
        let (x0, y0, x1, y1) = ((cx - hw) as i32, (cy - hh) as i32, (cx + hw) as i32, (cy + hh) as i32);
        if m.progress.visited.contains(r) {
            img.rect(x0, y0, x1, y1, srgb(theme.wall, 0.9, 235));
            img.frame(x0, y0, x1, y1, 2, srgb(theme.trim, 1.1, 255));
            let n = format!("{}", r.number());
            let size = (view.zoom * 0.16).clamp(10.0, 22.0);
            let ink = if theme.wall.iter().sum::<f32>() > 1.2 { [30, 26, 24, 230] } else { [240, 232, 214, 230] };
            serif.draw_centred(img, cx, cy - size * 0.55, &n, size, Face::Regular, ink);
            if m.solved(*r) {
                key_glyph(img, cx + hw - size * 0.9, cy - hh + size * 0.9, size * 0.7);
            }
            let e = wings.entry(room.wing_theme).or_insert((Vec2::ZERO, 0));
            e.0 += Vec2::new(cx, cy - hh);
            e.1 += 1;
        } else {
            // Glimpsed: a dotted outline and a question.
            let col = [130, 130, 150, 160];
            let dash = 6;
            for x in (x0..x1).step_by(dash * 2) {
                img.rect(x, y0, (x + dash as i32).min(x1), y0 + 2, col);
                img.rect(x, y1 - 2, (x + dash as i32).min(x1), y1, col);
            }
            for y in (y0..y1).step_by(dash * 2) {
                img.rect(x0, y, x0 + 2, (y + dash as i32).min(y1), col);
                img.rect(x1 - 2, y, x1, (y + dash as i32).min(y1), col);
            }
            serif.draw_centred(img, cx, cy - 10.0, "?", 18.0, Face::Italic, col);
        }
    }
    // ---- Wing names over their rooms ----
    for (theme, (sum, n)) in &wings {
        let at = sum / *n as f32;
        let t = &themes[*theme];
        let size = (view.zoom * 0.17).clamp(13.0, 24.0);
        serif.draw_centred(img, at.x + 1.0, at.y - size * 1.5 + 1.0, &t.wing, size, Face::Italic, [0, 0, 0, 160]);
        serif.draw_centred(img, at.x, at.y - size * 1.5, &t.wing, size, Face::Italic, srgb(t.light, 1.0, 245));
    }
    // ---- You are here ----
    let (px, py) = to_screen(player.0.x, player.0.z);
    let pulse = 1.0 + 0.15 * (time * 3.0).sin();
    img.disc(px, py, 9.0 * pulse, [255, 220, 140, 70]);
    let yaw = player.1;
    let fwd = Vec2::new(-yaw.sin(), -yaw.cos());
    let side = Vec2::new(-fwd.y, fwd.x);
    let tip = Vec2::new(px, py) + fwd * 9.0;
    let l = Vec2::new(px, py) - fwd * 5.0 + side * 6.0;
    let r = Vec2::new(px, py) - fwd * 5.0 - side * 6.0;
    img.line((tip.x, tip.y), (l.x, l.y), 2.5, [255, 236, 190, 255]);
    img.line((tip.x, tip.y), (r.x, r.y), 2.5, [255, 236, 190, 255]);
    img.line((l.x, l.y), (r.x, r.y), 2.5, [255, 236, 190, 255]);
    // ---- Heading and counters ----
    let title = (h * 0.045).clamp(20.0, 40.0);
    serif.draw_centred(img, w * 0.5, h * 0.03, "The Plan of the Museum", title, Face::Italic, [245, 232, 205, 255]);
    let line = format!(
        "{} {} found  \u{00b7}  {} {} discovered  \u{00b7}  {} {} stepped into  \u{00b7}  deepest: {}",
        m.keys(),
        if m.keys() == 1 { "key" } else { "keys" },
        m.rooms_found(),
        if m.rooms_found() == 1 { "room" } else { "rooms" },
        m.progress.entered,
        if m.progress.entered == 1 { "painting" } else { "paintings" },
        match m.progress.deepest {
            0 => "not yet inside a painting".to_string(),
            1 => "one painting in".to_string(),
            n => format!("{n} paintings in"),
        }
    );
    serif.draw_centred(img, w * 0.5, h * 0.03 + title * 1.25, &line, (title * 0.55).max(14.0), Face::Regular, [210, 200, 180, 230]);
}

/// A small old key, drawn at `(x, y)` about `s` pixels long.
pub fn key_glyph(img: &mut Image<'_>, x: f32, y: f32, s: f32) {
    let gold = [240, 196, 96, 255];
    img.disc(x - s * 0.35, y, s * 0.28, gold);
    img.disc(x - s * 0.35, y, s * 0.13, [40, 30, 20, 255]);
    img.line((x - s * 0.1, y), (x + s * 0.5, y), (s * 0.14).max(2.0), gold);
    img.line((x + s * 0.38, y), (x + s * 0.38, y + s * 0.2), (s * 0.12).max(2.0), gold);
    img.line((x + s * 0.5, y), (x + s * 0.5, y + s * 0.26), (s * 0.12).max(2.0), gold);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_map_shows_the_rooms_found_and_marks_the_player() {
        let mut m = Museum::new(super::super::state::tests::spec(), 2, None);
        m.grant_key(RoomId::ORIGIN);
        m.progress.visited.insert(RoomId::new(1, 0));
        let mut px = vec![0u8; 640 * 480 * 4];
        let mut img = Image { w: 640, h: 480, px: &mut px };
        let view = MapView::around(Vec3::ZERO, 480);
        paint(&m, &Serif::bitmap_only(), &mut img, &view, (Vec3::ZERO, 0.0), 0.0);
        // The first room is filled in its wing's wall colour where its middle is drawn.
        let at = |x: u32, y: u32| px[((y * 640 + x) * 4) as usize..((y * 640 + x) * 4 + 4) as usize].to_vec();
        let wall = srgb(m.spec.collection.themes[m.layout.room(RoomId::ORIGIN).theme].wall, 0.9, 235);
        let mid = at(320 + (view.zoom * 0.12) as u32, 240 + (view.zoom * 0.2) as u32);
        assert!((mid[0] as i32 - wall[0] as i32).abs() < 40, "{mid:?} vs {wall:?}");
    }
}
