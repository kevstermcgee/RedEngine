//! The online screens: **connect form, lobby, in-game HUD, results, connection banner** (ADR 0029), built on [`super::Layout`] like every
//! other screen, so they are painted, hit-tested and audited from one list of rectangles and `red_engine2 ui-shot` / `ui-check` cover them
//! at every window size without a GPU.
//!
//! Nothing here knows about sockets. A screen is a pure function of an [`OnlineView`] (what the client currently knows: phase, timer,
//! roster, who "me" is) or a [`ConnectForm`] (the text the player has typed), and a click is mapped to an [`OnlineAction`] by the id of
//! the button under the cursor. The graphical client fills the view from its `NetClient`; a game project can do the same, or draw its own
//! screens with the same helpers.

use super::{ellipsize, fit_scale, text_height, text_width, Layout, Rect};
use crate::net::protocol::{RosterEntry, ROSTER_IN_ROUND, ROSTER_READY};
use crate::sim::flow::Phase;
use crate::sim::kart::Driver;

const TEXT: [u8; 4] = [236, 238, 245, 255];
const DIM: [u8; 4] = [150, 156, 176, 255];
const GOLD: [u8; 4] = [255, 210, 74, 255];
const GREEN: [u8; 4] = [120, 230, 140, 255];
const RED: [u8; 4] = [255, 120, 110, 255];
const PANEL: [u8; 4] = [14, 17, 28, 236];
const EDGE: [u8; 4] = [90, 98, 130, 255];

/// What the client currently knows about the match: everything the online screens show.
#[derive(Debug, Clone)]
pub struct OnlineView {
    /// The phase of the match.
    pub phase: Phase,
    /// The round (`0` before the first).
    pub round: u16,
    /// Whole seconds until the phase changes by itself (`None` = no limit).
    pub secs_left: Option<u32>,
    /// Players needed before a countdown can begin.
    pub min_players: u8,
    /// Our player id.
    pub me: u8,
    /// Whether we have a body in the running round.
    pub in_round: bool,
    /// Everyone in the match, sorted by id.
    pub roster: Vec<RosterEntry>,
    /// Winner's player id after a round (`255` = none).
    pub winner: u8,
    /// Why the round ended (`0` rules, `1` time up, `2` score reached, `3` abandoned, `255` no round has ended).
    pub end_code: u8,
    /// The rule outcome word when `end_code == 0`.
    pub end_text: String,
    /// Our smoothed ping, ms.
    pub ping_ms: f32,
    /// The map's name.
    pub map: String,
    /// Whether the connection is currently lost (the client is trying again).
    pub reconnecting: bool,
    /// A refusal or other message to show (`None` = nothing).
    pub message: Option<String>,
    /// Our own combat state, for the shooter HUD (`None` = draw no health or weapon).
    pub combat: Option<CombatView>,
    /// Whether the map is a race: the character byte is then the animal to drive, and the lobby lets everyone choose one.
    pub race: bool,
}

/// What the shooter HUD shows about us: health, the weapon in hand and where it sits on the weapon ladder, and whether we are waiting to respawn.
#[derive(Debug, Clone, PartialEq)]
pub struct CombatView {
    /// Hit points.
    pub hp: u32,
    /// Full health.
    pub max_hp: u32,
    /// Name of the weapon in hand.
    pub weapon: String,
    /// `(rung, rungs)`: our 1-based position on the weapon ladder and its length (`None` = no ladder).
    pub rung: Option<(u32, u32)>,
    /// The weapon the next kill hands us (`None` on the last rung or with no ladder).
    pub next_weapon: Option<String>,
    /// Waiting to respawn.
    pub dead: bool,
    /// Whole seconds until we respawn (while dead).
    pub respawn_secs: u32,
    /// Spawn protection is active.
    pub protected: bool,
    /// A short line to flash (a level-up), if any.
    pub notice: Option<String>,
}

impl CombatView {
    /// A sample for `ui-shot` / `ui-check`: mid-ladder with a long weapon name, hurt.
    pub fn demo() -> CombatView {
        CombatView {
            hp: 62,
            max_hp: 100,
            weapon: "Longbow marksman rifle".into(),
            rung: Some((9, 12)),
            next_weapon: Some("Breach shotgun".into()),
            dead: false,
            respawn_secs: 0,
            protected: false,
            notice: Some("RUNG 9 - LONGBOW MARKSMAN RIFLE".into()),
        }
    }

    /// The same sample, dead and about to respawn.
    pub fn demo_dead() -> CombatView {
        CombatView { hp: 0, dead: true, respawn_secs: 2, notice: None, ..CombatView::demo() }
    }
}

impl OnlineView {
    /// Our own roster entry.
    pub fn me_entry(&self) -> Option<&RosterEntry> {
        self.roster.iter().find(|e| e.id == self.me)
    }

    /// Whether we pressed Ready.
    pub fn i_am_ready(&self) -> bool {
        self.me_entry().is_some_and(|e| e.flags & ROSTER_READY != 0)
    }

    /// Our character (`0` human, `1` rat).
    pub fn my_character(&self) -> u8 {
        self.me_entry().map_or(0, |e| e.character)
    }

    /// A sample view for `ui-shot` / `ui-check`: eight players (two ready) at `phase`, a long name among them.
    pub fn demo(phase: Phase) -> OnlineView {
        let names = ["Ada", "Bo", "Cheddar_The_Great_One", "Dee", "Eli", "Fay", "Gus", "Hana"];
        let roster = names
            .iter()
            .enumerate()
            .map(|(i, n)| RosterEntry {
                team: 0,
                id: i as u8,
                flags: if i % 3 == 0 { ROSTER_READY } else { 0 } | ROSTER_IN_ROUND,
                character: (i % 2) as u8,
                ping_ms: 12 + i as u16 * 17,
                score: [7, 3, 0, 5, 1, 9, 2, 4][i],
                name: n.to_string(),
            })
            .collect();
        OnlineView {
            phase,
            round: 3,
            secs_left: Some(if phase == Phase::Countdown { 3 } else { 95 }),
            min_players: 2,
            me: 1,
            in_round: phase == Phase::Playing,
            roster,
            winner: 5,
            end_code: 2,
            end_text: String::new(),
            ping_ms: 29.0,
            map: "test_lab".to_string(),
            reconnecting: false,
            message: None,
            race: false,
            combat: match phase {
                Phase::Playing => Some(CombatView::demo()),
                Phase::Countdown => Some(CombatView { notice: None, ..CombatView::demo() }),
                _ => None,
            },
        }
    }
}

/// Which online screen a view needs right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnlineScreen {
    /// Waiting for players and pressing Ready (in a race, choosing an animal).
    Lobby,
    /// The round is over.
    Results,
    /// In the world (playing, in a countdown, or watching): only the HUD is drawn over it.
    Hud,
}

/// The screen for `view`: the lobby and the results take over the window; everything else is the HUD over the world.
pub fn screen_for(view: &OnlineView) -> OnlineScreen {
    match view.phase {
        Phase::Waiting => OnlineScreen::Lobby,
        Phase::Results => OnlineScreen::Results,
        Phase::Countdown | Phase::Playing => OnlineScreen::Hud,
    }
}

/// What a click or key on an online screen asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnlineAction {
    /// Press or release Ready (a rematch vote on the results screen).
    ToggleReady,
    /// The previous free animal (a race lobby).
    PrevDriver,
    /// The next free animal (a race lobby).
    NextDriver,
    /// Leave the match and close the game.
    Leave,
}

/// The action for the button with this id (`ready`, `driver_prev`, `driver_next`, `leave`), if any.
pub fn action_for(button: &str) -> Option<OnlineAction> {
    match button {
        "ready" => Some(OnlineAction::ToggleReady),
        "driver_prev" => Some(OnlineAction::PrevDriver),
        "driver" | "driver_next" => Some(OnlineAction::NextDriver),
        "leave" => Some(OnlineAction::Leave),
        _ => None,
    }
}

/// The action under the cursor at `(x, y)` on `layout`.
pub fn action_at(layout: &Layout, x: f32, y: f32) -> Option<OnlineAction> {
    layout.button_at(x, y).and_then(action_for)
}

/// The wire byte of the animal `step` places (`1` next, `-1` previous) from ours, skipping animals other people have chosen; ours again if all are taken.
pub fn step_driver(v: &OnlineView, step: i8) -> u8 {
    let n = Driver::ALL.len() as i16;
    let mine = i16::from(v.my_character()).min(n - 1);
    for k in 1..=n {
        let cand = (mine + i16::from(step) * k).rem_euclid(n) as u8;
        if !v.roster.iter().any(|e| e.id != v.me && e.character == cand) {
            return cand;
        }
    }
    mine as u8
}

/// A roster entry's character as text: a body name, or in a race the animal.
fn what_name(race: bool, c: u8) -> &'static str {
    if race {
        Driver::from_wire(c).map_or("?", Driver::name)
    } else {
        character_name(c)
    }
}

fn character_name(c: u8) -> &'static str {
    crate::net::protocol::character_from_wire(c).name()
}

/// `M:SS` for a number of seconds.
pub fn clock(secs: u32) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

fn upper(s: &str) -> String {
    s.to_uppercase()
}

/// `text` cut with `...` to fit `max_w` at `scale`, keeping every row of a table at one size; the scale only drops (to 1) when even
/// the ellipsis cannot fit.
fn clip(text: &str, max_w: i32, scale: i32) -> (String, i32) {
    let scale = if text_width("...", scale) > max_w { fit_scale("...", max_w, scale) } else { scale };
    (ellipsize(text, max_w, scale), scale)
}

impl Layout {
    /// A label whose left edge is at `x0` (a table column).
    #[allow(clippy::too_many_arguments)]
    pub fn label_left(&mut self, id: &str, container: Option<usize>, x0: i32, y: i32, text: &str, scale: i32, max_w: i32, color: [u8; 4]) -> usize {
        let (text, scale) = clip(text, max_w, scale);
        let cx = x0 + text_width(&text, scale) / 2;
        self.label(id, container, cx, y, &text, scale, color)
    }

    /// A label whose right edge is at `x1`.
    #[allow(clippy::too_many_arguments)]
    pub fn label_right(&mut self, id: &str, container: Option<usize>, x1: i32, y: i32, text: &str, scale: i32, max_w: i32, color: [u8; 4]) -> usize {
        let (text, scale) = clip(text, max_w, scale);
        let cx = x1 - text_width(&text, scale) / 2;
        self.label(id, container, cx, y, &text, scale, color)
    }
}

/// A centred, bordered card for the lobby and the results; returns `(layout, card index, rect, base scale)`.
fn card(w: u32, h: u32, want_w: i32, rows_h: i32, id: &str) -> (Layout, usize, Rect, i32) {
    let mut l = Layout::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = (hi / 240).max(1);
    l.panel("backdrop", (0, 0, wi, hi), None, Some([4, 6, 12, 150]), None);
    // On a tiny window (scale 1) the card takes most of the width so table columns are not ellipsized to nothing.
    let cw = if s <= 1 { wi * 3 / 4 } else { want_w * s }.min(wi - 8);
    let ch = rows_h.min(hi - 8);
    let (x0, y0) = ((wi - cw) / 2, ((hi - ch) / 2).max(4));
    let rect = (x0, y0, x0 + cw, y0 + ch);
    let c = l.panel(id, rect, None, Some(PANEL), Some((EDGE, (s / 2).max(1))));
    (l, c, rect, s)
}

/// One roster row's text, gold when it is ours.
struct Cols {
    name: (i32, i32),
    what: (i32, i32),
    ping: (i32, i32),
    state: (i32, i32),
}

fn columns(x0: i32, x1: i32) -> Cols {
    let w = x1 - x0;
    let (a, b, c) = (x0 + w * 31 / 100, x0 + w * 51 / 100, x0 + w * 70 / 100);
    Cols { name: (x0, a - 4), what: (a, b - 4), ping: (b, c - 4), state: (c, x1) }
}

/// The results table has no ping column, so the type of character gets the room instead of being cut to `CO...`.
fn columns_results(x0: i32, x1: i32) -> Cols {
    let w = x1 - x0;
    let (a, b) = (x0 + w * 36 / 100, x0 + w * 72 / 100);
    Cols { name: (x0, a - 4), what: (a, b - 4), ping: (b, b), state: (b, x1) }
}

/// The lobby: who is here, who is ready, and buttons to ready up (and, in a race, pick an animal) and leave.
pub fn lobby_layout(w: u32, h: u32, v: &OnlineView, hover: Option<&str>) -> Layout {
    let (wi, hi) = (w as i32, h as i32);
    let s = (hi / 240).max(1);
    let rows = v.roster.len().clamp(1, 8) as i32;
    let row_h = text_height(s) + 3 * s;
    let bh = 14 * s;
    let msg_lines = v.message.as_deref().map_or(0, |m| super::wrap(&upper(m), (200 * s).min(wi - 8) - 12 * s, s).len().min(3) as i32);
    // Vertical budget: title, status, header, rows, buttons, hint, message.
    let content = 8 * s
        + text_height(s * 2)
        + 4 * s
        + text_height(s)
        + 6 * s
        + text_height(s)
        + 4 * s
        + rows * row_h
        + 8 * s
        + 2 * (bh + 3 * s)
        + bh
        + 6 * s
        + text_height(s)
        + msg_lines * (text_height(s) + 2 * s)
        + if v.race { text_height(s) + 3 * s } else { 0 }
        + 8 * s;
    let (mut l, c, rect, s) = card(w, h, 230, content, "card");
    let (x0, y0, x1, _) = rect;
    let cx = (x0 + x1) / 2;
    let inner = x1 - x0 - 16 * s;
    let pad = 8 * s;
    let mut y = y0 + pad;
    l.label_fit("title", Some(c), cx, y, "LOBBY", s * 2, inner, TEXT);
    y += text_height(s * 2) + 4 * s;
    let status = if v.reconnecting {
        "CONNECTION LOST - RECONNECTING...".to_string()
    } else if (v.roster.len() as u8) < v.min_players {
        format!("WAITING FOR PLAYERS ({}/{})", v.roster.len(), v.min_players)
    } else if v.roster.iter().all(|e| e.flags & ROSTER_READY != 0) {
        "EVERYONE IS READY - STARTING".to_string()
    } else {
        let waiting = v.roster.iter().filter(|e| e.flags & ROSTER_READY == 0).count();
        format!("WAITING FOR {waiting} PLAYER{} TO READY UP", if waiting == 1 { "" } else { "S" })
    };
    l.label_fit("status", Some(c), cx, y, &status, s, inner, GOLD);
    y += text_height(s) + 6 * s;
    let cols = columns(x0 + pad, x1 - pad);
    for (id, label, (a, b), right) in [
        ("h_name", "NAME", cols.name, false),
        ("h_what", if v.race { "ANIMAL" } else { "TYPE" }, cols.what, false),
        ("h_ping", "PING", cols.ping, false),
        ("h_state", "STATE", cols.state, true),
    ] {
        if right {
            l.label_right(id, Some(c), b, y, label, s, b - a, DIM);
        } else {
            l.label_left(id, Some(c), a, y, label, s, b - a, DIM);
        }
    }
    y += text_height(s) + 4 * s;
    for (i, e) in v.roster.iter().take(8).enumerate() {
        let mine = e.id == v.me;
        let col = if mine { GOLD } else { TEXT };
        l.label_left(&format!("r{i}_name"), Some(c), cols.name.0, y, &upper(&e.name), s, cols.name.1 - cols.name.0, col);
        l.label_left(&format!("r{i}_what"), Some(c), cols.what.0, y, what_name(v.race, e.character), s, cols.what.1 - cols.what.0, col);
        l.label_left(&format!("r{i}_ping"), Some(c), cols.ping.0, y, &format!("{} MS", e.ping_ms), s, cols.ping.1 - cols.ping.0, DIM);
        let (txt, tc) = if e.flags & ROSTER_READY != 0 { ("READY", GREEN) } else { ("NOT READY", DIM) };
        l.label_right(&format!("r{i}_state"), Some(c), cols.state.1, y, txt, s, cols.state.1 - cols.state.0, tc);
        y += row_h;
    }
    y += 8 * s;
    let (bx0, bx1) = (x0 + 24 * s, x1 - 24 * s);
    let ready = v.i_am_ready();
    let button = |l: &mut Layout, id: &str, y: i32, label: &str, good: bool| {
        let hot = hover == Some(id);
        let scale = fit_scale(label, bx1 - bx0 - 6 * s, s * 3 / 2);
        let edge = if good {
            GREEN
        } else if hot {
            GOLD
        } else {
            EDGE
        };
        l.button(
            id,
            (bx0, y, bx1, y + bh),
            Some(c),
            label,
            scale,
            if hot { [56, 62, 92, 255] } else { [30, 34, 52, 255] },
            (edge, (s / 2).max(1)),
            if good || hot { edge } else { TEXT },
        );
    };
    button(&mut l, "ready", y, if ready { "READY - CLICK TO CANCEL" } else { "READY" }, ready);
    y += bh + 3 * s;
    if v.race {
        // The animal to drive: previous / next arrows round its name, and a line about what it is good at.
        let arrow = 2 * bh;
        let hot = |id: &str| hover == Some(id);
        for (id, x_a, x_b, label) in [("driver_prev", bx0, bx0 + arrow, "<"), ("driver_next", bx1 - arrow, bx1, ">")] {
            l.button(
                id,
                (x_a, y, x_b, y + bh),
                Some(c),
                label,
                s * 3 / 2,
                if hot(id) { [56, 62, 92, 255] } else { [30, 34, 52, 255] },
                (if hot(id) { GOLD } else { EDGE }, (s / 2).max(1)),
                TEXT,
            );
        }
        let me_driver = Driver::from_wire(v.my_character()).unwrap_or(Driver::Duck);
        let name = upper(me_driver.name());
        let scale = fit_scale(&name, bx1 - bx0 - 2 * arrow - 8 * s, s * 3 / 2);
        l.button(
            "driver",
            (bx0 + arrow + 2 * s, y, bx1 - arrow - 2 * s, y + bh),
            Some(c),
            &name,
            scale,
            if hot("driver") { [56, 62, 92, 255] } else { [30, 34, 52, 255] },
            (if hot("driver") { GOLD } else { GREEN }, (s / 2).max(1)),
            GOLD,
        );
        y += bh + 3 * s;
        l.label_fit("perk", Some(c), cx, y, me_driver.spec().ability.blurb(), s, inner, DIM);
        y += text_height(s) + 3 * s;
    }
    button(&mut l, "leave", y, "LEAVE", false);
    y += bh + 6 * s;
    l.label_fit("hint", Some(c), cx, y, if v.race { "R READY   LEFT RIGHT ANIMAL   ESC LEAVE" } else { "R READY   ESC LEAVE" }, s, inner, DIM);
    y += text_height(s) + 2 * s;
    if let Some(m) = &v.message {
        for (i, line) in super::wrap(&upper(m), inner, s).iter().take(3).enumerate() {
            l.label(&format!("message_{}", i + 1), Some(c), cx, y + i as i32 * (text_height(s) + 2 * s), line, s, RED);
        }
    }
    l
}

/// The results: the winner, the scoreboard, and a rematch button (Ready again) beside Leave.
pub fn results_layout(w: u32, h: u32, v: &OnlineView, hover: Option<&str>) -> Layout {
    let s = (h as i32 / 240).max(1);
    let rows = v.roster.len().clamp(1, 8) as i32;
    let row_h = text_height(s) + 3 * s;
    let bh = 14 * s;
    let content = 8 * s + text_height(s * 2) + 4 * s + text_height(s) + 6 * s + text_height(s) + 4 * s + rows * row_h + 8 * s + 2 * (bh + 3 * s) + 8 * s;
    let (mut l, c, rect, s) = card(w, h, 200, content, "card");
    let (x0, y0, x1, _) = rect;
    let cx = (x0 + x1) / 2;
    let inner = x1 - x0 - 16 * s;
    let pad = 8 * s;
    let mut y = y0 + pad;
    l.label_fit("title", Some(c), cx, y, &format!("ROUND {} COMPLETE", v.round), s * 2, inner, TEXT);
    y += text_height(s * 2) + 4 * s;
    let winner = v.roster.iter().find(|e| e.id == v.winner);
    let reason = match v.end_code {
        0 => v.end_text.to_uppercase(),
        1 => "TIME UP".to_string(),
        2 => "SCORE REACHED".to_string(),
        3 => "EVERYONE LEFT".to_string(),
        _ => String::new(),
    };
    let i_won = winner.is_some_and(|e| e.id == v.me);
    let headline = match (winner, reason.is_empty()) {
        (Some(_), true) if i_won => "YOU WIN!".to_string(),
        (Some(_), false) if i_won => format!("YOU WIN!  ({reason})"),
        (Some(e), true) => format!("WINNER: {}", upper(&e.name)),
        (Some(e), false) => format!("WINNER: {}  ({reason})", upper(&e.name)),
        (None, true) => "DRAW".to_string(),
        (None, false) => reason.clone(),
    };
    l.label_fit("headline", Some(c), cx, y, &headline, s * 3 / 2, inner, if i_won { GREEN } else { GOLD });
    y += text_height(s * 3 / 2) + 6 * s;
    let cols = columns_results(x0 + pad, x1 - pad);
    l.label_left("h_rank", Some(c), cols.name.0, y, "PLAYER", s, cols.name.1 - cols.name.0, DIM);
    l.label_left("h_what", Some(c), cols.what.0, y, "TYPE", s, cols.what.1 - cols.what.0, DIM);
    l.label_right("h_score", Some(c), cols.state.1, y, "KILLS", s, cols.state.1 - cols.ping.0, DIM);
    y += text_height(s) + 4 * s;
    let mut ranked: Vec<&RosterEntry> = v.roster.iter().take(8).collect();
    ranked.sort_by(|a, b| b.score.cmp(&a.score).then(a.id.cmp(&b.id)));
    for (i, e) in ranked.iter().enumerate() {
        let col = if e.id == v.me { GOLD } else { TEXT };
        l.label_left(&format!("r{i}_name"), Some(c), cols.name.0, y, &format!("{}. {}", i + 1, upper(&e.name)), s, cols.name.1 - cols.name.0, col);
        l.label_left(&format!("r{i}_what"), Some(c), cols.what.0, y, what_name(v.race, e.character), s, cols.what.1 - cols.what.0, DIM);
        l.label_right(&format!("r{i}_score"), Some(c), cols.state.1, y, &e.score.to_string(), s, cols.state.1 - cols.ping.0, col);
        y += row_h;
    }
    y += 8 * s;
    let (bx0, bx1) = (x0 + 24 * s, x1 - 24 * s);
    let ready = v.i_am_ready();
    let back = v.secs_left.map(|t| format!("  (LOBBY IN {t})")).unwrap_or_default();
    let labels = [("ready", if ready { "REMATCH - WAITING FOR THE OTHERS".to_string() } else { format!("REMATCH{back}") }), ("leave", "LEAVE".to_string())];
    for (id, label) in labels {
        let hot = hover == Some(id);
        let good = id == "ready" && ready;
        let scale = fit_scale(&label, bx1 - bx0 - 6 * s, s * 3 / 2);
        let edge = if good {
            GREEN
        } else if hot {
            GOLD
        } else {
            EDGE
        };
        l.button(
            id,
            (bx0, y, bx1, y + bh),
            Some(c),
            &label,
            scale,
            if hot { [56, 62, 92, 255] } else { [30, 34, 52, 255] },
            (edge, (s / 2).max(1)),
            if good || hot { edge } else { TEXT },
        );
        y += bh + 3 * s;
    }
    l
}

/// The in-game HUD: ping (top left), round and timer (top centre), a compact scoreboard (top right), a big countdown or a "watching"
/// banner. The crosshair is drawn by the renderer; this layout adds only what a round needs, and stays out of the middle of the screen
/// except for the countdown. `cfg` (the scene's `hud` block) turns the ping, the round clock, the scoreboard and the combat readout
/// on and off; the countdown and connection banners are never suppressed (they are how a player learns what the game is doing).
pub fn hud_layout(w: u32, h: u32, v: &OnlineView, cfg: &crate::hud_config::HudConfig) -> Layout {
    let mut l = Layout::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = (hi / 240).max(1);
    let m = 3 * s;
    if cfg.enabled && cfg.show_ping {
        l.label_left("ping", None, m, m, &format!("PING {:.0} MS", v.ping_ms), s, wi / 4, if v.ping_ms > 150.0 { RED } else { DIM });
    }
    let head = match (v.phase, v.secs_left) {
        (Phase::Playing, Some(t)) => format!("ROUND {}  {}", v.round, clock(t)),
        (Phase::Playing, None) => format!("ROUND {}", v.round),
        _ => format!("ROUND {}", v.round),
    };
    if cfg.enabled && cfg.show_round {
        l.label_fit("timer", None, wi / 2, m, &head, s * 3 / 2, wi / 3, TEXT);
    }
    // Scoreboard: name and kills, top right, best first.
    let mut ranked: Vec<&RosterEntry> = if cfg.enabled && cfg.show_scoreboard { v.roster.iter().take(8).collect() } else { Vec::new() };
    ranked.sort_by(|a, b| b.score.cmp(&a.score).then(a.id.cmp(&b.id)));
    let col_w = (wi / 5).max(40 * s);
    let mut y = m;
    for (i, e) in ranked.iter().enumerate() {
        let col = if e.id == v.me { GOLD } else { TEXT };
        l.label_right(&format!("sb{i}_score"), None, wi - m, y, &e.score.to_string(), s, col_w / 3, col);
        l.label_right(&format!("sb{i}_name"), None, wi - m - col_w / 3 - 2 * s, y, &upper(&e.name), s, col_w * 2 / 3, col);
        y += text_height(s) + 2 * s;
    }
    if let Some(c) = v.combat.as_ref().filter(|_| cfg.shows_combat()) {
        combat_hud(&mut l, v, c);
    }
    if v.reconnecting {
        l.label_fit("banner", None, wi / 2, hi / 2 - text_height(s * 2) / 2, "CONNECTION LOST - RECONNECTING...", s * 2, wi - 8, RED);
    } else if v.phase == Phase::Countdown {
        let secs = v.secs_left.unwrap_or(0).to_string();
        let big = l.label("count", None, wi / 2, hi * 30 / 100, &secs, s * 8, GOLD);
        l.widgets[big].shadow = false; // a shadow offset by one big pixel reads as a glitch at this size
        let hint_y = hi * 30 / 100 + text_height(s * 8) + 6 * s;
        l.label_fit("count_hint", None, wi / 2, hint_y, "GET READY", s * 2, wi - 8, TEXT);
        // What the round is about, for someone who has not read anything, and the keys.
        if let Some((_, rungs)) = v.combat.as_ref().and_then(|c| c.rung) {
            let text = format!("FIRST TO {rungs} KILLS WINS - EVERY KILL GIVES YOU THE NEXT GUN");
            l.label_fit("objective", None, wi / 2, hint_y + text_height(s * 2) + 6 * s, &text, s, wi - 8, GOLD);
        }
        l.label_fit(
            "controls",
            None,
            wi / 2,
            hi * 80 / 100,
            "WASD MOVE   MOUSE AIM   CLICK FIRE   RIGHT CLICK SIGHTS   SPACE JUMP   SHIFT SPRINT",
            s,
            wi - 8,
            DIM,
        );
    } else if !v.in_round {
        l.label_fit("banner", None, wi / 2, hi - text_height(s * 2) - 8 * s, "ROUND IN PROGRESS - YOU JOIN THE NEXT ONE", s * 2, wi - 8, GOLD);
    }
    l
}

/// The player other than us who is on the last rung of a ladder of `rungs` (one kill from winning), the highest scorer if several.
pub fn final_rung_rival(v: &OnlineView, rungs: u32) -> Option<&RosterEntry> {
    v.roster.iter().filter(|e| e.id != v.me && e.score as u32 + 1 == rungs).max_by_key(|e| e.score)
}

/// The shooter's part of the HUD: a health bar (bottom left), the weapon and the ladder rungs (bottom right), who is winning (under the
/// timer), and the "eliminated" banner while waiting to respawn.
fn combat_hud(l: &mut Layout, v: &OnlineView, c: &CombatView) {
    let (wi, hi) = (l.w, l.h);
    let s = (hi / 240).max(1);
    let m = 3 * s;
    let backdrop = [8, 10, 18, 165];

    // Health: a framed bar with the number inside it, green then gold then red.
    let (bw, bh) = ((72 * s).min(wi / 3), 12 * s);
    let bar = (m, hi - m - bh, m + bw, hi - m);
    let frame = l.panel("hp_bar", bar, None, Some([10, 12, 20, 210]), Some((EDGE, (s / 2).max(1))));
    let inner = (bar.0 + s, bar.1 + s, bar.2 - s, bar.3 - s);
    let frac = if c.max_hp == 0 { 0.0 } else { (c.hp as f32 / c.max_hp as f32).clamp(0.0, 1.0) };
    let fill_w = ((inner.2 - inner.0) as f32 * frac).round() as i32;
    if fill_w > 0 {
        let color = if frac > 0.5 {
            [70, 190, 100, 240]
        } else if frac > 0.25 {
            [235, 185, 60, 240]
        } else {
            [225, 65, 55, 240]
        };
        l.panel("hp_fill", (inner.0, inner.1, inner.0 + fill_w, inner.3), Some(frame), Some(color), None);
    }
    let hp_scale = (s * 3 / 2).max(1);
    l.label("hp_text", Some(frame), (bar.0 + bar.2) / 2, bar.1 + (bh - text_height(hp_scale)) / 2, &c.hp.to_string(), hp_scale, TEXT);

    // The weapon in hand, and the ladder: one pip per rung, gold up to ours, on a dark backdrop so it reads over any scene.
    let right = wi - m;
    let max_w = (wi / 2 - m).max(40 * s);
    let name = upper(&c.weapon);
    let name_scale = fit_scale(&name, max_w, s * 3 / 2);
    let mut sub = String::new();
    if let Some((rung, rungs)) = c.rung {
        sub = format!("RUNG {rung}/{rungs}");
    }
    if let Some(next) = &c.next_weapon {
        sub = format!("{sub}  NEXT: {}", upper(next));
    }
    let sub = ellipsize(sub.trim(), max_w, s);
    let (pip_w, pip_h, gap) = (5 * s, 3 * s, s);
    let rungs = c.rung.map_or(0, |(_, n)| n.clamp(1, 16) as i32);
    let pips_w = if rungs > 0 { rungs * (pip_w + gap) - gap } else { 0 };
    let block_w = text_width(&name, name_scale).max(text_width(&sub, s)).max(pips_w);
    let mut block_h = text_height(name_scale);
    if !sub.is_empty() {
        block_h += text_height(s) + 2 * s;
    }
    if rungs > 0 {
        block_h += pip_h + 3 * s;
    }
    let (px1, py1) = (right + m / 2, hi - m / 2);
    l.panel("weapon_panel", (right - block_w - m, py1 - block_h - m - m / 2, px1, py1), None, Some(backdrop), None);
    let mut y = hi - m - m / 2;
    if rungs > 0 {
        let (rung, _) = c.rung.unwrap_or((0, 0));
        y -= pip_h;
        let x0 = right - pips_w;
        for i in 0..rungs {
            let (color, lift) = match (i + 1).cmp(&(rung as i32)) {
                std::cmp::Ordering::Less => ([205, 160, 45, 255], 0),
                std::cmp::Ordering::Equal => ([255, 244, 180, 255], s),
                std::cmp::Ordering::Greater => ([64, 70, 94, 255], 0),
            };
            let px = x0 + i * (pip_w + gap);
            l.panel(&format!("rung_{i}"), (px, y - lift, px + pip_w, y + pip_h), None, Some(color), None);
        }
        y -= 3 * s;
    }
    y -= text_height(name_scale);
    l.label_right("weapon", None, right, y, &name, name_scale, max_w, TEXT);
    if !sub.is_empty() {
        y -= 2 * s + text_height(s);
        l.label_right("rung_text", None, right, y, &sub, s, max_w, GOLD);
    }

    // Who is winning, under the timer.
    if let Some((_, rungs)) = c.rung {
        if let Some(best) = v.roster.iter().max_by(|a, b| a.score.cmp(&b.score).then(b.id.cmp(&a.id))) {
            let line =
                if best.id == v.me { format!("YOU LEAD  {}/{}", best.score, rungs) } else { format!("LEADER {}  {}/{}", upper(&best.name), best.score, rungs) };
            let color = if best.id == v.me { GOLD } else { DIM };
            l.label_fit("leader", None, wi / 2, 3 * s + text_height(s * 3 / 2) + 2 * s, &line, s, wi / 3, color);
        }
    }
    // The drama of a ladder: one kill from winning is said out loud, whether it is us or someone else.
    if let Some((rung, rungs)) = c.rung.filter(|_| !c.dead) {
        let line_y = 3 * s + text_height(s * 3 / 2) + 2 * s + text_height(s) + 2 * s;
        if rung >= rungs {
            l.label_fit("final", None, wi / 2, line_y, "FINAL WEAPON - ONE KILL TO WIN", s * 3 / 2, wi / 2, GOLD);
        } else if let Some(e) = final_rung_rival(v, rungs) {
            l.label_fit("final", None, wi / 2, line_y, &format!("{} IS ON THE FINAL WEAPON", upper(&e.name)), s * 3 / 2, wi / 2, RED);
        }
    }
    if c.protected && !c.dead {
        l.label_fit("protected", None, wi / 2, hi - m - text_height(s), "SPAWN PROTECTED", s, wi / 3, [140, 220, 255, 255]);
    }
    if let (Some(text), false) = (&c.notice, c.dead) {
        l.label_fit("notice", None, wi / 2, hi * 62 / 100, text, s * 2, wi * 6 / 10, GOLD);
    }
    if c.dead {
        let title = l.label_fit("dead_title", None, wi / 2, hi * 34 / 100, "ELIMINATED", s * 4, wi - 8, RED);
        let below = l.widgets[title].rect.3 + 4 * s;
        let text = if c.respawn_secs > 0 { format!("RESPAWNING IN {}", c.respawn_secs) } else { "RESPAWNING".to_string() };
        l.label_fit("dead_hint", None, wi / 2, below, &text, s * 2, wi - 8, TEXT);
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------
// the connect form
// ---------------------------------------------------------------------------------------------------------------------------------

/// Which field of the connect form has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// `HOST:PORT`.
    Address,
    /// The join key (drawn as asterisks).
    Key,
    /// The player's name.
    Name,
}

/// What the player has typed on the connect screen.
#[derive(Debug, Clone)]
pub struct ConnectForm {
    /// `HOST` or `HOST:PORT`.
    pub address: String,
    /// The join key (may be empty for an open server).
    pub key: String,
    /// The display name.
    pub name: String,
    /// The field that receives typing.
    pub focus: Field,
    /// An error or progress line (`connecting...`, `refused: ...`).
    pub message: Option<String>,
}

impl ConnectForm {
    /// A form with the default port's loopback address.
    pub fn new(address: &str, key: &str, name: &str) -> Self {
        ConnectForm { address: address.to_string(), key: key.to_string(), name: name.to_string(), focus: Field::Address, message: None }
    }

    fn field_mut(&mut self) -> (&mut String, usize) {
        match self.focus {
            Field::Address => (&mut self.address, 64),
            Field::Key => (&mut self.key, 64),
            Field::Name => (&mut self.name, crate::net::protocol::MAX_NAME),
        }
    }

    /// Types a character into the focused field (control characters and overflow are ignored).
    pub fn type_char(&mut self, c: char) {
        let (text, max) = self.field_mut();
        if !c.is_control() && text.len() + c.len_utf8() <= max {
            text.push(c);
        }
    }

    /// Deletes the last character of the focused field.
    pub fn backspace(&mut self) {
        self.field_mut().0.pop();
    }

    /// Moves the keyboard to the next field (Tab).
    pub fn next_field(&mut self) {
        self.focus = match self.focus {
            Field::Address => Field::Key,
            Field::Key => Field::Name,
            Field::Name => Field::Address,
        };
    }

    /// The address with the default port added when none was typed.
    pub fn address_with_port(&self) -> String {
        let a = self.address.trim();
        if a.contains(':') {
            a.to_string()
        } else {
            format!("{a}:{}", crate::net::DEFAULT_PORT)
        }
    }
}

/// What a click on the connect screen asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectAction {
    /// Focus a field.
    Focus(Field),
    /// Try to join.
    Connect,
    /// Give up on joining a server and play by yourself.
    Back,
}

/// The action for the button with this id on the connect screen.
pub fn connect_action_for(button: &str) -> Option<ConnectAction> {
    match button {
        "field_address" => Some(ConnectAction::Focus(Field::Address)),
        "field_key" => Some(ConnectAction::Focus(Field::Key)),
        "field_name" => Some(ConnectAction::Focus(Field::Name)),
        "connect" => Some(ConnectAction::Connect),
        "back" => Some(ConnectAction::Back),
        _ => None,
    }
}

/// The connect screen: three text fields (server, join key, name), CONNECT and BACK.
pub fn connect_layout(w: u32, h: u32, f: &ConnectForm, hover: Option<&str>) -> Layout {
    let (wi, hi) = (w as i32, h as i32);
    let s = (hi / 240).max(1);
    let fh = 14 * s;
    let msg_lines = f.message.as_deref().map_or(0, |m| super::wrap(&upper(m), (180 * s).min(wi - 8) - 12 * s, s).len().min(3) as i32);
    let content =
        8 * s + text_height(s * 2) + 6 * s + 3 * (text_height(s) + 2 * s + fh + 4 * s) + 2 * (fh + 3 * s) + msg_lines * (text_height(s) + 2 * s) + 10 * s;
    let (mut l, c, rect, s) = card(w, h, 190, content, "card");
    let (x0, y0, x1, _) = rect;
    let cx = (x0 + x1) / 2;
    let inner = x1 - x0 - 16 * s;
    let mut y = y0 + 8 * s;
    l.label_fit("title", Some(c), cx, y, "PLAY ONLINE", s * 2, inner, TEXT);
    y += text_height(s * 2) + 6 * s;
    let (fx0, fx1) = (x0 + 10 * s, x1 - 10 * s);
    for (field, id, caption, value, mask) in [
        (Field::Address, "field_address", "SERVER  (HOST:PORT)", &f.address, false),
        (Field::Key, "field_key", "JOIN KEY  (OPTIONAL)", &f.key, true),
        (Field::Name, "field_name", "YOUR NAME", &f.name, false),
    ] {
        l.label_left(&format!("{id}_caption"), Some(c), fx0, y, caption, s, fx1 - fx0, DIM);
        y += text_height(s) + 2 * s;
        let focused = f.focus == field;
        let shown = if mask { "*".repeat(value.chars().count()) } else { upper(value) };
        // Show the *end* of a long value (where the caret is), cut on the left.
        let room = fx1 - fx0 - 6 * s - text_width("_", s);
        let mut chars: Vec<char> = shown.chars().collect();
        while text_width(&chars.iter().collect::<String>(), s) > room && !chars.is_empty() {
            chars.remove(0);
        }
        let text = format!("{}{}", chars.iter().collect::<String>(), if focused { "_" } else { "" });
        let hot = hover == Some(id);
        l.button(
            id,
            (fx0, y, fx1, y + fh),
            Some(c),
            &text,
            s,
            if focused { [28, 32, 50, 255] } else { [20, 23, 36, 255] },
            (
                if focused {
                    GOLD
                } else if hot {
                    TEXT
                } else {
                    EDGE
                },
                (s / 2).max(1),
            ),
            TEXT,
        );
        y += fh + 4 * s;
    }
    y += 2 * s;
    for (id, label) in [("connect", "CONNECT"), ("back", "PLAY SOLO")] {
        let hot = hover == Some(id);
        let scale = fit_scale(label, fx1 - fx0 - 6 * s, s * 3 / 2);
        l.button(
            id,
            (fx0, y, fx1, y + fh),
            Some(c),
            label,
            scale,
            if hot { [56, 62, 92, 255] } else { [30, 34, 52, 255] },
            (if hot { GOLD } else { EDGE }, (s / 2).max(1)),
            if hot { GOLD } else { TEXT },
        );
        y += fh + 3 * s;
    }
    if let Some(m) = &f.message {
        for (i, line) in super::wrap(&upper(m), inner, s).iter().take(3).enumerate() {
            l.label(&format!("message_{}", i + 1), Some(c), cx, y + i as i32 * (text_height(s) + 2 * s), line, s, RED);
        }
    }
    l
}

#[cfg(test)]
mod tests {
    #[test]
    fn stepping_through_animals_skips_the_ones_others_have() {
        let mut v = OnlineView::demo(Phase::Waiting);
        v.race = true;
        for (i, e) in v.roster.iter_mut().enumerate() {
            e.character = i as u8; // everyone has a different animal; we (id 1) have the Bunny
        }
        v.roster.truncate(4); // Duck (0), Bunny (1: us), Deer (2), Coyote (3) are taken
        assert_eq!(step_driver(&v, 1), 4, "the next free animal after the Bunny is the Hawk");
        assert_eq!(step_driver(&v, -1), 7, "the previous free animal wraps round to the Beaver");
        v.roster.retain(|e| e.id == v.me);
        assert_eq!(step_driver(&v, 1), 2, "alone, the next animal is simply the next");
    }

    use super::*;

    /// The arena look (every element on): what these tests are about.
    fn hud_layout(w: u32, h: u32, v: &OnlineView) -> Layout {
        super::hud_layout(w, h, v, &crate::hud_config::HudConfig::default())
    }

    #[test]
    fn the_screen_follows_the_phase() {
        for (p, want) in [
            (Phase::Waiting, OnlineScreen::Lobby),
            (Phase::Countdown, OnlineScreen::Hud),
            (Phase::Playing, OnlineScreen::Hud),
            (Phase::Results, OnlineScreen::Results),
        ] {
            assert_eq!(screen_for(&OnlineView::demo(p)), want, "{p:?}");
        }
    }

    #[test]
    fn buttons_are_where_they_are_painted_and_clicks_map_to_actions() {
        for (w, h) in [(480u32, 270u32), (1280, 720), (1920, 1080), (500, 640)] {
            let v = OnlineView::demo(Phase::Waiting);
            let l = lobby_layout(w, h, &v, None);
            let mid = |r: Rect| (((r.0 + r.2) / 2) as f32, ((r.1 + r.3) / 2) as f32);
            for (id, want) in [("ready", OnlineAction::ToggleReady), ("leave", OnlineAction::Leave)] {
                let (x, y) = mid(l.rect_of(id).unwrap_or_else(|| panic!("{id} missing at {w}x{h}")));
                assert_eq!(action_at(&l, x, y), Some(want), "{id} at {w}x{h}");
            }
            assert_eq!(action_at(&l, 1.0, 1.0), None, "outside the card");
            let r = results_layout(w, h, &OnlineView::demo(Phase::Results), None);
            let (x, y) = mid(r.rect_of("ready").unwrap());
            assert_eq!(action_at(&r, x, y), Some(OnlineAction::ToggleReady), "rematch at {w}x{h}");
        }
    }

    #[test]
    fn the_lobby_says_what_it_is_waiting_for() {
        let text = |v: &OnlineView| lobby_layout(1280, 720, v, None).widgets.iter().find(|w| w.id == "status").and_then(|w| w.text.clone()).unwrap();
        let mut v = OnlineView::demo(Phase::Waiting);
        assert_eq!(text(&v), "WAITING FOR 5 PLAYERS TO READY UP");
        v.roster.truncate(1);
        assert_eq!(text(&v), "WAITING FOR PLAYERS (1/2)");
        v.min_players = 1;
        v.roster[0].flags = ROSTER_READY;
        assert_eq!(text(&v), "EVERYONE IS READY - STARTING");
        v.reconnecting = true;
        assert!(text(&v).contains("RECONNECTING"));
    }

    #[test]
    fn the_ready_button_reflects_our_own_state() {
        let mut v = OnlineView::demo(Phase::Waiting);
        let label = |v: &OnlineView| lobby_layout(1280, 720, v, None).widgets.iter().find(|w| w.id == "ready").and_then(|w| w.text.clone()).unwrap();
        assert!(!v.i_am_ready());
        assert_eq!(label(&v), "READY");
        v.roster[1].flags |= ROSTER_READY;
        assert!(v.i_am_ready());
        assert!(label(&v).contains("CANCEL"));
    }

    #[test]
    fn results_rank_by_kills_and_name_the_winner_or_the_draw() {
        let l = results_layout(1280, 720, &OnlineView::demo(Phase::Results), None);
        let t = |id: &str| l.widgets.iter().find(|w| w.id == id).and_then(|w| w.text.clone()).unwrap();
        assert_eq!(t("r0_name"), "1. FAY", "Fay has the most kills (9)");
        assert_eq!(t("headline"), "WINNER: FAY  (SCORE REACHED)");
        let mut mine = OnlineView::demo(Phase::Results);
        mine.winner = mine.me;
        let l = results_layout(1280, 720, &mine, None);
        assert_eq!(l.widgets.iter().find(|w| w.id == "headline").and_then(|w| w.text.clone()).unwrap(), "YOU WIN!  (SCORE REACHED)");
        let mut v = OnlineView::demo(Phase::Results);
        v.winner = 255;
        v.end_code = 1;
        let l = results_layout(1280, 720, &v, None);
        assert_eq!(l.widgets.iter().find(|w| w.id == "headline").and_then(|w| w.text.clone()).unwrap(), "TIME UP");
        v.end_code = 0;
        v.end_text = "victory".into();
        let l = results_layout(1280, 720, &v, None);
        assert_eq!(l.widgets.iter().find(|w| w.id == "headline").and_then(|w| w.text.clone()).unwrap(), "VICTORY");
    }

    #[test]
    fn the_countdown_says_what_the_round_is_about_and_shows_the_keys() {
        let text = |l: &Layout, id: &str| l.widgets.iter().find(|w| w.id == id).and_then(|w| w.text.clone());
        let l = hud_layout(1280, 720, &OnlineView::demo(Phase::Countdown));
        assert_eq!(text(&l, "objective").as_deref(), Some("FIRST TO 12 KILLS WINS - EVERY KILL GIVES YOU THE NEXT GUN"));
        assert!(text(&l, "controls").is_some_and(|t| t.contains("WASD") && t.contains("CLICK FIRE")));
        let mut no_ladder = OnlineView::demo(Phase::Countdown);
        no_ladder.combat = None;
        let l = hud_layout(1280, 720, &no_ladder);
        assert!(text(&l, "objective").is_none() && text(&l, "controls").is_some(), "the keys always show, the objective only for a ladder");
        assert!(text(&hud_layout(1280, 720, &OnlineView::demo(Phase::Playing)), "controls").is_none(), "and only during the countdown");
    }

    #[test]
    fn the_hud_shows_a_countdown_only_during_the_countdown_and_a_banner_to_spectators() {
        let has = |l: &Layout, id: &str| l.widgets.iter().any(|w| w.id == id);
        assert!(has(&hud_layout(1280, 720, &OnlineView::demo(Phase::Countdown)), "count"));
        let playing = hud_layout(1280, 720, &OnlineView::demo(Phase::Playing));
        assert!(!has(&playing, "count") && !has(&playing, "banner"));
        let mut spectator = OnlineView::demo(Phase::Playing);
        spectator.in_round = false;
        assert!(has(&hud_layout(1280, 720, &spectator), "banner"));
        assert_eq!(clock(95), "1:35");
    }

    #[test]
    fn the_shooter_hud_shows_health_weapon_ladder_and_who_leads() {
        let text = |l: &Layout, id: &str| l.widgets.iter().find(|w| w.id == id).and_then(|w| w.text.clone());
        let v = OnlineView::demo(Phase::Playing);
        let l = hud_layout(1280, 720, &v);
        assert_eq!(text(&l, "hp_text").as_deref(), Some("62"));
        assert_eq!(text(&l, "weapon").as_deref(), Some("LONGBOW MARKSMAN RIFLE"));
        assert_eq!(text(&l, "rung_text").as_deref(), Some("RUNG 9/12  NEXT: BREACH SHOTGUN"));
        assert_eq!(text(&l, "leader").as_deref(), Some("LEADER FAY  9/12"), "Fay has the most kills in the demo");
        assert_eq!(l.widgets.iter().filter(|w| w.id.starts_with("rung_") && w.id != "rung_text").count(), 12, "one pip per rung");
        assert!(text(&l, "dead_title").is_none() && text(&l, "notice").is_some());
        // The fill is proportional to health and changes colour as it falls.
        let fill = |hp: u32| {
            let mut v = OnlineView::demo(Phase::Playing);
            v.combat.as_mut().unwrap().hp = hp;
            let l = hud_layout(1280, 720, &v);
            l.widgets.iter().find(|w| w.id == "hp_fill").map(|w| (w.rect.2 - w.rect.0, w.fill.unwrap()))
        };
        let (full, green) = fill(100).unwrap();
        let (half, _) = fill(50).unwrap();
        let (low, red) = fill(10).unwrap();
        assert!((half * 2 - full).abs() <= 2 && low < half, "{full} {half} {low}");
        assert!(green[1] > green[0] && red[0] > red[1], "green when healthy, red when nearly dead");
        assert!(fill(0).is_none(), "an empty bar draws no fill");
        // If we lead, it says so.
        let mut v = OnlineView::demo(Phase::Playing);
        v.me = 5;
        assert_eq!(text(&hud_layout(1280, 720, &v), "leader").as_deref(), Some("YOU LEAD  9/12"));
    }

    #[test]
    fn the_hud_says_out_loud_when_someone_is_one_kill_from_winning() {
        let text = |l: &Layout, id: &str| l.widgets.iter().find(|w| w.id == id).and_then(|w| w.text.clone());
        let mut v = OnlineView::demo(Phase::Playing);
        assert!(text(&hud_layout(1280, 720, &v), "final").is_none(), "nobody is on the last rung in the demo (Fay leads with 9 of 12)");
        v.roster[5].score = 11;
        assert_eq!(text(&hud_layout(1280, 720, &v), "final").as_deref(), Some("FAY IS ON THE FINAL WEAPON"));
        assert_eq!(final_rung_rival(&v, 12).map(|e| e.id), Some(5));
        // On the last rung ourselves, it is a call to arms instead.
        v.combat.as_mut().unwrap().rung = Some((12, 12));
        v.combat.as_mut().unwrap().next_weapon = None;
        assert_eq!(text(&hud_layout(1280, 720, &v), "final").as_deref(), Some("FINAL WEAPON - ONE KILL TO WIN"));
        // Nothing while dead, and we are never our own rival.
        v.combat.as_mut().unwrap().dead = true;
        assert!(text(&hud_layout(1280, 720, &v), "final").is_none());
        v.me = 5;
        assert!(final_rung_rival(&v, 12).is_none());
    }

    #[test]
    fn the_dead_hud_says_eliminated_and_counts_down_the_respawn() {
        let text = |l: &Layout, id: &str| l.widgets.iter().find(|w| w.id == id).and_then(|w| w.text.clone());
        let mut v = OnlineView::demo(Phase::Playing);
        v.combat = Some(CombatView::demo_dead());
        let l = hud_layout(1280, 720, &v);
        assert_eq!(text(&l, "dead_title").as_deref(), Some("ELIMINATED"));
        assert_eq!(text(&l, "dead_hint").as_deref(), Some("RESPAWNING IN 2"));
        assert!(text(&l, "notice").is_none(), "no level-up chatter over the banner");
        v.combat.as_mut().unwrap().respawn_secs = 0;
        assert_eq!(text(&hud_layout(1280, 720, &v), "dead_hint").as_deref(), Some("RESPAWNING"));
        let mut v = OnlineView::demo(Phase::Playing);
        v.combat.as_mut().unwrap().protected = true;
        assert_eq!(text(&hud_layout(1280, 720, &v), "protected").as_deref(), Some("SPAWN PROTECTED"));
        v.combat = None;
        assert!(text(&hud_layout(1280, 720, &v), "hp_text").is_none(), "no combat state, no shooter HUD");
    }

    #[test]
    fn the_connect_form_edits_text_hides_the_key_and_adds_the_default_port() {
        let mut f = ConnectForm::new("", "", "");
        for c in "192.168.0.5".chars() {
            f.type_char(c);
        }
        assert_eq!(f.address_with_port(), format!("192.168.0.5:{}", crate::net::DEFAULT_PORT));
        f.next_field();
        for c in "s3cret".chars() {
            f.type_char(c);
        }
        f.type_char('\n');
        assert_eq!(f.key, "s3cret", "control characters are ignored");
        f.backspace();
        assert_eq!(f.key, "s3cre");
        let l = connect_layout(1280, 720, &f, None);
        let key_text = l.widgets.iter().find(|w| w.id == "field_key").and_then(|w| w.text.clone()).unwrap();
        assert!(key_text.starts_with("*****") && !key_text.contains("s3cre"), "the key is never drawn: {key_text}");
        for _ in 0..40 {
            f.next_field();
            f.type_char('x');
        }
        assert!(f.name.len() <= crate::net::protocol::MAX_NAME && f.address.len() <= 64, "fields are bounded");
        assert_eq!(connect_action_for("field_key"), Some(ConnectAction::Focus(Field::Key)));
        assert_eq!(connect_action_for("connect"), Some(ConnectAction::Connect));
        assert_eq!(connect_action_for("nothing"), None);
    }
}
