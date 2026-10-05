//! The 2D renderer: a CPU rasteriser that draws the virtual screen into an RGBA buffer.
//!
//! The same code makes the pictures in native tests, in `frame` (PNG) and in the browser (the buffer goes to a canvas), so a screenshot taken headless is what the player sees, to the pixel.
//! There is no GPU, no window and no floating-point image resampling: sprites are drawn at integer scales, circles by pixel-centre distance, text from a bitmap font. Scaling the virtual screen
//! to a window of another shape is [`layout`] and [`present`] (nearest neighbour, letterboxed), and [`window_to_view`] maps a pointer back, so resolution independence has one definition.

use crate::font;
use crate::game::*;
use crate::sim::Sim;

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
        for k in 0..3 {
            self.rgba[i + k] = ((c[k] as u32 * a + self.rgba[i + k] as u32 * (255 - a) + 127) / 255) as u8;
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
        for px in self.rgba.chunks_exact(4) {
            seen.insert([px[0], px[1], px[2]]);
            if px[..3] != bg[..3] {
                covered += 1;
            }
            for b in px {
                h = (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3);
            }
        }
        FrameStats { width: self.w, height: self.h, distinct_colors: seen.len(), covered: covered as f32 / (self.w * self.h).max(1) as f32, hash: format!("{h:016x}") }
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

/// Replaces `{var}` and `{var:3}` (zero-padded to 3 digits) with the variables' values.
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
        let (name, width) = match spec.split_once(':') {
            Some((n, w)) => (n, w.parse::<usize>().unwrap_or(0)),
            None => (spec, 0),
        };
        let v = sim.var(name).map_or_else(|| "?".to_string(), fmt_num);
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

/// Draws the current state of the game at its virtual resolution.
pub fn render(sim: &Sim) -> Frame {
    let def = &*sim.def;
    let mut f = Frame::new(def.view.width, def.view.height, def.view.background);
    let off = [sim.cam[0] - sim.shake_off[0], sim.cam[1] - sim.shake_off[1]];
    let mut order: Vec<usize> = (0..sim.entities.len()).filter(|&i| sim.entities[i].alive && !def.prefabs[sim.entities[i].prefab].hidden).collect();
    order.sort_by_key(|&i| (def.prefabs[sim.entities[i].prefab].layer, sim.entities[i].id));
    for i in order {
        let e = &sim.entities[i];
        let p = &def.prefabs[e.prefab];
        let (cx, cy) = (e.x - off[0], e.y - off[1]);
        match &p.shape {
            Shape::None => {}
            Shape::Rect { color } => f.rect((cx - p.size[0] * 0.5).round() as i32, (cy - p.size[1] * 0.5).round() as i32, p.size[0].round() as i32, p.size[1].round() as i32, *color),
            Shape::Circle { color } => f.disc((cx - p.size[0] * 0.5).round() as i32, (cy - p.size[1] * 0.5).round() as i32, p.size[0].round() as i32, *color),
            Shape::Sprite { sprite, scale } => {
                let sp = &def.sprites[*sprite];
                let s = *scale as i32;
                let frame = if sp.frames.len() > 1 { ((sim.tick as f32 * sp.fps / 60.0) as usize) % sp.frames.len() } else { 0 };
                let (x0, y0) = ((cx - (sp.w as i32 * s) as f32 * 0.5).round() as i32, (cy - (sp.h as i32 * s) as f32 * 0.5).round() as i32);
                for (k, px) in sp.frames[frame].iter().enumerate() {
                    if px[3] == 0 {
                        continue;
                    }
                    let (sx, sy) = ((k % sp.w) as i32, (k / sp.w) as i32);
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
    for p in &sim.particles {
        let mut c = p.color;
        let k = (p.life / p.max_life).clamp(0.0, 1.0);
        c[3] = (c[3] as f32 * k) as u8;
        let s = p.size.max(1.0).round() as i32;
        f.rect((p.x - off[0]).round() as i32 - s / 2, (p.y - off[1]).round() as i32 - s / 2, s, s, c);
    }
    let hover = |at: &[f32; 2], size: &[f32; 2]| sim_pointer(sim)[0] >= at[0] && sim_pointer(sim)[0] < at[0] + size[0] && sim_pointer(sim)[1] >= at[1] && sim_pointer(sim)[1] < at[1] + size[1];
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

fn sim_pointer(sim: &Sim) -> [f32; 2] {
    let w = sim.pointer_world();
    [w[0] - sim.cam[0], w[1] - sim.cam[1]]
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
        assert_eq!(fmt_num(2.5), "2.5");
        assert_eq!(fmt_num(3.0), "3");
        assert_eq!(fmt_num(1.0 / 3.0), "0.33");
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
}
