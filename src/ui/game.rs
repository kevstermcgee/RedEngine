//! The screens a scene's `ui` block asks for ([`crate::ui_config`]): the game HUD (objective, counters, labelled variables), the start card and the
//! end-of-match card. Pure functions of the window size, the declaration and the current state, built on [`Layout`] like every other screen, so
//! `ui-shot` draws them, `ui-check` audits them at every window size and the client paints exactly the same rectangles.

use super::{fit_scale, text_height, wrap, Layout};
use crate::ui_config::{number, Card, GameUi};

/// Adds `other`'s widgets after `layout`'s, re-pointing their containers.
fn append(layout: &mut Layout, other: Layout) {
    let offset = layout.widgets.len();
    for mut widget in other.widgets {
        widget.container = widget.container.map(|i| i + offset);
        layout.widgets.push(widget);
    }
}

/// Whether the plain outcome banner is drawn for an outcome. **Cards are an offline presentation** (a start card holds the game, an end card has a restart button; neither
/// exists online, where the server owns the round and its restart), so the banner stands in for the card wherever no card is actually shown: always when online, and
/// offline only for outcomes the scene declared no end card for. A scene written for cards must still tell its online players how the game ended.
pub fn outcome_banner_needed(online: bool, has_end_card: bool) -> bool {
    online || !has_end_card
}

/// The rules overlay for a scene: its `ui` block's HUD when it has one (friendly labels, counters, the objective), else the generic variables panel. An outcome without a card
/// on screen (no end card declared, or the player is online and cards are offline-only) gets the plain banner. Does not include a card. The desktop client paints this, so a screenshot is
/// what a player sees.
pub fn rules_overlay(scene: &crate::schema::Scene, w: u32, h: u32, vars: &[(&str, f64)], event: Option<&str>, outcome: Option<&str>, online: bool) -> Layout {
    let hud = &scene.hud;
    let Some(ui) = scene.ui.as_ref() else {
        return super::rules::hud_layout_for(w, h, vars, event, outcome, hud);
    };
    let mut layout = if hud.enabled { hud_layout(w, h, ui, vars, &hud.visible_vars(vars), event.filter(|_| hud.shows_events())) } else { Layout::new(w, h) };
    if let Some(o) = outcome.filter(|o| outcome_banner_needed(online, ui.end_card(o).is_some())) {
        append(&mut layout, super::rules::hud_layout_for(w, h, &[], None, Some(o), hud));
    }
    layout
}

const TEXT: [u8; 4] = [236, 238, 245, 255];
const DIM: [u8; 4] = [160, 166, 188, 255];
const GOLD: [u8; 4] = [255, 210, 74, 255];
const PANEL: [u8; 4] = [8, 11, 20, 185];
const LINE: [u8; 4] = [75, 82, 110, 220];

/// The in-game overlay: the objective along the top, then a panel of counters and the remaining labelled variables, then the newest rule event.
///
/// `vars` are all of the scene's variables; `shown` lists the ones the `hud` block allows as plain rows (so `hud.show_rules_vars: false` still
/// hides the raw rows while counters stay). Variables a counter already shows are not repeated.
pub fn hud_layout(w: u32, h: u32, ui: &GameUi, vars: &[(&str, f64)], shown: &[(&str, f64)], event: Option<&str>) -> Layout {
    let mut l = Layout::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = (hi / 240).max(1);

    if let Some(text) = ui.objective_text(vars).filter(|t| !t.is_empty()) {
        let max_w = (wi - 16 * s).min(260 * s);
        let text = text.to_uppercase();
        let lines = wrap(&text, max_w - 12 * s, s * 3 / 2 + s % 2);
        let scale = fit_scale(&text, max_w - 12 * s, s * 3 / 2 + s % 2).max(1);
        let lines = if lines.len() > 3 { wrap(&text, max_w - 12 * s, scale.max(1)) } else { lines };
        let line_h = text_height(scale) + 2 * scale;
        let bh = 12 * s + lines.len() as i32 * line_h;
        let x0 = (wi - max_w) / 2;
        let bar = l.panel("objective", (x0, 4 * s, x0 + max_w, 4 * s + bh), None, Some(PANEL), Some((GOLD, (s / 2).max(1))));
        for (i, line) in lines.iter().enumerate() {
            l.label(&format!("objective_{}", i + 1), Some(bar), wi / 2, 4 * s + 6 * s + i as i32 * line_h, line, scale, GOLD);
        }
    }

    let counters = ui.counter_rows(vars);
    let rows: Vec<(String, String)> = shown.iter().filter(|(n, _)| !ui.is_counted(n)).take(8).map(|(n, v)| (ui.label_of(n).to_string(), number(*v))).collect();
    let event = event.map(|e| format!("EVENT: {}", e.to_uppercase()));
    let n = counters.len() + rows.len() + usize::from(event.is_some());
    if n > 0 || ui.title.is_some() {
        let title = ui.title.as_ref().map(|t| t.to_uppercase());
        let panel_w = (140 * s).min(wi - 8);
        let row_h = text_height(s) + 3 * s;
        let top = if ui.objective_text(vars).is_some_and(|t| !t.is_empty()) { 4 * s + 40 * s } else { 4 * s };
        let top = top.min(hi / 3);
        let panel_h = 8 * s + (n + usize::from(title.is_some())) as i32 * row_h;
        let panel = l.panel("game_state", (4 * s, top, 4 * s + panel_w, (top + panel_h).min(hi - 2)), None, Some(PANEL), Some((LINE, 1)));
        let max_w = panel_w - 10 * s;
        let mut y = top + 5 * s;
        if let Some(t) = &title {
            l.label_fit("game_title", Some(panel), 4 * s + panel_w / 2, y, t, s, max_w, DIM);
            y += row_h;
        }
        for (i, (label, value)) in counters.iter().chain(rows.iter()).enumerate() {
            let text = format!("{}: {}", label.to_uppercase(), value);
            l.label_fit(&format!("row_{i}"), Some(panel), 4 * s + panel_w / 2, y, &text, s, max_w, TEXT);
            y += row_h;
        }
        if let Some(e) = &event {
            l.label_fit("game_event", Some(panel), 4 * s + panel_w / 2, y, e, s, max_w, GOLD);
        }
    }
    l
}

/// A full-screen card (the start screen, or the end of the match) with an optional button. `button_id` names the button (`start`, `restart`) for
/// hit-testing; `hover` highlights it.
pub fn card_layout(w: u32, h: u32, card: &Card, button_id: &str, hover: bool) -> Layout {
    let mut l = Layout::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = (hi / 240).max(1);
    let pw = (230 * s).min(wi - 8);
    let inner = pw - 24 * s;
    let title = card.title.to_uppercase();
    let title_scale = fit_scale(&title, inner, s * 2);
    // Measure first so the panel is as tall as its content and never clips it.
    let body_scale = s;
    let body: Vec<String> = wrap(&card.text.to_uppercase(), inner, body_scale);
    let body_line = text_height(body_scale) + 2 * body_scale;
    let bh = 22 * s;
    let mut ph = 16 * s;
    if !title.is_empty() {
        ph += text_height(title_scale) + 10 * s;
    }
    ph += body.len() as i32 * body_line;
    if card.button.is_some() {
        ph += 12 * s + bh;
    }
    let ph = ph.min(hi - 8);
    let x0 = (wi - pw) / 2;
    let y0 = ((hi - ph) / 2).max(0);
    l.panel("backdrop", (0, 0, wi, hi), None, Some([4, 6, 12, 150]), None);
    let panel = l.panel("card", (x0, y0, x0 + pw, y0 + ph), None, Some([14, 17, 28, 240]), Some((GOLD, (s / 2).max(1))));
    let mut y = y0 + 8 * s;
    if !title.is_empty() {
        l.label_fit("card_title", Some(panel), wi / 2, y, &title, title_scale, inner, GOLD);
        y += text_height(title_scale) + 10 * s;
    }
    for (i, line) in body.iter().enumerate() {
        l.label(&format!("card_text_{}", i + 1), Some(panel), wi / 2, y + i as i32 * body_line, line, body_scale, TEXT);
    }
    y += body.len() as i32 * body_line;
    if let Some(label) = &card.button {
        let label = label.to_uppercase();
        let bw = (pw - 40 * s).max(40 * s);
        let bx = (wi - bw) / 2;
        let scale = fit_scale(&label, bw - 6 * s, s * 3 / 2);
        l.button(
            button_id,
            (bx, y + 12 * s, bx + bw, y + 12 * s + bh),
            Some(panel),
            &label,
            scale,
            if hover { [56, 62, 92, 255] } else { [30, 34, 52, 255] },
            (if hover { GOLD } else { [90, 98, 130, 255] }, (s / 2).max(1)),
            if hover { GOLD } else { TEXT },
        );
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::rules::{parse_rules, Refs};
    use serde_json::json;

    fn scene_ui() -> GameUi {
        let root = json!({
            "vars": {"delivered": 0, "has_key": 0, "time_left": 90},
            "rules": [{"id": "win", "when": {"start": true}, "if": "delivered > 99", "do": [{"end": "victory"}]}],
            "ui": {
                "title": "Moonlight Delivery", "labels": {"has_key": "Key"},
                "counters": [{"var": "delivered", "of": 6, "label": "Parcels"}, {"var": "time_left", "label": "Time", "format": "clock"}],
                "objective": [{"if": "has_key == 0", "text": "Find the brass key in the courtyard, then bring every parcel to the depot before dawn"}, {"text": "Deliver {delivered} of 6"}],
                "start": {"title": "Moonlight Delivery", "text": "Carry six parcels across the sleeping town to the depot before the sun comes up. Parcels are heavy: pick them up with E.", "button": "Start"},
                "end": {"victory": {"title": "All delivered!", "text": "You delivered {delivered} parcels."}}
            }
        });
        let o = root.as_object().unwrap();
        let rules = parse_rules(o, &Refs::default()).unwrap();
        crate::ui_config::parse_ui(o, &rules).unwrap().unwrap()
    }

    const SIZES: [(u32, u32); 6] = [(480, 270), (640, 360), (1280, 720), (1920, 1080), (500, 640), (720, 720)];

    #[test]
    fn the_game_hud_shows_friendly_text_and_passes_the_audit_at_every_size() {
        let ui = scene_ui();
        let vars = [("delivered", 3.0), ("has_key", 0.0), ("time_left", 83.0)];
        for (w, h) in SIZES {
            let l = hud_layout(w, h, &ui, &vars, &vars, Some("parcel_in"));
            assert!(l.check().is_empty(), "{w}x{h}: {:?}", l.check());
        }
        let l = hud_layout(1280, 720, &ui, &vars, &vars, None);
        let texts: Vec<String> = l.widgets.iter().filter_map(|w| w.text.clone()).collect();
        assert!(texts.iter().any(|t| t == "PARCELS: 3 / 6") && texts.iter().any(|t| t == "TIME: 1:23"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "KEY: 0"), "an uncounted variable keeps its row, under its label: {texts:?}");
        assert!(!texts.iter().any(|t| t.starts_with("DELIVERED") || t.starts_with("HAS_KEY")), "no raw variable names: {texts:?}");
        assert!(texts.iter().any(|t| t.contains("BRASS KEY")), "the objective is shown: {texts:?}");
    }

    #[test]
    fn hiding_the_plain_rows_keeps_the_counters_and_the_objective_follows_the_state() {
        let ui = scene_ui();
        let vars = [("delivered", 3.0), ("has_key", 1.0), ("time_left", 10.0)];
        let l = hud_layout(1280, 720, &ui, &vars, &[], None);
        let texts: Vec<String> = l.widgets.iter().filter_map(|w| w.text.clone()).collect();
        assert!(texts.iter().any(|t| t == "PARCELS: 3 / 6") && !texts.iter().any(|t| t.starts_with("KEY")), "{texts:?}");
        assert!(texts.iter().any(|t| t == "DELIVER 3 OF 6"), "{texts:?}");
    }

    #[test]
    fn cards_fit_and_their_button_is_where_it_is_painted() {
        let ui = scene_ui();
        let vars = [("delivered", 6.0)];
        let start = ui.start_card(&vars).unwrap();
        let end = ui.filled_end_card("victory", &vars).unwrap();
        for (w, h) in SIZES {
            for (card, id) in [(&start, "start"), (&end, "restart")] {
                let l = card_layout(w, h, card, id, false);
                assert!(l.check().is_empty(), "{w}x{h} {id}: {:?}", l.check());
                let (x0, y0, x1, y1) = l.rect_of(id).unwrap();
                assert_eq!(l.button_at(((x0 + x1) / 2) as f32, ((y0 + y1) / 2) as f32), Some(id));
            }
        }
        assert!(card_layout(1280, 720, &end, "restart", false).widgets.iter().any(|w| w.text.as_deref() == Some("YOU DELIVERED 6 PARCELS.")));
        let no_button = Card { button: None, ..end };
        assert!(card_layout(640, 360, &no_button, "restart", false).rect_of("restart").is_none());
    }
}

#[cfg(test)]
mod banner_tests {
    use super::outcome_banner_needed;

    /// The intended behaviour, all four cases: a card replaces the banner only where a card is shown, which is offline.
    #[test]
    fn the_banner_stands_in_for_the_card_wherever_no_card_is_shown() {
        assert!(outcome_banner_needed(false, false), "offline, no card declared: the banner");
        assert!(!outcome_banner_needed(false, true), "offline, a card declared: the card, not both");
        assert!(outcome_banner_needed(true, false), "online, no card declared: the banner");
        assert!(outcome_banner_needed(true, true), "online, a card declared that never shows: the banner, or the result is invisible");
    }
}
