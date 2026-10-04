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
    &[
        "pause",
        "connect",
        "lobby",
        "countdown",
        "hud",
        "final",
        "death",
        "rules",
        "results",
        "race-hud",
        "race-start",
        "race-results",
        "race-lobby",
        "game-hud",
        "game-start",
        "game-end",
    ]
}

/// Window sizes `ui-check` audits every screen at: small, common, portrait and large.
pub const CHECK_SIZES: [(u32, u32); 9] = [(480, 270), (640, 360), (800, 600), (1024, 768), (1280, 720), (1920, 1080), (2560, 1440), (500, 640), (720, 720)];

/// Per-screen inputs (unused ones are ignored by a screen).
#[derive(Debug, Clone)]
pub struct ScreenOpts {
    /// Map name shown by the screens (`test_lab`).
    pub map: String,
    /// Extra status line (pause menu): may be long, it wraps.
    pub message: Option<String>,
    /// Which pause button is hovered.
    pub hover: Option<PauseAction>,
    /// Which button of the online screens is hovered (`ready`, `character`, `leave`, `connect`, `back`, `field_key`, ...).
    pub hover_id: Option<String>,
    /// Pause menu: whether music is currently on (shown by the MUSIC button's label).
    pub music_on: bool,
    /// Pause menu: whether sound effects are currently on (shown by the SOUND button's label).
    pub sfx_on: bool,
    /// The `game-*` screens: a scene's own `ui` block (`ui-shot --scene`); without one they show a demo game.
    pub game: Option<GameScreen>,
}

/// A scene's `ui` block with the state to draw it in (`ui-shot --scene scene.json --var delivered=3 --outcome victory`).
#[derive(Debug, Clone)]
pub struct GameScreen {
    /// The declaration.
    pub ui: crate::ui_config::GameUi,
    /// Every scene variable and its value.
    pub vars: Vec<(String, f64)>,
    /// The outcome the end card is for (`None`: the first card the block declares).
    pub outcome: Option<String>,
    /// The newest rule event shown under the counters.
    pub event: Option<String>,
}

impl Default for ScreenOpts {
    fn default() -> Self {
        ScreenOpts { map: String::new(), message: None, hover: None, hover_id: None, music_on: true, sfx_on: true, game: None }
    }
}

/// Builds the named screen for a `w` x `h` window, or `None` for an unknown name.
pub fn build(name: &str, w: u32, h: u32, opts: &ScreenOpts) -> Option<Layout> {
    match name {
        "pause" => {
            // With a scene's `ui` block that has a `pause` line, the game's own name and line (`ui-shot pause --scene marcel.json --var days_lived=4`).
            let own = opts.game.as_ref().and_then(|g| {
                let vars: Vec<(&str, f64)> = g.vars.iter().map(|(n, v)| (n.as_str(), *v)).collect();
                g.ui.pause_line(&vars).map(|line| (g.ui.title.clone().unwrap_or_else(|| opts.map.clone()), line))
            });
            Some(match &own {
                Some((name, line)) => pause_layout_with(w, h, PauseInfo::Game { name, line }, opts.message.as_deref(), opts.hover, opts.music_on, opts.sfx_on),
                None => pause_layout(w, h, &opts.map, opts.message.as_deref(), opts.hover, opts.music_on, opts.sfx_on),
            })
        }
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
        "game-hud" | "game-start" | "game-end" => Some(game_screen(name, w, h, opts)),
        "race-hud" => Some(super::race::race_hud_layout(w, h, &super::race::demo_racing())),
        "race-start" => Some(super::race::race_hud_layout(w, h, &super::race::demo_countdown())),
        "race-results" => Some(super::race::race_hud_layout(w, h, &super::race::demo_results())),
        _ => None,
    }
}

/// The `game-*` screens for a scene's own `ui` block, or for a demo game when none is given.
fn game_screen(name: &str, w: u32, h: u32, opts: &ScreenOpts) -> Layout {
    let demo;
    let g = match &opts.game {
        Some(g) => g,
        None => {
            demo = demo_game();
            &demo
        }
    };
    let vars: Vec<(&str, f64)> = g.vars.iter().map(|(n, v)| (n.as_str(), *v)).collect();
    match name {
        "game-start" => match g.ui.start_card(&vars) {
            Some(card) => super::game::card_layout(w, h, &card, "start", opts.hover_id.as_deref() == Some("start")),
            None => Layout::new(w, h),
        },
        "game-end" => {
            let outcome = g.outcome.clone().or_else(|| g.ui.end.first().map(|(o, _)| o.clone())).unwrap_or_default();
            match g.ui.filled_end_card(&outcome, &vars) {
                Some(card) => super::game::card_layout(w, h, &card, "restart", opts.hover_id.as_deref() == Some("restart")),
                None => Layout::new(w, h),
            }
        }
        _ => super::game::hud_layout(w, h, &g.ui, &vars, &vars, g.event.as_deref()),
    }
}

/// A small game for `ui-check` and `ui-shot game-*` to draw when no scene is given.
fn demo_game() -> GameScreen {
    let root = serde_json::json!({
        "vars": {"delivered": 3, "has_key": 1, "time_left": 83},
        "rules": [{"id": "win", "when": {"start": true}, "if": "delivered > 99", "do": [{"end": "victory"}]}],
        "ui": {
            "title": "Moonlight Delivery", "labels": {"has_key": "Key"},
            "counters": [{"var": "delivered", "of": 6, "label": "Parcels"}, {"var": "time_left", "label": "Time", "format": "clock"}],
            "objective": [{"if": "delivered < 6", "text": "Bring every parcel to the depot ({delivered} of 6)"}, {"text": "Open the garden gate"}],
            "start": {"title": "Moonlight Delivery", "text": "Carry six parcels across the sleeping town to the depot before the sun comes up. Pick them up with E."},
            "end": {"victory": {"title": "All delivered!", "text": "You made it with {time_left} seconds to spare."}}
        }
    });
    let parsed = root.as_object().and_then(|o| {
        let rules = crate::sim::rules::parse_rules(o, &crate::sim::rules::Refs::default()).ok()?;
        crate::ui_config::parse_ui(o, &rules).ok().flatten()
    });
    let ui = parsed.unwrap_or_default();
    GameScreen { ui, vars: vec![("delivered".into(), 3.0), ("has_key".into(), 1.0), ("time_left".into(), 83.0)], outcome: None, event: Some("parcel".into()) }
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
            hover_id: Some("ready".into()),
            music_on: false,
            sfx_on: false,
            game: None,
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

/// What a click on the pause menu asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseAction {
    /// Back to the game.
    Resume,
    /// Toggle music on or off (the `N` key does the same); saved, so it survives a relaunch.
    ToggleMusic,
    /// Save the current music loop as a WAV file wherever the player chooses (a native save dialog).
    DownloadMusic,
    /// Toggle sound effects on or off; saved, so it survives a relaunch.
    ToggleSfx,
    /// Toggle borderless fullscreen (the `F` key does the same).
    Fullscreen,
    /// Close the window.
    Quit,
}

/// The pause menu: a dimmed screen and a small panel with RESUME, the audio settings and QUIT GAME. The buttons
/// never move with the message (so clicks are stable); a long `message` wraps and grows the panel downward
/// instead. `music_on`/`sfx_on` decide the ON/OFF label of their buttons.
pub fn pause_layout(w: u32, h: u32, map: &str, message: Option<&str>, hover: Option<PauseAction>, music_on: bool, sfx_on: bool) -> Layout {
    pause_layout_with(w, h, PauseInfo::Map(map), message, hover, music_on, sfx_on)
}

/// What the pause menu says about the game under its title.
#[derive(Debug, Clone, Copy)]
pub enum PauseInfo<'a> {
    /// `MAP: <file name>`, the plain engine menu.
    Map(&'a str),
    /// The game's name and a line of its own (a clean-screen game's day count or score), instead of the map's file name.
    Game {
        /// The game's name.
        name: &'a str,
        /// The line under it.
        line: &'a str,
    },
}

/// [`pause_layout`] with the game's own name and line in place of the map name.
pub fn pause_layout_with(w: u32, h: u32, info: PauseInfo<'_>, message: Option<&str>, hover: Option<PauseAction>, music_on: bool, sfx_on: bool) -> Layout {
    let mut l = Layout::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = (hi / 240).max(1);
    let pw = (190 * s).min(wi - 8);
    // A game's own line takes one more row under the title.
    let extra = if matches!(info, PauseInfo::Game { .. }) { 10 * s } else { 0 };
    let base_h = 215 * s + extra;
    let (x0, y0) = ((wi - pw) / 2, ((hi - base_h) / 2).max(0));
    let cx = x0 + pw / 2;
    let inner = pw - 12 * s;
    let bx = (x0 + 14 * s, x0 + pw - 14 * s);
    let bh = 20 * s;
    let gap = 8 * s;
    let resume_y = y0 + 42 * s + extra;
    let music_y = resume_y + bh + gap;
    let sfx_y = music_y + bh + gap;
    let full_y = sfx_y + bh + gap;
    let quit_y = full_y + bh + gap;
    let hint_y = quit_y + bh + 7 * s;

    // Status lines (at most three) decide how far the panel extends below its base height.
    let status: Vec<String> = message.map(|m| wrap(&m.to_uppercase(), inner, s)).unwrap_or_default().into_iter().take(3).collect();
    let status_y = hint_y + 10 * s;
    let content_end = if status.is_empty() { hint_y + text_height(s) } else { status_y + status.len() as i32 * (text_height(s) + 2 * s) };
    let panel_bottom = (y0 + base_h).max(content_end + 8 * s).min(hi);

    l.panel("backdrop", (0, 0, wi, hi), None, Some([4, 6, 12, 120]), None);
    let panel = l.panel("panel", (x0, y0, x0 + pw, panel_bottom), None, Some([14, 17, 28, 235]), Some(([90, 98, 130, 255], (s / 2).max(2))));
    l.label_fit("title", Some(panel), cx, y0 + 8 * s, "PAUSED", s * 2, inner, TEXT);
    match info {
        PauseInfo::Map(map) => {
            l.label_fit("map", Some(panel), cx, y0 + 28 * s, &format!("MAP: {}", map.to_uppercase()), s, inner, DIM);
        }
        PauseInfo::Game { name, line } => {
            l.label_fit("map", Some(panel), cx, y0 + 28 * s, &name.to_uppercase(), s * 3 / 2, inner, GOLD);
            l.label_fit("game_line", Some(panel), cx, y0 + 41 * s, &line.to_uppercase(), s, inner, TEXT);
        }
    }

    let row = |id: &str, y: i32, bounds: (i32, i32), label: &str, action: PauseAction, l: &mut Layout| {
        let hot = hover == Some(action);
        let scale = fit_scale(label, bounds.1 - bounds.0 - 6 * s, s * 3 / 2);
        l.button(
            id,
            (bounds.0, y, bounds.1, y + bh),
            Some(panel),
            label,
            scale,
            if hot { [56, 62, 92, 255] } else { [30, 34, 52, 255] },
            (if hot { GOLD } else { [90, 98, 130, 255] }, (s / 2).max(1)),
            if hot { GOLD } else { TEXT },
        );
    };
    row("resume", resume_y, bx, "RESUME", PauseAction::Resume, &mut l);
    // The MUSIC row shares its line with the small "save the music" arrow button at the right.
    let arrow_bounds = (bx.1 - bh, bx.1);
    let music_bounds = (bx.0, arrow_bounds.0 - 4 * s);
    row("music", music_y, music_bounds, if music_on { "MUSIC: ON" } else { "MUSIC: OFF" }, PauseAction::ToggleMusic, &mut l);
    row("download_music", music_y, arrow_bounds, "\u{2193}", PauseAction::DownloadMusic, &mut l);
    row("sfx", sfx_y, bx, if sfx_on { "SOUND: ON" } else { "SOUND: OFF" }, PauseAction::ToggleSfx, &mut l);
    row("fullscreen", full_y, bx, "FULLSCREEN  (F)", PauseAction::Fullscreen, &mut l);
    row("quit", quit_y, bx, "QUIT GAME", PauseAction::Quit, &mut l);

    l.label_fit("esc_hint", Some(panel), cx, hint_y, "ESC RESUMES", s, inner, DIM);
    for (i, line) in status.iter().enumerate() {
        l.label(&format!("status_{}", i + 1), Some(panel), cx, status_y + i as i32 * (text_height(s) + 2 * s), line, s, DIM);
    }
    l
}

/// Which pause-menu button, if any, is under the cursor at `(x, y)` in a `w` x `h` window.
pub fn pause_action_at(w: u32, h: u32, x: f32, y: f32) -> Option<PauseAction> {
    match pause_layout(w, h, "", None, None, true, true).button_at(x, y) {
        Some("resume") => Some(PauseAction::Resume),
        Some("music") => Some(PauseAction::ToggleMusic),
        Some("download_music") => Some(PauseAction::DownloadMusic),
        Some("sfx") => Some(PauseAction::ToggleSfx),
        Some("fullscreen") => Some(PauseAction::Fullscreen),
        Some("quit") => Some(PauseAction::Quit),
        _ => None,
    }
}

/// Paints the pause menu as RGBA the size of the window. `hover` highlights a button; `status` is an optional
/// extra line; `music_on`/`sfx_on` set the two toggle buttons' labels.
pub fn paint_pause(w: u32, h: u32, map: &str, status: Option<&str>, hover: Option<PauseAction>, music_on: bool, sfx_on: bool) -> Vec<u8> {
    pause_layout(w, h, map, status, hover, music_on, sfx_on).paint().px
}

/// [`paint_pause`] with the game's own name and line (see [`PauseInfo::Game`]).
pub fn paint_pause_with(w: u32, h: u32, info: PauseInfo<'_>, status: Option<&str>, hover: Option<PauseAction>, music_on: bool, sfx_on: bool) -> Vec<u8> {
    pause_layout_with(w, h, info, status, hover, music_on, sfx_on).paint().px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_games_own_pause_line_replaces_the_map_name_and_the_buttons_stay_in_a_row_below_it() {
        let plain = pause_layout(1280, 720, "marcel", None, None, true, true);
        let own = pause_layout_with(1280, 720, PauseInfo::Game { name: "Marcel", line: "3 days lived" }, None, None, true, true);
        let text = |l: &Layout, id: &str| l.widgets.iter().find(|w| w.id == id).and_then(|w| w.text.clone());
        assert_eq!(text(&plain, "map").as_deref(), Some("MAP: MARCEL"));
        assert_eq!(text(&own, "map").as_deref(), Some("MARCEL"));
        assert_eq!(text(&own, "game_line").as_deref(), Some("3 DAYS LIVED"));
        let (rp, ro) = (plain.widgets.iter().find(|w| w.id == "resume").unwrap().rect, own.widgets.iter().find(|w| w.id == "resume").unwrap().rect);
        assert!(ro.1 > rp.1, "the buttons make room for the extra line");
        // Every button is still where `pause_action_at`-style hit testing finds it.
        assert_eq!(own.button_at(640.0, (ro.1 + ro.3) as f32 / 2.0), Some("resume"));
    }

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
            let l = pause_layout(w, h, "", None, None, true, true);
            let mid = |r: (i32, i32, i32, i32)| (((r.0 + r.2) / 2) as f32, ((r.1 + r.3) / 2) as f32);
            let (rx, ry) = mid(l.rect_of("resume").unwrap());
            let (qx, qy) = mid(l.rect_of("quit").unwrap());
            let (fx, fy) = mid(l.rect_of("fullscreen").unwrap());
            let (mx, my) = mid(l.rect_of("music").unwrap());
            let (dx, dy) = mid(l.rect_of("download_music").unwrap());
            let (sx, sy) = mid(l.rect_of("sfx").unwrap());
            assert_eq!(pause_action_at(w, h, fx, fy), Some(PauseAction::Fullscreen), "{w}x{h}");
            assert_eq!(pause_action_at(w, h, rx, ry), Some(PauseAction::Resume), "{w}x{h}");
            assert_eq!(pause_action_at(w, h, qx, qy), Some(PauseAction::Quit), "{w}x{h}");
            assert_eq!(pause_action_at(w, h, mx, my), Some(PauseAction::ToggleMusic), "{w}x{h}");
            assert_eq!(pause_action_at(w, h, dx, dy), Some(PauseAction::DownloadMusic), "{w}x{h}");
            assert_eq!(pause_action_at(w, h, sx, sy), Some(PauseAction::ToggleSfx), "{w}x{h}");
            assert_eq!(pause_action_at(w, h, 2.0, 2.0), None, "outside the panel");
            // A long message must not move the buttons.
            let long = pause_layout(w, h, "", Some("A LONG MESSAGE THAT WRAPS ONTO SEVERAL LINES OF THE PANEL"), None, true, true);
            assert_eq!(long.rect_of("resume"), l.rect_of("resume"), "{w}x{h}: buttons are anchored");
            // The music/download buttons never overlap: `check()` (run over every screen/size in `audit_all`)
            // already proves this everywhere, but asserting the gap directly here pins the intent down.
            assert!(l.rect_of("music").unwrap().2 <= l.rect_of("download_music").unwrap().0, "{w}x{h}");
        }
    }

    #[test]
    fn the_pause_menu_paints_something_and_hover_changes_it() {
        let (w, h) = (640, 360);
        let plain = paint_pause(w, h, "test_lab", None, None, true, true);
        assert_eq!(plain.len(), (w * h * 4) as usize);
        assert!(plain.chunks(4).any(|p| p[3] > 200 && p[0] > 200 && p[1] > 200), "bright text pixels");
        assert_ne!(plain, paint_pause(w, h, "test_lab", None, Some(PauseAction::Quit), true, true));
        assert_ne!(paint_pause(w, h, "test_lab", None, None, true, true), paint_pause(w, h, "test_lab", Some("online: player 1"), None, true, true));
        // The toggle labels actually reflect the on/off state passed in.
        assert_ne!(paint_pause(w, h, "test_lab", None, None, true, true), paint_pause(w, h, "test_lab", None, None, false, true));
        assert_ne!(paint_pause(w, h, "test_lab", None, None, true, true), paint_pause(w, h, "test_lab", None, None, true, false));
    }
}
