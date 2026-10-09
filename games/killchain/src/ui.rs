//! Killchain's 2-D screens (ADR 2026-09-30-killchain-loadout-shooter): the home menu, match setup, joining, the stats page, the team lobby, the
//! in-game HUD, the killcam caption, the scoreboard and the end-of-match menu, plus the scope picture.
//!
//! Every screen is a pure function of a window size and a small view struct to a [`Layout`], exactly like the engine's own screens, so they
//! are audited (`audit_all`) at every window size and can be rendered to a picture without a window. The HUD is deliberately tiny: your
//! health, the ammunition of the weapon in hand, the score and the clock, and nothing else.

use crate::objective_hud::{Marker, ObjectiveHud};
use crate::stats::Stats;
use red_engine2::net::protocol::{RosterEntry, ROSTER_BOT, ROSTER_READY};
use red_engine2::ui::online::ConnectForm;
use red_engine2::ui::{ellipsize, fit_scale, text_height, text_width, Canvas, Layout};
use red_engine2::uniforms::TEAM_NAMES;

const TEXT: [u8; 4] = [236, 238, 240, 255];
const DIM: [u8; 4] = [150, 156, 160, 255];
const ACCENT: [u8; 4] = [232, 98, 48, 255];
const PANEL: [u8; 4] = [10, 12, 14, 215];
const BUTTON: [u8; 4] = [24, 27, 30, 235];
const BUTTON_HOT: [u8; 4] = [48, 40, 34, 245];
const EDGE: [u8; 4] = [78, 84, 88, 255];
/// Ridgeback's colour on screen (army green, lifted to read on dark).
pub const RIDGEBACK: [u8; 4] = [150, 172, 78, 255];
/// Nightfall's colour on screen (navy, lifted to read on dark).
pub const NIGHTFALL: [u8; 4] = [92, 136, 214, 255];

/// A team's colour on screen (`0` is neutral).
pub fn team_color(team: u8) -> [u8; 4] {
    match team {
        1 => RIDGEBACK,
        2 => NIGHTFALL,
        _ => TEXT,
    }
}

fn scale_for(h: u32) -> i32 {
    (h as i32 / 240).max(1)
}

fn upper(s: &str) -> String {
    s.to_uppercase()
}

/// Screens the audit and the picture tool know.
pub fn all() -> &'static [&'static str] {
    &[
        "home",
        "solo",
        "host",
        "join",
        "stats",
        "lobby",
        "lobby-ffa",
        "lobby-duel",
        "results-ffa",
        "countdown",
        "hud",
        "hud-low",
        "hud-ffa",
        "hud-ctf",
        "hud-snd",
        "killcam",
        "scoreboard",
        "results",
        "results-draw",
        "pause",
    ]
}

/// A button with the game's look; `hot` highlights it.
#[allow(clippy::too_many_arguments)]
fn btn(l: &mut Layout, id: &str, rect: (i32, i32, i32, i32), container: Option<usize>, label: &str, scale: i32, hot: bool, selected: bool) {
    let (fill, frame, color) = if selected {
        ([62, 46, 36, 250], ACCENT, [255, 226, 200, 255])
    } else if hot {
        (BUTTON_HOT, ACCENT, [255, 214, 170, 255])
    } else {
        (BUTTON, EDGE, TEXT)
    };
    let s = fit_both(label, rect, scale);
    l.button(id, rect, container, label, s, fill, (frame, 1.max(scale / 2)), color);
}

/// The largest scale at most `want` at which `label` fits the button's width (with a little margin) and its height.
fn fit_both(label: &str, rect: (i32, i32, i32, i32), want: i32) -> i32 {
    let mut s = fit_scale(label, rect.2 - rect.0 - 2 * want.max(1), want);
    while s > 1 && text_height(s) > rect.3 - rect.1 {
        s -= 1;
    }
    s
}

/// A menu screen's frame: a dark wash, a left-hand panel with the game's name over an accent rule. Returns `(layout, panel index, panel rect, scale)`.
fn frame(w: u32, h: u32, subtitle: &str) -> (Layout, usize, (i32, i32, i32, i32), i32) {
    let mut l = Layout::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = scale_for(h);
    l.panel("wash", (0, 0, wi, hi), None, Some([3, 4, 6, 120]), None);
    let pw = (150 * s).min(wi - 8).max(wi / 3);
    let rect = (0, 0, pw, hi);
    let p = l.panel("panel", rect, None, Some([8, 10, 12, 226]), None);
    l.panel("rule", (pw - 2.max(s / 2), 0, pw, hi), Some(p), Some(ACCENT), None);
    let cx = (pw - 2) / 2;
    l.label_fit("title", Some(p), cx, 10 * s, "KILLCHAIN", s * 4, pw - 12 * s, TEXT);
    l.label_fit("subtitle", Some(p), cx, 10 * s + text_height(s * 4) + 3 * s, &upper(subtitle), s, pw - 12 * s, ACCENT);
    (l, p, rect, s)
}

// ---------------------------------------------------------------------------------------------------------------------------------
// home
// ---------------------------------------------------------------------------------------------------------------------------------

/// What the home screen's buttons do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeAction {
    /// Play against bots or alone on this machine.
    Solo,
    /// Start a game others can join.
    Host,
    /// Join somebody's game.
    Join,
    /// Show the lifetime statistics.
    Stats,
    /// Close the game.
    Quit,
}

/// The home screen's buttons, top to bottom: `(id, label, action)`.
pub const HOME_BUTTONS: [(&str, &str, HomeAction); 5] = [
    ("solo", "SOLO", HomeAction::Solo),
    ("host", "HOST", HomeAction::Host),
    ("join", "JOIN", HomeAction::Join),
    ("stats", "STATS", HomeAction::Stats),
    ("quit", "QUIT", HomeAction::Quit),
];

/// The action of a home-screen button id.
pub fn home_action(id: &str) -> Option<HomeAction> {
    HOME_BUTTONS.iter().find(|b| b.0 == id).map(|b| b.2)
}

/// The home screen: the name, five buttons, the version.
pub fn home_layout(w: u32, h: u32, hover: Option<&str>, version: &str) -> Layout {
    let (mut l, p, (x0, _, x1, _), s) = frame(w, h, "team deathmatch");
    let (bx0, bx1) = (x0 + 18 * s, x1 - 18 * s);
    let bh = 15 * s;
    let mut y = 10 * s + text_height(s * 4) + 3 * s + text_height(s) + 18 * s;
    for (id, label, _) in HOME_BUTTONS {
        btn(&mut l, id, (bx0, y, bx1, y + bh), Some(p), label, s * 2, hover == Some(id), false);
        y += bh + 5 * s;
    }
    let foot = format!("{} VS {}   {}", upper(TEAM_NAMES[0]), upper(TEAM_NAMES[1]), upper(version));
    l.label_fit("footer", Some(p), (x1 - 2) / 2, h as i32 - 10 * s, &foot, s, x1 - 10 * s, DIM);
    l
}

// ---------------------------------------------------------------------------------------------------------------------------------
// match setup (solo and host)
// ---------------------------------------------------------------------------------------------------------------------------------

/// What the match setup screen chooses.
#[derive(Debug, Clone, PartialEq)]
pub struct Setup {
    /// Fill empty places with bots (off by default: a match holds only the people who joined).
    pub bots: bool,
    /// Bot skill: 0 easy, 1 normal, 2 hard.
    pub skill: u8,
    /// What the match is played for: a [`ModeKind`] wire byte (`0` team deathmatch, `1` free for all, `2` capture the flag, `3` search and destroy).
    pub mode: u8,
    /// Most people on a team, `1` for a duel (free for all: half the players in the match).
    pub size: u8,
    /// What ends the match: team kills (team deathmatch), player kills (free for all), captures or rounds won, depending on the mode
    /// (`0` = no limit; see [`limits_for`]).
    pub kill_limit: u16,
    /// Minutes that end the match (`0` = no time limit).
    pub minutes: u16,
    /// Your name.
    pub name: String,
    /// The name field has the keyboard.
    pub typing: bool,
    /// The maps on offer, as short labels (an empty list or one map shows no choice).
    pub maps: Vec<String>,
    /// The chosen map (an index into `maps`).
    pub map: usize,
}

impl Default for Setup {
    fn default() -> Self {
        Setup { bots: false, skill: 1, mode: 0, size: 6, kill_limit: 50, minutes: 10, name: String::new(), typing: false, maps: Vec::new(), map: 0 }
    }
}

/// The kill limits on offer in team deathmatch.
pub const KILL_LIMITS: [u16; 5] = [25, 50, 75, 100, 0];
/// The modes on offer: `(button id suffix, short label)`; the index is the [`ModeKind`](red_engine2::sim::shooter::ModeKind) wire byte.
/// (The font has no ampersand, so search and destroy is BOMB on the button.)
pub const MODES: [&str; 4] = ["TDM", "FFA", "CTF", "BOMB"];
/// The team sizes on offer (people a side).
pub const SIZES: [u8; 4] = [1, 2, 3, 6];

/// What a mode's limit counts, as the setup screen's caption.
pub fn limit_caption(mode: u8) -> &'static str {
    match mode {
        1 => "KILLS TO WIN",
        2 => "CAPTURES TO WIN",
        3 => "ROUNDS TO WIN",
        _ => "TEAM KILL LIMIT",
    }
}

/// The limits on offer for a mode, and the default among them.
pub fn limits_for(mode: u8) -> (&'static [u16], u16) {
    match mode {
        1 => (&[10, 20, 30, 50, 0], 20),
        2 => (&[1, 3, 5, 10, 0], 3),
        3 => (&[3, 4, 6, 8, 0], 4),
        _ => (&KILL_LIMITS, 50),
    }
}

/// A short label for a team size in a mode: `1V1`, `3V3`; in free for all the number of players.
pub fn size_label(mode: u8, size: u8) -> String {
    if mode == 1 {
        (size as u16 * 2).to_string()
    } else {
        format!("{size}V{size}")
    }
}
/// The time limits on offer, minutes.
pub const TIME_LIMITS: [u16; 5] = [5, 10, 15, 20, 0];
/// The bot skills on offer: `(label, bot level name)`.
pub const SKILLS: [(&str, &str); 3] = [("EASY", "easy"), ("NORMAL", "normal"), ("HARD", "hard")];

/// What a setup-screen button changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupAction {
    /// Bots on or off.
    Bots(bool),
    /// A bot skill.
    Skill(u8),
    /// A kill limit (`0` = none).
    Kills(u16),
    /// A mode (a [`ModeKind`](red_engine2::sim::shooter::ModeKind) wire byte).
    Mode(u8),
    /// A map (an index into the setup's maps).
    Map(usize),
    /// A team size (people a side).
    Size(u8),
    /// A time limit in minutes (`0` = none).
    Minutes(u16),
    /// Focus the name field.
    Name,
    /// Start the match.
    Start,
    /// Back to the home screen.
    Back,
}

/// The action of a setup-screen button id.
pub fn setup_action(id: &str) -> Option<SetupAction> {
    if let Some(v) = id.strip_prefix("bots_") {
        return Some(SetupAction::Bots(v == "on"));
    }
    if let Some(v) = id.strip_prefix("skill_") {
        return v.parse().ok().map(SetupAction::Skill);
    }
    if let Some(v) = id.strip_prefix("map_") {
        return v.parse().ok().map(SetupAction::Map);
    }
    if let Some(v) = id.strip_prefix("mode_") {
        return v.parse().ok().filter(|m| *m < 4).map(SetupAction::Mode);
    }
    if let Some(v) = id.strip_prefix("size_") {
        return v.parse().ok().filter(|n| (1..=6).contains(n)).map(SetupAction::Size);
    }
    if let Some(v) = id.strip_prefix("kills_") {
        return v.parse().ok().map(SetupAction::Kills);
    }
    if let Some(v) = id.strip_prefix("time_") {
        return v.parse().ok().map(SetupAction::Minutes);
    }
    match id {
        "name" => Some(SetupAction::Name),
        "start" => Some(SetupAction::Start),
        "back" => Some(SetupAction::Back),
        _ => None,
    }
}

/// The match setup screen. `title` is `"solo"` or `"host"`; `note` is a line under the buttons (where friends join, or what went wrong).
pub fn setup_layout(w: u32, h: u32, title: &str, o: &Setup, note: Option<&str>, hover: Option<&str>) -> Layout {
    let (mut l, p, (x0, _, x1, _), s) = frame(w, h, title);
    let inner = x1 - x0 - 24 * s;
    let (lx, rx) = (x0 + 12 * s, x1 - 12 * s);
    let mut y = 10 * s + text_height(s * 4) + 3 * s + text_height(s) + 8 * s;
    let chip_h = 9 * s;
    let row = |l: &mut Layout, y: i32, caption: &str, chips: &[(String, String, bool)], dim: bool| {
        l.label_left(&format!("{}_caption", caption.to_lowercase().replace(' ', "_")), Some(p), lx, y, caption, s, inner, if dim { EDGE } else { DIM });
        let y = y + text_height(s) + s;
        let n = chips.len() as i32;
        let gap = 3 * s;
        let cw = ((rx - lx) - gap * (n - 1)) / n.max(1);
        for (i, (id, label, selected)) in chips.iter().enumerate() {
            let cx0 = lx + i as i32 * (cw + gap);
            btn(l, id, (cx0, y, cx0 + cw, y + chip_h), Some(p), label, s, hover == Some(id.as_str()) && !dim, *selected && !dim);
        }
    };
    let chip = |id: &str, label: &str, sel: bool| (id.to_string(), label.to_string(), sel);
    if o.maps.len() > 1 {
        let maps: Vec<_> = o.maps.iter().take(4).enumerate().map(|(i, label)| chip(&format!("map_{i}"), &upper(label), o.map == i)).collect();
        row(&mut l, y, "MAP", &maps, false);
        y += text_height(s) + s + chip_h + 3 * s;
    }
    let modes: Vec<_> = MODES.iter().enumerate().map(|(i, label)| chip(&format!("mode_{i}"), label, o.mode as usize == i)).collect();
    row(&mut l, y, "MODE", &modes, false);
    y += text_height(s) + s + chip_h + 3 * s;
    let sizes: Vec<_> = SIZES.iter().map(|&n| chip(&format!("size_{n}"), &size_label(o.mode, n), o.size == n)).collect();
    row(&mut l, y, if o.mode == 1 { "PLAYERS" } else { "TEAM SIZE" }, &sizes, false);
    y += text_height(s) + s + chip_h + 3 * s;
    let mut bots = vec![chip("bots_off", "NONE", !o.bots)];
    bots.extend(
        SKILLS
            .iter()
            .enumerate()
            .map(|(i, (label, _))| chip(&format!("skill_{i}"), if *label == "NORMAL" { "NORM" } else { label }, o.bots && o.skill as usize == i)),
    );
    row(&mut l, y, if o.mode == 1 { "BOTS FILL THE MATCH" } else { "BOTS FILL THE TEAMS" }, &bots, false);
    y += text_height(s) + s + chip_h + 3 * s;
    let kills: Vec<_> = limits_for(o.mode)
        .0
        .iter()
        .map(|&k| chip(&format!("kills_{k}"), &if k == 0 { "NONE".to_string() } else { k.to_string() }, o.kill_limit == k))
        .collect();
    row(&mut l, y, limit_caption(o.mode), &kills, false);
    y += text_height(s) + s + chip_h + 3 * s;
    let times: Vec<_> =
        TIME_LIMITS.iter().map(|&m| chip(&format!("time_{m}"), &if m == 0 { "NONE".to_string() } else { m.to_string() }, o.minutes == m)).collect();
    row(&mut l, y, "TIME LIMIT (MINUTES)", &times, false);
    y += text_height(s) + s + chip_h + 3 * s;
    // The name field.
    l.label_left("name_caption", Some(p), lx, y, "YOUR NAME", s, inner, DIM);
    y += text_height(s) + s;
    let shown = if o.typing {
        format!("{}_", upper(&o.name))
    } else if o.name.is_empty() {
        "PLAYER".to_string()
    } else {
        upper(&o.name)
    };
    let name_rect = (lx, y, rx, y + chip_h + s);
    let name_scale = fit_both("W", name_rect, s * 2);
    let shown = ellipsize(&shown, rx - lx - 8 * s, name_scale);
    l.button("name", name_rect, Some(p), &shown, name_scale, [16, 18, 20, 240], (if o.typing { ACCENT } else { EDGE }, 1), TEXT);
    y += chip_h + s + 3 * s;
    let half = (rx - lx - 4 * s) / 2;
    btn(&mut l, "start", (lx, y, lx + half, y + 12 * s), Some(p), "START", s * 2, hover == Some("start"), true);
    btn(&mut l, "back", (rx - half, y, rx, y + 12 * s), Some(p), "BACK", s * 2, hover == Some("back"), false);
    y += 12 * s + 3 * s;
    if let Some(n) = note {
        for (i, line) in red_engine2::ui::wrap(&upper(n), inner, s).into_iter().take(2).enumerate() {
            l.label_left(&format!("note_{i}"), Some(p), lx, y + i as i32 * (text_height(s) + 2 * s), &line, s, inner, DIM);
        }
    }
    l
}

// ---------------------------------------------------------------------------------------------------------------------------------
// join
// ---------------------------------------------------------------------------------------------------------------------------------

/// What a join-screen button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinAction {
    /// The code field has the keyboard.
    Code,
    /// The name field has the keyboard.
    Name,
    /// Paste the clipboard into the code field.
    Paste,
    /// Try to join.
    Connect,
    /// Back to the home screen.
    Back,
}

/// The action of a join-screen button id.
pub fn join_action(id: &str) -> Option<JoinAction> {
    match id {
        "field_address" => Some(JoinAction::Code),
        "field_name" => Some(JoinAction::Name),
        "paste" => Some(JoinAction::Paste),
        "connect" => Some(JoinAction::Connect),
        "back" => Some(JoinAction::Back),
        _ => None,
    }
}

/// The join screen: one field for the join code a friend sent (paste it), your name, JOIN and BACK. The form's `address` holds the code and its
/// `key` is unused; `message` is shown under the buttons.
pub fn join_layout(w: u32, h: u32, f: &ConnectForm, hover: Option<&str>) -> Layout {
    let (mut l, p, (x0, _, x1, _), s) = frame(w, h, "join a game");
    let inner = x1 - x0 - 24 * s;
    let (lx, rx) = (x0 + 12 * s, x1 - 12 * s);
    let mut y = 10 * s + text_height(s * 4) + 3 * s + text_height(s) + 10 * s;
    let field_h = 13 * s;
    for (id, caption, value, focused, max_chars) in [
        ("field_address", "JOIN CODE  (PASTE IT)", &f.address, f.focus == red_engine2::ui::online::Field::Address, 0usize),
        ("field_name", "YOUR NAME", &f.name, f.focus == red_engine2::ui::online::Field::Name, 0),
    ] {
        l.label_left(&format!("{id}_caption"), Some(p), lx, y, caption, s, inner, DIM);
        y += text_height(s) + 2 * s;
        let scale = fit_both("W", (lx, y, rx, y + field_h), s * 2).max(1);
        let room = (rx - lx - 8 * s) / ((5 + 1) * scale).max(1);
        let shown_full = if focused {
            format!("{}_", upper(value))
        } else if value.is_empty() {
            String::new()
        } else {
            upper(value)
        };
        let chars: Vec<char> = shown_full.chars().collect();
        // A long value shows its end (where the caret is).
        let shown: String = if chars.len() as i32 > room { chars[chars.len() - room as usize..].iter().collect() } else { shown_full };
        let _ = max_chars;
        l.button(id, (lx, y, rx, y + field_h), Some(p), &shown, scale, [16, 18, 20, 240], (if focused { ACCENT } else { EDGE }, 1), TEXT);
        y += field_h + 6 * s;
    }
    let third = (rx - lx - 6 * s) / 3;
    btn(&mut l, "connect", (lx, y, lx + third, y + 15 * s), Some(p), "JOIN", s * 2, hover == Some("connect"), true);
    btn(&mut l, "paste", (lx + third + 3 * s, y, lx + 2 * third + 3 * s, y + 15 * s), Some(p), "PASTE", s * 2, hover == Some("paste"), false);
    btn(&mut l, "back", (rx - third, y, rx, y + 15 * s), Some(p), "BACK", s * 2, hover == Some("back"), false);
    y += 15 * s + 6 * s;
    if let Some(m) = &f.message {
        for (i, line) in red_engine2::ui::wrap(&upper(m), inner, s).into_iter().take(4).enumerate() {
            l.label_left(&format!("message_{i}"), Some(p), lx, y + i as i32 * (text_height(s) + 2 * s), &line, s, inner, [255, 170, 120, 255]);
        }
    }
    l
}

// ---------------------------------------------------------------------------------------------------------------------------------
// stats
// ---------------------------------------------------------------------------------------------------------------------------------

/// The stats page: lifetime numbers in two columns and a BACK button.
pub fn stats_layout(w: u32, h: u32, st: &Stats, hover: Option<&str>) -> Layout {
    let mut l = Layout::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = scale_for(h);
    l.panel("wash", (0, 0, wi, hi), None, Some([3, 4, 6, 150]), None);
    let cw = (300 * s).min(wi - 8);
    let lines = st.lines();
    let per_col = lines.len().div_ceil(2) as i32;
    let row_h = text_height(s) + 6 * s;
    let ch = (34 * s + text_height(s * 3) + per_col * row_h + 34 * s).min(hi - 8);
    let (x0, y0) = ((wi - cw) / 2, ((hi - ch) / 2).max(4));
    let card = l.panel("card", (x0, y0, x0 + cw, y0 + ch), None, Some(PANEL), Some((EDGE, 1.max(s / 2))));
    let cx = x0 + cw / 2;
    l.label_fit("title", Some(card), cx, y0 + 10 * s, "YOUR STATS", s * 3, cw - 12 * s, TEXT);
    let colw = (cw - 28 * s) / 2;
    let top = y0 + 10 * s + text_height(s * 3) + 12 * s;
    for (i, line) in lines.iter().enumerate() {
        let (col, r) = (i as i32 / per_col, i as i32 % per_col);
        let lx = x0 + 10 * s + col * (colw + 8 * s);
        let y = top + r * row_h;
        l.label_left(&format!("stat_{i}_label"), Some(card), lx, y, &line.label, s, colw * 6 / 10, DIM);
        l.label_right(&format!("stat_{i}_value"), Some(card), lx + colw, y, &line.value, s, colw * 4 / 10 - 4 * s, TEXT);
    }
    let by = y0 + ch - 24 * s;
    btn(&mut l, "back", (cx - 40 * s, by, cx + 40 * s, by + 16 * s), Some(card), "BACK", s * 2, hover == Some("back"), true);
    l
}

// ---------------------------------------------------------------------------------------------------------------------------------
// lobby
// ---------------------------------------------------------------------------------------------------------------------------------

/// What the team lobby shows.
#[derive(Debug, Clone, Default)]
pub struct LobbyView {
    /// Everyone in the match.
    pub roster: Vec<RosterEntry>,
    /// Our player id.
    pub me: u8,
    /// Whether we pressed Ready.
    pub ready: bool,
    /// Kill limit (`0` = none).
    pub kill_limit: u16,
    /// Time limit in seconds (`0` = none).
    pub time_limit_secs: u16,
    /// Seconds left of a countdown, if one is running.
    pub countdown: Option<u32>,
    /// Where friends join, when this machine hosts.
    pub join_address: Option<String>,
    /// Connection trouble or other news.
    pub message: Option<String>,
    /// Whether this machine hosts the match (the button says START instead of READY).
    pub hosting: bool,
    /// The map's name.
    pub map: String,
    /// The mode (a [`ModeKind`](red_engine2::sim::shooter::ModeKind) wire byte).
    pub mode: u8,
    /// Most people on a team (`0` = the full six).
    pub team_size: u8,
    /// The soldier look we wear (`0` trooper, `1` scout, `2` heavy, `3` ghost).
    pub look: u8,
}

/// What a lobby button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LobbyAction {
    /// Wear a soldier look (`0` to `3`).
    Look(u8),
    /// Join a team (`1` or `2`).
    Team(u8),
    /// Toggle ready.
    Ready,
    /// Leave to the home screen.
    Leave,
}

/// The action of a lobby button id.
pub fn lobby_action(id: &str) -> Option<LobbyAction> {
    match id {
        "team_1" => Some(LobbyAction::Team(1)),
        "team_2" => Some(LobbyAction::Team(2)),
        "ready" => Some(LobbyAction::Ready),
        "leave" => Some(LobbyAction::Leave),
        id => id.strip_prefix("look_").and_then(|n| n.parse::<u8>().ok()).filter(|n| *n < 4).map(LobbyAction::Look),
    }
}

fn limit_text(mode: u8, kill_limit: u16, secs: u16) -> String {
    let unit = match mode {
        2 => "CAPTURES",
        3 => "ROUNDS",
        _ => "KILLS",
    };
    let k = if kill_limit == 0 { format!("NO {} LIMIT", &unit[..unit.len() - 1]) } else { format!("FIRST TO {kill_limit} {unit}") };
    let t = if secs == 0 { "NO TIME LIMIT".to_string() } else { format!("{} MIN", secs / 60) };
    format!("{k}   {t}")
}

/// A `m:ss` clock.
pub fn clock(secs: u32) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// The lobby: two team columns (click one to join it), the match rules, READY/START and LEAVE.
pub fn lobby_layout(w: u32, h: u32, v: &LobbyView, hover: Option<&str>) -> Layout {
    let mut l = Layout::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = scale_for(h);
    l.panel("wash", (0, 0, wi, hi), None, Some([3, 4, 6, 170]), None);
    let cw = (330 * s).min(wi - 8);
    let rows = 6;
    let row_h = text_height(s) + 5 * s;
    let ch = (104 * s + rows * row_h + 22 * s).min(hi - 8);
    let (x0, y0) = ((wi - cw) / 2, ((hi - ch) / 2).max(4));
    let card = l.panel("card", (x0, y0, x0 + cw, y0 + ch), None, Some(PANEL), Some((EDGE, 1.max(s / 2))));
    let cx = x0 + cw / 2;
    let mode_name = red_engine2::sim::shooter::ModeKind::from_wire(v.mode).title();
    let team_size = if v.team_size == 0 { 6 } else { v.team_size as usize };
    let title = if v.mode == 1 { "FREE FOR ALL".to_string() } else { format!("{} - CHOOSE YOUR TEAM", upper(mode_name)) };
    let title_scale = fit_scale(&title, cw - 12 * s, s * 3);
    l.label_fit("title", Some(card), cx, y0 + 8 * s, &title, title_scale, cw - 12 * s, TEXT);
    let size_note = if team_size == 1 {
        "   1 V 1".to_string()
    } else if v.mode == 1 {
        format!("   {} PLAYERS", team_size * 2)
    } else {
        format!("   {team_size} V {team_size}")
    };
    l.label_fit(
        "rules",
        Some(card),
        cx,
        y0 + 8 * s + text_height(s * 3) + 4 * s,
        &format!("{}{size_note}", limit_text(v.mode, v.kill_limit, v.time_limit_secs)),
        s,
        cw - 12 * s,
        DIM,
    );
    let mut top = y0 + 8 * s + text_height(s * 3) + 4 * s + text_height(s) + 6 * s;
    if let Some(a) = &v.join_address {
        l.label_fit("join_address", Some(card), cx, top, &format!("FRIENDS JOIN AT  {}", upper(a)), s, cw - 12 * s, [255, 214, 140, 255]);
        top += text_height(s) + 5 * s;
    }
    let gap = 6 * s;
    let colw = (cw - 20 * s - gap) / 2;
    let mut my_team = 0;
    if v.mode == 1 {
        // Free for all has no sides: one list of everyone in two columns.
        let label = format!("PLAYERS {}/{}", v.roster.len(), team_size * 2);
        l.label_fit("ffa_header", Some(card), cx, top + 4 * s, &label, s * 2, cw - 12 * s, ACCENT);
        for (k, e) in v.roster.iter().take(12).enumerate() {
            let (col, row) = (k as i32 / 6, k as i32 % 6);
            let bx0 = x0 + 10 * s + col * (colw + gap);
            let y = top + 16 * s + 4 * s + row * row_h;
            let mut name = upper(&e.name);
            if e.id == v.me {
                name = format!("> {name}");
            }
            let tag = if e.flags & ROSTER_BOT != 0 {
                "BOT"
            } else if e.flags & ROSTER_READY != 0 {
                "READY"
            } else {
                ""
            };
            l.label_left(&format!("p_name_{k}"), Some(card), bx0 + 3 * s, y, &name, s, colw * 66 / 100, if e.id == v.me { [255, 226, 160, 255] } else { TEXT });
            l.label_right(&format!("p_tag_{k}"), Some(card), bx0 + colw - 3 * s, y, tag, s, colw * 30 / 100, DIM);
        }
    }
    for team in (1..=2u8).filter(|_| v.mode != 1) {
        let members: Vec<&RosterEntry> = v.roster.iter().filter(|e| e.team == team).collect();
        if members.iter().any(|e| e.id == v.me) {
            my_team = team;
        }
        let bx0 = x0 + 10 * s + (team as i32 - 1) * (colw + gap);
        let label = format!("{} {}/{}", upper(TEAM_NAMES[team as usize - 1]), members.len(), team_size);
        let hot = hover == Some(if team == 1 { "team_1" } else { "team_2" });
        let selected = my_team == team;
        let id = if team == 1 { "team_1" } else { "team_2" };
        let color = team_color(team);
        let (fill, edge) = if selected {
            ([color[0] / 4, color[1] / 4, color[2] / 4, 250], color)
        } else if hot {
            (BUTTON_HOT, color)
        } else {
            (BUTTON, EDGE)
        };
        let sc = fit_scale(&label, colw - 6 * s, s * 2);
        l.button(id, (bx0, top, bx0 + colw, top + 16 * s), Some(card), &label, sc, fill, (edge, 1.max(s / 2)), color);
        for k in 0..rows {
            let y = top + 16 * s + 4 * s + k * row_h;
            if let Some(e) = members.get(k as usize) {
                let mut name = upper(&e.name);
                if e.id == v.me {
                    name = format!("> {name}");
                }
                let tag = if e.flags & ROSTER_BOT != 0 {
                    "BOT"
                } else if e.flags & ROSTER_READY != 0 {
                    "READY"
                } else {
                    ""
                };
                l.label_left(
                    &format!("t{team}_name_{k}"),
                    Some(card),
                    bx0 + 3 * s,
                    y,
                    &name,
                    s,
                    colw * 66 / 100,
                    if e.id == v.me { [255, 226, 160, 255] } else { TEXT },
                );
                l.label_right(&format!("t{team}_tag_{k}"), Some(card), bx0 + colw - 3 * s, y, tag, s, colw * 30 / 100, DIM);
            }
        }
    }
    // The look: four chips, the one worn lit.
    {
        let ly = y0 + ch - 24 * s - 16 * s;
        let n = red_engine2::player::Character::SOLDIER_LOOKS.len() as i32;
        let lgap = 3 * s;
        let cwid = ((cw - 20 * s) - lgap * (n - 1)) / n;
        l.label_left("look_caption", Some(card), x0 + 10 * s, ly - text_height(s) - 2 * s, "YOUR LOOK", s, cw - 20 * s, DIM);
        for (i, name) in red_engine2::player::Character::SOLDIER_LOOKS.iter().enumerate() {
            let bx = x0 + 10 * s + i as i32 * (cwid + lgap);
            let id = format!("look_{i}");
            btn(&mut l, &id, (bx, ly, bx + cwid, ly + 12 * s), Some(card), &upper(name), s, hover == Some(id.as_str()), v.look as usize == i);
        }
    }
    let by = y0 + ch - 24 * s;
    let main = if v.countdown.is_some() {
        format!("STARTING IN {}", v.countdown.unwrap_or(0))
    } else if v.hosting {
        if v.ready {
            "WAITING".to_string()
        } else {
            "START".to_string()
        }
    } else if v.ready {
        "READY!".to_string()
    } else {
        "READY".to_string()
    };
    btn(&mut l, "ready", (cx - 80 * s, by, cx + 14 * s, by + 16 * s), Some(card), &main, s * 2, hover == Some("ready"), v.ready);
    btn(&mut l, "leave", (cx + 20 * s, by, cx + 80 * s, by + 16 * s), Some(card), "LEAVE", s * 2, hover == Some("leave"), false);
    if let Some(m) = &v.message {
        l.label_fit("message", Some(card), cx, by - text_height(s) - 4 * s, &upper(m), s, cw - 12 * s, [255, 170, 120, 255]);
    }
    l
}

// ---------------------------------------------------------------------------------------------------------------------------------
// the HUD
// ---------------------------------------------------------------------------------------------------------------------------------

/// Everything the in-game HUD shows.
#[derive(Debug, Clone, Default)]
pub struct HudView {
    /// Hit points.
    pub hp: u32,
    /// Rounds in the magazine of the gun in hand (`None` for a knife or a grenade).
    pub loaded: Option<u16>,
    /// Rounds in reserve.
    pub reserve: u16,
    /// Name of the weapon in hand.
    pub weapon: String,
    /// Grenades carried, as one letter each (F frag, B flash, S smoke, I incendiary).
    pub grenades: Vec<char>,
    /// Reloading right now.
    pub reloading: bool,
    /// Seconds left in the match (`None` = no time limit).
    pub secs_left: Option<u32>,
    /// Kills per team.
    pub team_score: [u16; 2],
    /// Our team.
    pub team: u8,
    /// Kill limit (`0` = none).
    pub kill_limit: u16,
    /// A line in the middle of the screen (countdown numbers, `WARMUP`).
    pub center: Option<String>,
    /// Looking at a pickup that would be taken: its name.
    pub prompt: Option<String>,
    /// The mode (a [`ModeKind`](red_engine2::sim::shooter::ModeKind) wire byte: `0` team deathmatch, `1` free for all, `2` capture the flag, `3` search and destroy).
    pub mode: u8,
    /// Free for all: our kills and the best player's kills.
    pub ffa: Option<(u16, u16)>,
    /// Capture the flag and search and destroy: what is going on and what to do.
    pub objective: Option<ObjectiveHud>,
    /// The newest objective event as one line over the middle of the screen, and its colour.
    pub banner: Option<(String, [u8; 4])>,
    /// Labels over flags, the bomb and the sites, already placed on the screen.
    pub markers: Vec<Marker>,
}

/// The HUD: health bottom left, ammunition bottom right, score and clock at the top.
pub fn hud_layout(w: u32, h: u32, v: &HudView) -> Layout {
    let mut l = Layout::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = scale_for(h);
    let m = 7 * s;
    let low = v.hp <= 30;
    let hp_color = if low { [255, 90, 70, 255] } else { TEXT };
    l.label_left("hp", None, m, hi - m - text_height(s * 4), &v.hp.to_string(), s * 4, 60 * s, hp_color);
    // Ammunition: the magazine large, the reserve beside it small; above it the weapon's name, above that the grenades.
    let ammo_y = hi - m - text_height(s * 4);
    let mut weapon_line = upper(&v.weapon);
    if v.reloading {
        weapon_line.push_str("  RELOADING");
    }
    match v.loaded {
        Some(loaded) => {
            let reserve = format!("/ {}", v.reserve);
            let rw = text_width(&reserve, s * 2);
            l.label_right("reserve", None, wi - m, hi - m - text_height(s * 2), &reserve, s * 2, 60 * s, DIM);
            let color = if loaded == 0 { [255, 90, 70, 255] } else { TEXT };
            l.label_right("loaded", None, wi - m - rw - 4 * s, ammo_y, &loaded.to_string(), s * 4, 60 * s, color);
            l.label_right("weapon", None, wi - m, ammo_y - text_height(s) - 3 * s, &weapon_line, s, 120 * s, [110, 116, 120, 255]);
        }
        None => {
            let name = if v.weapon.eq_ignore_ascii_case("combat knife") { "KNIFE".to_string() } else { upper(&v.weapon) };
            l.label_right("hand", None, wi - m, hi - m - text_height(s * 2), &name, s * 2, 180 * s, DIM);
        }
    }
    if !v.grenades.is_empty() {
        let text: String = v.grenades.iter().map(|c| format!("{c} ")).collect::<String>().trim_end().to_string();
        let y = ammo_y - text_height(s) - 3 * s - text_height(s * 2) - 3 * s;
        l.label_right("grenades", None, wi - m, y, &text, s * 2, 60 * s, DIM);
    }
    // The score and the clock.
    let top = 5 * s;
    let mid = wi / 2;
    let clock_text = v.secs_left.map_or_else(|| "--:--".to_string(), clock);
    l.label("clock", None, mid, top, &clock_text, s * 2, TEXT);
    let gap = text_width("00:00", s * 2) / 2 + 10 * s;
    match v.ffa {
        // Free for all has no sides: you against the best of the others.
        Some((mine, best)) => {
            l.label_right("score_1", None, mid - gap, top, &format!("YOU {mine}"), s * 2, 60 * s, TEXT);
            l.label_left("score_2", None, mid + gap, top, &format!("BEST {best}"), s * 2, 60 * s, ACCENT);
        }
        None => {
            l.label_right("score_1", None, mid - gap, top, &v.team_score[0].to_string(), s * 2, 40 * s, RIDGEBACK);
            l.label_left("score_2", None, mid + gap, top, &v.team_score[1].to_string(), s * 2, 40 * s, NIGHTFALL);
        }
    }
    let mut below = top + text_height(s * 2) + 2 * s;
    if v.kill_limit > 0 {
        let unit = match v.mode {
            2 => " CAPTURES",
            3 => " ROUNDS",
            _ => "",
        };
        l.label("limit", None, mid, below, &format!("FIRST TO {}{unit}", v.kill_limit), s, [110, 116, 120, 255]);
        below += text_height(s) + 3 * s;
    }
    if let Some(o) = &v.objective {
        l.label_fit("objective_line", None, mid, below, &o.line, s, wi - 20 * s, TEXT);
        below += text_height(s) + 2 * s;
        if !o.sub.is_empty() {
            l.label_fit("objective_sub", None, mid, below, &o.sub, s, wi - 20 * s, o.sub_color);
        }
        if let Some((what, frac)) = &o.bar {
            let (bw, bh) = (90 * s, 5 * s);
            let by = hi * 3 / 5 + text_height(s * 2) + 6 * s;
            l.label("bar_caption", None, mid, by - text_height(s) - 2 * s, what, s, TEXT);
            l.panel("bar_frame", (mid - bw / 2, by, mid + bw / 2, by + bh), None, Some([10, 12, 14, 200]), Some(([200, 204, 208, 255], 1)));
            let fill = ((bw - 2) as f32 * frac.clamp(0.0, 1.0)) as i32;
            if fill > 0 {
                l.panel("bar_fill", (mid - bw / 2 + 1, by + 1, mid - bw / 2 + 1 + fill, by + bh - 1), None, Some(ACCENT), None);
            }
        }
    }
    if let Some((text, color)) = &v.banner {
        l.label_fit("banner", None, mid, hi / 3, text, s * 3, wi - 20 * s, *color);
    }
    for (i, m) in v.markers.iter().enumerate() {
        let (x, y) = (m.x.clamp(30.0 * s as f32, wi as f32 - 30.0 * s as f32) as i32, m.y.clamp(20.0 * s as f32, hi as f32 - 20.0 * s as f32) as i32);
        l.label_fit(&format!("marker_{i}"), None, x, y, &m.label, s, 56 * s, m.color);
    }
    if let Some(c) = &v.center {
        l.label("center", None, mid, hi / 4, c, s * 5, TEXT);
    }
    if let Some(p) = &v.prompt {
        l.label_fit("prompt", None, mid, hi * 3 / 5, &format!("E  {}", upper(p)), s * 2, wi - 20 * s, [255, 226, 160, 255]);
    }
    l
}

// ---------------------------------------------------------------------------------------------------------------------------------
// the killcam caption
// ---------------------------------------------------------------------------------------------------------------------------------

/// What the killcam caption says.
#[derive(Debug, Clone, Default)]
pub struct KillcamView {
    /// Who killed us.
    pub killer: String,
    /// The killer's team.
    pub killer_team: u8,
    /// With what.
    pub weapon: String,
    /// A headshot.
    pub headshot: bool,
    /// Seconds until we return.
    pub respawn_secs: u32,
    /// Showing the replay (`true`) or only the caption over the view of our own body (`false`).
    pub replay: bool,
}

/// The killcam's captions: who and with what at the bottom, the wait at the top. Bars top and bottom frame the picture.
pub fn killcam_layout(w: u32, h: u32, v: &KillcamView) -> Layout {
    let mut l = Layout::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = scale_for(h);
    let bar = 28 * s;
    l.panel("bar_top", (0, 0, wi, bar), None, Some([0, 0, 0, 170]), None);
    l.panel("bar_bottom", (0, hi - bar, wi, hi), None, Some([0, 0, 0, 170]), None);
    let cx = wi / 2;
    l.label_fit("replay", None, 14 * s + text_width("KILLCAM", s * 2) / 2, 6 * s, if v.replay { "KILLCAM" } else { "YOU DIED" }, s * 2, 80 * s, ACCENT);
    l.label_right("respawn", None, wi - 10 * s, 6 * s, &format!("BACK IN {}", v.respawn_secs), s * 2, 90 * s, TEXT);
    let headline = format!("{} KILLED YOU", upper(&v.killer));
    let shown_y = hi - bar + 3 * s;
    l.label_fit("killer", None, cx, shown_y, &headline, s * 2, wi - 20 * s, team_color(v.killer_team));
    let detail = if v.headshot { format!("{}   HEADSHOT", upper(&v.weapon)) } else { upper(&v.weapon) };
    l.label_fit("weapon", None, cx, shown_y + text_height(s * 2) + 2 * s, &detail, s, wi - 20 * s, DIM);
    l
}

// ---------------------------------------------------------------------------------------------------------------------------------
// scoreboard and results
// ---------------------------------------------------------------------------------------------------------------------------------

/// What the scoreboard and the results screen show.
#[derive(Debug, Clone, Default)]
pub struct BoardView {
    /// Everyone in the match.
    pub roster: Vec<RosterEntry>,
    /// Our player id.
    pub me: u8,
    /// Kills per team.
    pub team_score: [u16; 2],
    /// The winning team (`0` = a draw, or not over).
    pub winner_team: u8,
    /// Why it ended.
    pub reason: String,
    /// Our team.
    pub team: u8,
    /// Whether we pressed Play Again.
    pub ready: bool,
    /// People in the match still to press Play Again (humans only).
    pub waiting_for: usize,
    /// Whether the match is over (the results screen) or running (the Tab scoreboard).
    pub over: bool,
    /// The mode (a [`ModeKind`](red_engine2::sim::shooter::ModeKind) wire byte): free for all lists everyone in one table.
    pub mode: u8,
    /// Free for all: the winning player's id (`255` = nobody).
    pub winner: u8,
}

fn team_table(l: &mut Layout, card: usize, team: u8, rect: (i32, i32, i32, i32), v: &BoardView, s: i32) {
    let (x0, y0, x1, _) = rect;
    let color = team_color(team);
    let mut members: Vec<&RosterEntry> = v.roster.iter().filter(|e| e.team == team).collect();
    members.sort_by(|a, b| b.score.cmp(&a.score).then(a.id.cmp(&b.id)));
    let head = format!("{}  {}", upper(TEAM_NAMES[team as usize - 1]), v.team_score[team as usize - 1]);
    let head_scale = fit_scale(&head, x1 - x0, s * 2);
    l.label_left(&format!("tb{team}_head"), Some(card), x0, y0, &head, head_scale, x1 - x0, color);
    l.panel(&format!("tb{team}_rule"), (x0, y0 + text_height(s * 2) + 2 * s, x1, y0 + text_height(s * 2) + 2 * s + s.max(1)), Some(card), Some(color), None);
    let row_h = text_height(s) + 4 * s;
    for (k, e) in members.iter().take(6).enumerate() {
        let y = y0 + text_height(s * 2) + 7 * s + k as i32 * row_h;
        let mut name = upper(&e.name);
        if e.id == v.me {
            name = format!("> {name}");
        }
        l.label_left(&format!("tb{team}_n{k}"), Some(card), x0, y, &name, s, (x1 - x0) * 70 / 100, if e.id == v.me { [255, 226, 160, 255] } else { TEXT });
        l.label_right(&format!("tb{team}_s{k}"), Some(card), x1, y, &e.score.to_string(), s, (x1 - x0) * 25 / 100, TEXT);
    }
}

/// Free for all: everybody in one table of up to twelve, two columns, best first.
fn ffa_table(l: &mut Layout, card: usize, rect: (i32, i32, i32, i32), v: &BoardView, s: i32) {
    let (x0, y0, x1, _) = rect;
    let mut all: Vec<&RosterEntry> = v.roster.iter().collect();
    all.sort_by(|a, b| b.score.cmp(&a.score).then(a.id.cmp(&b.id)));
    l.label_left("ffa_head", Some(card), x0, y0, "PLAYERS", s * 2, x1 - x0, ACCENT);
    l.panel("ffa_rule", (x0, y0 + text_height(s * 2) + 2 * s, x1, y0 + text_height(s * 2) + 2 * s + s.max(1)), Some(card), Some(ACCENT), None);
    let gap = 10 * s;
    let colw = (x1 - x0 - gap) / 2;
    let row_h = text_height(s) + 4 * s;
    for (k, e) in all.iter().take(12).enumerate() {
        let (col, row) = (k as i32 / 6, k as i32 % 6);
        let cx0 = x0 + col * (colw + gap);
        let y = y0 + text_height(s * 2) + 7 * s + row * row_h;
        let mut name = format!("{}  {}", k + 1, upper(&e.name));
        if e.id == v.me {
            name = format!("> {name}");
        }
        l.label_left(&format!("ffa_n{k}"), Some(card), cx0, y, &name, s, colw * 72 / 100, if e.id == v.me { [255, 226, 160, 255] } else { TEXT });
        l.label_right(&format!("ffa_s{k}"), Some(card), cx0 + colw, y, &e.score.to_string(), s, colw * 24 / 100, TEXT);
    }
}

/// The Tab scoreboard: two tables over a dim wash.
pub fn scoreboard_layout(w: u32, h: u32, v: &BoardView) -> Layout {
    let mut l = Layout::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = scale_for(h);
    let cw = (330 * s).min(wi - 8);
    let ch = (20 * s + text_height(s * 2) + 7 * s + 6 * (text_height(s) + 4 * s) + 10 * s).min(hi - 8);
    let (x0, y0) = ((wi - cw) / 2, ((hi - ch) / 3).max(4));
    let card = l.panel("card", (x0, y0, x0 + cw, y0 + ch), None, Some([8, 10, 12, 215]), Some((EDGE, 1)));
    let gap = 12 * s;
    let colw = (cw - 20 * s - gap) / 2;
    if v.mode == 1 {
        ffa_table(&mut l, card, (x0 + 10 * s, y0 + 8 * s, x0 + cw - 10 * s, y0 + ch), v, s);
        return l;
    }
    for team in 1..=2u8 {
        let tx0 = x0 + 10 * s + (team as i32 - 1) * (colw + gap);
        team_table(&mut l, card, team, (tx0, y0 + 8 * s, tx0 + colw, y0 + ch), v, s);
    }
    l
}

/// What the end-of-match buttons do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultsAction {
    /// Another match with the same settings.
    PlayAgain,
    /// End the game and go to the home screen.
    Home,
}

/// The action of a results button id.
pub fn results_action(id: &str) -> Option<ResultsAction> {
    match id {
        "again" => Some(ResultsAction::PlayAgain),
        "home" => Some(ResultsAction::Home),
        _ => None,
    }
}

/// The end-of-match screen: who won, the final tables, PLAY AGAIN and HOME SCREEN.
pub fn results_layout(w: u32, h: u32, v: &BoardView, hover: Option<&str>) -> Layout {
    let mut l = Layout::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = scale_for(h);
    l.panel("wash", (0, 0, wi, hi), None, Some([3, 4, 6, 185]), None);
    let cw = (330 * s).min(wi - 8);
    let ch = (60 * s + text_height(s * 4) + 6 * (text_height(s) + 4 * s) + text_height(s * 2) + 40 * s).min(hi - 8);
    let (x0, y0) = ((wi - cw) / 2, ((hi - ch) / 2).max(4));
    let card = l.panel("card", (x0, y0, x0 + cw, y0 + ch), None, Some(PANEL), Some((EDGE, 1.max(s / 2))));
    let cx = x0 + cw / 2;
    let ffa_winner = (v.mode == 1).then(|| v.roster.iter().find(|e| e.id == v.winner)).flatten();
    let (banner, color) = match (v.mode, v.winner_team) {
        (1, _) => match ffa_winner {
            Some(e) => (format!("{} WINS", upper(&e.name)), ACCENT),
            None => ("DRAW".to_string(), TEXT),
        },
        (_, 1 | 2) => (format!("{} WINS", upper(TEAM_NAMES[v.winner_team as usize - 1])), team_color(v.winner_team)),
        _ => ("DRAW".to_string(), TEXT),
    };
    l.label_fit("banner", Some(card), cx, y0 + 8 * s, &banner, s * 4, cw - 12 * s, color);
    let verdict = if v.mode == 1 {
        match ffa_winner {
            Some(e) if e.id == v.me => "VICTORY",
            Some(_) => "DEFEAT",
            None => "NOBODY WINS",
        }
    } else if v.winner_team == 0 {
        "NOBODY WINS"
    } else if v.winner_team == v.team {
        "VICTORY"
    } else if v.team != 0 {
        "DEFEAT"
    } else {
        ""
    };
    l.label_fit(
        "score",
        Some(card),
        cx,
        y0 + 8 * s + text_height(s * 4) + 4 * s,
        &if v.mode == 1 {
            format!("{}   {}", verdict, upper(&v.reason))
        } else {
            format!("{} - {}   {}   {}", v.team_score[0], v.team_score[1], verdict, upper(&v.reason))
        },
        s * 2,
        cw - 12 * s,
        DIM,
    );
    let gap = 12 * s;
    let colw = (cw - 20 * s - gap) / 2;
    let top = y0 + 8 * s + text_height(s * 4) + 4 * s + text_height(s * 2) + 10 * s;
    if v.mode == 1 {
        ffa_table(&mut l, card, (x0 + 10 * s, top, x0 + cw - 10 * s, y0 + ch), v, s);
    }
    for team in (1..=2u8).filter(|_| v.mode != 1) {
        let tx0 = x0 + 10 * s + (team as i32 - 1) * (colw + gap);
        team_table(&mut l, card, team, (tx0, top, tx0 + colw, y0 + ch), v, s);
    }
    let by = y0 + ch - 24 * s;
    let again = if v.ready {
        if v.waiting_for > 0 {
            format!("WAITING ({})", v.waiting_for)
        } else {
            "STARTING".to_string()
        }
    } else {
        "PLAY AGAIN".to_string()
    };
    btn(&mut l, "again", (cx - 92 * s, by, cx - 2 * s, by + 16 * s), Some(card), &again, s * 2, hover == Some("again"), true);
    btn(&mut l, "home", (cx + 2 * s, by, cx + 92 * s, by + 16 * s), Some(card), "HOME SCREEN", s * 2, hover == Some("home"), false);
    l
}

// ---------------------------------------------------------------------------------------------------------------------------------
// pause
// ---------------------------------------------------------------------------------------------------------------------------------

/// What a pause-menu button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseChoice {
    /// Back to the match.
    Resume,
    /// Toggle fullscreen.
    Fullscreen,
    /// Leave the match for the home screen.
    Home,
}

/// The action of a pause button id.
pub fn pause_choice(id: &str) -> Option<PauseChoice> {
    match id {
        "resume" => Some(PauseChoice::Resume),
        "fullscreen" => Some(PauseChoice::Fullscreen),
        "home" => Some(PauseChoice::Home),
        _ => None,
    }
}

/// The pause menu: resume, fullscreen, leave the match.
pub fn pause_layout(w: u32, h: u32, hover: Option<&str>, hosting: bool) -> Layout {
    let mut l = Layout::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = scale_for(h);
    l.panel("wash", (0, 0, wi, hi), None, Some([3, 4, 6, 150]), None);
    let cw = (150 * s).min(wi - 8);
    let ch = 118 * s;
    let (x0, y0) = ((wi - cw) / 2, ((hi - ch) / 2).max(4));
    let card = l.panel("card", (x0, y0, x0 + cw, y0 + ch), None, Some(PANEL), Some((EDGE, 1.max(s / 2))));
    let cx = x0 + cw / 2;
    l.label_fit("title", Some(card), cx, y0 + 8 * s, "PAUSED", s * 3, cw - 12 * s, TEXT);
    let (bx0, bx1) = (x0 + 14 * s, x1(x0, cw) - 14 * s);
    let mut y = y0 + 8 * s + text_height(s * 3) + 10 * s;
    btn(&mut l, "resume", (bx0, y, bx1, y + 16 * s), Some(card), "RESUME", s * 2, hover == Some("resume"), true);
    y += 16 * s + 5 * s;
    btn(&mut l, "fullscreen", (bx0, y, bx1, y + 16 * s), Some(card), "FULLSCREEN", s * 2, hover == Some("fullscreen"), false);
    y += 16 * s + 5 * s;
    btn(&mut l, "home", (bx0, y, bx1, y + 16 * s), Some(card), if hosting { "END GAME" } else { "LEAVE GAME" }, s * 2, hover == Some("home"), false);
    y += 16 * s + 6 * s;
    l.label_fit("hint", Some(card), cx, y, if hosting { "ENDING CLOSES THE GAME FOR EVERYONE" } else { "ESC RESUMES" }, s, cw - 12 * s, DIM);
    l
}

fn x1(x0: i32, w: i32) -> i32 {
    x0 + w
}

// ---------------------------------------------------------------------------------------------------------------------------------
// the scope
// ---------------------------------------------------------------------------------------------------------------------------------

/// The picture over the screen while looking through a rifle scope: black outside a circle, a fine cross with a mil-dot ladder inside.
pub fn paint_scope(cv: &mut Canvas) {
    let (w, h) = (cv.w, cv.h);
    let (cx, cy) = (w / 2, h / 2);
    let radius = (h as f32 * 0.46) as i32;
    let r2 = (radius * radius) as i64;
    for y in 0..h {
        let dy = (y - cy) as i64;
        for x in 0..w {
            let dx = (x - cx) as i64;
            let d2 = dx * dx + dy * dy;
            if d2 > r2 {
                cv.blend(x, y, [0, 0, 0, 255]);
            } else if d2 > r2 - (radius as i64 * 6) {
                // A soft dark ring at the edge of the glass.
                let t = (d2 - (r2 - radius as i64 * 6)) as f32 / (radius as f32 * 6.0);
                cv.blend(x, y, [0, 0, 0, (t * 200.0) as u8]);
            }
        }
    }
    let line = [8, 8, 8, 235];
    let thick = (h / 540).max(1);
    cv.rect(cx - radius, cy - thick / 2, cx - radius / 12, cy + ((thick + 1) / 2), line);
    cv.rect(cx + radius / 12, cy - thick / 2, cx + radius, cy + ((thick + 1) / 2), line);
    cv.rect(cx - thick / 2, cy - radius, cx + ((thick + 1) / 2), cy - radius / 12, line);
    cv.rect(cx - thick / 2, cy + radius / 12, cx + ((thick + 1) / 2), cy + radius, line);
    // A ladder of ticks down the vertical wire and along the horizontal one.
    for k in 1..=4 {
        let off = radius * k / 10;
        let len = if k % 2 == 0 { radius / 40 } else { radius / 70 };
        cv.rect(cx - len, cy + off, cx + len + 1, cy + off + thick, line);
        cv.rect(cx - len, cy - off, cx + len + 1, cy - off + thick, line);
        cv.rect(cx + off, cy - len, cx + off + thick, cy + len + 1, line);
        cv.rect(cx - off, cy - len, cx - off + thick, cy + len + 1, line);
    }
    paint_optic_reticle(cv);
}

/// A small illuminated cross on the camera's center ray, legible over both light and dark scenery.
pub fn paint_optic_reticle(cv: &mut Canvas) {
    let (cx, cy) = (cv.w / 2, cv.h / 2);
    let t = (cv.h / 540).max(1);
    let r = 4 * t;
    cv.rect(cx - r - 1, cy - t - 1, cx + r + 2, cy + t + 2, [8, 8, 8, 255]);
    cv.rect(cx - t - 1, cy - r - 1, cx + t + 2, cy + r + 2, [8, 8, 8, 255]);
    cv.rect(cx - r, cy - t, cx + r + 1, cy + t + 1, [240, 70, 45, 255]);
    cv.rect(cx - t, cy - r, cx + t + 1, cy + r + 1, [240, 70, 45, 255]);
}

// ---------------------------------------------------------------------------------------------------------------------------------
// registry, audit, pictures
// ---------------------------------------------------------------------------------------------------------------------------------

/// A sample roster of twelve: six per team, one of them us.
pub fn demo_roster() -> Vec<RosterEntry> {
    let names = ["Kev", "Cousin Ben", "Rook", "Maverick", "Hex", "Pixel", "Bolt", "Nova", "Dusty", "Gizmo", "Sprocket", "Merlot"];
    (0..12u8)
        .map(|i| RosterEntry {
            id: i,
            team: 1 + i % 2,
            flags: if i > 3 {
                ROSTER_BOT | ROSTER_READY
            } else if i % 3 == 0 {
                ROSTER_READY
            } else {
                0
            },
            character: 6 + i % 2,
            ping_ms: 20 + i as u16,
            score: (12 - i as u16) * 2,
            name: names[i as usize].to_string(),
        })
        .collect()
}

/// Builds the named screen at a window size, filled with sample content.
pub fn build(name: &str, w: u32, h: u32) -> Option<Layout> {
    let board = BoardView {
        roster: demo_roster(),
        me: 0,
        team_score: [31, 27],
        winner_team: 1,
        reason: "kill limit".into(),
        team: 1,
        ready: false,
        waiting_for: 1,
        over: true,
        mode: 0,
        winner: 255,
    };
    let hud = HudView {
        hp: 100,
        loaded: Some(17),
        reserve: 68,
        weapon: "R9 service pistol".into(),
        grenades: vec!['F', 'B'],
        reloading: false,
        secs_left: Some(437),
        team_score: [12, 9],
        team: 1,
        kill_limit: 50,
        center: None,
        prompt: None,
        ..Default::default()
    };
    Some(match name {
        "home" => home_layout(w, h, None, "v1"),
        "solo" => setup_layout(
            w,
            h,
            "solo",
            &Setup { name: "Kev".into(), maps: vec!["Works".into(), "Quarry".into(), "Depot".into()], ..Default::default() },
            Some("Pick a mode and a size. Bots are optional."),
            None,
        ),
        "host" => setup_layout(
            w,
            h,
            "host",
            &Setup {
                bots: true,
                skill: 2,
                mode: 2,
                size: 3,
                kill_limit: 5,
                minutes: 15,
                name: "Kev".into(),
                typing: true,
                maps: vec!["Works".into(), "Quarry".into(), "Depot".into()],
                map: 1,
            },
            Some("Friends join at 203.0.113.9:27015 once the match starts."),
            Some("start"),
        ),
        "join" => {
            let mut f = ConnectForm::new("203.0.113.9:27015", "", "Kev");
            f.message = None;
            join_layout(w, h, &f, None)
        }
        "stats" => {
            let mut st = Stats {
                kills: 1423,
                deaths: 1107,
                headshots: 388,
                shots_fired: 21_840,
                shots_hit: 6_120,
                rounds_played: 96,
                rounds_won: 55,
                rounds_lost: 38,
                rounds_drawn: 3,
                ..Default::default()
            };
            st.time_in_matches_secs = 91_000.0;
            st.time_in_game_secs = 120_500.0;
            st.best_streak = 14;
            st.best_match_kills = 38;
            st.add_kill("Redline rifle", true, false, false);
            stats_layout(w, h, &st, None)
        }
        "lobby" => lobby_layout(
            w,
            h,
            &LobbyView {
                roster: demo_roster(),
                me: 0,
                ready: false,
                kill_limit: 50,
                time_limit_secs: 600,
                countdown: None,
                join_address: Some("H3PQXR-K7Q2-MZ4P-WTXA".into()),
                message: None,
                hosting: true,
                map: "foundry".into(),
                mode: 0,
                team_size: 6,
                look: 1,
            },
            None,
        ),
        "lobby-ffa" => lobby_layout(
            w,
            h,
            &LobbyView {
                roster: demo_roster().into_iter().map(|e| RosterEntry { team: 0, ..e }).collect(),
                me: 0,
                kill_limit: 20,
                time_limit_secs: 600,
                hosting: true,
                mode: 1,
                team_size: 6,
                ..Default::default()
            },
            None,
        ),
        "lobby-duel" => lobby_layout(
            w,
            h,
            &LobbyView {
                roster: demo_roster().into_iter().take(2).collect(),
                me: 0,
                kill_limit: 25,
                time_limit_secs: 300,
                hosting: true,
                mode: 0,
                team_size: 1,
                ..Default::default()
            },
            None,
        ),
        "results-ffa" => results_layout(
            w,
            h,
            &BoardView {
                mode: 1,
                winner: 2,
                winner_team: 0,
                team: 0,
                reason: "kill limit".into(),
                roster: demo_roster().into_iter().map(|e| RosterEntry { team: 0, ..e }).collect(),
                ..board
            },
            Some("again"),
        ),
        "countdown" => hud_layout(w, h, &HudView { center: Some("3".into()), ..hud }),
        "hud" => hud_layout(w, h, &hud),
        "hud-ffa" => hud_layout(w, h, &HudView { mode: 1, ffa: Some((11, 14)), kill_limit: 20, ..hud }),
        "hud-ctf" => hud_layout(
            w,
            h,
            &HudView {
                mode: 2,
                team_score: [1, 0],
                kill_limit: 3,
                objective: Some(ObjectiveHud {
                    line: "YOUR FLAG HOME   ENEMY FLAG TAKEN".into(),
                    sub: "YOU HAVE THE FLAG - BRING IT HOME".into(),
                    sub_color: crate::objective_hud::CALL,
                    bar: None,
                }),
                banner: Some(("YOU TOOK THE FLAG".into(), crate::objective_hud::GOOD)),
                markers: vec![
                    Marker { x: w as f32 * 0.22, y: h as f32 * 0.46, label: "YOUR FLAG  48M".into(), color: RIDGEBACK },
                    Marker { x: w as f32 * 0.78, y: h as f32 * 0.58, label: "ENEMY BASE  61M".into(), color: NIGHTFALL },
                ],
                ..hud
            },
        ),
        "hud-snd" => hud_layout(
            w,
            h,
            &HudView {
                mode: 3,
                team_score: [2, 1],
                kill_limit: 4,
                objective: Some(ObjectiveHud {
                    line: "ROUND 4   ATTACKING   3 V 2".into(),
                    sub: "BOMB PLANTED AT A   0:31   DEFEND IT".into(),
                    sub_color: crate::objective_hud::GOOD,
                    bar: Some(("DEFUSING".into(), 0.4)),
                }),
                banner: Some(("BOMB PLANTED - DEFEND IT".into(), crate::objective_hud::GOOD)),
                markers: vec![Marker { x: w as f32 * 0.55, y: h as f32 * 0.5, label: "BOMB  0:31".into(), color: crate::objective_hud::BAD }],
                ..hud
            },
        ),
        "hud-low" => hud_layout(w, h, &HudView { hp: 18, loaded: Some(0), reserve: 34, reloading: true, prompt: Some("Redline rifle".into()), ..hud }),
        "killcam" => killcam_layout(
            w,
            h,
            &KillcamView { killer: "Cousin Ben".into(), killer_team: 2, weapon: "Sentinel .338".into(), headshot: true, respawn_secs: 5, replay: true },
        ),
        "scoreboard" => scoreboard_layout(w, h, &BoardView { over: false, winner_team: 0, ..board }),
        "results" => results_layout(w, h, &board, None),
        "results-draw" => {
            results_layout(w, h, &BoardView { winner_team: 0, team_score: [40, 40], reason: "time up".into(), ready: true, ..board }, Some("home"))
        }
        "pause" => pause_layout(w, h, None, true),
        _ => return None,
    })
}

/// Audits every screen at the engine's check sizes; each entry is `(screen, size, problem)`.
pub fn audit_all() -> Vec<(String, (u32, u32), String)> {
    let mut out = Vec::new();
    for name in all() {
        for &(w, h) in &red_engine2::ui::screens::CHECK_SIZES {
            if let Some(l) = build(name, w, h) {
                for v in l.check() {
                    out.push((name.to_string(), (w, h), format!("[{}] {}: {}", v.code, v.widget, v.message)));
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_killchain_screen_passes_the_audit_at_every_size() {
        let v = audit_all();
        assert!(
            v.is_empty(),
            "{} layout problem(s):\n{}",
            v.len(),
            v.iter().map(|(s, (w, h), m)| format!("  {s} {w}x{h}: {m}")).collect::<Vec<_>>().join("\n")
        );
    }

    #[test]
    fn the_code_a_host_reads_out_is_never_cut_short_on_the_lobby_screen() {
        // The relay code carries the join key (`H3PQXR-K7Q2-MZ4P-WTXA`): an ellipsis in it would be a code nobody can join with.
        let code = "H3PQXR-K7Q2-MZ4P-WTXA";
        for &(w, h) in &super::super::screens::CHECK_SIZES {
            let view = LobbyView {
                roster: demo_roster(),
                me: 0,
                ready: false,
                kill_limit: 50,
                time_limit_secs: 600,
                countdown: None,
                join_address: Some(code.into()),
                message: None,
                hosting: true,
                map: "foundry".into(),
                mode: 0,
                team_size: 6,
                look: 1,
            };
            let l = lobby_layout(w, h, &view, None);
            let shown = l.widgets.iter().find(|x| x.id == "join_address").and_then(|x| x.text.clone()).unwrap_or_default();
            assert_eq!(shown, format!("FRIENDS JOIN AT  {code}"), "{w}x{h}: the whole code is on screen, not an ellipsis");
        }
    }

    #[test]
    fn the_home_menu_is_minimal_and_its_buttons_map_to_actions() {
        let l = home_layout(1280, 720, None, "v1");
        let buttons: Vec<&str> = l.widgets.iter().filter(|w| w.kind == red_engine2::ui::Kind::Button).map(|w| w.id.as_str()).collect();
        assert_eq!(buttons, ["solo", "host", "join", "stats", "quit"]);
        assert_eq!(home_action("stats"), Some(HomeAction::Stats));
        assert_eq!(home_action("nope"), None);
        let (x, y) = {
            let r = l.rect_of("join").unwrap();
            (((r.0 + r.2) / 2) as f32, ((r.1 + r.3) / 2) as f32)
        };
        assert_eq!(l.button_at(x, y), Some("join"));
    }

    #[test]
    fn setup_defaults_have_no_bots_and_every_choice_is_a_button() {
        let o = Setup::default();
        assert!(!o.bots, "a game holds only the people who joined unless bots are asked for");
        let l = setup_layout(1280, 720, "solo", &o, None, None);
        for id in ["bots_off", "skill_0", "skill_2", "mode_0", "mode_3", "size_1", "size_6", "kills_25", "kills_0", "time_5", "time_0", "name", "start", "back"]
        {
            assert!(l.rect_of(id).is_some(), "{id}");
            assert!(setup_action(id).is_some(), "{id}");
        }
        assert_eq!(setup_action("map_2"), Some(SetupAction::Map(2)));
        // With several maps installed there is a MAP row; with one there is none.
        let three =
            setup_layout(1280, 720, "solo", &Setup { maps: vec!["Works".into(), "Quarry".into(), "Depot".into()], map: 1, ..Default::default() }, None, None);
        assert!(three.rect_of("map_0").is_some() && three.rect_of("map_2").is_some());
        assert!(setup_layout(1280, 720, "solo", &Setup { maps: vec!["Works".into()], ..Default::default() }, None, None).rect_of("map_0").is_none());
        assert_eq!(setup_action("mode_2"), Some(SetupAction::Mode(2)));
        assert_eq!(setup_action("mode_9"), None);
        assert_eq!(setup_action("size_1"), Some(SetupAction::Size(1)));
        // Each mode's own limits are on offer: captures for capture the flag, rounds for search and destroy.
        for (mode, limit) in [(2u8, 5u16), (3, 6), (1, 30)] {
            let l = setup_layout(1280, 720, "solo", &Setup { mode, kill_limit: limit, ..Default::default() }, None, None);
            assert!(l.rect_of(&format!("kills_{limit}")).is_some(), "mode {mode} offers {limit}");
        }
        assert_eq!(setup_action("kills_75"), Some(SetupAction::Kills(75)));
        assert_eq!(setup_action("time_0"), Some(SetupAction::Minutes(0)));
        assert_eq!(setup_action("bots_on"), Some(SetupAction::Bots(true)), "the id still parses even though the screen has no such button");
    }

    #[test]
    fn the_hud_is_extremely_minimal() {
        let l = build("hud", 1280, 720).unwrap();
        let texts: Vec<&str> = l.widgets.iter().filter_map(|w| w.text.as_deref()).collect();
        assert!(texts.len() <= 10, "a handful of labels, no more: {texts:?}");
        assert!(texts.contains(&"100") && texts.contains(&"17"), "health and magazine: {texts:?}");
        assert!(l.widgets.iter().all(|w| w.fill.is_none()), "no panels, no boxes: nothing behind the numbers");
    }

    #[test]
    fn knife_label_and_optic_cross_remain_legible_at_small_and_large_resolutions() {
        for (w, h) in [(640, 360), (1280, 720), (1920, 1080)] {
            let hud = hud_layout(w, h, &HudView { weapon: "combat knife".into(), ..Default::default() });
            assert_eq!(hud.widgets.iter().find(|w| w.id == "hand").unwrap().text.as_deref(), Some("KNIFE"));
            let mut cv = Canvas::new(w, h);
            paint_optic_reticle(&mut cv);
            let at = ((h / 2 * w + w / 2) * 4) as usize;
            assert_eq!(&cv.px[at..at + 4], &[240, 70, 45, 255]);
            assert_eq!(cv.px[3], 0, "open optics leave the rest of the view clear");
        }
    }

    #[test]
    fn the_results_screen_asks_to_play_again_or_go_home() {
        let l = build("results", 1280, 720).unwrap();
        assert_eq!(results_action("again"), Some(ResultsAction::PlayAgain));
        assert_eq!(results_action("home"), Some(ResultsAction::Home));
        assert!(l.rect_of("again").is_some() && l.rect_of("home").is_some());
        let banner = l.widgets.iter().find(|w| w.id == "banner").unwrap();
        assert_eq!(banner.text.as_deref(), Some("RIDGEBACK WINS"));
    }

    #[test]
    fn the_lobby_shows_both_teams_and_the_scope_paints_a_circle() {
        let l = build("lobby", 1280, 720).unwrap();
        assert!(l.rect_of("team_1").is_some() && l.rect_of("team_2").is_some());
        assert_eq!(lobby_action("team_2"), Some(LobbyAction::Team(2)));
        let mut cv = Canvas::new(400, 300);
        paint_scope(&mut cv);
        let px = |x: usize, y: usize| cv.px[(y * 400 + x) * 4 + 3];
        assert_eq!(px(2, 2), 255, "the corners are black");
        assert_eq!(px(250, 100), 0, "the glass is clear");
    }
}
