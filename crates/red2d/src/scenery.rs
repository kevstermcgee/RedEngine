//! What a 2D game asks of the screen and of the platform around it, beyond its things and rules: a level's `sky` and parallax `backdrop`, HUD `image`s,
//! the camera's limits, the `pause_if` that stands the world still under a menu, the game's `icon`, and the `window` / `save` / `export_music` actions the
//! platform carries out (`sim::Request`). The types live here; `game::parse` calls these parsers.

use crate::fields::{check_fields, describe_value, opt, req, Field, Ty};
use crate::game::{color_ok, expr, num, pair, parse_color, range_of, range_or_num, Act, Color, Ctx, Names, Sprite, View, Widget, WidgetKind};
use crate::rules_expr::Expr;
use serde_json::{Map, Value};

/// One band of distant scenery (`backdrop`): its sprites repeat along the whole width, scrolled by `parallax` times the camera, so far things move slowly.
#[derive(Debug, Clone)]
pub struct Backdrop {
    /// The sprites it repeats (indexes into [`crate::game::GameDef::sprites`]), picked in a fixed pseudo-random order.
    pub sprites: Vec<usize>,
    /// The screen y of the sprites' bottom edge when the camera is at the top of the world.
    pub y: f32,
    /// How much of the camera's movement it follows: 0 = fixed to the screen (the far sky), 1 = moves with the world.
    pub parallax: f32,
    /// Gap between one sprite and the next, px: `[min, max]`.
    pub spacing: [f32; 2],
    /// Colour filled from the band's bottom edge down to the bottom of the screen (the ground the far hills stand on), if any.
    pub fill: Option<Color>,
    /// Which pseudo-random order (two bands with the same sprites look different with different seeds).
    pub seed: u64,
    /// Integer scale.
    pub scale: u32,
}

/// What a `window` action asks of the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowCmd {
    /// Borderless fullscreen on or off.
    Fullscreen,
    /// Close the game.
    Close,
}

/// A level's `sky`: `[top, bottom]`, two colours.
pub(crate) fn sky_ok(v: &Value) -> Result<(), String> {
    match v.as_array().map(Vec::as_slice) {
        Some([a, b]) => color_ok(a).and(color_ok(b)),
        _ => Err(format!("expected [top, bottom], two colours like [\"#6bb4ff\", \"#fff3c4\"], got {}", describe_value(v))),
    }
}

/// A level's `sky`, read (the checker is [`sky_ok`]).
pub(crate) fn sky_of(o: &Map<String, Value>) -> Option<(Color, Color)> {
    o.get("sky").and_then(Value::as_array).and_then(|a| match a.as_slice() {
        [t, b] => Some((parse_color(t.as_str()?)?, parse_color(b.as_str()?)?)),
        _ => None,
    })
}

const BACKDROP_FIELDS: &[Field] = &[
    req("sprites", Ty::Strs, "the sprites the band repeats (a fixed pseudo-random order)"),
    req("y", Ty::Num(Some((-4000.0, 4000.0))), "screen y of the sprites' bottom edge with the camera at the top of the world"),
    opt("parallax", Ty::Num(Some((0.0, 1.0))), "how much of the camera's movement it follows: 0 fixed to the screen, 1 with the world (default 0.5)"),
    opt("spacing", Ty::Custom(range_or_num), "px between sprites: a number or [min, max] (default 0)"),
    opt("fill", Ty::Custom(color_ok), "colour filled from the band's bottom edge to the bottom of the screen"),
    opt("seed", Ty::Num(Some((0.0, 1.0e9))), "which pseudo-random order (default 0)"),
    opt("scale", Ty::Num(Some((1.0, 8.0))), "whole-number scale (default 1)"),
];

/// A level's `backdrop`: bands of far scenery, drawn in order (the first is the farthest).
pub(crate) fn parse_backdrop(ctx: &mut Ctx, path: &str, v: &Value, sprites: &[Sprite]) -> Vec<Backdrop> {
    let Some(list) = v.as_array() else {
        ctx.err(path, format!("expected a list of bands, got {}", describe_value(v)));
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, item) in list.iter().enumerate() {
        let bp = format!("{path}[{i}]");
        let Some(o) = ctx.obj(&bp, item) else { continue };
        let before = ctx.errs.len();
        check_fields(&mut ctx.errs, &bp, o, BACKDROP_FIELDS);
        let mut ix = Vec::new();
        for (k, n) in o.get("sprites").and_then(Value::as_array).into_iter().flatten().enumerate() {
            let name = n.as_str().unwrap_or("");
            match sprites.iter().position(|sp| sp.name == name) {
                Some(j) => ix.push(j),
                None => ctx.err(format!("{bp}.sprites[{k}]"), format!("no sprite `{name}`{}", Ctx::near(name, sprites.iter().map(|sp| sp.name.clone())))),
            }
        }
        if ix.is_empty() && ctx.errs.len() == before {
            ctx.err(format!("{bp}.sprites"), "a band needs at least one sprite");
        }
        if ctx.errs.len() > before {
            continue;
        }
        out.push(Backdrop {
            sprites: ix,
            y: o.get("y").and_then(num).unwrap_or(0.0) as f32,
            parallax: o.get("parallax").and_then(num).unwrap_or(0.5) as f32,
            spacing: range_of(o, "spacing", [0.0, 0.0]),
            fill: o.get("fill").and_then(Value::as_str).and_then(parse_color),
            seed: o.get("seed").and_then(num).unwrap_or(0.0) as u64,
            scale: o.get("scale").and_then(num).unwrap_or(1.0).round().max(1.0) as u32,
        });
    }
    out
}

/// The actions a game asks the platform for: `window`, `save`, `export_music`.
pub(crate) fn parse_action(ctx: &mut Ctx, ap: &str, key: &str, arg: &Value, names: &Names) -> Option<Act> {
    match key {
        "window" => match arg.as_str() {
            Some("fullscreen") => Some(Act::Window(WindowCmd::Fullscreen)),
            Some("close") => Some(Act::Window(WindowCmd::Close)),
            _ => {
                ctx.err(
                    ap,
                    format!("expected \"fullscreen\" (toggle borderless fullscreen) or \"close\" (quit the game, saving first), got {}", describe_value(arg)),
                );
                None
            }
        },
        "save" => (arg == &Value::Bool(true)).then_some(Act::Save).or_else(|| {
            ctx.err(ap, "write {\"save\": true}");
            None
        }),
        _ if names.music.is_empty() => {
            ctx.err(ap, "the game has no `music` to export");
            None
        }
        _ => (arg == &Value::Bool(true)).then_some(Act::ExportMusic).or_else(|| {
            ctx.err(ap, "write {\"export_music\": true} (the track playing now)");
            None
        }),
    }
}

/// A HUD `image` widget: a sprite drawn on the screen at its top-left corner.
pub(crate) fn parse_image(ctx: &mut Ctx, path: &str, o: &Map<String, Value>, iv: &Value, show: Option<Expr>, sprites: &[Sprite]) -> Option<Widget> {
    let before = ctx.errs.len();
    check_fields(
        &mut ctx.errs,
        path,
        o,
        &[
            req("image", Ty::Str, "a sprite name: drawn on the screen (a HUD icon, a portrait, a title picture)"),
            req("at", Ty::Nums(&[2]), "[x, y] of its top-left corner"),
            opt("scale", Ty::Num(Some((1.0, 16.0))), "whole-number scale, default 1"),
            opt("show", Ty::Str, "shown only while this holds"),
        ],
    );
    let name = iv.as_str().unwrap_or("");
    let Some(sprite) = sprites.iter().position(|sp| sp.name == name) else {
        ctx.err(format!("{path}.image"), format!("no sprite `{name}`{}", Ctx::near(name, sprites.iter().map(|sp| sp.name.clone()))));
        return None;
    };
    if ctx.errs.len() > before {
        return None;
    }
    let scale = o.get("scale").and_then(num).unwrap_or(1.0).round().max(1.0) as u32;
    Some(Widget { kind: WidgetKind::Image { sprite, at: pair(&o["at"])?, scale }, show })
}

/// `view.camera.min_x` / `max_x`: expressions over the game's variables that bound the camera's left edge.
pub(crate) fn parse_camera_limits(ctx: &mut Ctx, root: &Map<String, Value>, view: &mut View, names: &Names) {
    let Some(c) = root.get("view").and_then(|v| v.get("camera")).and_then(Value::as_object) else { return };
    for (key, slot) in [("min_x", &mut view.min_x), ("max_x", &mut view.max_x)] {
        if let Some(v) = c.get(key) {
            *slot = match v {
                Value::String(src) => expr(ctx, &format!("view.camera.{key}"), src, names),
                Value::Number(n) => expr(ctx, &format!("view.camera.{key}"), &n.to_string(), names),
                other => {
                    ctx.err(format!("view.camera.{key}"), format!("expected a number or an expression, got {}", describe_value(other)));
                    None
                }
            };
        }
    }
}

/// The game's `pause_if` (an expression) and `icon` (a sprite).
pub(crate) fn parse_pause_and_icon(ctx: &mut Ctx, root: &Map<String, Value>, sprites: &[Sprite], names: &Names) -> (Option<Expr>, Option<usize>) {
    let pause_if = root.get("pause_if").and_then(|v| match v.as_str() {
        Some(src) => expr(ctx, "pause_if", src, names),
        None => {
            ctx.err("pause_if", format!("expected an expression like \"menu == 1\" (while it holds the world stands still), got {}", describe_value(v)));
            None
        }
    });
    let icon = root.get("icon").and_then(|v| {
        let name = v.as_str().unwrap_or("");
        let found = sprites.iter().position(|sp| sp.name == name);
        if found.is_none() {
            let near = Ctx::near(name, sprites.iter().map(|sp| sp.name.clone()));
            ctx.err("icon", format!("expected a sprite name (the window, taskbar and shortcut icon), got {}{near}", describe_value(v)));
        }
        found
    });
    (pause_if, icon)
}
