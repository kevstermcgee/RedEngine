//! The backdrop behind the engine's own 2-D screens (the connect form): a dark gradient studio, drawn by the ordinary live renderer, with the
//! text and panels painted over it through `crate::overlay`. The pause menu's layout lives in `crate::ui::screens` and is re-exported here.
//! There is no character selection screen: a player is whoever `--as`, `player.humans_play_as` or the default (Human) says.

use crate::schema::{Background, Camera, Light, LightKind, Material, Object, ObjectKind, PostSettings, PrimKind, Scene};
use crate::track::Track;
use crate::viewer::FpsCamera;
use glam::Vec3;

const CAMERA_DISTANCE: f32 = 5.0;
const CAMERA_FOV_DEG: f32 = 46.0;

fn hex(s: &str) -> Vec3 {
    crate::color::parse_hex_to_linear(s).expect("valid menu colour")
}

fn constant_material(color: &str, roughness: f32) -> Material {
    Material { color: Track::constant(hex(color)), metallic: 0.0, roughness, emissive: Vec3::ZERO, opacity: 1.0 }
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
        shadow_follow: false,
    }
}

/// The backdrop scene: a dark gradient studio (a floor and three lights).
pub fn backdrop_scene() -> Scene {
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
    let objects = vec![floor];
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
        player: Default::default(),
        jump_pads: Vec::new(),
        post: PostSettings::default(),
        rules: Default::default(),
        weapons: Default::default(),
        combat: Default::default(),
        bots: Default::default(),
        nav: None,
        hud: Default::default(),
        music: false,
        flashlight: false,
        death_text: None,
        teams: false,
        sky: None,
        ocean: None,
        shooter: None,
        race: None,
        lights: vec![
            point_light("key", Vec3::new(-2.5, 3.2, 3.0), "#ffe6c4", 26.0, 14.0),
            point_light("rim", Vec3::new(2.8, 2.6, -1.5), "#9cc0ff", 20.0, 12.0),
            point_light("fill", Vec3::new(0.0, 1.2, 4.5), "#ffffff", 9.0, 12.0),
        ],
        objects,
    }
}

/// The camera that frames [`backdrop_scene`].
pub fn backdrop_camera() -> FpsCamera {
    let mut cam = FpsCamera::new(Vec3::new(0.0, 1.1, CAMERA_DISTANCE), 0.0);
    cam.fov_deg = CAMERA_FOV_DEG;
    cam.pitch = -0.08;
    cam
}

// ---------------------------------------------------------------------------------------------
// 2-D screens: the text, panels and pause menu live in `crate::ui::screens` (headless, audited at many window
// sizes by `red_engine2 ui-check`); they are re-exported here so callers keep using `menu::paint_pause` & co.
// ---------------------------------------------------------------------------------------------

pub use crate::ui::screens::{paint_pause, pause_action_at, PauseAction};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backdrop_is_a_floor_and_lights_only() {
        let scene = backdrop_scene();
        assert_eq!(scene.objects.len(), 1, "no models: nothing to choose between");
        assert_eq!(scene.lights.len(), 3);
    }
}
