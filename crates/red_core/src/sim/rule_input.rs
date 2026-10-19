//! The rules read the input a player already sends (ADR 2026-10-19-rules-read-the-input-a-player-already-sends): `"when": {"input": "alt"}` fires
//! when a player presses the key (`"release": true` when they let go), with `me` that player, and `input(alt)` is 1 while the rule's player holds it.
//!
//! The server already receives every player's [`PlayerInput`] each tick; [`bits`] keeps the part a rule may name, the match keeps it per player
//! (`ServerPlayer::rule_input`), and the rules engine compares it with the tick before to see presses and releases. Nothing new travels: replays
//! hold the inputs already. `jump`, `interact` and `attack` are presses (the client sends them for one tick); `crouch`, `sprint` and `alt` are held.
//! `alt` is the right mouse button or the left trigger: in a rules game it does nothing else (the loadout shooter aims with it).

use super::player::PlayerInput;
use serde_json::Value;

/// The inputs a rule may name, in bit order, with the keys that make them (keyboard and mouse; gamepad).
pub const INPUTS: &[(&str, &str)] = &[
    ("jump", "Space; A"),
    ("crouch", "Ctrl, held; B"),
    ("sprint", "Shift, held; left stick click"),
    ("interact", "E; X"),
    ("attack", "left click; right trigger"),
    ("alt", "right mouse, held; left trigger"),
];

/// The bit of `alt` in [`bits`].
pub const ALT: u8 = 5;

/// The `input` trigger: which input (its bit in [`bits`]) and whether the rule fires when it is let go instead of when it is pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputTrigger {
    /// The input's bit.
    pub bit: u8,
    /// Fire on the release (`"release": true`), not the press.
    pub release: bool,
}

/// The rule-readable part of one input: bit `i` is `INPUTS[i]`.
pub fn bits(input: &PlayerInput) -> u8 {
    [input.jump, input.crouch, input.sprint, input.interact, input.attack, input.aim].iter().enumerate().fold(0, |b, (i, on)| b | (*on as u8) << i)
}

/// The bit of the input called `name`.
pub fn index(name: &str) -> Option<u8> {
    INPUTS.iter().position(|(n, _)| *n == name).map(|i| i as u8)
}

/// `name` as an input, or the sentence that says what the inputs are.
pub fn named(name: &str) -> Result<u8, String> {
    index(name).ok_or_else(|| {
        let names: Vec<&str> = INPUTS.iter().map(|(n, _)| *n).collect();
        let hint = crate::suggest::suggest(name, names.iter().copied()).first().map(|n| format!(" — did you mean `{n}`?")).unwrap_or_default();
        format!("no input `{name}`{hint} (inputs: {})", names.join(", "))
    })
}

/// The trigger `"input": name` with its optional `"release": true`.
pub fn trigger(name: &Value, release: Option<&Value>, path: &str, errs: &mut Vec<String>) -> Option<InputTrigger> {
    let bit = match name.as_str().map(named) {
        Some(Ok(b)) => b,
        Some(Err(e)) => {
            errs.push(format!("{path}: {e}"));
            return None;
        }
        None => {
            errs.push(format!("{path}: must be an input's name, like \"alt\" (fires when a player presses it; add \"release\": true for when they let go)"));
            return None;
        }
    };
    let release = match release {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            errs.push(format!("{path}: `release` must be true or false"));
            return None;
        }
    };
    Some(InputTrigger { bit, release })
}

/// `release` belongs to the `input` trigger only (`when` is the trigger object, `key` its trigger).
pub fn release_only_on_input(when: &serde_json::Map<String, Value>, key: &str, path: &str, errs: &mut Vec<String>) {
    if key != "input" && when.contains_key("release") {
        errs.push(format!("{path}.release: only an `input` trigger takes `release` (it fires when a player lets go of the input)"));
    }
}

/// Whether any rule of `set` reads the input at `bit`: a trigger on it or `input(name)` in a condition or a value. The client sends `alt` only
/// for such a scene (in the loadout shooter it aims, and that stays the shooter's).
pub fn reads(set: &super::rules::RuleSet, bit: u8) -> bool {
    use super::rules::{Action, When};
    set.rules.iter().any(|r| {
        matches!(r.when, When::Input(t) if t.bit == bit)
            || r.cond.as_ref().is_some_and(|c| c.reads_input(bit))
            || r.actions.iter().any(|a| match a {
                Action::Set { value, .. } | Action::SetMe { value, .. } | Action::SetIts { value, .. } | Action::Timer { secs: value, .. } => {
                    value.reads_input(bit)
                }
                _ => false,
            })
    })
}

/// Whether `t`'s input went down (or up, for a release trigger) between the bits `before` and `now`.
pub fn edge(t: InputTrigger, before: u8, now: u8) -> bool {
    let (was, is) = (before >> t.bit & 1 == 1, now >> t.bit & 1 == 1);
    if t.release {
        was && !is
    } else {
        is && !was
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::Character;
    use crate::sim::rules::{parse_rules, Refs, When};
    use crate::sim::rules_run::{RulePlayer, RulesEngine};
    use glam::Vec3;
    use serde_json::json;

    fn holding(slot: usize, input: PlayerInput) -> RulePlayer {
        RulePlayer { slot, pos: Vec3::new(9.0, 0.0, 9.0), radius: 0.3, height: 1.8, character: Character::Human, team: 0, input: bits(&input) }
    }

    #[test]
    fn the_bits_follow_the_names() {
        assert_eq!(bits(&PlayerInput { aim: true, ..Default::default() }), 1 << ALT);
        assert_eq!(bits(&PlayerInput { crouch: true, attack: true, ..Default::default() }), 1 << index("crouch").unwrap() | 1 << index("attack").unwrap());
        assert_eq!(bits(&PlayerInput { reload: true, drop: true, switch_weapon: true, forward: 1, ..Default::default() }), 0, "only the named inputs");
        let (press, release) = (InputTrigger { bit: ALT, release: false }, InputTrigger { bit: ALT, release: true });
        assert!(edge(press, 0, 1 << ALT) && !edge(press, 1 << ALT, 1 << ALT) && edge(release, 1 << ALT, 0) && !edge(release, 0, 0));
    }

    /// Hold your breath (Restless Tide, Borrowed Foundry): the press starts a breath timer for that player, the release ends it; the whistle
    /// (Crooked Ferry) is one press. Each player has their own edges.
    #[test]
    fn a_press_a_hold_and_a_release_fire_for_the_player_that_made_them() {
        let set = parse_rules(
            json!({"vars": {"breaths": 0, "exhales": 0, "held": 0, "whistles": 0}, "player_vars": {"held_by_me": 0}, "rules": [
                {"id": "in", "when": {"input": "alt"}, "do": [{"add": ["breaths", 1]}, {"set": ["me.held_by_me", "input(alt)"]}]},
                {"id": "out", "when": {"input": "alt", "release": true}, "do": [{"add": ["exhales", 1]}, {"set": ["me.held_by_me", "input(alt)"]}]},
                {"id": "whistle", "when": {"input": "interact"}, "if": "input(alt) == 0", "do": [{"add": ["whistles", 1]}]}
            ]})
            .as_object()
            .unwrap(),
            &Refs::default(),
        )
        .unwrap();
        assert_eq!(set.rules[1].when, When::Input(InputTrigger { bit: ALT, release: true }));
        let mut e = RulesEngine::new(set);
        let alt = PlayerInput { aim: true, ..Default::default() };
        let none = PlayerInput::default();
        let e_press = PlayerInput { interact: true, ..Default::default() };
        let both = PlayerInput { interact: true, aim: true, ..Default::default() };
        let steps = [
            [holding(0, none), holding(1, none)],
            [holding(0, alt), holding(1, none)],     // 0 breathes in
            [holding(0, alt), holding(1, alt)],      // 0 holds (nothing), 1 breathes in
            [holding(0, none), holding(1, alt)],     // 0 lets go
            [holding(0, e_press), holding(1, alt)],  // 0 whistles; 1's E is not pressed
            [holding(0, e_press), holding(1, both)], // 0 still holds E (no second whistle); 1 presses E while holding alt: no whistle
        ];
        for (t, players) in steps.iter().enumerate() {
            e.step(t as u64, players);
        }
        assert_eq!((e.var("breaths"), e.var("exhales"), e.var("whistles")), (Some(2.0), Some(1.0), Some(1.0)));
        assert_eq!((e.player_var(0, "held_by_me"), e.player_var(1, "held_by_me")), (Some(0.0), Some(1.0)), "input(alt) read the rule's own player");
    }

    #[test]
    fn a_wrong_input_says_which_there_are() {
        let e = parse_rules(
            json!({"rules": [
                {"id": "a", "when": {"input": "alr"}, "do": []},
                {"id": "b", "when": {"input": "alt", "release": "yes"}, "do": []},
                {"id": "c", "when": {"start": true, "release": true}, "do": []},
                {"id": "d", "when": {"start": true}, "if": "input(crouhc)", "do": []},
                {"id": "f", "when": {"every": 1}, "if": "input(alt)", "do": []}
            ]})
            .as_object()
            .unwrap(),
            &Refs::default(),
        )
        .unwrap_err()
        .join("\n");
        for want in [
            "no input `alr` — did you mean `alt`? (inputs: jump, crouch, sprint, interact, attack, alt)",
            "`release` must be true or false",
            "rules[2] (c).when.release: only an `input` trigger takes `release`",
            "no input `crouhc` — did you mean `crouch`?",
            "rules[4] (f): uses `me.` but a `every` trigger has no player",
        ] {
            assert!(e.contains(want), "missing `{want}` in:\n{e}");
        }
    }
}
