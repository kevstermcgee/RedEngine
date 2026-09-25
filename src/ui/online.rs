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
        }
    }
}

/// Which online screen a view needs right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnlineScreen {
    /// Choosing a character and pressing Ready.
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
    /// Switch between human and rat.
    ToggleCharacter,
    /// Leave the match and close the game.
    Leave,
}

/// The action for the button with this id (`ready`, `character`, `leave`), if any.
pub fn action_for(button: &str) -> Option<OnlineAction> {
    match button {
        "ready" => Some(OnlineAction::ToggleReady),
        "character" => Some(OnlineAction::ToggleCharacter),
        "leave" => Some(OnlineAction::Leave),
        _ => None,
    }
}

/// The action under the cursor at `(x, y)` on `layout`.
pub fn action_at(layout: &Layout, x: f32, y: f32) -> Option<OnlineAction> {
    layout.button_at(x, y).and_then(action_for)
}

fn character_name(c: u8) -> &'static str {
    if c == 1 {
        "RAT"
    } else {
        "HUMAN"
    }
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

/// The lobby: who is here, who is ready, and buttons to change character, ready up and leave.
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
        ("h_what", "TYPE", cols.what, false),
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
        l.label_left(&format!("r{i}_what"), Some(c), cols.what.0, y, character_name(e.character), s, cols.what.1 - cols.what.0, col);
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
    button(&mut l, "character", y, &format!("PLAY AS: {}", character_name(v.my_character())), false);
    y += bh + 3 * s;
    button(&mut l, "leave", y, "LEAVE", false);
    y += bh + 6 * s;
    l.label_fit("hint", Some(c), cx, y, "R READY   C CHARACTER   ESC LEAVE", s, inner, DIM);
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
    let headline = match (winner, reason.is_empty()) {
        (Some(e), true) => format!("WINNER: {}", upper(&e.name)),
        (Some(e), false) => format!("WINNER: {}  ({reason})", upper(&e.name)),
        (None, true) => "DRAW".to_string(),
        (None, false) => reason.clone(),
    };
    l.label_fit("headline", Some(c), cx, y, &headline, s * 3 / 2, inner, GOLD);
    y += text_height(s * 3 / 2) + 6 * s;
    let cols = columns(x0 + pad, x1 - pad);
    l.label_left("h_rank", Some(c), cols.name.0, y, "PLAYER", s, cols.name.1 - cols.name.0, DIM);
    l.label_left("h_what", Some(c), cols.what.0, y, "TYPE", s, cols.what.1 - cols.what.0, DIM);
    l.label_right("h_score", Some(c), cols.state.1, y, "KILLS", s, cols.state.1 - cols.ping.0, DIM);
    y += text_height(s) + 4 * s;
    let mut ranked: Vec<&RosterEntry> = v.roster.iter().take(8).collect();
    ranked.sort_by(|a, b| b.score.cmp(&a.score).then(a.id.cmp(&b.id)));
    for (i, e) in ranked.iter().enumerate() {
        let col = if e.id == v.me { GOLD } else { TEXT };
        l.label_left(&format!("r{i}_name"), Some(c), cols.name.0, y, &format!("{}. {}", i + 1, upper(&e.name)), s, cols.name.1 - cols.name.0, col);
        l.label_left(&format!("r{i}_what"), Some(c), cols.what.0, y, character_name(e.character), s, cols.what.1 - cols.what.0, DIM);
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
/// except for the countdown.
pub fn hud_layout(w: u32, h: u32, v: &OnlineView) -> Layout {
    let mut l = Layout::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = (hi / 240).max(1);
    let m = 3 * s;
    l.label_left("ping", None, m, m, &format!("PING {:.0} MS", v.ping_ms), s, wi / 4, if v.ping_ms > 150.0 { RED } else { DIM });
    let head = match (v.phase, v.secs_left) {
        (Phase::Playing, Some(t)) => format!("ROUND {}  {}", v.round, clock(t)),
        (Phase::Playing, None) => format!("ROUND {}", v.round),
        _ => format!("ROUND {}", v.round),
    };
    l.label_fit("timer", None, wi / 2, m, &head, s * 3 / 2, wi / 3, TEXT);
    // Scoreboard: name and kills, top right, best first.
    let mut ranked: Vec<&RosterEntry> = v.roster.iter().take(8).collect();
    ranked.sort_by(|a, b| b.score.cmp(&a.score).then(a.id.cmp(&b.id)));
    let col_w = (wi / 5).max(40 * s);
    let mut y = m;
    for (i, e) in ranked.iter().enumerate() {
        let col = if e.id == v.me { GOLD } else { TEXT };
        l.label_right(&format!("sb{i}_score"), None, wi - m, y, &e.score.to_string(), s, col_w / 3, col);
        l.label_right(&format!("sb{i}_name"), None, wi - m - col_w / 3 - 2 * s, y, &upper(&e.name), s, col_w * 2 / 3, col);
        y += text_height(s) + 2 * s;
    }
    if v.reconnecting {
        l.label_fit("banner", None, wi / 2, hi / 2 - text_height(s * 2) / 2, "CONNECTION LOST - RECONNECTING...", s * 2, wi - 8, RED);
    } else if v.phase == Phase::Countdown {
        let secs = v.secs_left.unwrap_or(0).to_string();
        let big = l.label("count", None, wi / 2, hi * 30 / 100, &secs, s * 8, GOLD);
        l.widgets[big].shadow = false; // a shadow offset by one big pixel reads as a glitch at this size
        l.label_fit("count_hint", None, wi / 2, hi * 30 / 100 + text_height(s * 8) + 6 * s, "GET READY", s * 2, wi - 8, TEXT);
    } else if !v.in_round {
        l.label_fit("banner", None, wi / 2, hi - text_height(s * 2) - 8 * s, "ROUND IN PROGRESS - YOU JOIN THE NEXT ONE", s * 2, wi - 8, GOLD);
    }
    l
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
    /// Back to the launch menu.
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
    for (id, label) in [("connect", "CONNECT"), ("back", "BACK")] {
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
    use super::*;

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
            for (id, want) in [("ready", OnlineAction::ToggleReady), ("character", OnlineAction::ToggleCharacter), ("leave", OnlineAction::Leave)] {
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
