//! The "RedEngine Games" title card a game plays before it starts, when a player launched it: `RE2_PLAYER=1`, which the game's own
//! `Play-<slug>.exe` and the RedEngine Games app set (`--intro` asks for it by hand). It runs for about two and a half seconds: the name fades in on
//! the site's dark plum, rising a little, a red line draws itself under it, then the card fades to black and the game begins. Any key, click or
//! button skips it.
//!
//! Both players show it: the 2D player (`play2d`) copies it into its window, and the 3D client draws it with an [`crate::overlay::Overlay`].
//! The card is painted on the CPU, at most [`MAX_W`] wide (a bigger window shows it scaled up), into a reused buffer.

use super::serif::{Face, Image, Serif};

/// How long the card plays, in seconds.
pub const SECONDS: f32 = 2.6;
/// The widest the card is painted (pixels).
pub const MAX_W: u32 = 1920;
/// The site's colours (RedEngineGames `site/palette.json`): background, text, accent.
const BACKGROUND: [f32; 3] = [22.0, 11.0, 19.0];
/// How much warmer the middle of the card is than its edges (a soft glow behind the name).
const GLOW: [f32; 3] = [26.0, 7.0, 15.0];
const INK: [u8; 4] = [0xF3, 0xE6, 0xEC, 255];
const ACCENT: [u8; 4] = [0xFF, 0x5C, 0x7A, 255];

/// Whether this launch was made for a player (`RE2_PLAYER=1`): the game then opens in fullscreen and plays the title card first.
pub fn for_player() -> bool {
    std::env::var("RE2_PLAYER").is_ok_and(|v| v.trim() == "1")
}

/// The card, with its lettering painted once per size.
pub struct TitleCard {
    font: Serif,
    layer: Option<Layer>,
}

/// The name on a transparent background (premultiplied RGBA) and where it sits.
struct Layer {
    w: u32,
    h: u32,
    px: Vec<u8>,
    /// Left, top, right and bottom of the name.
    text: [f32; 4],
}

fn smooth(a: f32, b: f32, t: f32) -> f32 {
    let x = ((t - a) / (b - a)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

impl Default for TitleCard {
    fn default() -> TitleCard {
        TitleCard::with_font(Serif::sans_bold())
    }
}

impl TitleCard {
    /// The card lettered in `font` (tests use [`Serif::bitmap_only`] for pixels that do not depend on the machine).
    pub fn with_font(font: Serif) -> TitleCard {
        TitleCard { font, layer: None }
    }

    /// The size the card is painted at for a `w`×`h` window: the same shape, no wider than [`MAX_W`].
    pub fn size_for(w: u32, h: u32) -> (u32, u32) {
        let (w, h) = (w.max(1), h.max(1));
        if w <= MAX_W {
            return (w, h);
        }
        (MAX_W, ((u64::from(h) * u64::from(MAX_W) / u64::from(w)) as u32).max(1))
    }

    fn layer(&mut self, w: u32, h: u32) -> &Layer {
        if !self.layer.as_ref().is_some_and(|l| l.w == w && l.h == h) {
            let parts = [("Red", INK), ("Engine", ACCENT), (" Games", INK)];
            let font = &self.font;
            let width = |size: f32| parts.iter().map(|(s, _)| font.measure(s, size, Face::Regular)).sum::<f32>();
            let mut size = (h as f32 * 0.11).clamp(14.0, 220.0);
            if width(size) > w as f32 * 0.8 {
                size *= w as f32 * 0.8 / width(size);
            }
            let tw = width(size);
            let (x0, y0) = ((w as f32 - tw) * 0.5, h as f32 * 0.5 - size * 0.55);
            let mut px = vec![0u8; (w * h * 4) as usize];
            let mut img = Image { w, h, px: &mut px };
            let mut x = x0;
            for (s, c) in parts {
                x += font.draw(&mut img, x, y0, s, size, Face::Regular, c);
            }
            self.layer = Some(Layer { w, h, px, text: [x0, y0, x0 + tw, y0 + size] });
        }
        self.layer.as_ref().expect("just made")
    }

    /// Paints the card `t` seconds in: opaque RGBA, `w`×`h` (from [`TitleCard::size_for`]), into `out`. False once the card is over, when
    /// nothing is painted.
    pub fn paint(&mut self, w: u32, h: u32, t: f32, out: &mut Vec<u8>) -> bool {
        if !(0.0..SECONDS).contains(&t) {
            return false;
        }
        let (w, h) = (w.max(1), h.max(1));
        let layer = self.layer(w, h);
        let [x0, y0, x1, y1] = layer.text;
        let shown = smooth(0.1, 0.7, t);
        let lift = ((1.0 - shown) * (y1 - y0) * 0.25).round() as i32;
        let fade = 1.0 - smooth(SECONDS - 0.6, SECONDS, t);
        let ink = shown * fade;
        out.clear();
        out.resize((w * h * 4) as usize, 255);
        let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
        let (rx, ry) = (1.0 / (w as f32 * 0.55), 1.0 / (h as f32 * 0.7));
        for y in 0..h as i32 {
            // The name rises into place: this row shows the name's row `lift` above it.
            let from = y - lift;
            let row = (y as u32 * w * 4) as usize;
            let dy = (y as f32 + 0.5 - cy) * ry;
            for x in 0..w as usize {
                let o = row + x * 4;
                let dx = (x as f32 + 0.5 - cx) * rx;
                let glow = (1.0 - dx * dx - dy * dy).max(0.0);
                let mut c = [0, 1, 2].map(|k| (BACKGROUND[k] + GLOW[k] * glow) * fade);
                if ink > 0.0 && (0..h as i32).contains(&from) {
                    let j = (from as u32 * w * 4) as usize + x * 4;
                    let a = f32::from(layer.px[j + 3]) / 255.0 * ink;
                    if a > 0.0 {
                        for (k, v) in c.iter_mut().enumerate() {
                            *v = *v * (1.0 - a) + f32::from(layer.px[j + k]) * ink;
                        }
                    }
                }
                out[o] = c[0].round() as u8;
                out[o + 1] = c[1].round() as u8;
                out[o + 2] = c[2].round() as u8;
            }
        }
        // The line under the name grows from its middle.
        let half = (x1 - x0) * 0.5 * smooth(0.45, 1.15, t);
        let thick = ((y1 - y0) * 0.07).max(2.0);
        let ly = y1 + (y1 - y0) * 0.18 + lift as f32;
        let line = ACCENT.map(|c| (f32::from(c) * fade).round() as u8);
        let mid = (x0 + x1) * 0.5;
        for y in (ly.round() as i32).max(0)..((ly + thick).round() as i32).min(h as i32) {
            for x in ((mid - half).round() as i32).max(0)..((mid + half).round() as i32).min(w as i32) {
                let o = ((y as u32 * w + x as u32) * 4) as usize;
                out[o..o + 3].copy_from_slice(&line[..3]);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(px: &[u8], f: impl Fn(&[u8]) -> bool) -> usize {
        px.chunks(4).filter(|p| f(p)).count()
    }

    #[test]
    fn the_card_fades_in_shows_the_name_and_fades_to_black() {
        for font in [Serif::sans_bold(), Serif::bitmap_only()] {
            let mut card = TitleCard::with_font(font);
            let mut px = Vec::new();
            assert!(card.paint(640, 360, 0.0, &mut px));
            assert_eq!(px.len(), 640 * 360 * 4);
            assert!(px.chunks(4).all(|p| p[0] <= 48 && p[1] <= 18 && p[2] <= 34 && p[3] == 255), "it starts as the background alone");
            assert_eq!(&px[..4], &[22, 11, 19, 255], "the corners are the site's background colour");
            assert!(card.paint(640, 360, 1.5, &mut px));
            let accent = count(&px, |p| p[0] > 200 && p[1] < 140 && p[2] > 90);
            let ink = count(&px, |p| p[0] > 200 && p[1] > 200 && p[2] > 200);
            assert!(accent > 300, "\"Engine\" and the line are drawn in the accent ({accent} px)");
            assert!(ink > 300, "\"Red\" and \"Games\" are drawn in the light ink ({ink} px)");
            assert!(card.paint(640, 360, SECONDS - 0.01, &mut px));
            assert!(px.chunks(4).all(|p| p[0] < 8 && p[1] < 8 && p[2] < 8), "it ends in black, where the game begins");
            assert!(!card.paint(640, 360, SECONDS, &mut px), "then it is over");
        }
    }

    #[test]
    fn the_name_fits_a_narrow_window_and_a_huge_one_is_painted_smaller() {
        let mut card = TitleCard::with_font(Serif::sans_bold());
        let mut px = Vec::new();
        assert!(card.paint(200, 400, 1.5, &mut px));
        let lit = |x: u32| (0..400u32).any(|y| px[((y * 200 + x) * 4) as usize] > 60);
        assert!(!lit(0) && !lit(199), "the name keeps clear of the edges of a narrow window");
        assert_eq!(TitleCard::size_for(3840, 2160), (1920, 1080));
        assert_eq!(TitleCard::size_for(1280, 720), (1280, 720));
    }
}
