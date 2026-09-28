//! Screenshots from the client (`--shot-at 5,10 --shot-dir out/`, the F12 key, a script's `shot` steps): the frame is rendered a second time into an offscreen
//! target and read back, so it needs no visible window and no focus, and two runs at once cannot photograph each other (ADR 2026-09-28-seeing-what-the-player-sees).

use super::frame::hidden_ids;
use super::*;
use red_engine2::capture::{flat_fraction, save_png, Capture};
use red_engine2::playscript::CameraSpec;

/// One picture taken.
#[derive(Debug, Clone)]
pub(crate) struct ShotRecord {
    pub name: String,
    pub file: PathBuf,
    pub secs: f32,
    pub camera: String,
    /// How much of the frame is one flat colour (1.0 = the renderer drew nothing).
    pub flat: f32,
}

/// What was asked for and what was taken.
#[derive(Default)]
pub(crate) struct Shots {
    dir: PathBuf,
    schedule: Vec<f32>,
    next: usize,
    queue: Vec<(String, CameraSpec)>,
    capture: Option<Capture>,
    pub taken: Vec<ShotRecord>,
    pub errors: Vec<String>,
}

impl Shots {
    /// Pictures at the given seconds of game time, written under `dir`.
    pub(crate) fn new(dir: PathBuf, mut schedule: Vec<f32>) -> Shots {
        schedule.sort_by(f32::total_cmp);
        Shots { dir, schedule, ..Shots::default() }
    }

    /// Asks for a picture on the next frame.
    pub(crate) fn request(&mut self, name: &str, camera: CameraSpec) {
        self.queue.push((name.to_string(), camera));
    }

    /// The pictures due at game time `secs`: scheduled ones whose time has come, then the ones asked for.
    fn due(&mut self, secs: f32) -> Vec<(String, CameraSpec)> {
        let mut due = Vec::new();
        while self.schedule.get(self.next).is_some_and(|t| *t <= secs) {
            due.push((format!("t{:.0}", self.schedule[self.next]), CameraSpec::First));
            self.next += 1;
        }
        due.append(&mut self.queue);
        due
    }

    /// Whether pictures are still waiting to be taken.
    pub(crate) fn pending(&self) -> bool {
        self.next < self.schedule.len() || !self.queue.is_empty()
    }

    fn path_for(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{:03}-{name}.png", self.taken.len() + 1))
    }
}

fn describe(camera: &CameraSpec) -> String {
    match camera {
        CameraSpec::First => "first".into(),
        CameraSpec::Third => "third".into(),
        CameraSpec::Overview => "overview".into(),
        CameraSpec::Follow(None) => "follow".into(),
        CameraSpec::Follow(Some(id)) => format!("follow:{id}"),
        CameraSpec::Free { .. } => "free".into(),
    }
}

/// A camera at `eye` looking at `at`.
fn look_from(eye: Vec3, at: Vec3, fov_deg: f32, template: &FpsCamera) -> FpsCamera {
    let d = at - eye;
    FpsCamera {
        position: eye,
        yaw: d.x.atan2(-d.z),
        pitch: d.y.atan2(Vec2::new(d.x, d.z).length()).clamp(-FpsCamera::PITCH_LIMIT, FpsCamera::PITCH_LIMIT),
        fov_deg,
        near: template.near,
        far: template.far,
    }
}

impl App {
    /// The camera a picture with `spec` is taken from, or why there is none.
    fn shot_camera(&self, spec: &CameraSpec) -> Result<FpsCamera, String> {
        let mine = &self.camera;
        let fov = self.scene.player.fov_deg;
        Ok(match spec {
            CameraSpec::First => FpsCamera { position: mine.position, yaw: mine.yaw, pitch: mine.pitch, fov_deg: mine.fov_deg, near: mine.near, far: mine.far },
            CameraSpec::Third => {
                let anchor = self.tick_eye();
                let behind = anchor - mine.forward() * self.body.third_person_distance + Vec3::Y * self.body.third_person_lift;
                look_from(behind, anchor, fov, mine)
            }
            CameraSpec::Overview => {
                let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
                for c in &self.colliders {
                    lo = lo.min(Vec2::new(c.min.x, c.min.y));
                    hi = hi.max(Vec2::new(c.max.x, c.max.y));
                }
                if lo.x > hi.x {
                    return Err("the map has no walls to frame an overview around".into());
                }
                let (centre, extent) = ((lo + hi) * 0.5, hi - lo);
                let span = extent.x.max(extent.y).max(10.0);
                look_from(Vec3::new(centre.x, span * 0.8 + 4.0, centre.y + span * 0.62), Vec3::new(centre.x, 0.0, centre.y), 60.0, mine)
            }
            CameraSpec::Follow(id) => {
                let bodies = self.net.as_ref().map(|n| n.bodies().to_vec()).unwrap_or_default();
                let me = Vec3::new(self.physics_pos.x, self.foot_y, self.physics_pos.y);
                let body = match id {
                    Some(id) => bodies.iter().find(|b| b.id == *id),
                    None => bodies.iter().min_by(|a, b| (a.pos - me).length().total_cmp(&(b.pos - me).length())),
                }
                .ok_or_else(|| "there is nobody to follow".to_string())?;
                let head = body.pos + Vec3::Y * 1.4;
                let toward_me = (me - body.pos).with_y(0.0).normalize_or_zero();
                let eye = head + toward_me * 3.0 + Vec3::Y * 0.8;
                look_from(eye, head, fov, mine)
            }
            CameraSpec::Free { eye, at, fov: f } => look_from(Vec3::from(*eye), Vec3::from(*at), f.unwrap_or(fov), mine),
        })
    }

    /// Takes the pictures that are due now (called once per frame, before the window's own frame is presented).
    pub(crate) fn take_due_shots(&mut self) {
        for (name, camera) in self.shots.due(self.play_secs) {
            match self.capture_shot(&name, &camera) {
                Ok(record) => {
                    println!("shot: {} ({} at {:.1} s, {:.0}% flat)", record.file.display(), record.camera, record.secs, record.flat * 100.0);
                    self.shots.taken.push(record);
                }
                Err(e) => {
                    let msg = format!("shot '{name}': {e}");
                    eprintln!("{msg}");
                    self.shots.errors.push(msg);
                }
            }
        }
    }

    /// Renders the current frame from `camera` into an offscreen target the size of the window's (or the headless frame's) and saves it as a PNG.
    pub(crate) fn capture_shot(&mut self, name: &str, camera: &CameraSpec) -> Result<ShotRecord, String> {
        let cam = self.shot_camera(camera)?;
        let first = *camera == CameraSpec::First;
        let path = self.shots.path_for(name);
        // The player's own body is scaled to nothing in first person; a picture from elsewhere shows it.
        let own_body = self.player_object_index;
        let saved_scale = self.scene.objects[own_body].scale.clone();
        if !first {
            self.scene.objects[own_body].scale = Track::constant(Vec3::ONE);
        }
        let image = self.render_capture(&cam, first);
        self.scene.objects[own_body].scale = saved_scale;
        let image = image?;
        save_png(&image, &path).map_err(|e| e.to_string())?;
        Ok(ShotRecord { name: name.to_string(), file: path, secs: self.play_secs, camera: describe(camera), flat: flat_fraction(&image) })
    }

    /// Draws the frame from `cam` into the capture target and reads it back.
    fn render_capture(&mut self, cam: &FpsCamera, first: bool) -> Result<image::RgbaImage, String> {
        let (weapon_transform, hand, carrying, dead, weapon) =
            (self.weapon_transform(), self.hand_prop_transform, self.carrying(), self.own_dead(), self.shown_weapon());
        let opts = FrameOptions {
            crosshair: first,
            viewmodel: first && !carrying && !dead,
            pickup: self.pickup_target.is_some(),
            weapon,
            muzzle_flash: (self.flash_left / MUZZLE_FLASH_TIME).clamp(0.0, 1.0),
            fx: self.feel.fx(self.camera.yaw),
            enemy: self.aim_enemy,
        };
        let highlighted = self.target_index.is_some();
        let t = if self.scene.duration > 0.0 { self.start.elapsed().as_secs_f32() % self.scene.duration } else { 0.0 };
        let hidden = hidden_ids(&self.net, &self.rules, &self.streaks, &self.scene, first.then_some(self.player_object_index));
        let Some(gpu) = self.gpu.as_mut() else {
            return Err(
                "there is no GPU adapter (a headless run starts one only for `--shot-at` or a script `shot` step); is there a GPU, or a software adapter?"
                    .into(),
            );
        };
        let Some(live) = gpu.live.as_mut() else { return Err("the game has not started yet: there is no frame to draw".into()) };
        let (w, h) = (gpu.config.width, gpu.config.height);
        if !self.shots.capture.as_ref().is_some_and(|c| c.matches(gpu.config.format, w, h)) {
            self.shots.capture = Some(Capture::new(&gpu.device, gpu.config.format, w, h).map_err(|e| e.to_string())?);
        }
        let capture = self.shots.capture.as_ref().ok_or("no capture target")?;
        live.set_hidden_objects(hidden);
        if let Some(net) = &self.net {
            live.set_remote_hands(net.remote_hands());
        }
        live.render_ex(&gpu.device, &gpu.queue, &self.scene, t, cam, capture.view(), highlighted, weapon_transform, hand, opts);
        capture.read_rgba(&gpu.device, &gpu.queue).map_err(|e| e.to_string())
    }
}
