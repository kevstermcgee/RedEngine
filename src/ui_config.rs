//! The scene's `ui` block: what a *game* (as opposed to the engine's generic overlay) says to its player.
//!
//! The standard client already draws a rules panel, but it shows raw variable names (`MOON_STAMPS: 3`), has no place for an instruction, and ends a
//! match with a bare outcome word. This block is the declarative layer over it: friendly labels, progress counters, an objective that follows the
//! game's state, a start card and a card per outcome with a restart button. It is presentation only (nothing here decides gameplay); the client, the
//! online client, `ui-shot` / `ui-check` and the headless state dump all read it the same way, so a screenshot shows what a player sees.
//!
//! ```json
//! "ui": {
//!   "title": "Moonlight Delivery",
//!   "labels": { "stamps": "Stamps" },
//!   "counters": [ { "var": "delivered", "of": 6, "label": "Parcels" }, { "var": "time_left", "label": "Time", "format": "clock" } ],
//!   "objective": [ { "if": "delivered >= 6", "text": "Open the garden gate" }, { "text": "Bring every parcel to the depot ({delivered} of 6)" } ],
//!   "start": { "title": "Moonlight Delivery", "text": "Carry the parcels to the depot before dawn.", "button": "Start" },
//!   "end": { "victory": { "title": "Delivered!", "text": "All {delivered} parcels made it.", "button": "Play again" },
//!            "default": { "title": "Time is up", "text": "Try again?" } }
//! }
//! ```
//!
//! Texts may name variables as `{name}`. Every variable, outcome and expression is checked when the scene loads, with a did-you-mean.

use crate::sim::rules::{Action, RuleSet, BUILTIN_VARS};
use crate::sim::rules_expr::{self, Expr};
use crate::strict::check_keys;
use serde_json::{Map, Value};

/// `ui` keys.
pub const UI_KEYS: &[&str] = &["title", "labels", "counters", "objective", "start", "end"];
const COUNTER_KEYS: &[&str] = &["var", "of", "label", "format"];
const OBJECTIVE_KEYS: &[&str] = &["if", "text"];
const CARD_KEYS: &[&str] = &["title", "text", "button"];
/// Longest texts the layouts are designed for (they wrap and shrink, but a paragraph is not an HUD).
const MAX_TITLE: usize = 60;
const MAX_TEXT: usize = 400;
const MAX_OBJECTIVE: usize = 200;

/// How a counter's number is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// `3`, `2.5`.
    #[default]
    Number,
    /// Seconds as `m:ss` (`83` is `1:23`).
    Clock,
}

/// What a counter counts toward.
#[derive(Debug, Clone, PartialEq)]
pub enum Of {
    /// Just the value.
    Nothing,
    /// A fixed target (`3 / 6`).
    Number(f64),
    /// Another variable's value (`3 / total`).
    Var(String),
}

/// A progress line in the HUD: `PARCELS 3 / 6`.
#[derive(Debug, Clone, PartialEq)]
pub struct Counter {
    /// The variable shown.
    pub var: String,
    /// What it is called on screen.
    pub label: String,
    /// What it counts toward.
    pub of: Of,
    /// How the number is written.
    pub format: Format,
}

/// One candidate line of the objective: the first whose condition holds is shown.
#[derive(Debug, Clone, PartialEq)]
pub struct Objective {
    /// The condition over the scene's variables (`None` = always).
    pub cond: Option<Expr>,
    /// The text, with `{var}` placeholders.
    pub text: String,
}

/// A full-screen card: the start screen and the end-of-match screens.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    /// The big line.
    pub title: String,
    /// The paragraph under it, with `{var}` placeholders.
    pub text: String,
    /// The button's label; `None` draws no button.
    pub button: Option<String>,
}

/// The parsed `ui` block.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GameUi {
    /// The game's name (shown on the HUD's top line and as the default start-card title).
    pub title: Option<String>,
    /// Friendly names for variables, `(var, label)`.
    pub labels: Vec<(String, String)>,
    /// Progress lines.
    pub counters: Vec<Counter>,
    /// Objective candidates, in order.
    pub objective: Vec<Objective>,
    /// The card shown before play.
    pub start: Option<Card>,
    /// Cards by outcome (`default` answers any outcome without its own).
    pub end: Vec<(String, Card)>,
    /// The scene's variable names, the order `Objective::cond` was compiled against.
    var_names: Vec<String>,
}

/// `3` not `3.00`, `2.5` not `2.50`.
pub fn number(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{v:.0}")
    } else {
        let s = format!("{v:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn clock(secs: f64) -> String {
    let s = secs.max(0.0).floor() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn lookup(vars: &[(&str, f64)], name: &str) -> f64 {
    vars.iter().find(|(n, _)| *n == name).map_or(0.0, |(_, v)| *v)
}

impl GameUi {
    /// What `var` is called on screen: its label, else its own name.
    pub fn label_of<'a>(&'a self, var: &'a str) -> &'a str {
        self.labels.iter().find(|(v, _)| v == var).map_or(var, |(_, l)| l.as_str())
    }

    /// `text` with every `{var}` replaced by the variable's current value.
    pub fn fill(&self, text: &str, vars: &[(&str, f64)]) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}') else { break };
            out.push_str(&rest[..open]);
            out.push_str(&number(lookup(vars, &rest[open + 1..open + close])));
            rest = &rest[open + close + 1..];
        }
        out.push_str(rest);
        out
    }

    /// The objective to show now: the first candidate whose condition holds, with its placeholders filled.
    pub fn objective_text(&self, vars: &[(&str, f64)]) -> Option<String> {
        let values: Vec<f64> = self.var_names.iter().map(|n| lookup(vars, n)).collect();
        self.objective.iter().find(|o| o.cond.as_ref().is_none_or(|c| c.truthy(&values))).map(|o| self.fill(&o.text, vars))
    }

    /// The counters as `(label, value)` text, e.g. `("PARCELS", "3 / 6")`.
    pub fn counter_rows(&self, vars: &[(&str, f64)]) -> Vec<(String, String)> {
        self.counters
            .iter()
            .map(|c| {
                let show = |v: f64| if c.format == Format::Clock { clock(v) } else { number(v) };
                let value = show(lookup(vars, &c.var));
                let text = match &c.of {
                    Of::Nothing => value,
                    Of::Number(n) => format!("{value} / {}", show(*n)),
                    Of::Var(v) => format!("{value} / {}", show(lookup(vars, v))),
                };
                (c.label.clone(), text)
            })
            .collect()
    }

    /// The card for a finished match: the one for `outcome`, else `default`.
    pub fn end_card(&self, outcome: &str) -> Option<&Card> {
        self.end.iter().find(|(o, _)| o == outcome).or_else(|| self.end.iter().find(|(o, _)| o == "default")).map(|(_, c)| c)
    }

    /// The start card with its text filled.
    pub fn start_card(&self, vars: &[(&str, f64)]) -> Option<Card> {
        self.start.as_ref().map(|c| self.filled(c, vars))
    }

    /// The end card for `outcome` with its text filled.
    pub fn filled_end_card(&self, outcome: &str, vars: &[(&str, f64)]) -> Option<Card> {
        self.end_card(outcome).map(|c| self.filled(c, vars))
    }

    fn filled(&self, c: &Card, vars: &[(&str, f64)]) -> Card {
        Card { title: self.fill(&c.title, vars), text: self.fill(&c.text, vars), button: c.button.clone() }
    }

    /// Variables the HUD's plain rows should not repeat because a counter already shows them.
    pub fn is_counted(&self, var: &str) -> bool {
        self.counters.iter().any(|c| c.var == var)
    }
}

/// Where a problem is, and the names it could have been.
struct Ctx<'a> {
    errs: Vec<String>,
    vars: &'a [String],
}

impl Ctx<'_> {
    fn err(&mut self, path: &str, msg: impl AsRef<str>) {
        self.errs.push(format!("{path}: {}", msg.as_ref()));
    }

    fn near(&self, name: &str, options: &[String]) -> String {
        match crate::prefabs::suggest(name, options.iter().map(String::as_str)).first() {
            Some(h) => format!(" — did you mean `{h}`?"),
            None => String::new(),
        }
    }

    /// A scene variable's name, or an error that lists the scene's variables.
    fn var(&mut self, path: &str, name: &str) -> bool {
        let ok = self.vars.iter().any(|v| v == name);
        if !ok {
            let hint = self.near(name, self.vars);
            self.err(
                path,
                format!(
                    "`{name}` is not one of the scene's `vars`{hint} (vars: {})",
                    if self.vars.is_empty() { "none".to_string() } else { self.vars.join(", ") }
                ),
            );
        }
        ok
    }

    /// A string of at most `max` characters whose `{placeholders}` are scene variables.
    fn text(&mut self, path: &str, v: Option<&Value>, max: usize) -> Option<String> {
        let s = match v {
            Some(Value::String(s)) => s.clone(),
            None => return None,
            Some(_) => {
                self.err(path, "must be a string");
                return None;
            }
        };
        if s.chars().count() > max {
            self.err(path, format!("{} characters is too long for a screen (at most {max}); shorten it", s.chars().count()));
        }
        let mut rest = s.as_str();
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}') else {
                self.err(path, "a `{` has no closing `}` (write a variable as {name})");
                break;
            };
            let name = &rest[open + 1..open + close];
            self.var(path, name);
            rest = &rest[open + close + 1..];
        }
        Some(s)
    }
}

fn card(ctx: &mut Ctx, path: &str, v: &Value, default_button: Option<&str>) -> Option<Card> {
    let Some(o) = v.as_object() else {
        ctx.err(path, "must be an object like {\"title\": \"Delivered!\", \"text\": \"...\", \"button\": \"Play again\"}");
        return None;
    };
    check_keys(&mut ctx.errs, path, o, CARD_KEYS);
    let title = ctx.text(&format!("{path}.title"), o.get("title"), MAX_TITLE);
    let text = ctx.text(&format!("{path}.text"), o.get("text"), MAX_TEXT);
    if title.is_none() && text.is_none() {
        ctx.err(path, "needs a `title` or a `text`");
    }
    let button = match o.get("button") {
        None => default_button.map(str::to_string),
        Some(Value::Bool(false)) => None,
        Some(Value::String(s)) if !s.is_empty() && s.chars().count() <= 24 => Some(s.clone()),
        Some(_) => {
            ctx.err(&format!("{path}.button"), "must be a label of at most 24 characters, or false for no button");
            None
        }
    };
    Some(Card { title: title.unwrap_or_default(), text: text.unwrap_or_default(), button })
}

/// Parses the scene's `ui` block against its rules (variables, `end` outcomes). `None` when the scene has no block.
pub fn parse_ui(root: &Map<String, Value>, rules: &RuleSet) -> Result<Option<GameUi>, Vec<String>> {
    let Some(raw) = root.get("ui") else { return Ok(None) };
    let Some(o) = raw.as_object() else { return Err(vec!["ui: must be an object like {\"objective\": \"Find the key\"}".to_string()]) };
    let vars: Vec<String> = rules.var_names[BUILTIN_VARS.len().min(rules.var_names.len())..].to_vec();
    let mut outcomes: Vec<String> =
        rules.rules.iter().flat_map(|r| r.actions.iter()).filter_map(|a| if let Action::End(s) = a { Some(s.clone()) } else { None }).collect();
    outcomes.sort();
    outcomes.dedup();
    let mut ctx = Ctx { errs: Vec::new(), vars: &vars };
    check_keys(&mut ctx.errs, "ui", o, UI_KEYS);
    let mut ui = GameUi { var_names: vars.clone(), ..GameUi::default() };

    ui.title = ctx.text("ui.title", o.get("title"), MAX_TITLE);

    match o.get("labels") {
        None => {}
        Some(Value::Object(m)) => {
            for (var, label) in m {
                ctx.var(&format!("ui.labels.{var}"), var);
                match label.as_str().filter(|l| !l.is_empty() && l.chars().count() <= 24) {
                    Some(l) => ui.labels.push((var.clone(), l.to_string())),
                    None => ctx.err(&format!("ui.labels.{var}"), "must be a short label (1 to 24 characters)"),
                }
            }
        }
        Some(_) => ctx.err("ui.labels", "must be an object like {\"stamps\": \"Stamps\"} (a variable, then what to call it)"),
    }

    match o.get("counters") {
        None => {}
        Some(Value::Array(list)) => {
            for (i, c) in list.iter().enumerate() {
                let p = format!("ui.counters[{i}]");
                let Some(co) = c.as_object() else {
                    ctx.err(&p, "must be an object like {\"var\": \"delivered\", \"of\": 6, \"label\": \"Parcels\"}");
                    continue;
                };
                check_keys(&mut ctx.errs, &p, co, COUNTER_KEYS);
                let Some(var) = co.get("var").and_then(Value::as_str) else {
                    ctx.err(&format!("{p}.var"), "missing (the variable to show)");
                    continue;
                };
                ctx.var(&format!("{p}.var"), var);
                let of = match co.get("of") {
                    None => Of::Nothing,
                    Some(Value::Number(n)) => Of::Number(n.as_f64().unwrap_or(0.0)),
                    Some(Value::String(v)) => {
                        ctx.var(&format!("{p}.of"), v);
                        Of::Var(v.clone())
                    }
                    Some(_) => {
                        ctx.err(&format!("{p}.of"), "must be a number or the name of a variable");
                        Of::Nothing
                    }
                };
                let format = match co.get("format").and_then(Value::as_str) {
                    None | Some("number") => Format::Number,
                    Some("clock") => Format::Clock,
                    Some(other) => {
                        ctx.err(&format!("{p}.format"), format!("`{other}` is not one of number, clock"));
                        Format::Number
                    }
                };
                let label = co.get("label").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| var.to_string());
                if label.chars().count() > 24 {
                    ctx.err(&format!("{p}.label"), "at most 24 characters");
                }
                ui.counters.push(Counter { var: var.to_string(), label, of, format });
            }
        }
        Some(_) => ctx.err("ui.counters", "must be a list of {var, of?, label?, format?}"),
    }

    match o.get("objective") {
        None => {}
        Some(Value::String(_)) => {
            if let Some(text) = ctx.text("ui.objective", o.get("objective"), MAX_OBJECTIVE) {
                ui.objective.push(Objective { cond: None, text });
            }
        }
        Some(Value::Array(list)) => {
            for (i, item) in list.iter().enumerate() {
                let p = format!("ui.objective[{i}]");
                let Some(io) = item.as_object() else {
                    ctx.err(&p, "must be an object like {\"if\": \"has_key == 0\", \"text\": \"Find the key\"}");
                    continue;
                };
                check_keys(&mut ctx.errs, &p, io, OBJECTIVE_KEYS);
                let cond = match io.get("if") {
                    None => None,
                    Some(Value::String(src)) => match rules_expr::parse(src, &vars) {
                        Ok(e) => Some(e),
                        Err(e) => {
                            ctx.err(&format!("{p}.if"), e.to_string());
                            None
                        }
                    },
                    Some(_) => {
                        ctx.err(&format!("{p}.if"), "must be an expression string over the scene's vars, like \"delivered >= 6\"");
                        None
                    }
                };
                if cond.is_none() && io.contains_key("if") {
                    continue;
                }
                match ctx.text(&format!("{p}.text"), io.get("text"), MAX_OBJECTIVE) {
                    Some(text) => {
                        if cond.is_none() && i + 1 < list.len() {
                            ctx.err(&p, "has no `if`, so it always matches and the lines after it can never show (put it last)");
                        }
                        ui.objective.push(Objective { cond, text });
                    }
                    None => ctx.err(&format!("{p}.text"), "missing"),
                }
            }
        }
        Some(_) => ctx.err("ui.objective", "must be a string or a list of {if?, text}"),
    }

    if let Some(s) = o.get("start") {
        ui.start = card(&mut ctx, "ui.start", s, Some("START"));
    }

    match o.get("end") {
        None => {}
        Some(Value::Object(m)) => {
            for (outcome, c) in m {
                if outcome != "default" && !outcomes.contains(outcome) {
                    let mut options = outcomes.clone();
                    options.push("default".to_string());
                    let hint = ctx.near(outcome, &options);
                    ctx.err(
                        &format!("ui.end.{outcome}"),
                        format!(
                            "no rule ends the match with `{outcome}`{hint} (outcomes: {}; `default` answers the rest)",
                            if outcomes.is_empty() { "none".to_string() } else { outcomes.join(", ") }
                        ),
                    );
                }
                if let Some(card) = card(&mut ctx, &format!("ui.end.{outcome}"), c, Some("PLAY AGAIN")) {
                    ui.end.push((outcome.clone(), card));
                }
            }
        }
        Some(_) => ctx.err("ui.end", "must be an object of outcome -> card, like {\"victory\": {\"title\": \"Delivered!\"}, \"default\": {...}}"),
    }

    if ctx.errs.is_empty() {
        Ok(Some(ui))
    } else {
        Err(ctx.errs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rules() -> RuleSet {
        let scene = json!({
            "vars": {"delivered": 0, "has_key": 0, "time_left": 90},
            "rules": [
                {"id": "win", "when": {"start": true}, "if": "delivered > 99", "do": [{"end": "victory"}]},
                {"id": "lose", "when": {"every": 1.0}, "if": "time_left <= 0", "do": [{"end": "timeout"}]}]
        });
        crate::sim::rules::parse_rules(scene.as_object().unwrap(), &crate::sim::rules::Refs::default()).unwrap()
    }

    fn ui(v: Value) -> Result<Option<GameUi>, Vec<String>> {
        parse_ui(json!({ "ui": v }).as_object().unwrap(), &rules())
    }

    fn full() -> GameUi {
        ui(json!({
            "title": "Moonlight",
            "labels": {"has_key": "Key"},
            "counters": [{"var": "delivered", "of": 6, "label": "Parcels"}, {"var": "time_left", "label": "Time", "format": "clock"}],
            "objective": [{"if": "has_key == 0", "text": "Find the key"}, {"if": "delivered < 6", "text": "Deliver {delivered} of 6"}, {"text": "Open the gate"}],
            "start": {"title": "Moonlight", "text": "Deliver the parcels."},
            "end": {"victory": {"title": "Delivered {delivered}!", "button": "Again"}, "default": {"title": "Time is up", "button": false}}
        }))
        .unwrap()
        .unwrap()
    }

    #[test]
    fn no_block_is_no_ui_and_a_full_block_parses() {
        assert!(parse_ui(&Map::new(), &rules()).unwrap().is_none());
        let u = full();
        assert_eq!((u.counters.len(), u.objective.len(), u.end.len()), (2, 3, 2));
        assert_eq!(u.start.as_ref().unwrap().button.as_deref(), Some("START"));
        assert_eq!(u.end_card("victory").unwrap().button.as_deref(), Some("Again"));
        assert_eq!(u.end_card("timeout").unwrap().title, "Time is up", "default answers an outcome without its own card");
        assert_eq!(u.end_card("timeout").unwrap().button, None, "button: false draws none");
    }

    #[test]
    fn labels_counters_and_the_objective_follow_the_variables() {
        let u = full();
        assert_eq!(u.label_of("has_key"), "Key");
        assert_eq!(u.label_of("delivered"), "delivered");
        let v = |d: f64, k: f64, t: f64| vec![("delivered", d), ("has_key", k), ("time_left", t)];
        assert_eq!(u.objective_text(&v(0.0, 0.0, 90.0)).as_deref(), Some("Find the key"));
        assert_eq!(u.objective_text(&v(2.0, 1.0, 90.0)).as_deref(), Some("Deliver 2 of 6"));
        assert_eq!(u.objective_text(&v(6.0, 1.0, 90.0)).as_deref(), Some("Open the gate"));
        assert_eq!(u.counter_rows(&v(3.0, 0.0, 83.0)), [("Parcels".to_string(), "3 / 6".to_string()), ("Time".to_string(), "1:23".to_string())]);
        assert_eq!(u.filled_end_card("victory", &v(6.0, 1.0, 0.0)).unwrap().title, "Delivered 6!");
        assert_eq!(number(2.5), "2.5");
        assert_eq!(number(3.0), "3");
        assert!(u.is_counted("delivered") && !u.is_counted("has_key"));
    }

    #[test]
    fn a_counter_can_count_toward_another_variable() {
        let u = ui(json!({"counters": [{"var": "delivered", "of": "time_left"}]})).unwrap().unwrap();
        assert_eq!(u.counter_rows(&[("delivered", 2.0), ("time_left", 9.0)])[0].1, "2 / 9");
    }

    #[test]
    fn every_mistake_is_named_with_a_suggestion() {
        let e = ui(json!({
            "labels": {"delivred": "Parcels"},
            "counters": [{"var": "delivered", "of": 6, "format": "roman"}, {"var": "nope"}, {"of": 3}],
            "objective": [{"if": "delivered >", "text": "x"}, {"if": "ghost > 1", "text": "y"}, {"text": "always"}, {"text": "never"}, {"if": "delivered > 1", "text": "Got {parcels}"}],
            "start": {"button": 4},
            "end": {"victry": {"title": "x"}, "victory": {"tittle": "y"}},
            "colour": "red"
        }))
        .unwrap_err()
        .join("\n");
        for want in [
            "ui.labels.delivred: `delivred` is not one of the scene's `vars` — did you mean `delivered`?",
            "ui.counters[0].format: `roman` is not one of number, clock",
            "ui.counters[1].var: `nope` is not one of the scene's `vars`",
            "ui.counters[2].var: missing",
            "ui.objective[0].if",
            "ui.objective[1].if",
            "has no `if`, so it always matches",
            "ui.objective[4].text: `parcels` is not one of the scene's `vars`",
            "ui.start: needs a `title` or a `text`",
            "ui.start.button: must be a label",
            "ui.end.victry: no rule ends the match with `victry` — did you mean `victory`?",
            "ui.end.victory.tittle",
            "ui.colour",
        ] {
            assert!(e.contains(want), "missing `{want}` in:\n{e}");
        }
    }

    #[test]
    fn texts_must_fit_a_screen_and_braces_must_close() {
        let long = "x".repeat(MAX_OBJECTIVE + 1);
        let e = ui(json!({"objective": long, "start": {"title": "Open {delivered"}})).unwrap_err().join("\n");
        assert!(e.contains("too long for a screen") && e.contains("no closing `}`"), "{e}");
    }
}
