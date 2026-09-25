//! The engine's own 2-D screens (launch menu, pause menu) built on [`super::Layout`], plus the registry `ui-shot` and
//! `ui-check` use to render and audit any screen at any window size without a GPU.
//!
//! A screen is a pure function `(window size, options) -> Layout`. Painting, click hit-testing and the audit all read
//! the same widget rectangles, so they cannot disagree. Add a screen here (or in a game project, following the same
//! shape), register it in [`build`], and `ui-check` will cover it at every size in [`CHECK_SIZES`].

use super::online::{connect_layout, hud_layout, lobby_layout, results_layout, ConnectForm, OnlineView};
use super::{fit_scale, text_height, wrap, Layout};
use crate::player::Character;
use crate::sim::flow::Phase;

const TEXT: [u8; 4] = [236, 238, 245, 255];
const DIM: [u8; 4] = [150, 156, 176, 255];
const GOLD: [u8; 4] = [255, 210, 74, 255];
const SHADE: [u8; 4] = [6, 8, 14, 0];

/// Screens `ui-shot` / `ui-check` know, in display order.
pub fn all() -> &'static [&'static str] {
    &["menu", "pause", "connect", "lobby", "countdown", "hud", "results"]
}

/// Window sizes `ui-check` audits every screen at: small, common, portrait and large.
pub const CHECK_SIZES: [(u32, u32); 9] = [(480, 270), (640, 360), (800, 600), (1024, 768), (1280, 720), (1920, 1080), (2560, 1440), (500, 640), (720, 720)];

/// Per-screen inputs (unused ones are ignored by a screen).
#[derive(Debug, Clone, Default)]
pub struct ScreenOpts {
    /// Map name shown by the screens (`test_lab`).
    pub map: String,
    /// Extra status line (pause menu): may be long, it wraps.
    pub message: Option<String>,
    /// Which pause button is hovered.
    pub hover: Option<PauseAction>,
    /// Launch menu selection.
    pub selected: Option<Character>,
    /// Which button of the online screens is hovered (`ready`, `character`, `leave`, `connect`, `back`, `field_key`, ...).
    pub hover_id: Option<String>,
}

/// Builds the named screen for a `w` x `h` window, or `None` for an unknown name.
pub fn build(name: &str, w: u32, h: u32, opts: &ScreenOpts) -> Option<Layout> {
    match name {
        "menu" => Some(menu_layout(w, h, opts.selected.unwrap_or(Character::Human), &opts.map)),
        "pause" => Some(pause_layout(w, h, &opts.map, opts.message.as_deref(), opts.hover)),
        "connect" => {
            let mut f = ConnectForm::new("play.example-game-server.net:27015", "correct-horse-battery", "Ada");
            f.message = opts.message.clone();
            Some(connect_layout(w, h, &f, opts.hover_id.as_deref()))
        }
        "lobby" => Some(lobby_layout(w, h, &demo(Phase::Waiting, opts), opts.hover_id.as_deref())),
        "countdown" => Some(hud_layout(w, h, &demo(Phase::Countdown, opts))),
        "hud" => Some(hud_layout(w, h, &demo(Phase::Playing, opts))),
        "results" => Some(results_layout(w, h, &demo(Phase::Results, opts), opts.hover_id.as_deref())),
        _ => None,
    }
}

/// The sample match the online screens show in `ui-shot` / `ui-check` (eight players, one with a very long name).
fn demo(phase: Phase, opts: &ScreenOpts) -> OnlineView {
    let mut v = OnlineView::demo(phase);
    if !opts.map.is_empty() {
        v.map = opts.map.clone();
    }
    v.message = opts.message.clone();
    v
}

/// Audits every screen at every [`CHECK_SIZES`] size (and with a long message on the pause screen).
/// Each entry is `(screen, size, violation text)`.
pub fn audit_all() -> Vec<(String, (u32, u32), String)> {
    let long = "CANNOT FIND THAT ADDRESS - CHECK IT AND YOUR INTERNET CONNECTION, THEN TRY AGAIN".to_string();
    let variants = [
        ScreenOpts { map: "test_lab".into(), ..Default::default() },
        ScreenOpts {
            map: "a_rather_long_map_name_for_the_title".into(),
            message: Some(long),
            hover: Some(PauseAction::Quit),
            selected: Some(Character::Rat),
            hover_id: Some("ready".into()),
        },
    ];
    let mut out = Vec::new();
    for name in all() {
        for &(w, h) in &CHECK_SIZES {
            for opts in &variants {
                if let Some(l) = build(name, w, h, opts) {
                    for v in l.check() {
                        out.push((name.to_string(), (w, h), format!("[{}] {}: {}", v.code, v.widget, v.message)));
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Which character is under a cursor at `x` in a `w`-wide window: the left half is the human, the right half the rat.
pub fn character_at(w: u32, x: f32) -> Character {
    if x < w as f32 * 0.5 {
        Character::Human
    } else {
        Character::Rat
    }
}

/// The launch screen ("Human or Cheddar the rat?") for a `w` x `h` window.
pub fn menu_layout(w: u32, h: u32, selected: Character, map: &str) -> Layout {
    let mut l = Layout::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = (hi / 240).max(1); // base text scale: 3 at 720p, 4 at 1080p
    let shade = |a: u8| [SHADE[0], SHADE[1], SHADE[2], a];

    // Dim the card that is not chosen so the choice reads at a glance; bands keep text legible over the 3-D backdrop.
    let unpicked_x0 = if selected == Character::Human { wi / 2 } else { 0 };
    l.panel("unpicked_dim", (unpicked_x0, 0, unpicked_x0 + wi / 2, hi), None, Some(shade(110)), None);
    let top = l.panel("band_top", (0, 0, wi, hi * 22 / 100), None, Some(shade(150)), None);
    let bottom_y = hi * 72 / 100;
    l.panel("band_bottom", (0, bottom_y, wi, hi), None, Some(shade(170)), None);

    let max_w = wi - 8;
    // PLAY ONLINE: a button in the top-right corner of the top band (the O key does the same).
    let (bw, bh, m) = ((98 * s).min(wi / 3), 12 * s, 3 * s);
    l.button(
        "online",
        (wi - bw - m, m, wi - m, m + bh),
        Some(top),
        "PLAY ONLINE (O)",
        fit_scale("PLAY ONLINE (O)", bw - 4 * s, s),
        [30, 34, 52, 255],
        (GOLD, (s / 2).max(1)),
        GOLD,
    );
    l.label_fit("brand", Some(top), wi / 2, hi * 4 / 100, "RED ENGINE 2", s, max_w, DIM);
    l.label_fit("heading", Some(top), wi / 2, hi * 9 / 100, "CHOOSE YOUR CHARACTER", s * 2, max_w, TEXT);
    l.label_fit("map", Some(top), wi / 2, hi * 16 / 100, &format!("MAP: {}", map.to_uppercase()), s, max_w, DIM);

    // The hint sits on the bottom edge; the card text above it must end before it starts.
    let hint = "CLICK A CHARACTER OR PRESS 1 / 2 TO PLAY  (ARROWS + ENTER WORK TOO)  -  O = PLAY ONLINE";
    let hint_scale = fit_scale(hint, max_w, s);
    let hint_y = hi - text_height(hint_scale) - 3 * hint_scale;

    let cards: [(Character, i32, &str, &[&str]); 2] = [
        (Character::Human, wi / 4, "1  HUMAN", &["TALL AND STRONG.", "SWINGS A BAT.", "WALK, OR SPRINT WITH SHIFT."]),
        (
            Character::Rat,
            wi * 3 / 4,
            "2  CHEDDAR THE RAT",
            &["SMALL, QUICK AND HARD TO SPOT.", "RUNS UNDER TABLES AND PLATFORMS.", "FITS THROUGH TIGHT GAPS.", "(SHOWN ABOUT 3X LIFE SIZE)"],
        ),
    ];
    for (who, cx, title, lines) in cards {
        let picked = who == selected;
        let col_w = wi / 2 - 8;
        let column = l.panel(&format!("column_{who:?}").to_lowercase(), (cx - wi / 4, bottom_y, cx + wi / 4, hi), None, None, None);
        let title_y = hi * 74 / 100;
        l.label_fit(&format!("title_{who:?}").to_lowercase(), Some(column), cx, title_y, title, s * 2, col_w, if picked { GOLD } else { TEXT });
        // Body text: the largest scale whose wrapped lines still end above the hint.
        let body_y = title_y + 16 * s;
        let avail = (hint_y - 2 * s) - body_y;
        let body_scale =
            (1..=s).rev().find(|&sc| lines.iter().map(|t| wrap(t, col_w, sc).len() as i32).sum::<i32>() * (text_height(sc) + 2 * sc) <= avail).unwrap_or(1);
        let mut y = body_y;
        for (i, line) in lines.iter().enumerate() {
            y = l.label_wrapped(&format!("body_{who:?}_{i}").to_lowercase(), Some(column), cx, y, line, body_scale, col_w, if picked { TEXT } else { DIM });
        }
        if picked {
            let (x0, x1) = (cx - wi / 4 + 3 * s, cx + wi / 4 - 3 * s);
            let frame = l.panel(&format!("selected_{who:?}").to_lowercase(), (x0, hi * 24 / 100, x1, hi * 71 / 100), None, None, Some((GOLD, (s / 2).max(2))));
            l.label_fit("selected_tag", Some(frame), cx, hi * 71 / 100 - 9 * s - (s / 2).max(2), "< SELECTED >", s, x1 - x0 - 8, GOLD);
        }
    }
    l.label_fit("hint", None, wi / 2, hint_y, hint, s, max_w, DIM);
    l
}

/// What a click on the pause menu asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseAction {
    /// Back to the game.
    Resume,
    /// Close the window.
    Quit,
}

/// The pause menu: a dimmed screen and a small panel with RESUME and QUIT GAME. The buttons never move with the
/// message (so clicks are stable); a long `message` wraps and grows the panel downward instead.
pub fn pause_layout(w: u32, h: u32, map: &str, message: Option<&str>, hover: Option<PauseAction>) -> Layout {
    let mut l = Layout::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = (hi / 240).max(1);
    let pw = (190 * s).min(wi - 8);
    let base_h = 126 * s;
    let (x0, y0) = ((wi - pw) / 2, ((hi - base_h) / 2).max(0));
    let cx = x0 + pw / 2;
    let inner = pw - 12 * s;
    let bx = (x0 + 14 * s, x0 + pw - 14 * s);
    let bh = 20 * s;
    let resume_y = y0 + 42 * s;
    let quit_y = resume_y + bh + 8 * s;
    let hint_y = quit_y + bh + 7 * s;

    // Status lines (at most three) decide how far the panel extends below its base height.
    let status: Vec<String> = message.map(|m| wrap(&m.to_uppercase(), inner, s)).unwrap_or_default().into_iter().take(3).collect();
    let status_y = hint_y + 10 * s;
    let content_end = if status.is_empty() { hint_y + text_height(s) } else { status_y + status.len() as i32 * (text_height(s) + 2 * s) };
    let panel_bottom = (y0 + base_h).max(content_end + 8 * s).min(hi);

    l.panel("backdrop", (0, 0, wi, hi), None, Some([4, 6, 12, 120]), None);
    let panel = l.panel("panel", (x0, y0, x0 + pw, panel_bottom), None, Some([14, 17, 28, 235]), Some(([90, 98, 130, 255], (s / 2).max(2))));
    l.label_fit("title", Some(panel), cx, y0 + 8 * s, "PAUSED", s * 2, inner, TEXT);
    l.label_fit("map", Some(panel), cx, y0 + 28 * s, &format!("MAP: {}", map.to_uppercase()), s, inner, DIM);
    for (id, y, label, action) in [("resume", resume_y, "RESUME", PauseAction::Resume), ("quit", quit_y, "QUIT GAME", PauseAction::Quit)] {
        let hot = hover == Some(action);
        let scale = fit_scale(label, bx.1 - bx.0 - 6 * s, s * 3 / 2);
        l.button(
            id,
            (bx.0, y, bx.1, y + bh),
            Some(panel),
            label,
            scale,
            if hot { [56, 62, 92, 255] } else { [30, 34, 52, 255] },
            (if hot { GOLD } else { [90, 98, 130, 255] }, (s / 2).max(1)),
            if hot { GOLD } else { TEXT },
        );
    }
    l.label_fit("esc_hint", Some(panel), cx, hint_y, "ESC RESUMES", s, inner, DIM);
    for (i, line) in status.iter().enumerate() {
        l.label(&format!("status_{}", i + 1), Some(panel), cx, status_y + i as i32 * (text_height(s) + 2 * s), line, s, DIM);
    }
    l
}

/// Which pause-menu button, if any, is under the cursor at `(x, y)` in a `w` x `h` window.
pub fn pause_action_at(w: u32, h: u32, x: f32, y: f32) -> Option<PauseAction> {
    match pause_layout(w, h, "", None, None).button_at(x, y) {
        Some("resume") => Some(PauseAction::Resume),
        Some("quit") => Some(PauseAction::Quit),
        _ => None,
    }
}

/// Paints the launch menu's text and panels as RGBA the size of the window. `map` is the scene's file stem.
pub fn paint(w: u32, h: u32, selected: Character, map: &str) -> Vec<u8> {
    menu_layout(w, h, selected, map).paint().px
}

/// Paints the pause menu as RGBA the size of the window. `hover` highlights a button; `status` is an optional extra line.
pub fn paint_pause(w: u32, h: u32, map: &str, status: Option<&str>, hover: Option<PauseAction>) -> Vec<u8> {
    pause_layout(w, h, map, status, hover).paint().px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_screen_passes_the_audit_at_every_size() {
        let v = audit_all();
        assert!(
            v.is_empty(),
            "{} layout problem(s):\n{}",
            v.len(),
            v.iter().map(|(s, (w, h), m)| format!("  {s} {w}x{h}: {m}")).collect::<Vec<_>>().join("\n")
        );
    }

    #[test]
    fn the_pause_menu_buttons_are_where_they_are_painted_and_clicks_hit_them() {
        for (w, h) in [(640u32, 360u32), (1024, 768), (1920, 1080), (500, 640)] {
            let l = pause_layout(w, h, "", None, None);
            let mid = |r: (i32, i32, i32, i32)| (((r.0 + r.2) / 2) as f32, ((r.1 + r.3) / 2) as f32);
            let (rx, ry) = mid(l.rect_of("resume").unwrap());
            let (qx, qy) = mid(l.rect_of("quit").unwrap());
            assert_eq!(pause_action_at(w, h, rx, ry), Some(PauseAction::Resume), "{w}x{h}");
            assert_eq!(pause_action_at(w, h, qx, qy), Some(PauseAction::Quit), "{w}x{h}");
            assert_eq!(pause_action_at(w, h, 2.0, 2.0), None, "outside the panel");
            // A long message must not move the buttons.
            let long = pause_layout(w, h, "", Some("A LONG MESSAGE THAT WRAPS ONTO SEVERAL LINES OF THE PANEL"), None);
            assert_eq!(long.rect_of("resume"), l.rect_of("resume"), "{w}x{h}: buttons are anchored");
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
        let gold_left = |img: &[u8]| {
            (0..h).any(|y| {
                (0..w / 2).any(|x| {
                    let p = &img[((y * w + x) * 4) as usize..][..4];
                    p[0] > 240 && p[1] > 190 && p[2] < 100 && p[3] > 240
                })
            })
        };
        assert!(gold_left(&human) && !gold_left(&rat));
    }
}
