//! The scene's `ui` block in the offline client: the game HUD, the start card (play waits for it) and the end card with its restart button
//! (ADR 2026-10-03-a-scene-declared-game-ui). Online the HUD uses the same labels, counters and objective; the cards are offline only, because
//! a shared match cannot wait for one player's click.

use super::*;
use red_engine2::ui::game;
use red_engine2::ui_config::Card;

/// Which card is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CardKind {
    /// Before play: the simulation waits for its button.
    Start,
    /// After an `end` rule: its button (if any) starts the game over.
    End,
}

/// The card on screen and whether the mouse is over its button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CardState {
    pub kind: CardKind,
    pub hover: bool,
}

impl App {
    /// The rules overlay for a scene (see [`red_engine2::ui::game::rules_overlay`]); online, outcomes get the plain banner because cards are offline-only.
    pub(crate) fn rules_overlay(&self, w: u32, h: u32, vars: &[(&str, f64)], event: Option<&str>, outcome: Option<&str>) -> red_engine2::ui::Layout {
        game::rules_overlay(&self.scene, w, h, vars, event, outcome, self.net.is_some())
    }

    /// The card's text and the id of its button, as it reads now.
    pub(crate) fn card_content(&self) -> Option<(Card, &'static str)> {
        let ui = self.scene.ui.as_ref()?;
        let vars = self.rules.vars();
        match self.card?.kind {
            CardKind::Start => ui.start_card(&vars).map(|c| (c, "start")),
            CardKind::End => ui.filled_end_card(self.rules.ended()?, &vars).map(|c| (c, "restart")),
        }
    }

    /// The card's layout at this window size, if one is up.
    pub(crate) fn card_layout_now(&self, w: u32, h: u32) -> Option<red_engine2::ui::Layout> {
        let (card, id) = self.card_content()?;
        Some(game::card_layout(w, h, &card, id, self.card?.hover))
    }

    /// Puts the start card up (offline, when the scene has one and this is not a restart): play waits and the mouse is free.
    pub(crate) fn open_start_card(&mut self) {
        if self.net.is_some() || self.skip_start_card || !self.scene.ui.as_ref().is_some_and(|u| u.start.is_some()) {
            return;
        }
        self.show_card(CardKind::Start);
    }

    /// Puts the end card up the moment the match has ended, if the scene declared one for that outcome.
    pub(crate) fn sync_card(&mut self) {
        if self.net.is_some() || self.card.is_some() {
            return;
        }
        let (Some(ui), Some(outcome)) = (self.scene.ui.as_ref(), self.rules.ended()) else { return };
        if ui.end_card(outcome).is_some() {
            self.show_card(CardKind::End);
        }
    }

    fn show_card(&mut self, kind: CardKind) {
        self.card = Some(CardState { kind, hover: false });
        self.keys.clear(); // no key-release events arrive while a card has the keyboard
        self.sprint_held = false;
        self.set_grab(false);
        self.rule_hud_painted = None;
    }

    /// The card's button was used (click, Enter, Space, E, or a script's `press`). False when the card has no button.
    pub(crate) fn card_activate(&mut self) -> bool {
        let Some(state) = self.card else { return false };
        if !self.card_content().is_some_and(|(c, _)| c.button.is_some()) {
            return false;
        }
        match state.kind {
            CardKind::Start => {
                self.card = None;
                self.rule_hud_painted = None;
                self.last_frame = Instant::now();
                self.set_grab(!self.paused);
            }
            CardKind::End => self.restart_game(),
        }
        true
    }

    /// The mouse moved over a card: highlight its button.
    pub(crate) fn card_hover(&mut self, x: f32, y: f32) {
        self.cursor = (x, y);
        let Some((w, h)) = self.window_size() else { return };
        let over = self.card_layout_now(w, h).is_some_and(|l| l.button_at(x, y).is_some());
        if let Some(card) = self.card.as_mut().filter(|c| c.hover != over) {
            card.hover = over;
            self.rule_hud_painted = None;
        }
    }

    /// A left click while a card is up.
    pub(crate) fn card_click(&mut self) {
        let Some((w, h)) = self.window_size() else { return };
        if self.card_layout_now(w, h).is_some_and(|l| l.button_at(self.cursor.0, self.cursor.1).is_some()) {
            self.card_activate();
        }
    }

    /// A script's `press: id`: use the on-screen button with that id.
    pub(crate) fn press_button(&mut self, id: &str) -> Result<(), String> {
        match self.card_content() {
            Some((card, button)) if button == id && card.button.is_some() => {
                self.card_activate();
                Ok(())
            }
            Some((card, button)) => Err(format!(
                "no button `{id}` on screen (the {} card has {})",
                if self.card.is_some_and(|c| c.kind == CardKind::Start) { "start" } else { "end" },
                if card.button.is_some() { format!("`{button}`") } else { "no button".to_string() }
            )),
            None => Err(format!("no button `{id}` on screen (no card is up)")),
        }
    }

    /// Plays the scene again from the top: a fresh game built from the same file (so an edit shows), in the same window, as the map switcher does.
    fn restart_game(&mut self) {
        let path = self.scene_path.clone();
        let scene = match red_engine2::load_scene(&path) {
            Ok(scene) => scene,
            Err(errors) => {
                eprintln!("Restart failed: {}", errors.join("; "));
                return;
            }
        };
        let who = self.character;
        let mut next = App::new(scene, path, Some(who), None, None);
        next.window = self.window.take();
        next.gpu = self.gpu.take();
        next.view_mode = self.view_mode;
        next.music_on = self.music_on;
        next.sfx_on = self.sfx_on;
        next.skip_start_card = true;
        // A headless run's script clock, pictures and notes carry on across the restart.
        next.headless = self.headless;
        next.virtual_size = self.virtual_size;
        next.play_secs = self.play_secs;
        next.shots = std::mem::take(&mut self.shots);
        next.snapshots = std::mem::take(&mut self.snapshots);
        next.failures = std::mem::take(&mut self.failures);
        next.start_game(who);
        next.grabbed = self.grabbed || self.headless;
        *self = next;
    }
}
