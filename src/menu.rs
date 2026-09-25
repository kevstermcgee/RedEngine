//! The launch screen: "Human or Cheddar the rat?".
//!
//! Two pieces, both free of window/GPU types so they are unit-testable:
//! * [`menu_scene`] / [`animate`] — a tiny 3-D backdrop (a dark studio with a pedestal under each
//!   character, both models slowly turning) drawn by the ordinary live renderer, and
//! * [`paint`] — the 2-D text and panels, painted on the CPU into an RGBA image the size of the
//!   window with the engine's own 5x7 bitmap font and shown through `crate::overlay`.
//!
//! [`character_at`] maps a cursor position to a character so a click picks one.

use crate::characters::{human_object, rat_object};
use crate::player::Character;
use crate::schema::{Background, Camera, Light, LightKind, Material, Object, ObjectKind, PostSettings, PrimKind, Scene};
use crate::track::Track;
use crate::viewer::FpsCamera;
use glam::Vec3;

/// The rat is drawn this many times life size so its face reads next to a 1.8 m human.
pub const RAT_PREVIEW_SCALE: f32 = 3.6;
/// Height of the podium the enlarged rat stands on, so it sits at a person's chest height.
const RAT_PODIUM: f32 = 0.55;

const HUMAN_INDEX: usize = 2;
const RAT_INDEX: usize = 4;
const CAMERA_DISTANCE: f32 = 5.0;
const CAMERA_FOV_DEG: f32 = 46.0;

fn hex(s: &str) -> Vec3 {
    crate::color::parse_hex_to_linear(s).expect("valid menu colour")
}

fn constant_material(color: &str, roughness: f32) -> Material {
    Material { color: Track::constant(hex(color)), metallic: 0.0, roughness, emissive: Vec3::ZERO }
}

fn pedestal(id: &str, radius: f32, height: f32) -> Object {
    Object {
        id: id.to_string(),
        position: Track::constant(Vec3::new(0.0, height * 0.5, 0.0)),
        rotation: Track::constant(Vec3::ZERO),
        scale: Track::constant(Vec3::ONE),
        material: Some(constant_material("#2c2f3a", 0.5)),
        collide: false,
        prefab: None,
        movable: None,
        kind: ObjectKind::Prim(PrimKind::Cylinder { radius, height }),
    }
}

fn point_light(id: &str, pos: Vec3, color: &str, intensity: f32, range: f32) -> Light {
    Light {
        id: id.to_string(),
        kind: LightKind::Point { position: Track::constant(pos), range },
        color: Track::constant(hex(color)),
        intensity: Track::constant(intensity),
        cast_shadows: false,
        shadow_radius: 10.0,
        shadow_center: Vec3::ZERO,
    }
}

/// The backdrop scene: a dark gradient studio, a pedestal and a model for each character.
/// Objects: 0 floor, 1/2 human pedestal + human, 3/4 rat pedestal + rat. [`animate`] places them.
pub fn menu_scene() -> Scene {
    let floor = Object {
        id: "floor".to_string(),
        position: Track::constant(Vec3::ZERO),
        rotation: Track::constant(Vec3::ZERO),
        scale: Track::constant(Vec3::ONE),
        material: Some(constant_material("#1a1c24", 0.8)),
        collide: false,
        prefab: None,
        movable: None,
        kind: ObjectKind::Prim(PrimKind::Plane { size: (40.0, 40.0) }),
    };
    let objects = vec![floor, pedestal("human_pedestal", 1.05, 0.06), human_object("human"), pedestal("rat_pedestal", 1.45, RAT_PODIUM), rat_object("rat")];
    debug_assert!(matches!(objects[HUMAN_INDEX].kind, ObjectKind::Humanoid(_)) && matches!(objects[RAT_INDEX].kind, ObjectKind::Rat(_)));
    Scene {
        fps: 30,
        duration: 0.0,
        width: 1280,
        height: 720,
        background: Background::Gradient { top: hex("#0b0d14"), bottom: hex("#23283a") },
        ambient_color: hex("#9aa4c4"),
        ambient_intensity: 0.22,
        camera: Camera {
            fov: Track::constant(CAMERA_FOV_DEG),
            near: 0.05,
            far: 100.0,
            position: Track::constant(Vec3::new(0.0, 1.1, CAMERA_DISTANCE)),
            target: Track::constant(Vec3::new(0.0, 0.95, 0.0)),
            roll: Track::constant(0.0),
        },
        post: PostSettings::default(),
        rules: Default::default(),
        weapons: Default::default(),
        lights: vec![
            point_light("key", Vec3::new(-2.5, 3.2, 3.0), "#ffe6c4", 26.0, 14.0),
            point_light("rim", Vec3::new(2.8, 2.6, -1.5), "#9cc0ff", 20.0, 12.0),
            point_light("fill", Vec3::new(0.0, 1.2, 4.5), "#ffffff", 9.0, 12.0),
        ],
        objects,
    }
}

/// The camera that frames [`menu_scene`].
pub fn menu_camera() -> FpsCamera {
    let mut cam = FpsCamera::new(Vec3::new(0.0, 1.1, CAMERA_DISTANCE), 0.0);
    cam.fov_deg = CAMERA_FOV_DEG;
    cam.pitch = -0.08;
    cam
}

/// Where a character's model stands for a window of `aspect` (so it sits over its own half of the
/// screen whatever the window shape): x of the character's centre, in world units.
fn slot_x(aspect: f32, which: Character) -> f32 {
    let half_width = CAMERA_DISTANCE * (CAMERA_FOV_DEG.to_radians() * 0.5).tan() * aspect;
    let x = half_width * 0.5;
    match which {
        Character::Human => -x,
        Character::Rat => x,
    }
}

/// Places and turns the two models: each slowly sways, the `selected` one a little wider and
/// faster, and the rat trots on the spot while it is the selected one.
pub fn animate(scene: &mut Scene, aspect: f32, time: f32, selected: Character) {
    let place = |scene: &mut Scene, ped: usize, model: usize, which: Character, scale: f32, y: f32| {
        let x = slot_x(aspect, which);
        let picked = which == selected;
        let sway = (time * if picked { 0.9 } else { 0.5 }).sin() * if picked { 38.0 } else { 16.0 };
        scene.objects[ped].position = Track::constant(Vec3::new(x, y * 0.5, 0.0));
        let m = &mut scene.objects[model];
        m.position = Track::constant(Vec3::new(x, y, 0.0));
        m.rotation = Track::constant(Vec3::new(0.0, sway, 0.0));
        m.scale = Track::constant(Vec3::splat(scale));
    };
    place(scene, 1, HUMAN_INDEX, Character::Human, 1.0, 0.06);
    place(scene, 3, RAT_INDEX, Character::Rat, RAT_PREVIEW_SCALE, RAT_PODIUM);
    if let ObjectKind::Rat(r) = &mut scene.objects[RAT_INDEX].kind {
        let trot = selected == Character::Rat;
        r.gait = Track::constant(if trot { time * 14.0 } else { 0.0 });
        r.stride = Track::constant(if trot { 0.55 } else { 0.0 });
        r.sway = Track::constant(time * 1.3);
    }
}

// ---------------------------------------------------------------------------------------------
// 2-D screens: the text, panels and pause menu live in `crate::ui::screens` (headless, audited at many window
// sizes by `red_engine2 ui-check`); they are re-exported here so callers keep using `menu::paint` & co.
// ---------------------------------------------------------------------------------------------

pub use crate::ui::screens::{character_at, menu_layout, paint, paint_pause, pause_action_at, PauseAction};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_has_the_two_models_where_animate_expects() {
        let mut scene = menu_scene();
        animate(&mut scene, 16.0 / 9.0, 1.0, Character::Rat);
        assert!(matches!(scene.objects[HUMAN_INDEX].kind, ObjectKind::Humanoid(_)));
        assert!(matches!(scene.objects[RAT_INDEX].kind, ObjectKind::Rat(_)));
        // The two models stand on opposite sides of the screen centre.
        let (hx, rx) = (scene.objects[HUMAN_INDEX].position.sample(0.0).x, scene.objects[RAT_INDEX].position.sample(0.0).x);
        assert!(hx < -0.5 && rx > 0.5, "{hx} {rx}");
    }
}
