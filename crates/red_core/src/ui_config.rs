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
//!   "pause": "{stamps} stamps so far",
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
pub const UI_KEYS: &[&str] = &["title", "labels", "counters", "objective", "start", "pause", "end"];
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
    /// A line for the pause menu, with `{var}` and `{var:word|words}` filled in ("{days_lived:day|days} lived"): where a game with a clean screen (`hud.enabled: false`)
    /// keeps what its HUD would have said.
    pub pause: Option<String>,
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
            let inner = &rest[open + 1..open + close];
            // `{days:day|days}` is the number and the right word for it: "1 day", "3 days".
            match inner.split_once(':').and_then(|(name, words)| Some((name, words.split_once('|')?))) {
                Some((name, (one, many))) => {
                    let v = lookup(vars, name);
                    out.push_str(&format!("{} {}", number(v), if (v - 1.0).abs() < 1e-9 { one } else { many }));
                }
                None => out.push_str(&number(lookup(vars, inner))),
            }
            rest = &rest[open + close + 1..];
        }
        out.push_str(rest);
        out
    }

    /// The game's name with its `{var}` placeholders filled (the HUD's top line, the pause menu).
    pub fn title_text(&self, vars: &[(&str, f64)]) -> Option<String> {
        self.title.as_ref().map(|t| self.fill(t, vars))
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

    /// The pause-menu line with its variables filled in.
    pub fn pause_line(&self, vars: &[(&str, f64)]) -> Option<String> {
        self.pause.as_ref().map(|t| self.fill(t, vars))
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

/// The variable names a text's `{name}` and `{name:one|many}` placeholders read.
fn placeholders(text: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else { break };
        names.push(rest[open + 1..open + close].split(':').next().unwrap_or_default());
        rest = &rest[open + close + 1..];
    }
    names
}

/// A scene variable the standard client's in-game HUD reads by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudRead {
    /// Its index among the scene's own variables (declaration order, the built-ins left out: the order of `RulesEngine::vars`).
    pub var: usize,
    /// The first place that reads it: `ui.counters[0].var`, `ui.objective[1].if`, `hud.custom_vars`, ...
    pub at: String,
}

/// The variables the standard client's in-game HUD reads, each once: the counters, the objective's conditions and texts, then the plain rows the state
/// panel lists. `vars` are the scene's own variable names in declaration order. Online the HUD has only the variables the server sent
/// (`net::protocol::MAX_RULE_VARS` of them), so these are sent first (`net::protocol::rule_vars_to_send`), and `lint` reports a HUD that reads more
/// ([`check_hud_reads`]). The title is here for the HUD's top line; the pause menu (which shows the title and `ui.pause` even with the HUD off) and the
/// audio layers are added by [`client_reads`]. Cards are not here: online they do not read the server's variables.
pub fn hud_reads(ui: Option<&GameUi>, hud: &crate::hud_config::HudConfig, vars: &[String]) -> Vec<HudRead> {
    let mut out: Vec<HudRead> = Vec::new();
    if !hud.enabled {
        return out;
    }
    let mut add = |name: &str, at: String| {
        if let Some(var) = vars.iter().position(|v| v == name).filter(|i| !out.iter().any(|r| r.var == *i)) {
            out.push(HudRead { var, at });
        }
    };
    if let Some(ui) = ui {
        for (k, c) in ui.counters.iter().enumerate() {
            add(&c.var, format!("ui.counters[{k}].var"));
            if let Of::Var(v) = &c.of {
                add(v, format!("ui.counters[{k}].of"));
            }
        }
        for (k, o) in ui.objective.iter().enumerate() {
            let mut read = Vec::new();
            if let Some(cond) = &o.cond {
                cond.read_vars(&mut read);
            }
            for i in read {
                if let Some(name) = ui.var_names.get(i) {
                    add(name, format!("ui.objective[{k}].if"));
                }
            }
            for name in placeholders(&o.text) {
                add(name, format!("ui.objective[{k}].text"));
            }
        }
        for name in ui.title.as_deref().map(placeholders).unwrap_or_default() {
            add(name, "ui.title".to_string());
        }
    }
    let zeros: Vec<(&str, f64)> = vars.iter().map(|n| (n.as_str(), 0.0)).collect();
    let rows = hud.visible_vars(&zeros).into_iter().filter(|(n, _)| !ui.is_some_and(|u| u.is_counted(n))).take(crate::hud_config::HUD_ROWS);
    let at = if hud.show_rules_vars { "hud.show_rules_vars" } else { "hud.custom_vars" };
    for (name, _) in rows {
        add(name, at.to_string());
    }
    out
}

/// Everything the standard client reads from the server's variables: the HUD's ([`hud_reads`]), then the title's placeholders and the pause menu's
/// line (the pause menu shows both, even with the HUD off: `ui.title`, `ui.pause`) and the audio layers' variables (`audio.layers[].var`, the names in
/// `audio_vars`). The online client copies what arrives into its own rules, which it does not run, so these follow the match too.
pub fn client_reads(ui: Option<&GameUi>, hud: &crate::hud_config::HudConfig, audio_vars: &[&str], vars: &[String]) -> Vec<HudRead> {
    let mut out = hud_reads(ui, hud, vars);
    let title = ui.and_then(|u| u.title.as_deref()).map(placeholders).unwrap_or_default();
    let pause = ui.and_then(|u| u.pause.as_deref()).map(placeholders).unwrap_or_default();
    let extra = title
        .into_iter()
        .map(|n| (n, "ui.title".to_string()))
        .chain(pause.into_iter().map(|n| (n, "ui.pause".to_string())))
        .chain(audio_vars.iter().enumerate().map(|(i, n)| (*n, format!("audio.layers[{i}].var"))));
    for (name, at) in extra {
        if let Some(var) = vars.iter().position(|v| v == name).filter(|i| !out.iter().any(|r| r.var == *i)) {
            out.push(HudRead { var, at });
        }
    }
    out
}

/// What is wrong with a HUD that could not show what it reads online: more variables than a networked match sends (`MAX_RULE_VARS`, one message naming
/// the ones that would not arrive). It used to show 0 online with no message (Quiet Compass, Idea Forge 2026-10-15); `lint` reports it as `net-hud`.
/// (Names do not travel since protocol v17, so their length no longer matters.)
pub fn check_hud_reads(ui: Option<&GameUi>, hud: &crate::hud_config::HudConfig, audio_vars: &[&str], vars: &[String]) -> Vec<String> {
    use crate::net::protocol::MAX_RULE_VARS;
    let reads = client_reads(ui, hud, audio_vars, vars);
    let mut out = Vec::new();
    if reads.len() > MAX_RULE_VARS {
        let late: Vec<String> = reads[MAX_RULE_VARS..].iter().take(6).map(|r| format!("`{}` ({})", vars[r.var], r.at)).collect();
        let more = reads.len() - MAX_RULE_VARS - late.len();
        out.push(format!(
            "the client reads {} variables (HUD, pause line, audio layers), but a networked match (red_server, play-local, playtest) sends at most {MAX_RULE_VARS} (net::protocol::MAX_RULE_VARS), so online {}{} would show 0; read at most {MAX_RULE_VARS}: drop a counter or an objective variable, or list fewer rows (`hud.show_rules_vars: false` with `hud.custom_vars`, or a `_name` variable, which is never a row)",
            reads.len(),
            late.join(", "),
            if more > 0 { format!(" and {more} more") } else { String::new() }
        ));
    }
    out
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
            let inner = &rest[open + 1..open + close];
            // `{days:day|days}`: the variable, then the word for one and the word for the rest.
            let name = match inner.split_once(':') {
                Some((name, words)) if words.split_once('|').is_some_and(|(a, b)| !a.is_empty() && !b.is_empty()) => name,
                Some((_, _)) => {
                    self.err(path, format!("`{{{inner}}}`: after the `:` write the word for one and the word for the rest, like {{days:day|days}}"));
                    inner.split(':').next().unwrap_or(inner)
                }
                None => inner,
            };
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
    ui.pause = ctx.text("ui.pause", o.get("pause"), MAX_TITLE);

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
    fn a_count_takes_its_plural_with_it() {
        let u = ui(json!({"start": {"title": "T", "text": "{delivered:parcel|parcels} so far"}})).unwrap().unwrap();
        assert_eq!(u.fill("{delivered:parcel|parcels}!", &[("delivered", 1.0)]), "1 parcel!");
        assert_eq!(u.fill("{delivered:parcel|parcels}!", &[("delivered", 0.0)]), "0 parcels!");
        assert_eq!(u.fill("{delivered:parcel|parcels}!", &[("delivered", 3.0)]), "3 parcels!");
        assert!(ui(json!({"start": {"title": "{delivered:parcel}"}})).unwrap_err().join(" ").contains("word for one"));
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

    fn many(n: usize) -> (RuleSet, Vec<String>) {
        let names: Vec<String> = (0..n).map(|i| format!("v{i:02}")).collect();
        let vars: Map<String, Value> = names.iter().map(|n| (n.clone(), json!(0))).collect();
        let set = crate::sim::rules::parse_rules(json!({ "vars": vars }).as_object().unwrap(), &crate::sim::rules::Refs::default()).unwrap();
        (set, names)
    }

    #[test]
    fn the_hud_reads_counters_then_the_objective_then_its_rows_each_once() {
        let (set, names) = many(20);
        let u = parse_ui(
            json!({"ui": {"counters": [{"var": "v17", "of": "v19"}, {"var": "v02"}], "objective": [{"if": "v19 > v05", "text": "{v18:left|left} {v17}"}, {"text": "x"}]}})
                .as_object()
                .unwrap(),
            &set,
        )
        .unwrap()
        .unwrap();
        let hud = crate::hud_config::HudConfig::default();
        let reads = hud_reads(Some(&u), &hud, &names);
        let at: Vec<(&str, &str)> = reads.iter().map(|r| (names[r.var].as_str(), r.at.as_str())).collect();
        assert_eq!(
            &at[..5],
            [
                ("v17", "ui.counters[0].var"),
                ("v19", "ui.counters[0].of"),
                ("v02", "ui.counters[1].var"),
                ("v05", "ui.objective[0].if"),
                ("v18", "ui.objective[0].text")
            ]
        );
        // Then the first HUD_ROWS shown variables no counter shows: v00, v01, v03, v04, v05 (already read), v06, v07, v08.
        let rows: Vec<&str> = reads[5..].iter().map(|r| names[r.var].as_str()).collect();
        assert_eq!(rows, ["v00", "v01", "v03", "v04", "v06", "v07", "v08"]);
        let custom = crate::hud_config::HudConfig { show_rules_vars: false, custom_vars: vec!["v12".into(), "v02".into()], ..hud.clone() };
        assert_eq!(hud_reads(Some(&u), &custom, &names).last().map(|r| (r.var, r.at.as_str())), Some((12, "hud.custom_vars")));
        assert!(hud_reads(Some(&u), &crate::hud_config::HudConfig { enabled: false, ..hud.clone() }, &names).is_empty(), "a hidden HUD reads nothing");
        assert_eq!(hud_reads(None, &hud, &names).len(), crate::hud_config::HUD_ROWS, "no ui block: the panel's rows");
        assert!(check_hud_reads(Some(&u), &hud, &[], &names).is_empty(), "13 reads fit");
    }

    #[test]
    fn the_client_also_reads_the_title_the_pause_line_and_the_audio_layers_and_fills_the_title() {
        let (set, names) = many(8);
        let u = parse_ui(json!({"ui": {"title": "Day {v06}", "pause": "{v05:win|wins} so far", "counters": [{"var": "v01"}]}}).as_object().unwrap(), &set)
            .unwrap()
            .unwrap();
        let hidden = crate::hud_config::HudConfig { enabled: false, ..Default::default() };
        let reads: Vec<(usize, String)> = client_reads(Some(&u), &hidden, &["v07"], &names).into_iter().map(|r| (r.var, r.at)).collect();
        assert_eq!(
            reads,
            [(6, "ui.title".to_string()), (5, "ui.pause".to_string()), (7, "audio.layers[0].var".to_string())],
            "with the HUD off the pause menu still shows the title and its line, and the audio reads"
        );
        let shown = crate::hud_config::HudConfig { show_rules_vars: false, ..Default::default() };
        let reads: Vec<(usize, String)> = client_reads(Some(&u), &shown, &["v07", "v01"], &names).into_iter().map(|r| (r.var, r.at)).collect();
        assert_eq!(reads[..2], [(1, "ui.counters[0].var".to_string()), (6, "ui.title".to_string())], "{reads:?}");
        assert_eq!(reads.len(), 4, "each once: {reads:?}");
        assert_eq!(u.title_text(&[("v06", 3.0)]).as_deref(), Some("Day 3"), "the title's placeholders are filled, not printed");
    }

    #[test]
    fn a_hud_that_reads_more_than_the_wire_sends_is_an_error_that_names_the_cap_and_the_fix() {
        // 65 counters of 70 variables: one more than protocol v17's 64.
        let (set, names) = many(70);
        let counters: Vec<Value> = names.iter().take(65).map(|n| json!({"var": n})).collect();
        let u = parse_ui(json!({"ui": {"counters": counters}}).as_object().unwrap(), &set).unwrap().unwrap();
        let no_rows = crate::hud_config::HudConfig { show_rules_vars: false, ..Default::default() };
        let e = check_hud_reads(Some(&u), &no_rows, &[], &names);
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].starts_with("the client reads 65 variables") && e[0].contains("online `v64` (ui.counters[64].var) would show 0"), "{}", e[0]);
        for want in ["at most 64 (net::protocol::MAX_RULE_VARS)", "drop a counter", "hud.custom_vars"] {
            assert!(e[0].contains(want), "missing `{want}` in {}", e[0]);
        }
        let e = check_hud_reads(Some(&u), &crate::hud_config::HudConfig::default(), &[], &names);
        assert!(e.len() == 1 && e[0].starts_with("the client reads 70 variables") && e[0].contains("`v69` (hud.show_rules_vars)"), "the rows count too: {e:?}");
    }

    #[test]
    fn texts_must_fit_a_screen_and_braces_must_close() {
        let long = "x".repeat(MAX_OBJECTIVE + 1);
        let e = ui(json!({"objective": long, "start": {"title": "Open {delivered"}})).unwrap_err().join("\n");
        assert!(e.contains("too long for a screen") && e.contains("no closing `}`"), "{e}");
    }
}
