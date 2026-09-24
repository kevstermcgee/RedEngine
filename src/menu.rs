//! The launch screen: "Human or Cheddar the rat?".
//!
//! Two pieces, both free of window/GPU types so they are unit-testable:
//! * [`menu_scene`] / [`animate`] — a tiny 3-D backdrop (a dark studio with a pedestal under each
//!   character, both models slowly turning) drawn by the ordinary live renderer, and
//! * [`paint`] — the 2-D text and panels, painted on the CPU into an RGBA image the size of the
//!   window with the engine's own 5x7 bitmap font and shown through `crate::overlay`.
//!
//! [`character_at`] maps a cursor position to a character so a click picks one.

use crate::characters::{human_object, rat_object};
use crate::player::Character;
use crate::schema::{Background, Camera, Light, LightKind, Material, Object, ObjectKind, PostSettings, PrimKind, Scene};
use crate::tools::font::{glyph, GLYPH_H, GLYPH_W};
use crate::track::Track;
use crate::viewer::FpsCamera;
use glam::Vec3;

/// The rat is drawn this many times life size so its face reads next to a 1.8 m human.
pub const RAT_PREVIEW_SCALE: f32 = 3.6;
/// Height of the podium the enlarged rat stands on, so it sits at a person's chest height.
const RAT_PODIUM: f32 = 0.55;

const HUMAN_INDEX: usize = 2;
const RAT_INDEX: usize = 4;
const CAMERA_DISTANCE: f32 = 5.0;
const CAMERA_FOV_DEG: f32 = 46.0;

fn hex(s: &str) -> Vec3 {
    crate::color::parse_hex_to_linear(s).expect("valid menu colour")
}

fn constant_material(color: &str, roughness: f32) -> Material {
    Material { color: Track::constant(hex(color)), metallic: 0.0, roughness, emissive: Vec3::ZERO }
}

fn pedestal(id: &str, radius: f32, height: f32) -> Object {
    Object {
        id: id.to_string(),
        position: Track::constant(Vec3::new(0.0, height * 0.5, 0.0)),
        rotation: Track::constant(Vec3::ZERO),
        scale: Track::constant(Vec3::ONE),
        material: Some(constant_material("#2c2f3a", 0.5)),
        collide: false,
        prefab: None,
        movable: None,
        kind: ObjectKind::Prim(PrimKind::Cylinder { radius, height }),
    }
}

fn point_light(id: &str, pos: Vec3, color: &str, intensity: f32, range: f32) -> Light {
    Light {
        id: id.to_string(),
        kind: LightKind::Point { position: Track::constant(pos), range },
        color: Track::constant(hex(color)),
        intensity: Track::constant(intensity),
        cast_shadows: false,
        shadow_radius: 10.0,
        shadow_center: Vec3::ZERO,
    }
}

/// The backdrop scene: a dark gradient studio, a pedestal and a model for each character.
/// Objects: 0 floor, 1/2 human pedestal + human, 3/4 rat pedestal + rat. [`animate`] places them.
pub fn menu_scene() -> Scene {
    let floor = Object {
        id: "floor".to_string(),
        position: Track::constant(Vec3::ZERO),
        rotation: Track::constant(Vec3::ZERO),
        scale: Track::constant(Vec3::ONE),
        material: Some(constant_material("#1a1c24", 0.8)),
        collide: false,
        prefab: None,
        movable: None,
        kind: ObjectKind::Prim(PrimKind::Plane { size: (40.0, 40.0) }),
    };
    let objects = vec![floor, pedestal("human_pedestal", 1.05, 0.06), human_object("human"), pedestal("rat_pedestal", 1.45, RAT_PODIUM), rat_object("rat")];
    debug_assert!(matches!(objects[HUMAN_INDEX].kind, ObjectKind::Humanoid(_)) && matches!(objects[RAT_INDEX].kind, ObjectKind::Rat(_)));
    Scene {
        fps: 30,
        duration: 0.0,
        width: 1280,
        height: 720,
        background: Background::Gradient { top: hex("#0b0d14"), bottom: hex("#23283a") },
        ambient_color: hex("#9aa4c4"),
        ambient_intensity: 0.22,
        camera: Camera {
            fov: Track::constant(CAMERA_FOV_DEG),
            near: 0.05,
            far: 100.0,
            position: Track::constant(Vec3::new(0.0, 1.1, CAMERA_DISTANCE)),
            target: Track::constant(Vec3::new(0.0, 0.95, 0.0)),
            roll: Track::constant(0.0),
        },
        post: PostSettings::default(),
        lights: vec![
            point_light("key", Vec3::new(-2.5, 3.2, 3.0), "#ffe6c4", 26.0, 14.0),
            point_light("rim", Vec3::new(2.8, 2.6, -1.5), "#9cc0ff", 20.0, 12.0),
            point_light("fill", Vec3::new(0.0, 1.2, 4.5), "#ffffff", 9.0, 12.0),
        ],
        objects,
    }
}

/// The camera that frames [`menu_scene`].
pub fn menu_camera() -> FpsCamera {
    let mut cam = FpsCamera::new(Vec3::new(0.0, 1.1, CAMERA_DISTANCE), 0.0);
    cam.fov_deg = CAMERA_FOV_DEG;
    cam.pitch = -0.08;
    cam
}

/// Where a character's model stands for a window of `aspect` (so it sits over its own half of the
/// screen whatever the window shape): x of the character's centre, in world units.
fn slot_x(aspect: f32, which: Character) -> f32 {
    let half_width = CAMERA_DISTANCE * (CAMERA_FOV_DEG.to_radians() * 0.5).tan() * aspect;
    let x = half_width * 0.5;
    match which {
        Character::Human => -x,
        Character::Rat => x,
    }
}

/// Places and turns the two models: each slowly sways, the `selected` one a little wider and
/// faster, and the rat trots on the spot while it is the selected one.
pub fn animate(scene: &mut Scene, aspect: f32, time: f32, selected: Character) {
    let place = |scene: &mut Scene, ped: usize, model: usize, which: Character, scale: f32, y: f32| {
        let x = slot_x(aspect, which);
        let picked = which == selected;
        let sway = (time * if picked { 0.9 } else { 0.5 }).sin() * if picked { 38.0 } else { 16.0 };
        scene.objects[ped].position = Track::constant(Vec3::new(x, y * 0.5, 0.0));
                let m = &mut scene.objects[model];
        m.position = Track::constant(Vec3::new(x, y, 0.0));
        m.rotation = Track::constant(Vec3::new(0.0, sway, 0.0));
        m.scale = Track::constant(Vec3::splat(scale));
    };
    place(scene, 1, HUMAN_INDEX, Character::Human, 1.0, 0.06);
    place(scene, 3, RAT_INDEX, Character::Rat, RAT_PREVIEW_SCALE, RAT_PODIUM);
    if let ObjectKind::Rat(r) = &mut scene.objects[RAT_INDEX].kind {
        let trot = selected == Character::Rat;
        r.gait = Track::constant(if trot { time * 14.0 } else { 0.0 });
        r.stride = Track::constant(if trot { 0.55 } else { 0.0 });
        r.sway = Track::constant(time * 1.3);
    }
}

// ---------------------------------------------------------------------------------------------
// 2-D painting
// ---------------------------------------------------------------------------------------------

/// A CPU RGBA canvas (straight alpha, row 0 at the top).
struct Canvas {
    w: i32,
    h: i32,
    px: Vec<u8>,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Self {
        Canvas { w: w as i32, h: h as i32, px: vec![0; (w * h * 4) as usize] }
    }

    /// Source-over blend of `c` (RGBA) at `(x, y)`.
    fn blend(&mut self, x: i32, y: i32, c: [u8; 4]) {
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

    fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: [u8; 4]) {
        for y in y0.max(0)..y1.min(self.h) {
            for x in x0.max(0)..x1.min(self.w) {
                self.blend(x, y, c);
            }
        }
    }

    fn frame(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, thick: i32, c: [u8; 4]) {
        self.rect(x0, y0, x1, y0 + thick, c);
        self.rect(x0, y1 - thick, x1, y1, c);
        self.rect(x0, y0, x0 + thick, y1, c);
        self.rect(x1 - thick, y0, x1, y1, c);
    }

    fn text_width(text: &str, scale: i32) -> i32 {
        let n = text.chars().count() as i32;
        if n == 0 { 0 } else { n * (GLYPH_W + 1) * scale - scale }
    }

    fn ink(&mut self, x: i32, y: i32, text: &str, scale: i32, c: [u8; 4]) {
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
    fn text_centered(&mut self, cx: i32, y: i32, text: &str, scale: i32, c: [u8; 4]) {
        let x = cx - Self::text_width(text, scale) / 2;
        self.ink(x + scale, y + scale, text, scale, [0, 0, 0, (c[3] as f32 * 0.7) as u8]);
        self.ink(x, y, text, scale, c);
    }
}

/// Which character is under a cursor at `(x, y)` in a `w`-wide window: the left half is the
/// human, the right half the rat.
pub fn character_at(w: u32, x: f32) -> Character {
    if x < w as f32 * 0.5 { Character::Human } else { Character::Rat }
}

/// Paints the menu's text and panels for a `w` x `h` window. `map` is the scene's file stem.
pub fn paint(w: u32, h: u32, selected: Character, map: &str) -> Vec<u8> {
    let mut cv = Canvas::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = (hi / 240).max(1); // base text scale: 3 at 720p, 4 at 1080p
    let text = [236, 238, 245, 255];
    let dim = [150, 156, 176, 255];
    let gold = [255, 210, 74, 255];

    // Dim the card that is not chosen so the choice reads at a glance.
    let (half_x, unpicked_x0) = (wi / 2, if selected == Character::Human { wi / 2 } else { 0 });
    cv.rect(unpicked_x0, 0, unpicked_x0 + half_x, hi, [6, 8, 14, 110]);

    // Top and bottom bands so text is legible over whatever the 3-D backdrop is doing.
    cv.rect(0, 0, wi, hi * 22 / 100, [6, 8, 14, 150]);
    cv.rect(0, hi * 72 / 100, wi, hi, [6, 8, 14, 170]);

    cv.text_centered(wi / 2, hi * 4 / 100, "RED ENGINE 2", s, dim);
    cv.text_centered(wi / 2, hi * 9 / 100, "CHOOSE YOUR CHARACTER", s * 2, text);
    let map_line = format!("MAP: {}", map.to_uppercase());
    cv.text_centered(wi / 2, hi * 16 / 100, &map_line, s, dim);

    let cards = [
        (Character::Human, wi / 4, "1  HUMAN", ["TALL AND STRONG.", "SWINGS A BAT.", "WALK, OR SPRINT WITH SHIFT.", ""]),
        (Character::Rat, wi * 3 / 4, "2  CHEDDAR THE RAT", ["SMALL, QUICK AND HARD TO SPOT.", "RUNS UNDER TABLES AND PLATFORMS.", "FITS THROUGH TIGHT GAPS.", "(SHOWN ABOUT 3X LIFE SIZE)"]),
    ];
    for (who, cx, title, lines) in cards {
        let picked = who == selected;
        let title_c = if picked { gold } else { text };
        cv.text_centered(cx, hi * 74 / 100, title, s * 2, title_c);
        for (i, line) in lines.iter().enumerate() {
            cv.text_centered(cx, hi * 74 / 100 + (16 * s) + i as i32 * (10 * s), line, s, if picked { text } else { dim });
        }
        if picked {
            let (x0, x1) = (cx - wi / 4 + 3 * s, cx + wi / 4 - 3 * s);
            cv.frame(x0, hi * 24 / 100, x1, hi * 71 / 100, (s / 2).max(2), gold);
            cv.text_centered(cx, hi * 71 / 100 - 9 * s, "< SELECTED >", s, gold);
        }
    }

    cv.text_centered(wi / 2, hi - GLYPH_H * s - 3 * s, "CLICK A CHARACTER OR PRESS 1 / 2 TO PLAY  (ARROWS + ENTER WORK TOO)", s, dim);
    cv.px
}

/// What a click on the pause menu asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseAction {
    /// Back to the game.
    Resume,
    /// Close the window.
    Quit,
}

/// Rectangles `(x0, y0, x1, y1)` of the pause menu for a `w` x `h` window.
struct PauseLayout {
    panel: (i32, i32, i32, i32),
    resume: (i32, i32, i32, i32),
    quit: (i32, i32, i32, i32),
    scale: i32,
}

fn pause_layout(w: u32, h: u32) -> PauseLayout {
    let (wi, hi) = (w as i32, h as i32);
    let s = (hi / 240).max(1);
    let pw = (190 * s).min(wi - 8);
    let ph = 126 * s;
    let (x0, y0) = ((wi - pw) / 2, (hi - ph) / 2);
    let (bx0, bx1, bh) = (x0 + 14 * s, x0 + pw - 14 * s, 20 * s);
    let resume_y = y0 + 42 * s;
    let quit_y = resume_y + bh + 8 * s;
    PauseLayout { panel: (x0, y0, x0 + pw, y0 + ph), resume: (bx0, resume_y, bx1, resume_y + bh), quit: (bx0, quit_y, bx1, quit_y + bh), scale: s }
}

fn inside(r: (i32, i32, i32, i32), x: f32, y: f32) -> bool {
    x >= r.0 as f32 && x < r.2 as f32 && y >= r.1 as f32 && y < r.3 as f32
}

/// Which pause-menu button, if any, is under the cursor at `(x, y)` in a `w` x `h` window.
pub fn pause_action_at(w: u32, h: u32, x: f32, y: f32) -> Option<PauseAction> {
    let l = pause_layout(w, h);
    if inside(l.resume, x, y) {
        Some(PauseAction::Resume)
    } else if inside(l.quit, x, y) {
        Some(PauseAction::Quit)
    } else {
        None
    }
}

/// Paints the pause menu (a dimmed screen, a small panel with RESUME and QUIT GAME) as an RGBA image the
/// size of the window. `hover` highlights a button; `status` is an optional extra line (e.g. the online
/// connection).
pub fn paint_pause(w: u32, h: u32, map: &str, status: Option<&str>, hover: Option<PauseAction>) -> Vec<u8> {
    let mut cv = Canvas::new(w, h);
    let l = pause_layout(w, h);
    let s = l.scale;
    let (text, dim, gold) = ([236, 238, 245, 255], [150, 156, 176, 255], [255, 210, 74, 255]);
    cv.rect(0, 0, w as i32, h as i32, [4, 6, 12, 120]);
    let (x0, y0, x1, y1) = l.panel;
    cv.rect(x0, y0, x1, y1, [14, 17, 28, 235]);
    cv.frame(x0, y0, x1, y1, (s / 2).max(2), [90, 98, 130, 255]);
    let cx = (x0 + x1) / 2;
    cv.text_centered(cx, y0 + 8 * s, "PAUSED", s * 2, text);
    let map_line = format!("MAP: {}", map.to_uppercase());
    cv.text_centered(cx, y0 + 28 * s, &map_line, s, dim);
    for (rect, label, action) in [(l.resume, "RESUME", PauseAction::Resume), (l.quit, "QUIT GAME", PauseAction::Quit)] {
        let hot = hover == Some(action);
        cv.rect(rect.0, rect.1, rect.2, rect.3, if hot { [56, 62, 92, 255] } else { [30, 34, 52, 255] });
        cv.frame(rect.0, rect.1, rect.2, rect.3, (s / 2).max(1), if hot { gold } else { [90, 98, 130, 255] });
        cv.text_centered((rect.0 + rect.2) / 2, rect.1 + (rect.3 - rect.1 - GLYPH_H * s * 3 / 2) / 2, label, s * 3 / 2, if hot { gold } else { text });
    }
    let hint_y = l.quit.3 + 7 * s;
    cv.text_centered(cx, hint_y, "ESC RESUMES", s, dim);
    if let Some(st) = status {
        cv.text_centered(cx, hint_y + 10 * s, &st.to_uppercase(), s, dim);
    }
    cv.px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pause_menu_buttons_are_where_they_are_painted_and_clicks_hit_them() {
        for (w, h) in [(640u32, 360u32), (1024, 768), (1920, 1080), (500, 640)] {
            let l = pause_layout(w, h);
            let mid = |r: (i32, i32, i32, i32)| (((r.0 + r.2) / 2) as f32, ((r.1 + r.3) / 2) as f32);
            let (rx, ry) = mid(l.resume);
            let (qx, qy) = mid(l.quit);
            assert_eq!(pause_action_at(w, h, rx, ry), Some(PauseAction::Resume), "{w}x{h}");
            assert_eq!(pause_action_at(w, h, qx, qy), Some(PauseAction::Quit), "{w}x{h}");
            assert_eq!(pause_action_at(w, h, 2.0, 2.0), None, "outside the panel");
            assert!(l.panel.0 >= 0 && l.panel.2 <= w as i32 && l.panel.3 <= h as i32, "the panel fits a {w}x{h} window");
            assert!(l.resume.3 < l.quit.1, "buttons do not overlap");
        }
    }

    #[test]
    fn the_pause_menu_paints_something_and_hover_changes_it() {
        let (w, h) = (640, 360);
        let plain = paint_pause(w, h, "test_lab", None, None);
        assert_eq!(plain.len(), (w * h * 4) as usize);
        assert!(plain.chunks(4).any(|p| p[3] > 200 && p[0] > 200 && p[1] > 200), "bright text pixels");
        assert_ne!(plain, paint_pause(w, h, "test_lab", None, Some(PauseAction::Quit)));
        assert_ne!(paint_pause(w, h, "test_lab", None, None), paint_pause(w, h, "test_lab", Some("online: player 1"), None));
    }

    #[test]
    fn scene_has_the_two_models_where_animate_expects() {
        let mut scene = menu_scene();
        animate(&mut scene, 16.0 / 9.0, 1.0, Character::Rat);
        assert!(matches!(scene.objects[HUMAN_INDEX].kind, ObjectKind::Humanoid(_)));
        assert!(matches!(scene.objects[RAT_INDEX].kind, ObjectKind::Rat(_)));
        // The two models stand on opposite sides of the screen centre.
        let (hx, rx) = (scene.objects[HUMAN_INDEX].position.sample(0.0).x, scene.objects[RAT_INDEX].position.sample(0.0).x);
        assert!(hx < -0.5 && rx > 0.5, "{hx} {rx}");
    }

    #[test]
    fn clicks_pick_the_side_they_land_on() {
        assert_eq!(character_at(1000, 100.0), Character::Human);
        assert_eq!(character_at(1000, 900.0), Character::Rat);
    }

    #[test]
    fn paint_draws_text_and_marks_the_selection() {
        let (w, h) = (640, 360);
        let human = paint(w, h, Character::Human, "house");
        let rat = paint(w, h, Character::Rat, "house");
        assert_eq!(human.len(), (w * h * 4) as usize);
        assert!(human.chunks(4).any(|p| p[3] > 200 && p[0] > 200 && p[1] > 200), "some bright opaque text pixels");
        assert_ne!(human, rat, "the selection changes what is painted");
        // Gold frame pixels appear only on the selected side.
        let gold_left = |img: &[u8]| (0..h).any(|y| (0..w / 2).any(|x| { let p = &img[((y * w + x) * 4) as usize..][..4]; p[0] > 240 && p[1] > 190 && p[2] < 100 && p[3] > 240 }));
        assert!(gold_left(&human) && !gold_left(&rat));
    }
}
