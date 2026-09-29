//! Rule state as HUD data: what the standard rules HUD shows (scene variables, the newest event, the outcome), taken
//! from a [`RulesEngine`] and laid out with the audited [`crate::ui`] kit. Presentation only: nothing here decides
//! gameplay, it reads the one rules engine that does.

use crate::sim::rules_run::{GameEvent, RulesEngine};
use crate::ui::Layout;

/// How long a non-terminal event stays on the HUD, in simulation ticks (2 s at 60 Hz).
pub const EVENT_SHOW_TICKS: u64 = 120;

/// The newest non-terminal rule event and until which tick it is shown.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecentEvent {
    name: Option<String>,
    until: u64,
}

impl RecentEvent {
    /// Feeds the events a tick produced (`end:*` outcomes are skipped: the outcome banner shows those).
    pub fn observe<'a>(&mut self, events: impl IntoIterator<Item = &'a GameEvent>, tick: u64) {
        for e in events {
            if !e.name.starts_with("end:") {
                self.name = Some(e.name.clone());
                self.until = tick + EVENT_SHOW_TICKS;
            }
        }
    }

    /// The event to show at `tick`, if one is still fresh.
    pub fn current(&self, tick: u64) -> Option<&str> {
        self.name.as_deref().filter(|_| tick < self.until)
    }
}

/// Everything the rules HUD draws, as plain data (compare it to decide whether to repaint).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HudState {
    /// The scene's own variables, `(name, value)`, in declaration order.
    pub vars: Vec<(String, f64)>,
    /// A recent event name.
    pub event: Option<String>,
    /// The outcome once the match has ended.
    pub outcome: Option<String>,
}

impl HudState {
    /// The HUD for `rules` with an optional recent event (see [`RecentEvent::current`]).
    pub fn from_rules(rules: &RulesEngine, event: Option<&str>) -> Self {
        HudState {
            vars: rules.vars().into_iter().map(|(n, v)| (n.to_string(), v)).collect(),
            event: event.map(str::to_string),
            outcome: rules.ended().map(str::to_string),
        }
    }

    /// True when the HUD would draw nothing.
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty() && self.event.is_none() && self.outcome.is_none()
    }

    /// A string that changes exactly when the painted HUD would (a cheap repaint key).
    pub fn key(&self) -> String {
        format!("{:?}|{:?}|{:?}", self.vars, self.event, self.outcome)
    }

    /// [`Self::layout`] as the scene's `hud` block asks (whitelisted variables, no event line, or nothing but the outcome banner).
    pub fn layout_for(&self, w: u32, h: u32, cfg: &crate::hud_config::HudConfig) -> Layout {
        let vars: Vec<(&str, f64)> = self.vars.iter().map(|(n, v)| (n.as_str(), *v)).collect();
        crate::ui::rules::hud_layout_for(w, h, &vars, self.event.as_deref(), self.outcome.as_deref(), cfg)
    }

    /// The standard rules HUD for a `w` x `h` window ([`crate::ui::rules::hud_layout`]); audit it with `Layout::check`.
    pub fn layout(&self, w: u32, h: u32) -> Layout {
        let vars: Vec<(&str, f64)> = self.vars.iter().map(|(n, v)| (n.as_str(), *v)).collect();
        crate::ui::rules::hud_layout(w, h, &vars, self.event.as_deref(), self.outcome.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(name: &str) -> GameEvent {
        GameEvent { tick: 0, rule: "r".into(), name: name.into(), slot: None }
    }

    #[test]
    fn recent_events_expire_and_outcomes_are_not_events() {
        let mut r = RecentEvent::default();
        r.observe(&[ev("switch_a"), ev("end:victory")], 10);
        assert_eq!(r.current(10), Some("switch_a"));
        assert_eq!(r.current(10 + EVENT_SHOW_TICKS), None);
    }

    #[test]
    fn the_key_tracks_every_visible_change() {
        let a = HudState { vars: vec![("switches".into(), 1.0)], event: None, outcome: None };
        let mut b = a.clone();
        assert_eq!(a.key(), b.key());
        b.outcome = Some("victory".into());
        assert_ne!(a.key(), b.key());
        assert!(b.layout(1280, 720).check().is_empty());
        assert!(HudState::default().is_empty());
    }
}
