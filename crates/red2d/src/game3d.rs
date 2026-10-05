//! The 3D elements of a hybrid game: models, 3D viewports, a 3D world view, and how they are written in the game file.
//!
//! A hybrid game is still one `*.game2d.json` with one simulation: entities move on a plane, rules run, input is the same. What changes is how things are *drawn*, and only
//! where the game asks for it:
//!
//! * `models`: named 3D objects assembled from a few primitives (box, sphere, cylinder, cone, pyramid, torus, plane) and from sprites pressed into voxels.
//! * a prefab `shape` of `{"model": name}`: a thing in the 2D world that is drawn as a (spinning, tilting, tinted) 3D model: a boss, a pickup, a ship.
//! * `ui` widgets and `layers3d` entries of `{"view3d": {...}}`: a small 3D scene in a rectangle of the screen (a rotating portrait in the HUD, a 3D backdrop behind the sprites).
//! * `view.world3d`: the whole world seen in perspective (the entities on a ground, tile-map walls standing up, sprites as billboards); the HUD stays flat on top.
//! * `{"minimap": ...}` widgets: a flat map of the world over either kind of game.
//!
//! Everything is validated at load: a model that names no primitive, a viewport that shows a model that does not exist, an angle that is not a number or an expression.

use crate::fields::{check_keys, describe_value};
use crate::game::{num, pair, parse_color, Color, Ctx, Names, Sprite, Val};
use crate::raster3d::{self, Mesh, Shape, Tri, V3};
use serde_json::{Map, Value};

/// The most triangles one model may have, and all models together.
pub const MAX_MODEL_TRIS: usize = 6_000;
/// Total triangles over all models.
pub const MAX_TOTAL_TRIS: usize = 30_000;

/// One directional light.
#[derive(Debug, Clone, Copy)]
pub struct Light3 {
    /// Direction the light travels.
    pub dir: V3,
    /// Brightness of unlit faces.
    pub ambient: f32,
    /// Three flat brightness steps.
    pub toon: bool,
}

impl Default for Light3 {
    fn default() -> Self {
        let l = raster3d::Light::default();
        Light3 { dir: l.dir, ambient: l.ambient, toon: l.toon }
    }
}

impl Light3 {
    /// For the renderer.
    pub fn light(self) -> raster3d::Light {
        raster3d::Light { dir: self.dir, ambient: self.ambient, toon: self.toon }
    }
}

/// A named 3D object.
#[derive(Debug, Clone)]
pub struct Model {
    /// Its name.
    pub name: String,
    /// Its triangles.
    pub mesh: Mesh,
    /// The radius of the sphere about its origin that holds it.
    pub radius: f32,
}

/// A model drawn as the look of a prefab.
#[derive(Debug, Clone)]
pub struct ModelShape {
    /// Index into [`crate::game::GameDef::models`].
    pub model: usize,
    /// Rotation about the vertical axis, degrees (a number or an expression such as `"time * 60"`).
    pub yaw: Val,
    /// Tilt forward/back, degrees.
    pub pitch: Val,
    /// Roll, degrees.
    pub roll: Val,
    /// Scale.
    pub scale: f32,
    /// How far above the model the camera looks down from, degrees.
    pub elevation: f32,
    /// Tint multiplied into every colour.
    pub tint: Option<Color>,
    /// Light.
    pub light: Light3,
}

/// A model placed in a viewport.
#[derive(Debug, Clone)]
pub struct ModelItem {
    /// Index into the models.
    pub model: usize,
    /// Position.
    pub at: V3,
    /// Yaw, degrees.
    pub yaw: Val,
    /// Pitch, degrees.
    pub pitch: Val,
    /// Roll, degrees.
    pub roll: Val,
    /// Scale.
    pub scale: f32,
    /// Tint.
    pub tint: Option<Color>,
}

/// A 3D scene drawn into a rectangle of the screen.
#[derive(Debug, Clone)]
pub struct View3d {
    /// Top-left on the screen.
    pub at: [f32; 2],
    /// Size on the screen.
    pub size: [f32; 2],
    /// Camera position.
    pub eye: V3,
    /// Look at.
    pub target: V3,
    /// Vertical field of view, degrees.
    pub fov: f32,
    /// Orthographic half-height instead of perspective.
    pub ortho: Option<f32>,
    /// Behind the scene (alpha 0 = see-through).
    pub background: Color,
    /// What is in it.
    pub items: Vec<ModelItem>,
    /// Light.
    pub light: Light3,
}

/// A viewport in the world, ordered with the entities by `layer`.
#[derive(Debug, Clone)]
pub struct Layer3d {
    /// Draw order against entities (below 0 is behind most things).
    pub layer: i32,
    /// The scene (its `at`/`size` default to the whole screen).
    pub view: View3d,
}

/// How 2D plane coordinates map into the 3D world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plane {
    /// The 2D plane is the floor: x stays x, y becomes depth (top-down and arena games).
    Ground,
    /// The 2D plane is a wall seen from the front: y becomes height, drawn upside down from screen y (side-scrollers).
    Wall,
}

/// The floor of a world view.
#[derive(Debug, Clone, Copy)]
pub struct GroundSpec {
    /// Colour.
    pub color: Color,
    /// The second colour of a checker, if any.
    pub alt: Option<Color>,
    /// Tile size in px.
    pub tile: f32,
}

/// The whole world seen in 3D.
#[derive(Debug, Clone)]
pub struct World3d {
    /// How the plane maps.
    pub plane: Plane,
    /// Camera distance from what it looks at.
    pub distance: f32,
    /// Degrees above the horizon (89 = straight down).
    pub pitch: f32,
    /// Rotation about the vertical axis, degrees (number or expression).
    pub yaw: Val,
    /// Field of view, degrees.
    pub fov: f32,
    /// The floor.
    pub ground: Option<GroundSpec>,
    /// Light.
    pub light: Light3,
    /// Colour behind everything.
    pub sky: Color,
}

impl Plane {
    /// A point of the 2D plane (and a height above it) in the 3D world.
    pub fn to3(self, x: f32, y: f32, lift: f32) -> V3 {
        match self {
            Plane::Ground => [x, lift, y],
            Plane::Wall => [x, -y + lift, 0.0],
        }
    }
    /// The 2D plane's point under a 3D one (the height is dropped).
    pub fn from3(self, p: V3) -> [f32; 2] {
        match self {
            Plane::Ground => [p[0], p[2]],
            Plane::Wall => [p[0], -p[1]],
        }
    }
    /// The 3D plane the 2D one lies in: a point on it and its normal.
    pub fn surface(self) -> (V3, V3) {
        match self {
            Plane::Ground => ([0.0; 3], [0.0, 1.0, 0.0]),
            Plane::Wall => ([0.0; 3], [0.0, 0.0, 1.0]),
        }
    }
}

/// The camera of a world view looking at the 2D point `center`.
pub fn world_camera(w: &World3d, vars: &[f64], center: [f32; 2]) -> raster3d::Camera {
    let target = w.plane.to3(center[0], center[1], 0.0);
    let (p, y) = (w.pitch.to_radians(), (w.yaw.eval(vars) as f32).to_radians());
    let eye = [
        target[0] + w.distance * libm::cosf(p) * libm::sinf(y),
        target[1] + w.distance * libm::sinf(p),
        target[2] + w.distance * libm::cosf(p) * libm::cosf(y),
    ];
    raster3d::Camera { eye, target, up: [0.0, 1.0, 0.0], fov: w.fov, ortho: None }
}

/// Where the 2D point under screen pixel `(sx, sy)` of a `w x h` screen is, seen through a world view (None if the pixel looks at the sky).
pub fn world_point(world: &World3d, vars: &[f64], center: [f32; 2], w: u32, h: u32, sx: f32, sy: f32) -> Option<[f32; 2]> {
    let cam = world_camera(world, vars, center);
    let (origin, n) = world.plane.surface();
    raster3d::unproject_to_plane(&cam, w, h, sx, sy, origin, n).map(|p| world.plane.from3(p))
}

fn v3(v: &Value) -> Option<V3> {
    let a = v.as_array()?;
    if a.len() != 3 {
        return None;
    }
    Some([num(&a[0])? as f32, num(&a[1])? as f32, num(&a[2])? as f32])
}

fn want_v3(ctx: &mut Ctx, path: &str, v: Option<&Value>, default: V3) -> V3 {
    match v {
        None => default,
        Some(x) => v3(x).unwrap_or_else(|| {
            ctx.err(path, format!("expected [x, y, z] (three numbers), got {}", describe_value(x)));
            default
        }),
    }
}

fn want_num(ctx: &mut Ctx, path: &str, v: Option<&Value>, default: f32, lo: f32, hi: f32) -> f32 {
    match v {
        None => default,
        Some(x) => match num(x).filter(|n| (f64::from(lo)..=f64::from(hi)).contains(n)) {
            Some(n) => n as f32,
            None => {
                ctx.err(path, format!("expected a number from {lo} to {hi}, got {}", describe_value(x)));
                default
            }
        },
    }
}

fn want_color(ctx: &mut Ctx, path: &str, v: Option<&Value>, default: Color) -> Color {
    match v {
        None => default,
        Some(x) => match x.as_str().and_then(parse_color) {
            Some(c) => c,
            None => {
                ctx.err(path, format!("expected a color string like \"#ffcc00\", got {}", describe_value(x)));
                default
            }
        },
    }
}

fn want_val(ctx: &mut Ctx, path: &str, v: Option<&Value>, names: &Names) -> Val {
    match v {
        None => Val::Num(0.0),
        Some(x) => crate::game::val(ctx, path, x, names).unwrap_or(Val::Num(0.0)),
    }
}

/// `{"dir": [x,y,z], "ambient": 0.4, "toon": true}`.
pub(crate) fn parse_light(ctx: &mut Ctx, path: &str, v: Option<&Value>) -> Light3 {
    let mut l = Light3::default();
    let Some(v) = v else { return l };
    let Some(o) = ctx.obj(path, v) else { return l };
    check_keys(&mut ctx.errs, path, o, &["dir", "ambient", "toon"]);
    l.dir = want_v3(ctx, &format!("{path}.dir"), o.get("dir"), l.dir);
    l.ambient = want_num(ctx, &format!("{path}.ambient"), o.get("ambient"), l.ambient, 0.0, 1.0);
    if let Some(t) = o.get("toon") {
        match t.as_bool() {
            Some(b) => l.toon = b,
            None => ctx.err(format!("{path}.toon"), format!("expected true or false, got {}", describe_value(t))),
        }
    }
    l
}

const SHAPES: &[&str] = &["box", "sphere", "cylinder", "cone", "pyramid", "torus", "plane", "voxels"];

fn parse_part(ctx: &mut Ctx, path: &str, v: &Value, sprites: &[Sprite]) -> Option<(Vec<Tri>, V3, V3)> {
    let o = ctx.obj(path, v)?;
    check_keys(
        &mut ctx.errs,
        path,
        o,
        &["shape", "size", "radius", "height", "base", "major", "minor", "segments", "sprite", "depth", "cell", "at", "rot", "color"],
    );
    let kind = o.get("shape").and_then(Value::as_str).unwrap_or("");
    if !SHAPES.contains(&kind) {
        let near = crate::suggest::suggest(kind, SHAPES.iter().copied());
        ctx.err(
            format!("{path}.shape"),
            format!("`{kind}` is not a shape{} (shapes: {})", near.first().map(|n| format!(" — did you mean `{n}`?")).unwrap_or_default(), SHAPES.join(", ")),
        );
        return None;
    }
    let before = ctx.errs.len();
    let color = want_color(ctx, &format!("{path}.color"), o.get("color"), [200, 200, 200, 255]);
    let rgb = [color[0], color[1], color[2]];
    let seg = want_num(ctx, &format!("{path}.segments"), o.get("segments"), 12.0, 3.0, 48.0) as u32;
    let pos = |ctx: &mut Ctx, key: &str, default: f32| want_num(ctx, &format!("{path}.{key}"), o.get(key), default, 0.001, 1000.0);
    let tris = match kind {
        "box" => {
            let s = want_v3(ctx, &format!("{path}.size"), o.get("size"), [1.0, 1.0, 1.0]);
            if s.iter().any(|d| !(0.001..=1000.0).contains(d)) {
                ctx.err(format!("{path}.size"), "each side is 0.001 to 1000");
            }
            raster3d::build(&Shape::Box(s), rgb)
        }
        "sphere" => raster3d::build(&Shape::Sphere { radius: pos(ctx, "radius", 1.0), seg }, rgb),
        "cylinder" => raster3d::build(&Shape::Cylinder { radius: pos(ctx, "radius", 1.0), height: pos(ctx, "height", 1.0), seg }, rgb),
        "cone" => raster3d::build(&Shape::Cone { radius: pos(ctx, "radius", 1.0), height: pos(ctx, "height", 1.0), seg }, rgb),
        "pyramid" => raster3d::build(&Shape::Pyramid { base: pos(ctx, "base", 1.0), height: pos(ctx, "height", 1.0) }, rgb),
        "torus" => raster3d::build(&Shape::Torus { major: pos(ctx, "major", 1.0), minor: pos(ctx, "minor", 0.25), seg }, rgb),
        "plane" => match o.get("size").and_then(pair) {
            Some(p) if p[0] > 0.0 && p[1] > 0.0 => raster3d::build(&Shape::Plane(p), rgb),
            _ => {
                ctx.err(format!("{path}.size"), "a plane needs size [x, z] (two numbers above 0)");
                Vec::new()
            }
        },
        _ => {
            let name = o.get("sprite").and_then(Value::as_str).unwrap_or("");
            match sprites.iter().find(|s| s.name == name) {
                Some(sp) => raster3d::voxels(
                    &sp.frames[0],
                    sp.w,
                    sp.h,
                    want_num(ctx, &format!("{path}.depth"), o.get("depth"), 2.0, 1.0, 16.0) as u32,
                    pos(ctx, "cell", 0.1),
                ),
                None => {
                    ctx.err(format!("{path}.sprite"), format!("voxels need a `sprite` that exists{}", Ctx::near(name, sprites.iter().map(|s| s.name.clone()))));
                    Vec::new()
                }
            }
        }
    };
    let at = want_v3(ctx, &format!("{path}.at"), o.get("at"), [0.0; 3]);
    let rot = want_v3(ctx, &format!("{path}.rot"), o.get("rot"), [0.0; 3]);
    (ctx.errs.len() == before).then_some((tris, at, rot))
}

/// Parses the root `models` object.
pub(crate) fn parse_models(ctx: &mut Ctx, root: &Map<String, Value>, sprites: &[Sprite]) -> Vec<Model> {
    let mut out: Vec<Model> = Vec::new();
    let Some(mv) = root.get("models") else { return out };
    let Some(o) = ctx.obj("models", mv) else { return out };
    let mut total = 0;
    for (name, def) in o {
        if crate::fields::is_extension_key(name) {
            continue;
        }
        let path = format!("models.{name}");
        let Some(d) = ctx.obj(&path, def) else { continue };
        check_keys(&mut ctx.errs, &path, d, &["parts"]);
        let Some(parts) = d.get("parts").and_then(Value::as_array).filter(|p| !p.is_empty() && p.len() <= 64) else {
            ctx.err(&path, "needs `parts`: a list of 1 to 64 shapes, like [{\"shape\": \"box\", \"size\": [1, 2, 1], \"color\": \"#cc3333\"}]");
            continue;
        };
        let built: Vec<(Vec<Tri>, V3, V3)> = parts.iter().enumerate().filter_map(|(i, p)| parse_part(ctx, &format!("{path}.parts[{i}]"), p, sprites)).collect();
        if built.len() != parts.len() {
            continue;
        }
        let mesh = raster3d::assemble(&built);
        if mesh.tris.len() > MAX_MODEL_TRIS {
            ctx.err(&path, format!("{} triangles: a model may have at most {MAX_MODEL_TRIS} (lower `segments`, or use fewer parts)", mesh.tris.len()));
            continue;
        }
        total += mesh.tris.len();
        if total > MAX_TOTAL_TRIS {
            ctx.err("models", format!("all models together have more than {MAX_TOTAL_TRIS} triangles"));
            break;
        }
        let radius = mesh.radius().max(0.001);
        out.push(Model { name: name.clone(), mesh, radius });
    }
    out
}

fn model_index(ctx: &mut Ctx, path: &str, name: &str, models: &[Model]) -> Option<usize> {
    match models.iter().position(|m| m.name == name) {
        Some(i) => Some(i),
        None => {
            ctx.err(path, format!("no model `{name}`{} — define it in `models`", Ctx::near(name, models.iter().map(|m| m.name.clone()))));
            None
        }
    }
}

/// A prefab `shape` of `{"model": name, ...}`; the second value is the box it is drawn in.
pub(crate) fn parse_model_shape(ctx: &mut Ctx, path: &str, o: &Map<String, Value>, models: &[Model], names: &Names) -> Option<(ModelShape, [f32; 2])> {
    let name = o.get("model")?.as_str().unwrap_or("");
    let model = model_index(ctx, &format!("{path}.model"), name, models)?;
    let before = ctx.errs.len();
    let fit = match o.get("fit") {
        None => [48.0, 48.0],
        Some(v) => match pair(v).filter(|p| p[0] > 0.0 && p[1] > 0.0) {
            Some(p) => p,
            None => {
                ctx.err(format!("{path}.fit"), format!("expected [width, height] in px, both above 0, got {}", describe_value(v)));
                [48.0, 48.0]
            }
        },
    };
    let ms = ModelShape {
        model,
        yaw: want_val(ctx, &format!("{path}.yaw"), o.get("yaw"), names),
        pitch: want_val(ctx, &format!("{path}.pitch"), o.get("pitch"), names),
        roll: want_val(ctx, &format!("{path}.roll"), o.get("roll"), names),
        scale: want_num(ctx, &format!("{path}.scale"), o.get("scale"), 1.0, 0.01, 100.0),
        elevation: want_num(ctx, &format!("{path}.elevation"), o.get("elevation"), 20.0, -80.0, 80.0),
        tint: o.get("tint").map(|_| want_color(ctx, &format!("{path}.tint"), o.get("tint"), [255; 4])),
        light: parse_light(ctx, &format!("{path}.light"), o.get("light")),
    };
    (ctx.errs.len() == before).then_some((ms, fit))
}

const ITEM_KEYS: &[&str] = &["model", "at", "yaw", "pitch", "roll", "scale", "tint"];

fn parse_item(ctx: &mut Ctx, path: &str, v: &Value, models: &[Model], names: &Names) -> Option<ModelItem> {
    let o = ctx.obj(path, v)?;
    check_keys(&mut ctx.errs, path, o, ITEM_KEYS);
    let model = model_index(ctx, &format!("{path}.model"), o.get("model").and_then(Value::as_str).unwrap_or(""), models)?;
    Some(ModelItem {
        model,
        at: want_v3(ctx, &format!("{path}.at"), o.get("at"), [0.0; 3]),
        yaw: want_val(ctx, &format!("{path}.yaw"), o.get("yaw"), names),
        pitch: want_val(ctx, &format!("{path}.pitch"), o.get("pitch"), names),
        roll: want_val(ctx, &format!("{path}.roll"), o.get("roll"), names),
        scale: want_num(ctx, &format!("{path}.scale"), o.get("scale"), 1.0, 0.01, 100.0),
        tint: o.get("tint").map(|_| want_color(ctx, &format!("{path}.tint"), o.get("tint"), [255; 4])),
    })
}

/// A `view3d` object. `screen` is the virtual screen size, the default for `size`.
pub(crate) fn parse_view3d(ctx: &mut Ctx, path: &str, v: &Value, models: &[Model], names: &Names, screen: [f32; 2]) -> Option<View3d> {
    let o = ctx.obj(path, v)?;
    check_keys(&mut ctx.errs, path, o, &["at", "size", "camera", "background", "items", "light"]);
    let before = ctx.errs.len();
    let at = match o.get("at") {
        None => [0.0, 0.0],
        Some(x) => pair(x).unwrap_or_else(|| {
            ctx.err(format!("{path}.at"), format!("expected [x, y] screen pixels, got {}", describe_value(x)));
            [0.0; 2]
        }),
    };
    let size = match o.get("size") {
        None => screen,
        Some(x) => match pair(x).filter(|p| p[0] >= 8.0 && p[1] >= 8.0) {
            Some(p) => p,
            None => {
                ctx.err(format!("{path}.size"), format!("expected [width, height] in screen pixels (each at least 8), got {}", describe_value(x)));
                screen
            }
        },
    };
    let (mut eye, mut target, mut fov, mut ortho) = ([0.0, 2.0, 6.0], [0.0; 3], 45.0, None);
    if let Some(c) = o.get("camera") {
        if let Some(co) = ctx.obj(&format!("{path}.camera"), c) {
            check_keys(&mut ctx.errs, &format!("{path}.camera"), co, &["eye", "target", "fov", "ortho"]);
            eye = want_v3(ctx, &format!("{path}.camera.eye"), co.get("eye"), eye);
            target = want_v3(ctx, &format!("{path}.camera.target"), co.get("target"), target);
            fov = want_num(ctx, &format!("{path}.camera.fov"), co.get("fov"), fov, 10.0, 120.0);
            if co.contains_key("ortho") {
                ortho = Some(want_num(ctx, &format!("{path}.camera.ortho"), co.get("ortho"), 3.0, 0.01, 10000.0));
            }
            if (0..3).all(|i| (eye[i] - target[i]).abs() < 1e-6) {
                ctx.err(format!("{path}.camera"), "`eye` and `target` are the same point: the camera has nothing to look along");
            }
        }
    }
    let mut items = Vec::new();
    match o.get("items") {
        Some(Value::Array(list)) => {
            for (i, it) in list.iter().enumerate() {
                if let Some(m) = parse_item(ctx, &format!("{path}.items[{i}]"), it, models, names) {
                    items.push(m);
                }
            }
        }
        Some(other) => {
            ctx.err(format!("{path}.items"), format!("expected a list of {{\"model\": name, \"at\": [x, y, z], \"yaw\": ...}}, got {}", describe_value(other)))
        }
        None => ctx.err(path, "a view3d needs `items`: the models in the scene"),
    }
    let background = want_color(ctx, &format!("{path}.background"), o.get("background"), [0, 0, 0, 0]);
    let light = parse_light(ctx, &format!("{path}.light"), o.get("light"));
    (ctx.errs.len() == before).then_some(View3d { at, size, eye, target, fov, ortho, background, items, light })
}

/// `view.world3d`.
pub(crate) fn parse_world3d(ctx: &mut Ctx, v: &Value, names: &Names) -> Option<World3d> {
    let path = "view.world3d";
    let o = ctx.obj(path, v)?;
    check_keys(&mut ctx.errs, path, o, &["plane", "distance", "pitch", "yaw", "fov", "ground", "light", "sky"]);
    let before = ctx.errs.len();
    let plane = match o.get("plane").map(|p| p.as_str()) {
        None | Some(Some("ground")) => Plane::Ground,
        Some(Some("wall")) => Plane::Wall,
        Some(_) => {
            ctx.err(
                format!("{path}.plane"),
                "expected \"ground\" (y of the 2D plane becomes depth: top-down and arena games) or \"wall\" (y becomes height: side-scrollers)",
            );
            Plane::Ground
        }
    };
    let ground = match o.get("ground") {
        None => None,
        Some(g) => ctx.obj(&format!("{path}.ground"), g).map(|go| {
            check_keys(&mut ctx.errs, &format!("{path}.ground"), go, &["color", "alt", "tile"]);
            GroundSpec {
                color: want_color(ctx, &format!("{path}.ground.color"), go.get("color"), [40, 52, 80, 255]),
                alt: go.get("alt").map(|_| want_color(ctx, &format!("{path}.ground.alt"), go.get("alt"), [48, 62, 96, 255])),
                tile: want_num(ctx, &format!("{path}.ground.tile"), go.get("tile"), 32.0, 4.0, 512.0),
            }
        }),
    };
    let w = World3d {
        plane,
        distance: want_num(ctx, &format!("{path}.distance"), o.get("distance"), 150.0, 10.0, 5000.0),
        pitch: want_num(ctx, &format!("{path}.pitch"), o.get("pitch"), if plane == Plane::Wall { 8.0 } else { 55.0 }, 0.0, 89.0),
        yaw: want_val(ctx, &format!("{path}.yaw"), o.get("yaw"), names),
        fov: want_num(ctx, &format!("{path}.fov"), o.get("fov"), 50.0, 15.0, 110.0),
        ground,
        light: parse_light(ctx, &format!("{path}.light"), o.get("light")),
        sky: want_color(ctx, &format!("{path}.sky"), o.get("sky"), [20, 26, 44, 255]),
    };
    (ctx.errs.len() == before).then_some(w)
}

#[cfg(test)]
mod tests {
    use crate::game::{parse, Shape};

    fn game(extra: &str, prefab_shape: &str, ui: &str, view_extra: &str) -> Result<crate::game::GameDef, Vec<String>> {
        parse(&format!(
            r##"{{"game2d":1,"id":"t","title":"T","description":"d",
            "capabilities":{{"presentation":"hybrid","platforms":["web"],"networking":"offline","input":["keyboard"],"persistence":[]}},
            "view":{{"width":160,"height":90{view_extra}}},"sprites":{{"s":{{"rows":["aa","ab"],"palette":{{"a":"#f00","b":"#00f"}}}}}},
            "models":{{"boss":{{"parts":[{{"shape":"box","size":[2,2,2],"color":"#a33"}},{{"shape":"sphere","radius":1,"at":[0,1.5,0],"color":"#ccc"}},{{"shape":"voxels","sprite":"s","cell":0.2}}]}}}}{extra},
            "vars":{{"phase":0}},"prefabs":{{"p":{{"tag":"p","shape":{prefab_shape},"move":{{"drift":[10,0]}}}}}},"scene":[{{"prefab":"p","at":[40,40]}}],"ui":[{ui}],"rules":[]}}"##
        ))
    }

    #[test]
    fn a_model_shape_a_viewport_and_a_world_view_parse() {
        let g = game("", r##"{"model":"boss","yaw":"time * 60","fit":[40,40],"light":{"toon":true}}"##, r##"{"view3d":{"at":[4,4],"size":[40,40],"camera":{"eye":[0,2,5],"target":[0,1,0]},"items":[{"model":"boss","yaw":"phase * 10"}]}},{"minimap":{"at":[100,4],"size":[50,30],"colors":{"p":"#fff"}}}"##, "").unwrap();
        assert!(matches!(g.prefabs[0].shape, Shape::Model(_)));
        assert_eq!(g.models.len(), 1);
        assert!(g.models[0].mesh.tris.len() > 20 && g.models[0].radius > 1.0);
        assert!(g.uses_3d());
        let g = game(
            "",
            r##"{"rect":[8,8],"color":"#fff"}"##,
            "",
            r##","world3d":{"pitch":60,"ground":{"color":"#223","alt":"#334","tile":16},"yaw":"time * 5"}"##,
        )
        .unwrap();
        assert!(g.world3d.is_some() && g.uses_3d());
    }

    #[test]
    fn mistakes_in_3d_elements_say_what_is_wrong() {
        let e = |extra: &str, shape: &str, ui: &str, view: &str| game(extra, shape, ui, view).unwrap_err().join("\n");
        assert!(e("", r#"{"model":"bos"}"#, "", "").contains("no model `bos` — did you mean `boss`"));
        assert!(e(r#","x":1"#, r##"{"rect":[8,8],"color":"#fff"}"##, "", "").contains("unknown field"));
        assert!(e("", r#"{"model":"boss","yaw":"timee"}"#, "", "").contains("timee"));
        assert!(e("", r#"{"model":"boss","fit":[0,4]}"#, "", "").contains("both above 0"));
        assert!(e("", r#"{"model":"boss"}"#, r#"{"view3d":{"items":[{"model":"ghost"}]}}"#, "").contains("no model `ghost`"));
        assert!(e("", r#"{"model":"boss"}"#, r#"{"view3d":{"camera":{"eye":[1,1,1],"target":[1,1,1]},"items":[]}}"#, "").contains("same point"));
        assert!(e("", r#"{"model":"boss"}"#, "", r#","world3d":{"plane":"floor"}"#).contains("\"ground\""));
        assert!(e("", r#"{"model":"boss"}"#, "", r#","world3d":{"pitch":120}"#).contains("0 to 89"));
        let m = game("", r#"{"model":"boss"}"#, "", "").unwrap().models.len();
        assert_eq!(m, 1);
    }

    #[test]
    fn models_name_their_mistakes_and_have_limits() {
        let bad = |parts: &str| {
            parse(&format!(
                r##"{{"game2d":1,"id":"t","title":"T","description":"d","capabilities":{{"presentation":"hybrid","platforms":["web"]}},"view":{{"width":160,"height":90}},
                "models":{{"m":{{"parts":{parts}}}}},"prefabs":{{"p":{{"shape":{{"model":"m"}}}}}},"scene":[{{"prefab":"p","at":[1,1]}}]}}"##
            ))
            .unwrap_err()
            .join("\n")
        };
        assert!(bad(r#"[{"shape":"sphre"}]"#).contains("did you mean `sphere`"));
        assert!(bad(r#"[{"shape":"box","size":[1,2]}]"#).contains("[x, y, z]"));
        assert!(bad(r##"[{"shape":"sphere","radius":1,"color":"red"}]"##).contains("color string"));
        assert!(bad("[]").contains("needs `parts`"));
        assert!(bad(r#"[{"shape":"voxels","sprite":"nope"}]"#).contains("voxels need a `sprite` that exists"));
        assert!(bad(r#"[{"shape":"sphere","radius":1,"segments":99}]"#).contains("3 to 48"));
    }
}
