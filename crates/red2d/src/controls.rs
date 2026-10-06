//! Touch controllers: the few simple on-screen pads a 2D game can choose from for phones and tablets.
//!
//! A pad is drawn by the web page **below** the game (never over it) and only on a device whose main pointer is a finger; a desktop keeps the keyboard, the mouse and any game
//! controller. A pad does nothing a key could not: it holds and releases the same input *actions* (`left right up down action secondary pause`) that the keyboard and a gamepad
//! drive, so the simulation never knows which one was used and a headless scenario tests exactly what a thumb does.
//!
//! A game picks a layout in `controls`; with none, [`infer`] picks one from what the game reads (a platformer mover gets the platformer pad, a top-down mover a d-pad, a game of
//! clicks only needs no pad). Either way the choice is recorded in the package manifest, validated here (a pad button that drives an action nothing reads is an error), and
//! shown by `validate`.

use crate::fields::describe_value;
use crate::game::{KeyMode, Move, Prefab, Rule, When, ACTIONS};
use serde_json::Value;

/// The layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadLayout {
    /// A cross on the left (four directions, diagonals by sliding), up to two buttons on the right: top-down games.
    Dpad,
    /// A round joystick on the left (the same four directions), up to two buttons on the right: games that want a looser feel than a cross.
    Stick,
    /// Two large left and right buttons, a jump button and a second button on the right: platformers.
    Platformer,
    /// Two large left and right buttons and one optional button: catch, dodge, paddle and lane games.
    Lr,
    /// No pad: the game is played by tapping and dragging on the picture (management, puzzle, card and click games). Only a pause button, if the game has one.
    Tap,
}

impl PadLayout {
    /// Every layout.
    pub const ALL: &'static [PadLayout] = &[PadLayout::Dpad, PadLayout::Stick, PadLayout::Platformer, PadLayout::Lr, PadLayout::Tap];
    /// The JSON name.
    pub fn name(self) -> &'static str {
        match self {
            PadLayout::Dpad => "dpad",
            PadLayout::Stick => "stick",
            PadLayout::Platformer => "platformer",
            PadLayout::Lr => "lr",
            PadLayout::Tap => "tap",
        }
    }
    /// From the JSON name.
    pub fn parse(s: &str) -> Option<PadLayout> {
        PadLayout::ALL.iter().copied().find(|l| l.name() == s)
    }
    /// All names.
    pub fn names() -> Vec<&'static str> {
        PadLayout::ALL.iter().map(|l| l.name()).collect()
    }
    /// The directions this layout drives.
    pub fn directions(self) -> &'static [&'static str] {
        match self {
            PadLayout::Dpad | PadLayout::Stick => &["left", "right", "up", "down"],
            PadLayout::Platformer | PadLayout::Lr => &["left", "right"],
            PadLayout::Tap => &[],
        }
    }
    /// One line for `describe 2d`.
    pub fn about(self) -> &'static str {
        match self {
            PadLayout::Dpad => "cross + A/B: top-down movement",
            PadLayout::Stick => "round stick + A/B: top-down, looser feel",
            PadLayout::Platformer => "left/right + JUMP/B: platformers",
            PadLayout::Lr => "left/right (+ one button): catch, dodge, paddle",
            PadLayout::Tap => "no pad, tap and drag the picture: management, puzzle, clicks",
        }
    }
}

/// One round button on the right of a pad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PadButton {
    /// `a` or `b`.
    pub id: &'static str,
    /// What is written on it.
    pub label: String,
    /// The input action it holds.
    pub action: String,
}

/// The controller a game has on a phone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Controls {
    /// The layout.
    pub layout: PadLayout,
    /// The buttons that are shown.
    pub buttons: Vec<PadButton>,
    /// Whether a small pause button is shown (the game has a `press: pause` rule).
    pub pause: bool,
    /// Written in the game file (else chosen by [`infer`]).
    pub declared: bool,
}

impl Controls {
    /// Whether the page shows anything at all.
    pub fn visible(&self) -> bool {
        self.layout != PadLayout::Tap || self.pause || !self.buttons.is_empty()
    }
    /// A short description for reports: `platformer pad (JUMP, B, pause), inferred`.
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = self.buttons.iter().map(|b| b.label.clone()).collect();
        if self.pause {
            parts.push("pause".into());
        }
        format!(
            "{} pad{}{}",
            self.layout.name(),
            if parts.is_empty() { String::new() } else { format!(" ({})", parts.join(", ")) },
            if self.declared { "" } else { ", inferred" }
        )
    }
}

/// Whether anything in the game reads an input action: a `keys` mover or a `press` rule.
pub fn reads_action(prefabs: &[Prefab], rules: &[Rule], action: &str) -> bool {
    let by_mover = prefabs.iter().any(|p| match &p.mv {
        Move::Keys { mode: KeyMode::TopDown, .. } => matches!(action, "left" | "right" | "up" | "down"),
        Move::Keys { mode: KeyMode::Platformer, .. } => matches!(action, "left" | "right" | "up" | "action"),
        _ => false,
    });
    by_mover || rules.iter().any(|r| matches!(&r.when, When::Press(a) if a == action))
}

fn default_button(id: &'static str, layout: PadLayout) -> PadButton {
    match (id, layout) {
        ("a", PadLayout::Platformer) => PadButton { id: "a", label: "JUMP".into(), action: "action".into() },
        ("a", _) => PadButton { id: "a", label: "A".into(), action: "action".into() },
        _ => PadButton { id: "b", label: "B".into(), action: "secondary".into() },
    }
}

/// The pad a game gets when it does not say: chosen from what it reads.
pub fn infer(prefabs: &[Prefab], rules: &[Rule]) -> Controls {
    let platformer = prefabs.iter().any(|p| matches!(p.mv, Move::Keys { mode: KeyMode::Platformer, .. }));
    let topdown = prefabs.iter().any(|p| matches!(p.mv, Move::Keys { mode: KeyMode::TopDown, .. }));
    let pressed = |a: &str| rules.iter().any(|r| matches!(&r.when, When::Press(x) if x == a));
    let layout = if platformer {
        PadLayout::Platformer
    } else if topdown || pressed("up") || pressed("down") {
        PadLayout::Dpad
    } else if pressed("left") || pressed("right") {
        PadLayout::Lr
    } else {
        PadLayout::Tap
    };
    let mut buttons = Vec::new();
    for id in ["a", "b"] {
        let b = default_button(id, layout);
        if layout != PadLayout::Tap && reads_action(prefabs, rules, &b.action) {
            buttons.push(b);
        }
    }
    Controls { layout, buttons, pause: pressed("pause"), declared: false }
}

fn button(path: &str, id: &'static str, v: &Value, layout: PadLayout, prefabs: &[Prefab], rules: &[Rule], errs: &mut Vec<String>) -> Option<PadButton> {
    let mut b = default_button(id, layout);
    match v {
        Value::Bool(false) => return None,
        Value::Bool(true) => {}
        Value::String(s) => b.label = s.clone(),
        Value::Object(o) => {
            for k in o.keys() {
                if !["label", "action"].contains(&k.as_str()) {
                    errs.push(format!("{path}.{k}: unknown field (a button has `label` and `action`)"));
                }
            }
            if let Some(l) = o.get("label").and_then(Value::as_str) {
                b.label = l.to_string();
            }
            if let Some(a) = o.get("action").and_then(Value::as_str) {
                if !ACTIONS.contains(&a) {
                    let near = crate::suggest::suggest(a, ACTIONS.iter().copied());
                    errs.push(format!(
                        "{path}.action: `{a}` is not an input action{} (actions: {})",
                        near.first().map(|n| format!(" — did you mean `{n}`?")).unwrap_or_default(),
                        ACTIONS.join(", ")
                    ));
                    return None;
                }
                b.action = a.to_string();
            }
        }
        other => {
            errs.push(format!(
                "{path}: expected a label like \"JUMP\", false to hide it, or {{\"label\": ..., \"action\": ...}}, got {}",
                describe_value(other)
            ));
            return None;
        }
    }
    if b.label.is_empty() || b.label.chars().count() > 8 {
        errs.push(format!("{path}: a button label is 1 to 8 characters (it is drawn inside a round button), got `{}`", b.label));
    }
    if !reads_action(prefabs, rules, &b.action) {
        errs.push(format!(
            "{path}: button {} drives `{}`, but nothing in the game reads it (no `keys` mover that uses it and no `press: {}` rule): hide it with `\"{id}\": false`, or add the rule",
            b.label, b.action, b.action
        ));
    }
    Some(b)
}

/// Parses `controls` (a layout name, or `{layout, a, b, pause}`) and checks it against what the game reads. Errors are `path: message` lines.
pub fn parse(v: &Value, prefabs: &[Prefab], rules: &[Rule], errs: &mut Vec<String>) -> Option<Controls> {
    let (layout_name, obj) = match v {
        Value::String(s) => (s.as_str(), None),
        Value::Object(o) => {
            for k in o.keys() {
                if !["layout", "a", "b", "pause"].contains(&k.as_str()) {
                    let near = crate::suggest::suggest(k, ["layout", "a", "b", "pause"].into_iter());
                    errs.push(format!(
                        "controls.{k}: unknown field{} (fields: layout, a, b, pause)",
                        near.first().map(|n| format!(" — did you mean `{n}`?")).unwrap_or_default()
                    ));
                }
            }
            match o.get("layout").and_then(Value::as_str) {
                Some(l) => (l, Some(o)),
                None => {
                    errs.push(format!("controls.layout: missing: choose one of {}", PadLayout::names().join(", ")));
                    return None;
                }
            }
        }
        other => {
            errs.push(format!("controls: expected a layout name ({}) or an object, got {}", PadLayout::names().join(", "), describe_value(other)));
            return None;
        }
    };
    let Some(layout) = PadLayout::parse(layout_name) else {
        let near = crate::suggest::suggest(layout_name, PadLayout::ALL.iter().map(|l| l.name()));
        errs.push(format!(
            "controls.layout: `{layout_name}` is not a layout{} (layouts: {})",
            near.first().map(|n| format!(" — did you mean `{n}`?")).unwrap_or_default(),
            PadLayout::ALL.iter().map(|l| format!("{} = {}", l.name(), l.about())).collect::<Vec<_>>().join("; ")
        ));
        return None;
    };
    let before = errs.len();
    // The directions must be read by something.
    let silent: Vec<&str> = layout.directions().iter().copied().filter(|d| !reads_action(prefabs, rules, d)).collect();
    if layout != PadLayout::Tap && silent.len() == layout.directions().len() {
        errs.push(format!(
            "controls.layout: the {} pad drives {} but nothing in the game reads them (no `keys` mover and no `press` rule): give a prefab `\"move\": {{\"keys\": {{...}}}}`, or use `tap`",
            layout.name(),
            layout.directions().join("/")
        ));
    } else if layout == PadLayout::Platformer && !prefabs.iter().any(|p| matches!(p.mv, Move::Keys { mode: KeyMode::Platformer, .. })) {
        errs.push("controls.layout: the platformer pad is for a game with a `keys` mover in `platformer` mode (gravity, jump); for other movement use `dpad`, `stick` or `lr`".to_string());
    }
    let mut buttons = Vec::new();
    for id in ["a", "b"] {
        let given = obj.and_then(|o| o.get(id));
        let path = format!("controls.{id}");
        match given {
            Some(g) => {
                if layout == PadLayout::Tap && !matches!(g, Value::Bool(false)) {
                    errs.push(format!(
                        "{path}: the tap layout has no buttons (the picture is the controller); use `dpad`, `stick`, `platformer` or `lr` for buttons"
                    ));
                } else if let Some(b) = button(&path, if id == "a" { "a" } else { "b" }, g, layout, prefabs, rules, errs) {
                    buttons.push(b);
                }
            }
            None => {
                let b = default_button(if id == "a" { "a" } else { "b" }, layout);
                if layout != PadLayout::Tap && !(layout == PadLayout::Lr && id == "b") && reads_action(prefabs, rules, &b.action) {
                    buttons.push(b);
                }
            }
        }
    }
    if buttons.len() == 2 && buttons[0].action == buttons[1].action {
        errs.push(format!("controls.b: both buttons drive `{}`: give one a different `action`", buttons[0].action));
    }
    let pause = match obj.and_then(|o| o.get("pause")) {
        Some(Value::Bool(b)) => {
            if *b && !reads_action(prefabs, rules, "pause") {
                errs.push("controls.pause: the pause button drives `pause`, but no rule has `\"press\": \"pause\"`: add one (for example `{\"set\": [\"paused\", \"1 - paused\"]}`), or remove `pause`".to_string());
            }
            *b
        }
        Some(other) => {
            errs.push(format!("controls.pause: expected true or false, got {}", describe_value(other)));
            false
        }
        None => reads_action(prefabs, rules, "pause"),
    };
    (errs.len() == before).then_some(Controls { layout, buttons, pause, declared: true })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::parse as parse_game;

    fn game(mover: &str, controls: &str, rules: &str) -> Result<crate::game::GameDef, Vec<String>> {
        parse_game(&format!(
            r##"{{"game2d":1,"id":"t","title":"T","description":"d",
            "capabilities":{{"presentation":"2d","platforms":["web"],"networking":"offline","input":["keyboard","touch"],"persistence":[]}},
            "view":{{"width":160,"height":90}},"vars":{{"s":0}},
            "prefabs":{{"p":{{"shape":{{"rect":[8,8],"color":"#fc0"}},"body":{{"type":"dynamic","gravity":{g}}},"move":{mover}}}}},
            "scene":[{{"prefab":"p","at":[20,20]}}]{controls}{rules}}}"##,
            g = if mover.contains("platformer") { 600 } else { 0 }
        ))
    }

    const TOPDOWN: &str = r#"{"keys":{"mode":"topdown","speed":60}}"#;
    const PLATFORMER: &str = r#"{"keys":{"mode":"platformer","speed":60,"jump":200}}"#;

    #[test]
    fn a_pad_is_inferred_from_what_the_game_reads() {
        let g = game(TOPDOWN, "", "").unwrap();
        assert_eq!((g.controls.layout, g.controls.declared), (PadLayout::Dpad, false));
        assert!(g.controls.buttons.is_empty(), "nothing reads A or B, so no buttons");
        let g = game(PLATFORMER, "", "").unwrap();
        assert_eq!(g.controls.layout, PadLayout::Platformer);
        assert_eq!(
            g.controls.buttons.iter().map(|b| (b.label.as_str(), b.action.as_str())).collect::<Vec<_>>(),
            [("JUMP", "action")],
            "the jump reads `action`; nothing reads `secondary`"
        );
        let rules = r#","rules":[{"when":{"press":"secondary"},"do":[{"add":["s",1]}]},{"when":{"press":"pause"},"do":[{"add":["s",1]}]}]"#;
        let g = game(TOPDOWN, "", rules).unwrap();
        assert_eq!(g.controls.buttons.iter().map(|b| b.id).collect::<Vec<_>>(), ["b"]);
        assert!(g.controls.pause && g.controls.describe().contains("inferred"), "{}", g.controls.describe());
        let g = game(r#"{"drift":[0,0]}"#, "", "").unwrap();
        assert!(!g.controls.visible(), "a game with nothing to press needs no pad: {}", g.controls.describe());
    }

    #[test]
    fn a_game_chooses_its_layout_and_labels() {
        let g = game(
            PLATFORMER,
            r#","controls":{"layout":"platformer","a":"HOP","b":{"label":"DASH","action":"secondary"}}"#,
            r#","rules":[{"when":{"press":"secondary"},"do":[{"add":["s",1]}]}]"#,
        )
        .unwrap();
        assert!(g.controls.declared);
        assert_eq!(g.controls.buttons.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(), ["HOP", "DASH"]);
        let g = game(TOPDOWN, r#","controls":"stick""#, "").unwrap();
        assert_eq!(g.controls.layout, PadLayout::Stick);
        let g = game(PLATFORMER, r#","controls":{"layout":"lr","a":false}"#, "").unwrap();
        assert_eq!((g.controls.layout, g.controls.buttons.len()), (PadLayout::Lr, 0));
    }

    #[test]
    fn a_pad_that_would_do_nothing_is_refused_with_the_reason() {
        let e = |controls: &str, rules: &str, mover: &str| game(mover, controls, rules).unwrap_err().join("\n");
        assert!(e(r#","controls":"dpad""#, "", r#"{"drift":[0,0]}"#).contains("nothing in the game reads them"));
        assert!(e(r#","controls":"platformer""#, "", TOPDOWN).contains("`keys` mover in `platformer` mode"));
        assert!(e(r#","controls":{"layout":"dpad","a":"GO"}"#, "", TOPDOWN).contains("drives `action`, but nothing in the game reads it"));
        assert!(e(r#","controls":{"layout":"dpad","pause":true}"#, "", TOPDOWN).contains("no rule has `\"press\": \"pause\"`"));
        assert!(e(r#","controls":{"layout":"tap","a":"X"}"#, "", TOPDOWN).contains("the tap layout has no buttons"));
        let t = e(r#","controls":"dpda""#, "", TOPDOWN);
        assert!(t.contains("did you mean `dpad`") && t.contains("platformer = left/right"), "{t}");
        assert!(e(r#","controls":{"layout":"dpad","buttonz":1}"#, "", TOPDOWN).contains("unknown field"));
        assert!(e(r#","controls":{"layout":"dpad","a":"WAY TOO LONG"}"#, "", TOPDOWN).contains("1 to 8 characters"));
        let rules = r#","rules":[{"when":{"press":"secondary"},"do":[{"add":["s",1]}]}]"#;
        assert!(e(r#","controls":{"layout":"dpad","a":{"label":"X","action":"secondary"},"b":{"label":"Y","action":"secondary"}}"#, rules, TOPDOWN)
            .contains("both buttons drive"));
        assert!(e(r#","controls":{"layout":"dpad","a":{"action":"jump"}}"#, "", TOPDOWN).contains("not an input action"));
    }

    #[test]
    fn controls_need_touch_declared_and_a_game_without_touch_is_advised() {
        let text = |input: &str| {
            format!(
                r##"{{"game2d":1,"id":"t","title":"T","description":"d","capabilities":{{"presentation":"2d","platforms":["web"],"networking":"offline","input":{input},"persistence":[]}},
                "view":{{"width":160,"height":90}},"prefabs":{{"p":{{"shape":{{"rect":[8,8],"color":"#fc0"}},"move":{TOPDOWN}}}}},"scene":[{{"prefab":"p","at":[20,20]}}],"controls":"dpad"}}"##
            )
        };
        let e = parse_game(&text(r#"["keyboard"]"#)).unwrap_err().join("\n");
        assert!(e.contains("capabilities.input") && e.contains("`touch` is not declared") && e.contains("controls"), "{e}");
        assert!(parse_game(&text(r#"["keyboard","touch"]"#)).is_ok());
    }
}
