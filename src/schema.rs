use crate::color::parse_hex_to_linear;
use crate::easing::Ease;
use crate::gpu::MAX_LIGHTS;
use crate::props::PropKind;
use crate::track::{Keyframe, Lerp, Track};
use glam::Vec3;
use serde_json::{Map, Value};

// ---------------------------------------------------------------------------------------------
// Compiled scene (what the renderer actually walks)
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct Scene {
    pub fps: u32,
    pub duration: f32,
    pub width: u32,
    pub height: u32,
    pub background: Background,
    pub ambient_color: Vec3,
    pub ambient_intensity: f32,
    pub camera: Camera,
    pub lights: Vec<Light>,
    pub objects: Vec<Object>,
}

#[derive(Debug)]
pub enum Background {
    Flat(Vec3),
    Gradient { top: Vec3, bottom: Vec3 },
}

#[derive(Debug)]
pub struct Camera {
    pub fov: Track<f32>,
    pub near: f32,
    pub far: f32,
    pub position: Track<Vec3>,
    pub target: Track<Vec3>,
    pub roll: Track<f32>,
}

#[derive(Debug)]
pub enum LightKind {
    Directional { direction: Track<Vec3> },
    Point { position: Track<Vec3>, range: f32 },
}

#[derive(Debug)]
pub struct Light {
    pub id: String,
    pub kind: LightKind,
    pub color: Track<Vec3>,
    pub intensity: Track<f32>,
    pub cast_shadows: bool,
    pub shadow_radius: f32,
}

#[derive(Clone, Debug)]
pub struct Material {
    pub color: Track<Vec3>,
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: Vec3,
}

impl Material {
    fn default_gray() -> Self {
        Material {
            color: Track::constant(Vec3::splat(0.7)),
            metallic: 0.0,
            roughness: 0.6,
            emissive: Vec3::ZERO,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PrimKind {
    Box { size: Vec3 },
    Sphere { radius: f32 },
    Cylinder { radius: f32, height: f32 },
    Cone { radius: f32, height: f32 },
    Capsule { radius: f32, height: f32 },
    Plane { size: (f32, f32) },
}

#[derive(Debug)]
pub struct Pose {
    pub spine: Track<Vec3>,
    pub head: Track<Vec3>,
    pub l_shoulder: Track<Vec3>,
    pub r_shoulder: Track<Vec3>,
    pub l_elbow: Track<f32>,
    pub r_elbow: Track<f32>,
    pub l_hip: Track<Vec3>,
    pub r_hip: Track<Vec3>,
    pub l_knee: Track<f32>,
    pub r_knee: Track<f32>,
}

#[derive(Debug)]
pub struct HumanoidDef {
    pub height: f32,
    pub build: f32,
    pub material: Material,
    pub pose: Pose,
}

/// A prop-hunt prop (see `crate::props`): a schema-level object kind that expands into a
/// handful of primitive parts, the same way `Humanoid` expands into a posed capsule rig,
/// rather than something a map author hand-nests as a `group` of boxes every time.
#[derive(Debug)]
pub struct PropDef {
    pub kind: PropKind,
    pub material: Material,
}

#[derive(Debug)]
pub enum ObjectKind {
    Prim(PrimKind),
    Group(Vec<Object>),
    Humanoid(Box<HumanoidDef>),
    Prop(Box<PropDef>),
}

#[derive(Debug)]
pub struct Object {
    pub id: String,
    pub position: Track<Vec3>,
    pub rotation: Track<Vec3>,
    pub scale: Track<Vec3>,
    pub material: Option<Material>,
    pub kind: ObjectKind,
}

// ---------------------------------------------------------------------------------------------
// Parsing (JSON -> Scene) with precise "path: message" error collection
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct Ctx {
    errors: Vec<String>,
}

impl Ctx {
    fn err(&mut self, path: &str, msg: impl std::fmt::Display) {
        self.errors.push(format!("{path}: {msg}"));
    }
}

fn as_f32(v: &Value) -> Option<f32> {
    v.as_f64().map(|x| x as f32)
}

fn as_vec3(v: &Value) -> Result<Vec3, String> {
    let arr = v.as_array().ok_or("must be a [x,y,z] array")?;
    if arr.len() != 3 {
        return Err("must have exactly 3 numbers".to_string());
    }
    let x = as_f32(&arr[0]).ok_or("component must be a number")?;
    let y = as_f32(&arr[1]).ok_or("component must be a number")?;
    let z = as_f32(&arr[2]).ok_or("component must be a number")?;
    Ok(Vec3::new(x, y, z))
}

fn as_scale_vec3(v: &Value) -> Result<Vec3, String> {
    if let Some(n) = as_f32(v) {
        return Ok(Vec3::splat(n));
    }
    as_vec3(v)
}

fn as_color_vec3(v: &Value) -> Result<Vec3, String> {
    let s = v.as_str().ok_or("must be a '#rrggbb' hex string")?;
    parse_hex_to_linear(s)
}

/// Shared engine for every track field: accepts either a bare leaf value or
/// `{"keyframes": [{"t":..,"value":..,"ease":..}, ...]}`.
fn build_track<T: Lerp + Copy>(
    ctx: &mut Ctx,
    raw: &Value,
    path: &str,
    default: T,
    parse_leaf: impl Fn(&Value) -> Result<T, String>,
) -> Track<T> {
    if let Some(obj) = raw.as_object() {
        if let Some(kfs_raw) = obj.get("keyframes") {
            let Some(arr) = kfs_raw.as_array() else {
                ctx.err(&format!("{path}.keyframes"), "must be an array");
                return Track::constant(default);
            };
            if arr.is_empty() {
                ctx.err(&format!("{path}.keyframes"), "must not be empty");
                return Track::constant(default);
            }
            let mut kfs = Vec::new();
            let mut last_t: Option<f32> = None;
            for (i, item) in arr.iter().enumerate() {
                let kpath = format!("{path}.keyframes[{i}]");
                let Some(kobj) = item.as_object() else {
                    ctx.err(&kpath, "must be an object with 't' and 'value'");
                    continue;
                };
                let t = match kobj.get("t").and_then(as_f32) {
                    Some(t) => t,
                    None => {
                        ctx.err(&format!("{kpath}.t"), "missing or not a number");
                        continue;
                    }
                };
                if let Some(prev) = last_t {
                    if t < prev {
                        ctx.err(&format!("{kpath}.t"), "keyframe t values must be sorted ascending");
                    }
                }
                last_t = Some(t);
                let value = match kobj.get("value") {
                    None => {
                        ctx.err(&format!("{kpath}.value"), "missing");
                        continue;
                    }
                    Some(vraw) => match parse_leaf(vraw) {
                        Ok(v) => v,
                        Err(e) => {
                            ctx.err(&format!("{kpath}.value"), e);
                            continue;
                        }
                    },
                };
                let ease = match kobj.get("ease") {
                    None => Ease::Linear,
                    Some(Value::String(s)) => match Ease::parse(s) {
                        Ok(e) => e,
                        Err(msg) => {
                            ctx.err(&format!("{kpath}.ease"), msg);
                            Ease::Linear
                        }
                    },
                    Some(_) => {
                        ctx.err(&format!("{kpath}.ease"), "must be a string");
                        Ease::Linear
                    }
                };
                kfs.push(Keyframe { t, value, ease });
            }
            return if kfs.is_empty() { Track::constant(default) } else { Track::Keyframed(kfs) };
        }
    }
    match parse_leaf(raw) {
        Ok(v) => Track::constant(v),
        Err(e) => {
            ctx.err(path, e);
            Track::constant(default)
        }
    }
}

fn float_field(ctx: &mut Ctx, obj: &Map<String, Value>, key: &str, path: &str, default: f32) -> Track<f32> {
    match obj.get(key) {
        None => Track::constant(default),
        Some(v) => build_track(ctx, v, &format!("{path}.{key}"), default, |leaf| {
            as_f32(leaf).ok_or_else(|| "must be a number".to_string())
        }),
    }
}

fn vec3_field(ctx: &mut Ctx, obj: &Map<String, Value>, key: &str, path: &str, default: Vec3) -> Track<Vec3> {
    match obj.get(key) {
        None => Track::constant(default),
        Some(v) => build_track(ctx, v, &format!("{path}.{key}"), default, as_vec3),
    }
}

fn scale_field(ctx: &mut Ctx, obj: &Map<String, Value>, key: &str, path: &str, default: Vec3) -> Track<Vec3> {
    match obj.get(key) {
        None => Track::constant(default),
        Some(v) => build_track(ctx, v, &format!("{path}.{key}"), default, as_scale_vec3),
    }
}

fn color_field(ctx: &mut Ctx, obj: &Map<String, Value>, key: &str, path: &str, default: Vec3) -> Track<Vec3> {
    match obj.get(key) {
        None => Track::constant(default),
        Some(v) => build_track(ctx, v, &format!("{path}.{key}"), default, as_color_vec3),
    }
}

fn plain_f32(obj: &Map<String, Value>, key: &str, default: f32) -> f32 {
    obj.get(key).and_then(as_f32).unwrap_or(default)
}

fn plain_hex(ctx: &mut Ctx, obj: &Map<String, Value>, key: &str, path: &str, default: Vec3) -> Vec3 {
    match obj.get(key) {
        None => default,
        Some(v) => match as_color_vec3(v) {
            Ok(c) => c,
            Err(e) => {
                ctx.err(&format!("{path}.{key}"), e);
                default
            }
        },
    }
}

fn parse_material(ctx: &mut Ctx, obj: &Map<String, Value>, path: &str) -> Material {
    match obj.get("material").and_then(Value::as_object) {
        None => Material::default_gray(),
        Some(m) => {
            let mpath = format!("{path}.material");
            Material {
                color: color_field(ctx, m, "color", &mpath, Vec3::splat(0.7)),
                metallic: plain_f32(m, "metallic", 0.0).clamp(0.0, 1.0),
                roughness: plain_f32(m, "roughness", 0.6).clamp(0.04, 1.0),
                emissive: plain_hex(ctx, m, "emissive", &mpath, Vec3::ZERO),
            }
        }
    }
}

fn default_camera() -> Camera {
    Camera {
        fov: Track::constant(50.0),
        near: 0.1,
        far: 200.0,
        position: Track::constant(Vec3::new(0.0, 2.0, 8.0)),
        target: Track::constant(Vec3::new(0.0, 1.0, 0.0)),
        roll: Track::constant(0.0),
    }
}

fn parse_camera(ctx: &mut Ctx, obj: &Map<String, Value>) -> Camera {
    Camera {
        fov: float_field(ctx, obj, "fov", "camera", 50.0),
        near: plain_f32(obj, "near", 0.1).max(0.001),
        far: plain_f32(obj, "far", 200.0),
        position: vec3_field(ctx, obj, "position", "camera", Vec3::new(0.0, 2.0, 8.0)),
        target: vec3_field(ctx, obj, "target", "camera", Vec3::new(0.0, 1.0, 0.0)),
        roll: float_field(ctx, obj, "roll", "camera", 0.0),
    }
}

fn parse_light(ctx: &mut Ctx, obj: &Map<String, Value>, path: &str) -> Light {
    let id = obj.get("id").and_then(Value::as_str).unwrap_or("light").to_string();
    let cast_shadows = obj.get("cast_shadows").and_then(Value::as_bool).unwrap_or(false);
    let shadow_radius = plain_f32(obj, "shadow_radius", 15.0);
    let color = color_field(ctx, obj, "color", path, Vec3::ONE);
    let kind_name = obj.get("type").and_then(Value::as_str);
    let kind = match kind_name {
        Some("directional") => LightKind::Directional {
            direction: vec3_field(ctx, obj, "direction", path, Vec3::new(-0.4, -1.0, -0.3)),
        },
        Some("point") => LightKind::Point {
            position: vec3_field(ctx, obj, "position", path, Vec3::new(0.0, 3.0, 0.0)),
            range: plain_f32(obj, "range", 20.0).max(0.01),
        },
        Some(other) => {
            ctx.err(&format!("{path}.type"), format!("unknown light type '{other}' (expected 'directional' or 'point')"));
            LightKind::Directional { direction: Track::constant(Vec3::new(-0.4, -1.0, -0.3)) }
        }
        None => {
            ctx.err(&format!("{path}.type"), "missing (expected 'directional' or 'point')");
            LightKind::Directional { direction: Track::constant(Vec3::new(-0.4, -1.0, -0.3)) }
        }
    };
    if cast_shadows && !matches!(kind, LightKind::Directional { .. }) {
        ctx.err(&format!("{path}.cast_shadows"), "only a 'directional' light may cast shadows");
    }
    let intensity_default = match kind {
        LightKind::Directional { .. } => 2.0,
        LightKind::Point { .. } => 12.0,
    };
    Light {
        id,
        kind,
        color,
        intensity: float_field(ctx, obj, "intensity", path, intensity_default),
        cast_shadows,
        shadow_radius,
    }
}

fn parse_prim(ctx: &mut Ctx, ty: &str, obj: &Map<String, Value>, path: &str) -> PrimKind {
    match ty {
        "box" => {
            let size = match obj.get("size") {
                Some(v) => as_vec3(v).unwrap_or_else(|e| {
                    ctx.err(&format!("{path}.size"), e);
                    Vec3::ONE
                }),
                None => Vec3::ONE,
            };
            PrimKind::Box { size }
        }
        "sphere" => PrimKind::Sphere { radius: plain_f32(obj, "radius", 0.5) },
        "cylinder" => PrimKind::Cylinder { radius: plain_f32(obj, "radius", 0.5), height: plain_f32(obj, "height", 1.0) },
        "cone" => PrimKind::Cone { radius: plain_f32(obj, "radius", 0.5), height: plain_f32(obj, "height", 1.0) },
        "capsule" => PrimKind::Capsule { radius: plain_f32(obj, "radius", 0.3), height: plain_f32(obj, "height", 1.0) },
        "plane" => {
            let (w, d) = match obj.get("size").and_then(Value::as_array) {
                Some(a) if a.len() == 2 => (as_f32(&a[0]).unwrap_or(10.0), as_f32(&a[1]).unwrap_or(10.0)),
                _ => (10.0, 10.0),
            };
            PrimKind::Plane { size: (w, d) }
        }
        _ => unreachable!("caller already validated type"),
    }
}

fn parse_pose_track_vec3(ctx: &mut Ctx, obj: &Map<String, Value>, key: &str, path: &str) -> Track<Vec3> {
    vec3_field(ctx, obj, key, path, Vec3::ZERO)
}

fn parse_pose_track_f32(ctx: &mut Ctx, obj: &Map<String, Value>, key: &str, path: &str) -> Track<f32> {
    float_field(ctx, obj, key, path, 0.0)
}

fn parse_humanoid(ctx: &mut Ctx, obj: &Map<String, Value>, path: &str) -> HumanoidDef {
    let height = plain_f32(obj, "height", 1.8).max(0.1);
    let build = plain_f32(obj, "build", 1.0).max(0.05);
    let material = parse_material(ctx, obj, path);
    let pose_obj = obj.get("pose").and_then(Value::as_object).cloned().unwrap_or_default();
    let ppath = format!("{path}.pose");
    let pose = Pose {
        spine: parse_pose_track_vec3(ctx, &pose_obj, "spine", &ppath),
        head: parse_pose_track_vec3(ctx, &pose_obj, "head", &ppath),
        l_shoulder: parse_pose_track_vec3(ctx, &pose_obj, "l_shoulder", &ppath),
        r_shoulder: parse_pose_track_vec3(ctx, &pose_obj, "r_shoulder", &ppath),
        l_elbow: parse_pose_track_f32(ctx, &pose_obj, "l_elbow", &ppath),
        r_elbow: parse_pose_track_f32(ctx, &pose_obj, "r_elbow", &ppath),
        l_hip: parse_pose_track_vec3(ctx, &pose_obj, "l_hip", &ppath),
        r_hip: parse_pose_track_vec3(ctx, &pose_obj, "r_hip", &ppath),
        l_knee: parse_pose_track_f32(ctx, &pose_obj, "l_knee", &ppath),
        r_knee: parse_pose_track_f32(ctx, &pose_obj, "r_knee", &ppath),
    };
    HumanoidDef { height, build, material, pose }
}

fn parse_prop(ctx: &mut Ctx, obj: &Map<String, Value>, path: &str) -> PropDef {
    let kind = match obj.get("prop").and_then(Value::as_str) {
        Some(name) => match PropKind::from_name(name) {
            Some(k) => k,
            None => {
                let names: Vec<&str> = PropKind::ALL.iter().map(|k| k.name()).collect();
                ctx.err(&format!("{path}.prop"), format!("unknown prop '{name}' (expected one of: {})", names.join(", ")));
                PropKind::Crate
            }
        },
        None => {
            ctx.err(&format!("{path}.prop"), "missing (a prop object needs a 'prop' kind string)");
            PropKind::Crate
        }
    };
    PropDef { kind, material: parse_material(ctx, obj, path) }
}

const PRIM_TYPES: &[&str] = &["box", "sphere", "cylinder", "cone", "capsule", "plane"];

fn parse_object(ctx: &mut Ctx, raw: &Value, path: &str) -> Object {
    let Some(obj) = raw.as_object() else {
        ctx.err(path, "must be an object");
        return Object {
            id: path.to_string(),
            position: Track::constant(Vec3::ZERO),
            rotation: Track::constant(Vec3::ZERO),
            scale: Track::constant(Vec3::ONE),
            material: Some(Material::default_gray()),
            kind: ObjectKind::Prim(PrimKind::Sphere { radius: 0.5 }),
        };
    };
    let id = match obj.get("id").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => {
            ctx.err(&format!("{path}.id"), "missing (every object needs a unique 'id' string)");
            path.to_string()
        }
    };
    let ty = obj.get("type").and_then(Value::as_str);
    let position = vec3_field(ctx, obj, "position", &id, Vec3::ZERO);
    let rotation = vec3_field(ctx, obj, "rotation", &id, Vec3::ZERO);
    let scale = scale_field(ctx, obj, "scale", &id, Vec3::ONE);

    let (kind, material) = match ty {
        Some(t) if PRIM_TYPES.contains(&t) => {
            (ObjectKind::Prim(parse_prim(ctx, t, obj, &id)), Some(parse_material(ctx, obj, &id)))
        }
        Some("group") => {
            let children = match obj.get("children").and_then(Value::as_array) {
                Some(arr) => arr
                    .iter()
                    .enumerate()
                    .map(|(i, c)| parse_object(ctx, c, &format!("{id}.children[{i}]")))
                    .collect(),
                None => {
                    ctx.err(&format!("{id}.children"), "missing (a group needs a 'children' array)");
                    Vec::new()
                }
            };
            (ObjectKind::Group(children), None)
        }
        Some("humanoid") => (ObjectKind::Humanoid(Box::new(parse_humanoid(ctx, obj, &id))), None),
        Some("prop") => (ObjectKind::Prop(Box::new(parse_prop(ctx, obj, &id))), None),
        Some(other) => {
            ctx.err(
                &format!("{id}.type"),
                format!(
                    "unknown type '{other}' (expected box, sphere, cylinder, cone, capsule, plane, group, humanoid, or prop)"
                ),
            );
            (ObjectKind::Prim(PrimKind::Sphere { radius: 0.5 }), Some(Material::default_gray()))
        }
        None => {
            ctx.err(&format!("{id}.type"), "missing");
            (ObjectKind::Prim(PrimKind::Sphere { radius: 0.5 }), Some(Material::default_gray()))
        }
    };

    Object { id, position, rotation, scale, material, kind }
}

pub fn parse_scene(text: &str) -> Result<Scene, Vec<String>> {
    let value: Value = serde_json::from_str(text).map_err(|e| vec![format!("json: {e}")])?;
    let mut ctx = Ctx::default();
    let Some(root) = value.as_object() else {
        return Err(vec!["root: scene must be a JSON object".to_string()]);
    };

    let meta = root.get("meta").and_then(Value::as_object);
    let fps = meta.and_then(|m| m.get("fps")).and_then(Value::as_u64).unwrap_or(30).max(1) as u32;
    let duration = meta.and_then(|m| m.get("duration")).and_then(as_f32).unwrap_or(4.0);
    if duration <= 0.0 {
        ctx.err("meta.duration", "must be > 0");
    }
    let (width, height) = meta
        .and_then(|m| m.get("resolution"))
        .and_then(Value::as_array)
        .filter(|a| a.len() == 2)
        .and_then(|a| Some((a[0].as_u64()? as u32, a[1].as_u64()? as u32)))
        .unwrap_or((1280, 720));

    let default_sky_top = Vec3::new(0.42, 0.62, 0.88);
    let default_sky_bottom = Vec3::new(0.90, 0.94, 0.99);
    let background = match root.get("background").and_then(Value::as_object) {
        None => Background::Gradient { top: default_sky_top, bottom: default_sky_bottom },
        Some(bg) => {
            if bg.contains_key("color") {
                Background::Flat(plain_hex(&mut ctx, bg, "color", "background", Vec3::splat(0.05)))
            } else {
                Background::Gradient {
                    top: plain_hex(&mut ctx, bg, "sky_top", "background", default_sky_top),
                    bottom: plain_hex(&mut ctx, bg, "sky_bottom", "background", default_sky_bottom),
                }
            }
        }
    };

    let ambient = root.get("ambient").and_then(Value::as_object);
    let ambient_color = match ambient {
        Some(a) => plain_hex(&mut ctx, a, "color", "ambient", Vec3::ONE),
        None => Vec3::ONE,
    };
    let ambient_intensity = ambient.map(|a| plain_f32(a, "intensity", 0.25)).unwrap_or(0.25);

    let camera = match root.get("camera").and_then(Value::as_object) {
        None => {
            ctx.err("camera", "missing");
            default_camera()
        }
        Some(c) => parse_camera(&mut ctx, c),
    };

    let mut lights = Vec::new();
    if let Some(arr) = root.get("lights").and_then(Value::as_array) {
        if arr.len() > MAX_LIGHTS {
            ctx.err("lights", format!("at most {MAX_LIGHTS} lights are allowed"));
        }
        let mut shadow_casters = 0;
        for (i, lv) in arr.iter().enumerate() {
            let path = format!("lights[{i}]");
            match lv.as_object() {
                Some(lobj) => {
                    let light = parse_light(&mut ctx, lobj, &path);
                    if light.cast_shadows {
                        shadow_casters += 1;
                    }
                    lights.push(light);
                }
                None => ctx.err(&path, "must be an object"),
            }
        }
        if shadow_casters > 1 {
            ctx.err("lights", "at most one light may set cast_shadows: true");
        }
    }

    let mut objects = Vec::new();
    match root.get("objects").and_then(Value::as_array) {
        Some(arr) => {
            for (i, ov) in arr.iter().enumerate() {
                objects.push(parse_object(&mut ctx, ov, &format!("objects[{i}]")));
            }
        }
        None => ctx.err("objects", "missing (must be an array, may be empty)"),
    }

    if !ctx.errors.is_empty() {
        return Err(ctx.errors);
    }

    Ok(Scene {
        fps,
        duration,
        width: width.max(2) + width % 2,
        height: height.max(2) + height % 2,
        background,
        ambient_color,
        ambient_intensity,
        camera,
        lights,
        objects,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_scene_parses() {
        let json = r#"{"camera":{"position":[0,2,8],"target":[0,0,0]},"objects":[]}"#;
        let scene = parse_scene(json).expect("should parse");
        assert_eq!(scene.fps, 30);
        assert_eq!(scene.objects.len(), 0);
    }

    #[test]
    fn missing_object_type_is_reported_with_precise_path() {
        let json = r#"{"camera":{},"objects":[{"id":"thing"}]}"#;
        let errs = parse_scene(json).unwrap_err();
        assert!(errs.iter().any(|e| e == "thing.type: missing"), "{errs:?}");
    }

    #[test]
    fn unsorted_keyframes_are_rejected() {
        let json = r#"{"camera":{},"objects":[
            {"id":"a","type":"sphere","position":{"keyframes":[{"t":1,"value":[0,0,0]},{"t":0,"value":[1,1,1]}]}}
        ]}"#;
        let errs = parse_scene(json).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("sorted ascending")), "{errs:?}");
    }

    #[test]
    fn multiple_shadow_casters_rejected() {
        let json = r#"{"camera":{},"objects":[],"lights":[
            {"id":"a","type":"directional","cast_shadows":true},
            {"id":"b","type":"directional","cast_shadows":true}
        ]}"#;
        let errs = parse_scene(json).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("at most one light")), "{errs:?}");
    }

    #[test]
    fn group_and_humanoid_parse() {
        let json = r##"{"camera":{},"objects":[
            {"id":"g","type":"group","children":[
                {"id":"g.post","type":"cylinder","material":{"color":"#8a6240"}}
            ]},
            {"id":"h","type":"humanoid","pose":{"l_elbow":15}}
        ]}"##;
        let scene = parse_scene(json).expect("should parse");
        assert_eq!(scene.objects.len(), 2);
        assert!(matches!(scene.objects[0].kind, ObjectKind::Group(_)));
        assert!(matches!(scene.objects[1].kind, ObjectKind::Humanoid(_)));
    }
}
