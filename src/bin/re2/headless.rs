//! `re2 --headless`: the real client loop with no window (ADR 2026-09-28-seeing-what-the-player-sees).
//!
//! The windowed client is a state machine that a `winit` event loop steps; nothing in it needs a window except drawing. This module steps the same `App` by hand
//! (`update` + `draw`, 60 frames a second of game time), played by a [`Script`] instead of a keyboard, with a null renderer, or an offscreen one when pictures are asked
//! for. What the player would see is not looked at but *dumped* (`dump.rs`): which remote players are drawn, the HUD's text, the sounds, the crosshair, every counter.
//! An expectation in the script that does not hold, a warning from the session (a player nobody can see) or a picture that cannot be taken makes the exit code 1.

use super::*;
use red_engine2::playscript::{CameraSpec, Driver, Key, Policy, Pose, Runner, Script};
use red_engine2::sim::approach::Target;
use serde_json::{json, Value};

/// What the command line asked of a headless run.
pub(crate) struct Options {
    pub enabled: bool,
    pub script: Option<PathBuf>,
    pub dump: Option<PathBuf>,
    pub shot_at: Vec<f32>,
    pub shot_dir: PathBuf,
    pub size: (u32, u32),
    pub playtest: bool,
    pub secs: Option<f32>,
    pub shots: Option<usize>,
    pub out: Option<PathBuf>,
}

/// The seconds of game time per frame: the client's fixed simulation step.
const DT: f32 = 1.0 / 60.0;
/// How long a run may take beyond its script's own length before it is called stuck, seconds.
const GRACE_SECS: f32 = 60.0;

/// The cameras a playtest takes its pictures from, in turn: what the player sees, then the views an AI cannot get any other way.
const PLAYTEST_CAMERAS: [&str; 6] = ["first", "first", "third", "overview", "follow", "first"];

/// A scripted session of `secs` seconds with `shots` pictures: wait for the round to start, then play in turns as a walker, a sentry and a walker again, taking a
/// picture from each camera in turn, and finish by asserting that nobody was left undrawn.
pub(crate) fn playtest_script(secs: f32, shots: usize, online: bool) -> Script {
    let shots = shots.max(1);
    let spacing = (secs.max(1.0) / shots as f32).max(0.5);
    let mut steps: Vec<Value> = Vec::new();
    if online {
        steps.push(json!({"wait_for": {"at": "/online/in_round", "eq": true, "within": 30, "msg": "the round never started"}}));
    }
    for i in 0..shots {
        let camera = PLAYTEST_CAMERAS[i % PLAYTEST_CAMERAS.len()];
        // Nobody to follow in a solo map: look from above instead.
        let camera = if camera == "follow" && !online { "overview" } else { camera };
        let policy = ["walker", "sentry", "walker"][(i * 3 / shots).min(2)];
        steps.push(json!({"policy": policy}));
        steps.push(json!({"wait": spacing}));
        steps.push(json!({"shot": format!("{:02}-{camera}", i + 1), "camera": camera}));
    }
    steps.push(json!({"snapshot": "final"}));
    if online {
        steps.push(json!({"expect": {"at": "/remote/undrawn", "eq": 0, "msg": "every other player must have an avatar to be drawn"}}));
    }
    Script::parse(&json!({"policy": "walker", "steps": steps}).to_string()).unwrap_or_else(|e| panic!("the playtest script is valid: {e:?}"))
}

impl Driver for App {
    fn hold_keys(&mut self, keys: &[Key], down: bool) {
        for key in keys {
            let code = match key {
                Key::Forward => KeyCode::KeyW,
                Key::Back => KeyCode::KeyS,
                Key::Left => KeyCode::KeyA,
                Key::Right => KeyCode::KeyD,
                Key::Crouch => KeyCode::ControlLeft,
                Key::Sprint => {
                    self.sprint_held = down;
                    continue;
                }
            };
            if down {
                self.keys.insert(code);
            } else {
                self.keys.remove(&code);
            }
        }
    }

    fn set_look(&mut self, yaw_deg: Option<f32>, pitch_deg: Option<f32>) {
        if let Some(y) = yaw_deg {
            self.camera.yaw = y.to_radians();
        }
        if let Some(p) = pitch_deg {
            self.camera.pitch = p.to_radians().clamp(-FpsCamera::PITCH_LIMIT, FpsCamera::PITCH_LIMIT);
        }
    }

    fn add_yaw(&mut self, deg: f32) {
        self.camera.yaw += deg.to_radians();
    }

    fn jump(&mut self) {
        self.jump_queued = true;
    }

    fn fire(&mut self, down: bool) {
        if down {
            self.press_primary();
        } else {
            self.attack_held = false;
        }
    }

    fn scroll(&mut self, lines: f32) {
        self.on_scroll(lines);
    }

    fn interact(&mut self) {
        App::interact(self);
    }

    fn aim_at_nearest(&mut self) -> bool {
        self.aim_at_nearest_visible()
    }

    fn set_view(&mut self, third: bool) {
        self.view_mode = if third { ViewMode::ThirdPerson } else { ViewMode::FirstPerson };
    }

    fn set_policy(&mut self, policy: Policy) {
        let (walk, aim, fire) = match policy {
            Policy::Idle => (None, false, false),
            Policy::Sentry => (Some("still"), true, false),
            Policy::Walker => (Some("circle:30"), false, true),
        };
        self.autowalk = walk.map(str::to_string);
        self.autoaim = aim;
        self.autofire = fire;
    }

    fn shot(&mut self, name: &str, camera: &CameraSpec) -> Result<(), String> {
        if self.gpu.is_none() {
            return Err("there is no GPU adapter to draw with (this machine has none, not even a software one; `red_engine2 doctor` says what it has)".into());
        }
        // Nobody to follow: the picture is still worth having, from above.
        let camera = match camera {
            CameraSpec::Follow(_) if self.net.as_ref().is_none_or(|n| n.bodies().is_empty()) => CameraSpec::Overview,
            other => other.clone(),
        };
        self.shots.request(name, camera);
        Ok(())
    }

    fn snapshot(&mut self, name: &str) {
        let mut state = self.state_dump();
        if let Some(o) = state.as_object_mut() {
            o.remove("snapshots"); // a snapshot does not carry the ones before it
        }
        self.snapshots.push((name.to_string(), state));
    }

    fn state(&self) -> Value {
        self.state_dump()
    }

    fn say(&mut self, text: &str) {
        println!("script: {text}");
    }

    fn press(&mut self, id: &str) -> Result<(), String> {
        self.press_button(id)
    }

    fn pose(&self) -> Option<Pose> {
        Some(Pose { pos: self.physics_pos, eye: self.tick_eye(), pickup_reach: self.body.pickup_reach })
    }

    /// The scene's own copy of the object: a loose prop's pose is written into it every frame, offline from the physics world and online from the
    /// server's snapshots, so this is where the prop is now either way.
    fn locate(&self, id: &str) -> Option<Target> {
        let object = self.scene.objects.iter().find(|o| o.id == id)?;
        let item = red_engine2::collide::interactables_of(std::slice::from_ref(object)).into_iter().next()?;
        Some(Target::from_bounds(item.min, item.max))
    }

    /// Offline the physics world knows the holder; online it is the server's to know and the client cannot tell.
    fn carrying(&self, id: &str) -> Option<bool> {
        let props = self.props.as_ref()?;
        let index = self.scene.objects.iter().position(|o| o.id == id)?;
        Some(props.prop_of_object(index).is_some_and(|p| props.held() == Some(p)))
    }
}

impl App {
    /// Turns to the nearest other player with nothing solid between (a sentry does not see through walls). Whether there was one.
    pub(crate) fn aim_at_nearest_visible(&mut self) -> bool {
        let eye = self.tick_eye();
        let bodies = self.net.as_ref().map(|n| n.bodies().to_vec()).unwrap_or_default();
        let visible = bodies.into_iter().filter(|b| !b.dead).filter(|b| {
            let d = b.pos + Vec3::Y - eye;
            raycast_shapes(eye, d.normalize_or_zero(), d.length(), &self.hit_shapes).is_none()
        });
        match visible.min_by(|a, c| (a.pos - eye).length().total_cmp(&(c.pos - eye).length())) {
            Some(b) => {
                let d = b.pos + Vec3::Y - eye;
                self.camera.yaw = d.x.atan2(-d.z);
                self.camera.pitch = d.y.atan2(Vec2::new(d.x, d.z).length()).clamp(-1.2, 1.2);
                true
            }
            None => false,
        }
    }

    /// Gives a headless run a GPU that draws to no window (a software adapter if that is all there is); `Err` says why there is none.
    fn init_offscreen_gpu(&mut self, size: (u32, u32)) -> Result<(), String> {
        let gpu = red_engine2::gpu::Gpu::new().map_err(|e| e.to_string())?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.0,
            height: size.1,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
        };
        self.gpu = Some(GpuState { surface: None, device: gpu.device, queue: gpu.queue, config, live: None, backdrop: None });
        self.gpu_kind = "offscreen".to_string();
        Ok(())
    }
}

/// Plays the game headless until the script ends (or `--secs`), writes what was asked for, and returns the exit code: 0 when every expectation held and nothing
/// went undrawn, 1 when something failed, 2 when the run could not start.
pub(crate) fn run(app: &mut App, opts: Options) -> i32 {
    let script = match (&opts.script, opts.playtest) {
        (Some(path), _) => {
            match std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display())).and_then(|t| Script::parse(&t).map_err(|e| e.join("\n"))) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("script: {e}");
                    return 2;
                }
            }
        }
        (None, true) => playtest_script(opts.secs.unwrap_or(60.0), opts.shots.unwrap_or(12), app.net_server.is_some()),
        // No script: just be there for a while, then report.
        (None, false) => Script::parse(&json!({"steps": [{"wait": opts.secs.unwrap_or(5.0)}]}).to_string()).unwrap_or_else(|e| panic!("{e:?}")),
    };
    let needs_gpu = !opts.shot_at.is_empty() || opts.playtest || script_takes_shots(&script);
    app.headless = true;
    app.virtual_size = Some(opts.size);
    app.shots = shots::Shots::new(opts.out.clone().map_or(opts.shot_dir.clone(), |o| o.join("shots")), opts.shot_at.clone());
    if needs_gpu {
        if let Err(e) = app.init_offscreen_gpu(opts.size) {
            eprintln!("headless: no GPU to draw pictures with ({e}); the run goes on without a renderer and any picture is reported as failed");
        }
    }
    let who = app.forced_character.unwrap_or(Character::Human);
    app.start_game(who);
    app.grabbed = true; // there is no window to grab the mouse for, but the game plays
                        // The most game time the run may take: the script's own length (its waits counted at their longest) plus a grace period.
    let limit = opts.secs.map_or(script.nominal_secs(), |s| s.max(script.nominal_secs())) + GRACE_SECS;
    let mut runner = Runner::new(script);
    app.set_policy(runner.policy());
    let started = Instant::now();
    let realtime = app.net.is_some(); // a server on another thread runs on the wall clock, so the client keeps step with it
    let mut frame = 0u64;
    let mut timed_out = false;
    let mut grace = 0;
    loop {
        runner.advance(DT, app);
        app.update(DT);
        app.draw();
        frame += 1;
        if runner.finished() {
            grace += 1;
            if grace > 30 && !app.shots.pending() {
                break; // half a second for the last effects to show up in the state
            }
        }
        if app.play_secs > limit {
            timed_out = true;
            break;
        }
        if realtime {
            let due = started + std::time::Duration::from_secs_f64(frame as f64 / 60.0);
            if let Some(wait) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
        }
    }
    // Let go of everything, so a server does not hold a phantom player, and a key is not left down.
    app.hold_keys(&Key::ALL.map(|(_, k)| k), false);
    app.fire(false);
    app.failures.extend(runner.failures.iter().cloned());
    if timed_out {
        app.failures.push(format!(
            "the script did not finish within {limit:.0} s of game time (a `wait_for` that never came true, or a script that is longer than it says)"
        ));
    }
    if let Some(net) = app.net.as_mut() {
        net.client.disconnect();
    }
    finish(app, &opts)
}

/// Whether any step of `script` takes a picture.
fn script_takes_shots(script: &Script) -> bool {
    script.steps.iter().any(|s| matches!(s, red_engine2::playscript::Step::Shot { .. }))
}

/// Writes the dump, the playtest's contact sheet and report, prints the verdict and returns the exit code.
fn finish(app: &mut App, opts: &Options) -> i32 {
    if opts.playtest {
        let out = opts.out.clone().unwrap_or_else(|| PathBuf::from("out/playtest"));
        let tiles: Vec<red_engine2::tools::sheet::Tile> = app
            .shots
            .taken
            .iter()
            .filter_map(|s| {
                image::open(&s.file).ok().map(|i| red_engine2::tools::sheet::Tile { image: i.to_rgba8(), title: format!("{} {:.0}s", s.name, s.secs) })
            })
            .collect();
        let mut sheet_path = None;
        if !tiles.is_empty() {
            let sheet = red_engine2::tools::sheet::contact_sheet(&tiles, 4, 480);
            let path = out.join("contact-sheet.png");
            match std::fs::create_dir_all(&out).map_err(|e| e.to_string()).and_then(|_| sheet.save(&path).map_err(|e| e.to_string())) {
                Ok(()) => {
                    println!("contact sheet: {}", path.display());
                    sheet_path = Some(path.display().to_string());
                }
                Err(e) => app.failures.push(format!("contact sheet: {e}")),
            }
        }
        let mut state = app.state_dump();
        state["sheet"] = json!(sheet_path);
        let report = out.join("playtest.json");
        match std::fs::create_dir_all(&out)
            .map_err(|e| e.to_string())
            .and_then(|_| std::fs::write(&report, serde_json::to_string_pretty(&state).unwrap_or_default() + "\n").map_err(|e| e.to_string()))
        {
            Ok(()) => println!("report: {}", report.display()),
            Err(e) => app.failures.push(format!("report: {e}")),
        }
    }
    if let Some(path) = &opts.dump {
        match app.write_dump(path) {
            Ok(()) => println!("state: {}", path.display()),
            Err(e) => app.failures.push(e),
        }
    }
    let mut failures = app.failures.clone();
    failures.extend(app.shots.errors.iter().cloned());
    failures.dedup();
    if failures.is_empty() {
        println!("headless: ok ({:.1} s of game time, {} frames, {} picture(s))", app.play_secs, app.frame_no, app.shots.taken.len());
        0
    } else {
        for f in &failures {
            eprintln!("FAILED: {f}");
        }
        eprintln!("headless: {} failure(s)", failures.len());
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_playtest_script_waits_for_the_round_shoots_from_every_camera_and_ends_by_checking_nobody_is_undrawn() {
        let s = playtest_script(60.0, 12, true);
        assert!(matches!(s.steps.first(), Some(red_engine2::playscript::Step::Expect(e)) if e.at == "/online/in_round" && e.within == 30.0));
        let shots: Vec<&red_engine2::playscript::Step> = s.steps.iter().filter(|s| matches!(s, red_engine2::playscript::Step::Shot { .. })).collect();
        assert_eq!(shots.len(), 12);
        for camera in [CameraSpec::First, CameraSpec::Third, CameraSpec::Overview, CameraSpec::Follow(None)] {
            assert!(shots.iter().any(|s| matches!(s, red_engine2::playscript::Step::Shot { camera: c, .. } if *c == camera)), "{camera:?}");
        }
        assert!(matches!(s.steps.last(), Some(red_engine2::playscript::Step::Expect(e)) if e.at == "/remote/undrawn"));
        assert!((s.nominal_secs() - (60.0 + 30.0 + 0.0)).abs() < 1.0, "sixty seconds of play plus the round-start allowance: {}", s.nominal_secs());
    }

    #[test]
    fn a_solo_playtest_has_no_round_to_wait_for_nobody_to_follow_and_nothing_to_check_online() {
        let s = playtest_script(30.0, 6, false);
        assert!(!s.steps.iter().any(|s| matches!(s, red_engine2::playscript::Step::Expect(_))));
        assert!(!s.steps.iter().any(|s| matches!(s, red_engine2::playscript::Step::Shot { camera: CameraSpec::Follow(_), .. })));
    }
}
