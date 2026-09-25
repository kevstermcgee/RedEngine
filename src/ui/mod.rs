//! A tiny pixel-UI toolkit that can be **seen and audited without a window**.
//!
//! A screen is a [`Layout`]: a list of [`Widget`]s (panels, buttons, labels) with their rectangles, text and colours.
//! The same list is (1) painted onto a CPU [`Canvas`] with the engine's 5x7 bitmap font and (2) audited by
//! [`Layout::check`], so what is checked is exactly what is drawn. Nothing here touches a GPU, a window or an
//! audio device: it runs in the headless build, in CI and inside a game project's tests.
//!
//! Why it exists: hand-rolled UI code with magic numbers (`ph = 150 * s`, `y0 + 92 * s`) hides three bug classes that
//! were found the hard way: a taller panel clamped up into the title at 720p, text wider than its panel, and a button
//! rectangle that is not where it is painted. Build screens with the layout helpers instead (`label_fit`, `label_wrapped`
//! measure the text for you), add the screen to [`screens::all`], and `red_engine2 ui-check` / `ui-shot` cover it.
//!
//! * `red_engine2 ui-shot <screen> out.png --size 1280x720 --hover resume --message "..."` renders a screen to a PNG.
//! * `red_engine2 ui-check` audits every screen at many window sizes (also a test, `ui::screens::tests`).

pub mod online;
pub mod screens;

use crate::tools::font::{glyph, GLYPH_H, GLYPH_W};
use image::{Rgb, RgbImage};

/// `(x0, y0, x1, y1)` in pixels, `x1`/`y1` exclusive; row 0 is the top of the window.
pub type Rect = (i32, i32, i32, i32);

/// Width of `text` in the bitmap font at integer `scale`.
pub fn text_width(text: &str, scale: i32) -> i32 {
    let n = text.chars().count() as i32;
    if n == 0 {
        0
    } else {
        n * (GLYPH_W + 1) * scale - scale
    }
}

/// Height of one line of text at `scale`.
pub fn text_height(scale: i32) -> i32 {
    GLYPH_H * scale
}

/// The largest scale `<= want` (never below 1) at which `text` fits in `max_w` pixels.
pub fn fit_scale(text: &str, max_w: i32, want: i32) -> i32 {
    let mut s = want.max(1);
    while s > 1 && text_width(text, s) > max_w {
        s -= 1;
    }
    s
}

/// `text` cut with a trailing `...` so it is no wider than `max_w` at `scale` (unchanged if it already fits).
pub fn ellipsize(text: &str, max_w: i32, scale: i32) -> String {
    if text_width(text, scale) <= max_w {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() && text_width(&format!("{}...", chars.iter().collect::<String>()), scale) > max_w {
        chars.pop();
    }
    format!("{}...", chars.iter().collect::<String>())
}

/// Greedy word-wrap of `text` into lines no wider than `max_w` at `scale` (a single over-long word is cut).
pub fn wrap(text: &str, max_w: i32, scale: i32) -> Vec<String> {
    let per_line = (((max_w + scale) / ((GLYPH_W + 1) * scale)).max(1)) as usize;
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let mut word = word.to_string();
        while word.chars().count() > per_line {
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            lines.push(word.chars().take(per_line).collect());
            word = word.chars().skip(per_line).collect();
        }
        let need = cur.chars().count() + usize::from(!cur.is_empty()) + word.chars().count();
        if need > per_line && !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(&word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// A CPU RGBA canvas (straight alpha, row 0 at the top).
pub struct Canvas {
    /// Width in pixels.
    pub w: i32,
    /// Height in pixels.
    pub h: i32,
    /// RGBA bytes, `w * h * 4`.
    pub px: Vec<u8>,
}

impl Canvas {
    /// A transparent canvas.
    pub fn new(w: u32, h: u32) -> Self {
        Canvas { w: w as i32, h: h as i32, px: vec![0; (w * h * 4) as usize] }
    }

    /// Source-over blend of `c` (RGBA) at `(x, y)`.
    pub fn blend(&mut self, x: i32, y: i32, c: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        let i = ((y * self.w + x) * 4) as usize;
        let a = c[3] as f32 / 255.0;
        let da = self.px[i + 3] as f32 / 255.0;
        let out_a = a + da * (1.0 - a);
        if out_a <= 0.0 {
            return;
        }
        for k in 0..3 {
            let v = (c[k] as f32 * a + self.px[i + k] as f32 * da * (1.0 - a)) / out_a;
            self.px[i + k] = v.round() as u8;
        }
        self.px[i + 3] = (out_a * 255.0).round() as u8;
    }

    /// Fills a rectangle.
    pub fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: [u8; 4]) {
        for y in y0.max(0)..y1.min(self.h) {
            for x in x0.max(0)..x1.min(self.w) {
                self.blend(x, y, c);
            }
        }
    }

    /// Draws a rectangle outline `thick` pixels wide, inside the rectangle.
    pub fn frame(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, thick: i32, c: [u8; 4]) {
        self.rect(x0, y0, x1, y0 + thick, c);
        self.rect(x0, y1 - thick, x1, y1, c);
        self.rect(x0, y0, x0 + thick, y1, c);
        self.rect(x1 - thick, y0, x1, y1, c);
    }

    /// Text with its top-left at `(x, y)`; characters the font lacks are blanks.
    pub fn ink(&mut self, x: i32, y: i32, text: &str, scale: i32, c: [u8; 4]) {
        let mut cx = x;
        for ch in text.chars() {
            if let Some(rows) = glyph(ch) {
                for (ry, row) in rows.iter().enumerate() {
                    for (rx, b) in row.bytes().enumerate() {
                        if b == b'#' {
                            self.rect(cx + rx as i32 * scale, y + ry as i32 * scale, cx + (rx as i32 + 1) * scale, y + (ry as i32 + 1) * scale, c);
                        }
                    }
                }
            }
            cx += (GLYPH_W + 1) * scale;
        }
    }

    /// Text centred on `cx` with its top at `y`, over a soft dark drop shadow.
    pub fn text_centered(&mut self, cx: i32, y: i32, text: &str, scale: i32, c: [u8; 4]) {
        let x = cx - text_width(text, scale) / 2;
        self.ink(x + scale, y + scale, text, scale, [0, 0, 0, (c[3] as f32 * 0.7) as u8]);
        self.ink(x, y, text, scale, c);
    }

    /// The canvas composited over a vertical gradient backdrop (what a screenshot of the UI over a game frame looks like).
    pub fn to_image_over(&self, top: [u8; 3], bottom: [u8; 3]) -> RgbImage {
        let mut img = RgbImage::new(self.w as u32, self.h as u32);
        for y in 0..self.h {
            let t = y as f32 / (self.h - 1).max(1) as f32;
            let bg: Vec<f32> = (0..3).map(|k| top[k] as f32 * (1.0 - t) + bottom[k] as f32 * t).collect();
            for x in 0..self.w {
                let i = ((y * self.w + x) * 4) as usize;
                let a = self.px[i + 3] as f32 / 255.0;
                let c = |k: usize| (self.px[i + k] as f32 * a + bg[k] * (1.0 - a)).round() as u8;
                img.put_pixel(x as u32, y as u32, Rgb([c(0), c(1), c(2)]));
            }
        }
        img
    }
}

/// What a widget is (drives the audit rules; painting is by its style fields).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A container or backdrop (fill and/or frame).
    Panel,
    /// Something clickable; its label must fit inside it.
    Button,
    /// Text; its rectangle is the text's own bounds.
    Label,
}

/// One element of a screen.
#[derive(Debug, Clone)]
pub struct Widget {
    /// Stable name (`resume`, `title`, `status_1`): used in audit messages and hit-testing.
    pub id: String,
    /// What it is.
    pub kind: Kind,
    /// Where it is, in window pixels.
    pub rect: Rect,
    /// Index of the widget this one must stay inside (`None` = the window).
    pub container: Option<usize>,
    /// Text drawn centred in the rectangle (labels: the rectangle *is* the text bounds).
    pub text: Option<String>,
    /// Text scale (pixels per font pixel).
    pub scale: i32,
    /// Text colour.
    pub color: [u8; 4],
    /// Background fill.
    pub fill: Option<[u8; 4]>,
    /// Outline colour and thickness.
    pub frame: Option<([u8; 4], i32)>,
    /// Whether text gets the soft drop shadow.
    pub shadow: bool,
}

/// A problem the audit found.
#[derive(Debug, Clone)]
pub struct Violation {
    /// Stable code: `offscreen`, `outside-container`, `text-overflow`, `overlap`.
    pub code: &'static str,
    /// The widget id it is about.
    pub widget: String,
    /// Numbers and a hint.
    pub message: String,
}

/// A whole screen at one window size.
#[derive(Debug, Clone)]
pub struct Layout {
    /// Window width.
    pub w: i32,
    /// Window height.
    pub h: i32,
    /// Widgets in paint order (later ones draw on top).
    pub widgets: Vec<Widget>,
}

impl Layout {
    /// An empty screen for a `w` x `h` window.
    pub fn new(w: u32, h: u32) -> Self {
        Layout { w: w as i32, h: h as i32, widgets: Vec::new() }
    }

    /// The window rectangle.
    pub fn screen(&self) -> Rect {
        (0, 0, self.w, self.h)
    }

    /// Adds a widget and returns its index (use it as another widget's `container`).
    pub fn add(&mut self, w: Widget) -> usize {
        self.widgets.push(w);
        self.widgets.len() - 1
    }

    /// A panel (backdrop/container) with optional fill and frame.
    pub fn panel(&mut self, id: &str, rect: Rect, container: Option<usize>, fill: Option<[u8; 4]>, frame: Option<([u8; 4], i32)>) -> usize {
        self.add(Widget { id: id.into(), kind: Kind::Panel, rect, container, text: None, scale: 1, color: [255; 4], fill, frame, shadow: false })
    }

    /// A button with a centred label.
    #[allow(clippy::too_many_arguments)]
    pub fn button(
        &mut self,
        id: &str,
        rect: Rect,
        container: Option<usize>,
        label: &str,
        scale: i32,
        fill: [u8; 4],
        frame: ([u8; 4], i32),
        color: [u8; 4],
    ) -> usize {
        self.add(Widget {
            id: id.into(),
            kind: Kind::Button,
            rect,
            container,
            text: Some(label.into()),
            scale,
            color,
            fill: Some(fill),
            frame: Some(frame),
            shadow: true,
        })
    }

    /// A one-line label centred on `cx` with its top at `y`; its rectangle is measured from the text.
    pub fn label(&mut self, id: &str, container: Option<usize>, cx: i32, y: i32, text: &str, scale: i32, color: [u8; 4]) -> usize {
        let tw = text_width(text, scale);
        let x0 = cx - tw / 2;
        self.add(Widget {
            id: id.into(),
            kind: Kind::Label,
            rect: (x0, y, x0 + tw, y + text_height(scale)),
            container,
            text: Some(text.into()),
            scale,
            color,
            fill: None,
            frame: None,
            shadow: true,
        })
    }

    /// Like [`Self::label`] but shrinks the scale (down to 1) until the text fits `max_w`, and ellipsizes it if even that is too wide.
    #[allow(clippy::too_many_arguments)]
    pub fn label_fit(&mut self, id: &str, container: Option<usize>, cx: i32, y: i32, text: &str, want_scale: i32, max_w: i32, color: [u8; 4]) -> usize {
        let scale = fit_scale(text, max_w, want_scale);
        self.label(id, container, cx, y, &ellipsize(text, max_w, scale), scale, color)
    }

    /// Word-wraps `text` into centred lines (`id_1`, `id_2`, ...) no wider than `max_w`; returns the y just below the block.
    #[allow(clippy::too_many_arguments)]
    pub fn label_wrapped(&mut self, id: &str, container: Option<usize>, cx: i32, y: i32, text: &str, scale: i32, max_w: i32, color: [u8; 4]) -> i32 {
        let mut y = y;
        for (i, line) in wrap(text, max_w, scale).iter().enumerate() {
            self.label(&format!("{id}_{}", i + 1), container, cx, y, line, scale, color);
            y += text_height(scale) + scale * 2;
        }
        y
    }

    /// The rectangle of the widget with this id.
    pub fn rect_of(&self, id: &str) -> Option<Rect> {
        self.widgets.iter().find(|w| w.id == id).map(|w| w.rect)
    }

    /// The id of the topmost button under `(x, y)`.
    pub fn button_at(&self, x: f32, y: f32) -> Option<&str> {
        self.widgets.iter().rev().find(|w| w.kind == Kind::Button && inside(w.rect, x, y)).map(|w| w.id.as_str())
    }

    /// Paints every widget in order onto a fresh canvas the size of the window.
    pub fn paint(&self) -> Canvas {
        let mut cv = Canvas::new(self.w as u32, self.h as u32);
        for w in &self.widgets {
            let (x0, y0, x1, y1) = w.rect;
            if let Some(f) = w.fill {
                cv.rect(x0, y0, x1, y1, f);
            }
            if let Some((c, t)) = w.frame {
                cv.frame(x0, y0, x1, y1, t, c);
            }
            if let Some(text) = &w.text {
                let cx = (x0 + x1) / 2;
                let ty = if w.kind == Kind::Label { y0 } else { y0 + (y1 - y0 - text_height(w.scale)) / 2 };
                if w.shadow {
                    cv.text_centered(cx, ty, text, w.scale, w.color);
                } else {
                    cv.ink(cx - text_width(text, w.scale) / 2, ty, text, w.scale, w.color);
                }
            }
        }
        cv
    }

    /// The audit: everything on screen, inside its container, not overflowing, not overlapping a sibling.
    pub fn check(&self) -> Vec<Violation> {
        let mut out = Vec::new();
        let within = |inner: Rect, outer: Rect| inner.0 >= outer.0 && inner.1 >= outer.1 && inner.2 <= outer.2 && inner.3 <= outer.3;
        for (i, w) in self.widgets.iter().enumerate() {
            if !within(w.rect, self.screen()) {
                out.push(Violation { code: "offscreen", widget: w.id.clone(), message: format!("{:?} is outside the {}x{} window", w.rect, self.w, self.h) });
            }
            if let Some(c) = w.container {
                let cr = self.widgets[c].rect;
                if !within(w.rect, cr) {
                    out.push(Violation {
                        code: "outside-container",
                        widget: w.id.clone(),
                        message: format!("{:?} sticks out of '{}' {:?}", w.rect, self.widgets[c].id, cr),
                    });
                }
            }
            if let (Some(t), Kind::Button) = (&w.text, w.kind) {
                let tw = text_width(t, w.scale);
                if tw > w.rect.2 - w.rect.0 || text_height(w.scale) > w.rect.3 - w.rect.1 {
                    out.push(Violation {
                        code: "text-overflow",
                        widget: w.id.clone(),
                        message: format!("label '{t}' is {tw}x{} px but the button is {}x{}", text_height(w.scale), w.rect.2 - w.rect.0, w.rect.3 - w.rect.1),
                    });
                }
            }
            // Text and buttons must not overlap each other anywhere on the screen (panels are backdrops and may).
            for (j, o) in self.widgets.iter().enumerate().skip(i + 1) {
                let solid = |k: Kind| k != Kind::Panel;
                if solid(w.kind) && solid(o.kind) && overlaps(w.rect, o.rect) {
                    out.push(Violation {
                        code: "overlap",
                        widget: w.id.clone(),
                        message: format!("overlaps '{}' ({:?} vs {:?}; widgets {i} and {j})", o.id, w.rect, o.rect),
                    });
                }
            }
        }
        out
    }
}

fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 as f32 && x < r.2 as f32 && y >= r.1 as f32 && y < r.3 as f32
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_and_fitting_respect_the_width() {
        let msg = "CANNOT FIND THAT ADDRESS - CHECK IT AND YOUR INTERNET";
        for max_w in [80, 160, 320] {
            for scale in [1, 2, 3] {
                for line in wrap(msg, max_w, scale) {
                    assert!(text_width(&line, scale) <= max_w, "'{line}' at scale {scale} is wider than {max_w}");
                }
            }
        }
        assert_eq!(wrap("A B", 1000, 1), vec!["A B".to_string()]);
        let cut = ellipsize("A_VERY_LONG_MAP_NAME_INDEED", 60, 1);
        assert!(cut.ends_with("...") && text_width(&cut, 1) <= 60, "{cut}");
        assert_eq!(ellipsize("SHORT", 60, 1), "SHORT");
        assert!(text_width(msg, fit_scale(msg, 200, 3)) <= 200 || fit_scale(msg, 200, 3) == 1);
        assert_eq!(fit_scale("HI", 1000, 3), 3);
    }

    #[test]
    fn the_audit_finds_each_bug_class() {
        let mut l = Layout::new(200, 100);
        let panel = l.panel("panel", (20, 10, 120, 60), None, Some([20, 20, 30, 255]), None);
        l.button("ok", (30, 20, 60, 30), Some(panel), "A VERY LONG LABEL", 1, [0; 4], ([0; 4], 1), [255; 4]);
        l.label("wide", Some(panel), 70, 40, "THIS TEXT IS WIDER THAN ITS PANEL", 1, [255; 4]);
        l.label("off", None, 190, 95, "OFF", 2, [255; 4]);
        l.label("clash", Some(panel), 70, 42, "OVERLAPS", 1, [255; 4]);
        let codes: Vec<&str> = l.check().iter().map(|v| v.code).collect();
        for want in ["text-overflow", "outside-container", "offscreen", "overlap"] {
            assert!(codes.contains(&want), "{want} not reported: {codes:?}");
        }
        // A clean layout reports nothing, and painting it produces pixels where the widgets are.
        let mut ok = Layout::new(200, 100);
        let p = ok.panel("p", (10, 10, 190, 90), None, Some([200, 0, 0, 255]), None);
        ok.label("t", Some(p), 100, 20, "HELLO", 2, [255, 255, 255, 255]);
        assert!(ok.check().is_empty(), "{:?}", ok.check());
        let cv = ok.paint();
        assert_eq!(&cv.px[((50 * 200 + 100) * 4) as usize..][..4], &[200, 0, 0, 255]);
    }

    #[test]
    fn hit_testing_uses_the_same_rectangles_as_painting() {
        let mut l = Layout::new(100, 100);
        l.button("a", (10, 10, 50, 30), None, "A", 1, [1; 4], ([2; 4], 1), [255; 4]);
        assert_eq!(l.button_at(20.0, 20.0), Some("a"));
        assert_eq!(l.button_at(60.0, 20.0), None);
    }
}
