//! The engine's own 2-D screens (pause menu, connect form, lobby, HUDs) built on [`super::Layout`], plus the registry `ui-shot` and
//! `ui-check` use to render and audit any screen at any window size without a GPU.
//!
//! A screen is a pure function `(window size, options) -> Layout`. Painting, click hit-testing and the audit all read
//! the same widget rectangles, so they cannot disagree. Add a screen here (or in a game project, following the same
//! shape), register it in [`build`], and `ui-check` will cover it at every size in [`CHECK_SIZES`].

use super::online::{connect_layout, hud_layout, lobby_layout, results_layout, CombatView, ConnectForm, OnlineView};
use super::rules::hud_layout as rules_hud_layout;
use super::{fit_scale, text_height, wrap, Layout};
use crate::sim::flow::Phase;

const TEXT: [u8; 4] = [236, 238, 245, 255];
const DIM: [u8; 4] = [150, 156, 176, 255];
const GOLD: [u8; 4] = [255, 210, 74, 255];

/// Screens `ui-shot` / `ui-check` know, in display order.
pub fn all() -> &'static [&'static str] {
    &["pause", "connect", "lobby", "countdown", "hud", "final", "death", "rules", "results", "race-hud", "race-start", "race-results", "race-lobby"]
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
    /// Which button of the online screens is hovered (`ready`, `character`, `leave`, `connect`, `back`, `field_key`, ...).
    pub hover_id: Option<String>,
}

/// Builds the named screen for a `w` x `h` window, or `None` for an unknown name.
pub fn build(name: &str, w: u32, h: u32, opts: &ScreenOpts) -> Option<Layout> {
    match name {
        "pause" => Some(pause_layout(w, h, &opts.map, opts.message.as_deref(), opts.hover)),
        "connect" => {
            let mut f = ConnectForm::new("play.example-game-server.net:27015", "correct-horse-battery", "Ada");
            f.message = opts.message.clone();
            Some(connect_layout(w, h, &f, opts.hover_id.as_deref()))
        }
        "lobby" => Some(lobby_layout(w, h, &demo(Phase::Waiting, opts), opts.hover_id.as_deref())),
        "countdown" => Some(hud_layout(w, h, &demo(Phase::Countdown, opts), &crate::hud_config::HudConfig::default())),
        "hud" => Some(hud_layout(w, h, &demo(Phase::Playing, opts), &crate::hud_config::HudConfig::default())),
        "final" => {
            let mut v = demo(Phase::Playing, opts);
            v.roster[5].score = 11; // Fay is one kill from winning
            Some(hud_layout(w, h, &v, &crate::hud_config::HudConfig::default()))
        }
        "death" => {
            let mut v = demo(Phase::Playing, opts);
            v.combat = Some(CombatView::demo_dead());
            Some(hud_layout(w, h, &v, &crate::hud_config::HudConfig::default()))
        }
        "rules" => Some(rules_hud_layout(w, h, &[("score", 3.0), ("coins_left", 1.0)], Some("coin"), opts.message.as_deref())),
        "race-lobby" => {
            let mut v = demo(Phase::Waiting, opts);
            v.race = true;
            for (i, e) in v.roster.iter_mut().enumerate() {
                e.character = ((i + 3) % 8) as u8;
            }
            Some(lobby_layout(w, h, &v, opts.hover_id.as_deref()))
        }
        "results" => Some(results_layout(w, h, &demo(Phase::Results, opts), opts.hover_id.as_deref())),
        "race-hud" => Some(super::race::race_hud_layout(w, h, &super::race::demo_racing())),
        "race-start" => Some(super::race::race_hud_layout(w, h, &super::race::demo_countdown())),
        "race-results" => Some(super::race::race_hud_layout(w, h, &super::race::demo_results())),
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
        ScreenOpts { map: "a_rather_long_map_name_for_the_title".into(), message: Some(long), hover: Some(PauseAction::Quit), hover_id: Some("ready".into()) },
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

/// What a click on the pause menu asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseAction {
    /// Back to the game.
    Resume,
    /// Toggle borderless fullscreen (the `F` key does the same).
    Fullscreen,
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
    let base_h = 150 * s;
    let (x0, y0) = ((wi - pw) / 2, ((hi - base_h) / 2).max(0));
    let cx = x0 + pw / 2;
    let inner = pw - 12 * s;
    let bx = (x0 + 14 * s, x0 + pw - 14 * s);
    let bh = 20 * s;
    let resume_y = y0 + 42 * s;
    let full_y = resume_y + bh + 8 * s;
    let quit_y = full_y + bh + 8 * s;
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
    for (id, y, label, action) in [
        ("resume", resume_y, "RESUME", PauseAction::Resume),
        ("fullscreen", full_y, "FULLSCREEN  (F)", PauseAction::Fullscreen),
        ("quit", quit_y, "QUIT GAME", PauseAction::Quit),
    ] {
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
        Some("fullscreen") => Some(PauseAction::Fullscreen),
        Some("quit") => Some(PauseAction::Quit),
        _ => None,
    }
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
            let (fx, fy) = mid(l.rect_of("fullscreen").unwrap());
            assert_eq!(pause_action_at(w, h, fx, fy), Some(PauseAction::Fullscreen), "{w}x{h}");
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
}
