//! The scene's `hud` block: which parts of the standard client's on-screen display are drawn.
//!
//! A shooter needs health, ammunition, a scoreboard, a round clock and a ping readout on screen. A beach stroll, a puzzle or a
//! cinematic needs none of them. This block is presentation only (nothing here decides gameplay) and is read by the offline
//! client, the online client and the headless state dump the same way, so what a scene asks for is what a screenshot shows.
//!
//! ```json
//! "hud": { "enabled": true, "show_combat": false, "show_ping": false, "show_scoreboard": false, "show_round": false,
//!          "show_rules_vars": false, "custom_vars": ["shells"] }
//! ```
//!
//! Every flag defaults to `true` (the arena look) except in a `peaceful` scene, where combat, crosshair, ping, scoreboard and round are
//! off unless the block turns them on. `enabled: false` is the one switch for a clean screen: nothing is drawn but an outcome banner
//! (the match ended) and a lost-connection warning.

use crate::strict::check_keys;
use serde_json::{Map, Value};

/// `hud` keys.
pub const HUD_KEYS: &[&str] =
    &["enabled", "show_scoreboard", "show_round", "show_ping", "show_combat", "show_crosshair", "show_events", "show_help", "show_rules_vars", "custom_vars"];

/// What the HUD draws (see the module docs). `Default` is the arena look: everything on.
#[derive(Debug, Clone, PartialEq)]
pub struct HudConfig {
    /// Master switch: false draws no HUD at all (outcome and connection banners excepted).
    pub enabled: bool,
    /// The compact scoreboard, online.
    pub show_scoreboard: bool,
    /// The round number and clock, online.
    pub show_round: bool,
    /// The ping readout, online.
    pub show_ping: bool,
    /// Health, ammunition, the weapon name, damage vignette and hit markers.
    pub show_combat: bool,
    /// The aim reticle.
    pub show_crosshair: bool,
    /// The newest rule event ("EVENT: ...") in the state panel.
    pub show_events: bool,
    /// The sandbox's control-hints bar along the bottom (`M: MAPS ... WEAPONS`, the id of what you look at) that a game project's `re2` shows.
    pub show_help: bool,
    /// The scene's game variables as a panel; when false only `custom_vars` are listed.
    pub show_rules_vars: bool,
    /// The variables shown when `show_rules_vars` is false (in this order).
    pub custom_vars: Vec<String>,
}

impl Default for HudConfig {
    fn default() -> Self {
        HudConfig {
            enabled: true,
            show_scoreboard: true,
            show_round: true,
            show_ping: true,
            show_combat: true,
            show_crosshair: true,
            show_events: true,
            show_help: true,
            show_rules_vars: true,
            custom_vars: Vec::new(),
        }
    }
}

impl HudConfig {
    /// The defaults of a scene whose player is `peaceful`: nothing about fighting or netcode on screen.
    pub fn peaceful_default() -> Self {
        HudConfig {
            show_scoreboard: false,
            show_round: false,
            show_ping: false,
            show_combat: false,
            show_crosshair: false,
            show_help: false,
            ..HudConfig::default()
        }
    }

    /// Whether the rule-state panel (variables and event) can show anything at all.
    pub fn shows_rule_panel(&self) -> bool {
        self.enabled && (self.show_rules_vars || !self.custom_vars.is_empty() || self.show_events)
    }

    /// The variables to list, in display order: all of them (`show_rules_vars`), or only the whitelisted ones that exist. A name
    /// starting with `_` is a game's internal state and is never listed by `show_rules_vars`, but a whitelist may name it.
    pub fn visible_vars<'a>(&self, vars: &[(&'a str, f64)]) -> Vec<(&'a str, f64)> {
        if !self.enabled {
            return Vec::new();
        }
        if self.show_rules_vars {
            return vars.iter().filter(|(name, _)| !name.starts_with('_')).copied().collect();
        }
        self.custom_vars.iter().filter_map(|want| vars.iter().find(|(name, _)| name == want).copied()).collect()
    }

    /// Whether the rule event line is drawn.
    pub fn shows_events(&self) -> bool {
        self.enabled && self.show_events
    }

    /// Whether the sandbox control-hints bar is drawn.
    pub fn shows_help(&self) -> bool {
        self.enabled && self.show_help
    }

    /// Whether the reticle is drawn.
    pub fn shows_crosshair(&self) -> bool {
        self.enabled && self.show_crosshair
    }

    /// Whether health, ammunition and damage feedback are drawn.
    pub fn shows_combat(&self) -> bool {
        self.enabled && self.show_combat
    }
}

fn flag(ctx: &mut Vec<String>, obj: &Map<String, Value>, key: &str, default: bool) -> bool {
    match obj.get(key) {
        None => default,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            ctx.push(format!("hud.{key}: must be true or false"));
            default
        }
    }
}

/// Parses the scene's `hud` block; `peaceful` selects the defaults (see the module docs). A scene without the block gets them too.
pub fn parse_hud(root: &Map<String, Value>, peaceful: bool) -> Result<HudConfig, Vec<String>> {
    let base = if peaceful { HudConfig::peaceful_default() } else { HudConfig::default() };
    let Some(raw) = root.get("hud") else { return Ok(base) };
    let Some(obj) = raw.as_object() else { return Err(vec!["hud: must be an object like {\"enabled\": false}".to_string()]) };
    let mut errors = Vec::new();
    check_keys(&mut errors, "hud", obj, HUD_KEYS);
    let mut cfg = HudConfig {
        enabled: flag(&mut errors, obj, "enabled", base.enabled),
        show_scoreboard: flag(&mut errors, obj, "show_scoreboard", base.show_scoreboard),
        show_round: flag(&mut errors, obj, "show_round", base.show_round),
        show_ping: flag(&mut errors, obj, "show_ping", base.show_ping),
        show_combat: flag(&mut errors, obj, "show_combat", base.show_combat),
        show_crosshair: flag(&mut errors, obj, "show_crosshair", base.show_crosshair),
        show_events: flag(&mut errors, obj, "show_events", base.show_events),
        show_help: flag(&mut errors, obj, "show_help", base.show_help),
        show_rules_vars: flag(&mut errors, obj, "show_rules_vars", base.show_rules_vars),
        custom_vars: Vec::new(),
    };
    match obj.get("custom_vars") {
        None => {}
        Some(Value::Array(items)) => {
            for (i, item) in items.iter().enumerate() {
                match item.as_str() {
                    Some(name) => cfg.custom_vars.push(name.to_string()),
                    None => errors.push(format!("hud.custom_vars[{i}]: must be a variable name string")),
                }
            }
        }
        Some(_) => errors.push("hud.custom_vars: must be an array of variable names".to_string()),
    }
    // A whitelist implies the panel: naming variables while `show_rules_vars` is left at its default would list all of them.
    if !cfg.custom_vars.is_empty() && !obj.contains_key("show_rules_vars") {
        cfg.show_rules_vars = false;
    }
    // Naming a variable that the scene does not declare is the classic silent typo: caught when the scene is checked (`validate_vars`).
    if errors.is_empty() {
        Ok(cfg)
    } else {
        Err(errors)
    }
}

/// The `custom_vars` that the scene's `vars` do not declare (a typo would otherwise draw nothing, silently).
pub fn unknown_custom_vars<'a>(cfg: &'a HudConfig, declared: &[&str]) -> Vec<&'a str> {
    cfg.custom_vars.iter().map(String::as_str).filter(|n| !declared.contains(n)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(v: Value, peaceful: bool) -> Result<HudConfig, Vec<String>> {
        parse_hud(v.as_object().unwrap(), peaceful)
    }

    #[test]
    fn no_block_means_the_arena_look_or_the_peaceful_one() {
        assert_eq!(parse(json!({}), false).unwrap(), HudConfig::default());
        let p = parse(json!({}), true).unwrap();
        assert!(p.enabled && !p.show_help && !p.show_combat && !p.show_crosshair && !p.show_ping && !p.show_scoreboard && !p.show_round);
    }

    #[test]
    fn disabled_draws_nothing_and_a_whitelist_lists_only_its_names() {
        let vars = [("shells", 3.0), ("shell_1", 1.0), ("_timer", 9.0)];
        let off = parse(json!({"hud": {"enabled": false}}), false).unwrap();
        assert!(off.visible_vars(&vars).is_empty() && !off.shows_crosshair() && !off.shows_events() && !off.shows_rule_panel());
        let only = parse(json!({"hud": {"custom_vars": ["shells"]}}), false).unwrap();
        assert_eq!(only.visible_vars(&vars), vec![("shells", 3.0)]);
        let all = parse(json!({"hud": {}}), false).unwrap();
        assert_eq!(all.visible_vars(&vars), vec![("shells", 3.0), ("shell_1", 1.0)], "underscore names stay hidden");
    }

    #[test]
    fn mistakes_are_reported_with_their_path() {
        let e = parse(json!({"hud": {"show_pings": true}}), false).unwrap_err();
        assert!(e.iter().any(|m| m.contains("show_pings")), "{e:?}");
        let e = parse(json!({"hud": {"enabled": "no"}}), false).unwrap_err();
        assert!(e.iter().any(|m| m.contains("hud.enabled")), "{e:?}");
        let cfg = parse(json!({"hud": {"custom_vars": ["shels"]}}), false).unwrap();
        assert_eq!(unknown_custom_vars(&cfg, &["shells"]), vec!["shels"]);
    }
}
