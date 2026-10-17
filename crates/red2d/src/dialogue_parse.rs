//! Reading a `dialogue` widget: its lines, choices and gotos, and the two state variables (`<id>_line`, `<id>_choice`) every dialogue declares.
//! The types are in `game` (`Dialogue`, `DialogueLine`, `Choice`, `Goto`); `game::parse` calls these.

use crate::fields::{check_fields, opt, req, Field, Ty};
use crate::game::*;
use serde_json::{Map, Value};

const DIALOGUE_FIELDS: &[Field] = &[
    req("id", Ty::Str, "a name: its state is the variables <id>_line and <id>_choice"),
    req("lines", Ty::Custom(any_ok), "[{\"who\": \"NAME\", \"text\": \"...\"}, ...] in order"),
    req("at", Ty::Nums(&[2]), "[x, y] top-left of the box"),
    req("size", Ty::Nums(&[2]), "[w, h]"),
    opt("color", Ty::Custom(color_ok), "text, default white"),
    opt("back", Ty::Custom(color_ok), "the box, default dark"),
    opt("accent", Ty::Custom(color_ok), "speaker name and the highlighted choice, default gold"),
    opt("scale", Ty::Custom(scale_ok), "1 to 16, default 1"),
    opt("advance", Ty::OneOf(ACTIONS), "the input action that moves on, default action"),
    opt("pause", Ty::Bool, "the world stands still while it is open (default true)"),
    opt("done", Ty::Str, "an event emitted when it closes"),
];
const LINE_FIELDS: &[Field] = &[
    req("text", Ty::Custom(text_source_ok), "the words ({var} placeholders), or {\"pick\": table, \"by\": expression}"),
    opt("id", Ty::Str, "a name a goto can jump to"),
    opt("who", Ty::Str, "the speaker, drawn above the text"),
    opt("emit", Ty::Str, "an event emitted when the player moves past the line"),
    opt("goto", Ty::Str, "a line id, or \"end\": where to go after this line (default the next)"),
    opt("choices", Ty::Custom(any_ok), "[{\"label\": \"YES\", \"emit\": \"said_yes\", \"goto\": \"line_id\"|\"end\"}] (up to 9)"),
];
const CHOICE_FIELDS: &[Field] = &[
    req("label", Ty::Str, "what the player picks"),
    opt("emit", Ty::Str, "the event it emits"),
    opt("goto", Ty::Str, "a line id, or \"end\" (default the next line)"),
];

/// Parses a `dialogue` widget. Its two state variables were declared by [`dialogue_vars`] before the rules were read.
pub(crate) fn parse_dialogue(ctx: &mut Ctx, path: &str, v: &Value, names: &Names, tables: &[(String, Vec<String>)]) -> Option<Dialogue> {
    let o = ctx.obj(path, v)?;
    let before = ctx.errs.len();
    check_fields(&mut ctx.errs, path, o, DIALOGUE_FIELDS);
    let id = o.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let lines_v = o.get("lines").and_then(Value::as_array).cloned().unwrap_or_default();
    if lines_v.is_empty() {
        ctx.err(format!("{path}.lines"), "a dialogue needs at least one line: [{\"text\": \"...\"}]");
    }
    let ids: Vec<Option<String>> = lines_v.iter().map(|l| l.get("id").and_then(Value::as_str).map(str::to_string)).collect();
    let goto = |ctx: &mut Ctx, p: &str, g: Option<&Value>| -> Option<Goto> {
        let g = g?.as_str()?;
        if g == "end" {
            return Some(Goto::End);
        }
        match ids.iter().position(|x| x.as_deref() == Some(g)) {
            Some(i) => Some(Goto::Line(i)),
            None => {
                ctx.err(p.to_string(), format!("no line with id `{g}`{} (or \"end\")", Ctx::near(g, ids.iter().flatten().cloned())));
                None
            }
        }
    };
    let mut lines = Vec::new();
    for (li, lv) in lines_v.iter().enumerate() {
        let lp = format!("{path}.lines[{li}]");
        let Some(lo) = ctx.obj(&lp, lv) else { continue };
        check_fields(&mut ctx.errs, &lp, lo, LINE_FIELDS);
        let pick = match lo.get("text").and_then(Value::as_object) {
            Some(po) => parse_pick(ctx, &format!("{lp}.text"), po, tables, names),
            None => None,
        };
        let text = lo.get("text").and_then(Value::as_str).unwrap_or("").to_string();
        if !text.is_empty() {
            check_template(ctx, &format!("{lp}.text"), &text, names);
        }
        let mut choices = Vec::new();
        let cv = lo.get("choices").and_then(Value::as_array).cloned().unwrap_or_default();
        if cv.len() > 9 {
            ctx.err(format!("{lp}.choices"), format!("{} choices; at most 9 (keys 1-9)", cv.len()));
        }
        for (ci, c) in cv.iter().enumerate() {
            let cp = format!("{lp}.choices[{ci}]");
            let Some(co) = ctx.obj(&cp, c) else { continue };
            check_fields(&mut ctx.errs, &cp, co, CHOICE_FIELDS);
            choices.push(Choice {
                label: co.get("label").and_then(Value::as_str).unwrap_or("").to_string(),
                emit: co.get("emit").and_then(Value::as_str).map(str::to_string),
                goto: goto(ctx, &format!("{cp}.goto"), co.get("goto")),
            });
        }
        lines.push(DialogueLine {
            id: ids[li].clone(),
            who: lo.get("who").and_then(Value::as_str).map(str::to_string),
            text,
            pick,
            emit: lo.get("emit").and_then(Value::as_str).map(str::to_string),
            goto: goto(ctx, &format!("{lp}.goto"), lo.get("goto")),
            choices,
        });
    }
    let var = |n: String| names.vars.iter().position(|v| *v == n);
    let (Some(line_var), Some(choice_var)) = (var(format!("{id}_line")), var(format!("{id}_choice"))) else {
        ctx.err(format!("{path}.id"), "a dialogue id is letters, digits and _ (its state lives in <id>_line and <id>_choice)");
        return None;
    };
    if ctx.errs.len() > before {
        return None;
    }
    let col = |k: &str, d: Color| o.get(k).and_then(Value::as_str).and_then(parse_color).unwrap_or(d);
    Some(Dialogue {
        id,
        lines,
        at: pair(&o["at"])?,
        size: pair(&o["size"])?,
        color: col("color", [255, 255, 255, 255]),
        back: col("back", [16, 20, 32, 230]),
        accent: col("accent", [255, 204, 102, 255]),
        scale: o.get("scale").and_then(Value::as_u64).unwrap_or(1) as u32,
        advance: o.get("advance").and_then(Value::as_str).and_then(|a| ACTIONS.iter().position(|x| *x == a)).unwrap_or(4),
        pause: o.get("pause").and_then(Value::as_bool).unwrap_or(true),
        done: o.get("done").and_then(Value::as_str).map(str::to_string),
        line_var,
        choice_var,
    })
}

/// The state variables of every dialogue in `ui` (`<id>_line`, `<id>_choice`), declared before anything reads the variables so rules can use them.
pub(crate) fn dialogue_vars(root: &Map<String, Value>) -> Vec<String> {
    let mut out = Vec::new();
    for w in root.get("ui").and_then(Value::as_array).into_iter().flatten() {
        if let Some(id) = w.get("dialogue").and_then(|d| d.get("id")).and_then(Value::as_str) {
            if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !id.starts_with(|c: char| c.is_ascii_digit()) {
                out.push(format!("{id}_line"));
                out.push(format!("{id}_choice"));
            }
        }
    }
    out
}
