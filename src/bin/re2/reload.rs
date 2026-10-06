//! Content hot reload (ADR 2026-10-06-hot-reload-of-scene-content): `re2` watches the scene file it was started on and, when it is saved, re-validates and applies it.
//!
//! * **Valid** content replaces the running content in place: the scene, its rules, the collision world, the loose props and the renderer's meshes are rebuilt, while the
//!   player keeps their place, view, character, weapon and ammunition. Rule variables and loose props start again from the new scene (their old state belongs to the old
//!   content); audio and the scene's music are not restarted (an `audio` edit needs a restart).
//! * **Invalid** content changes nothing: the old content keeps running and the same `error: path: message` lines `validate` prints go to the terminal, with the first one
//!   on screen. Fixing the file and saving again applies it.
//! * **Off** online (the server's map hash would reject a different map), in split screen, behind a start card and when `RE2_RELOAD=0`.
//!
//! The watch is a poll of the file's modification time every quarter second; a change is applied only when the file's *content* changed, so an editor that touches or
//! saves twice causes one reload. It runs in `App::update`, so the windowed client and `re2 --headless` (where the tests drive it) share one path.

use super::*;
use std::time::{Duration, SystemTime};

/// How often the file is looked at.
const POLL: Duration = Duration::from_millis(250);
/// How long the on-screen note about a reload stays up, seconds.
const NOTICE_SECS: f32 = 3.0;
/// How long the on-screen note about a failed reload stays up (the terminal keeps all of it), seconds.
const ERROR_NOTICE_SECS: f32 = 12.0;

/// A poll of one file that reports a new text only when its content changed.
pub(crate) struct Watch {
    path: PathBuf,
    mtime: Option<SystemTime>,
    hash: u64,
    next: Instant,
}

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

impl Watch {
    /// Starts watching `path`, taking its present content as the one already running.
    pub(crate) fn new(path: PathBuf) -> Self {
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let hash = std::fs::read_to_string(&path).map(|t| fnv(&t)).unwrap_or(0);
        Watch { path, mtime, hash, next: Instant::now() + POLL }
    }

    /// The new text when the file changed since the last look (at most every [`POLL`]); `None` otherwise, including while the file cannot be read (it is mid-save).
    pub(crate) fn poll(&mut self, now: Instant) -> Option<String> {
        if now < self.next {
            return None;
        }
        self.next = now + POLL;
        let mtime = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if mtime == self.mtime {
            return None;
        }
        let text = std::fs::read_to_string(&self.path).ok()?;
        self.mtime = mtime;
        let hash = fnv(&text);
        if hash == self.hash {
            return None;
        }
        self.hash = hash;
        Some(text)
    }
}

/// Whether the environment allows hot reload (`RE2_RELOAD=0` turns it off).
pub(crate) fn enabled_by_env() -> bool {
    std::env::var("RE2_RELOAD").map_or(true, |v| v != "0")
}

impl App {
    /// Whether this session may swap its content: offline, one local player, in the map.
    pub(crate) fn hot_reload_allowed(&self) -> bool {
        self.phase == Phase::Playing
            && self.net.is_none()
            && self.net_server.is_none()
            && self.pending_net.is_none()
            && self.want_players <= 1
            && self.locals.is_empty()
    }

    /// Starts watching the scene file when the session allows it (called once the game has started).
    pub(crate) fn start_hot_reload_watch(&mut self) {
        self.reload_watch = (enabled_by_env() && self.hot_reload_allowed()).then(|| Watch::new(self.scene_path.clone()));
    }

    /// Once per frame: applies the scene file if it was saved since the last look.
    pub(crate) fn hot_reload_check(&mut self) {
        if !self.hot_reload_allowed() {
            return;
        }
        let Some(text) = self.reload_watch.as_mut().and_then(|w| w.poll(Instant::now())) else { return };
        let started = Instant::now();
        let parsed = red_engine2::schema::parse_scene_in(&text, self.scene_path.parent());
        match parsed {
            Err(errs) => {
                for e in &errs {
                    eprintln!("error: {e}");
                }
                self.reload_failed = Some(errs.clone());
                self.notice = Some((format!("RELOAD FAILED - {}", errs.first().map_or("invalid scene", String::as_str)), ERROR_NOTICE_SECS));
                eprintln!("{}: not reloaded ({} error(s)); the previous content keeps running", self.scene_path.display(), errs.len());
            }
            Ok(scene) => {
                self.apply_scene(scene, &text);
                self.reload_failed = None;
                self.reloads += 1;
                self.notice = Some(("RELOADED".to_string(), NOTICE_SECS));
                println!("{}: reloaded in {} ms", self.scene_path.display(), started.elapsed().as_millis());
            }
        }
    }

    /// Replaces the running content with `scene` (already valid), keeping the player's place, view, character, weapon and ammunition. Mirrors the offline half of
    /// `start_game`: everything derived from the scene is derived again, everything that belongs to the window, the device, the audio and the player is left alone.
    fn apply_scene(&mut self, mut scene: Scene, text: &str) {
        let who = self.character;
        self.player_object_index = scene.objects.len();
        scene.objects.push(build_player_object(who));
        self.scene = scene;
        if let Ok(spawns) = parse_spawns(text) {
            if !spawns.is_empty() {
                self.spawns = spawns;
            }
        }
        self.camera.fov_deg = self.scene.player.fov_deg;
        self.fov_deg = self.scene.player.fov_deg;
        self.pad_launch = self.scene.jump_pads.iter().map(|p| p.launch_speed).reduce(f32::min);
        // Rules start again: a variable's value belongs to the content it was counted in. What the game keeps between sessions (`persist`) comes back as it does at start.
        let mut rules = RulesEngine::new(self.scene.rules.clone()).with_wrap(self.scene.player.expanse.wrap);
        self.saved_vars = if self.scene.rules.persist.is_empty() { Default::default() } else { red_engine2::settings::load_vars(&self.settings_key) };
        for (name, value) in &self.saved_vars {
            rules.set_var(name, *value);
        }
        self.rules = rules;
        // The loose props and the collision world, exactly as `start_game` builds them.
        let props = PropWorld::new(&self.scene, Some(self.player_object_index));
        let loose = props.movable_indices();
        self.rules.bind_props(|id| self.scene.objects.iter().position(|o| o.id == id).and_then(|i| props.prop_of_object(i)));
        self.physical = Some(PhysicalWorld::new(&self.scene, &loose));
        self.rebuild_collision_world();
        self.hit_shapes = collect_hit_shapes_where(&self.scene, |i| i != self.player_object_index && !loose.contains(&i));
        self.props = Some(props);
        self.pickup_target = None;
        self.target_index = None;
        self.rule_event = None;
        self.rule_event_until = 0;
        self.rule_hud_painted = None;
        self.fresh_events.clear();
        // The pool of glowing boxes for tracers and sparks joins the scene before the renderer takes its meshes from it.
        self.streaks = Some(red_engine2::streaks::Streaks::new(red_engine2::streaks::add_pool(&mut self.scene)));
        if let Some(gpu) = self.gpu.as_mut() {
            let (w, h) = (gpu.config.width, gpu.config.height);
            gpu.live = Some(if self.scene.player.mode.is_peaceful() {
                LiveRenderer::world(&gpu.device, gpu.config.format, &self.scene, w, h)
            } else {
                LiveRenderer::new(&gpu.device, gpu.config.format, &self.scene, w, h)
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("re2_reload_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("scene.json")
    }

    #[test]
    fn the_watch_reports_a_change_once_and_ignores_a_touch() {
        let path = tmp("watch");
        std::fs::write(&path, "one").unwrap();
        let mut w = Watch::new(path.clone());
        let mut now = Instant::now() + POLL * 2;
        assert_eq!(w.poll(now), None, "nothing changed");
        // A touch (new mtime, same content) is not a change.
        std::fs::write(&path, "one").unwrap();
        now += POLL * 2;
        assert_eq!(w.poll(now), None);
        std::fs::write(&path, "two").unwrap();
        let t = std::fs::metadata(&path).unwrap().modified().unwrap() + Duration::from_secs(2);
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(t).unwrap();
        now += POLL * 2;
        assert_eq!(w.poll(now).as_deref(), Some("two"));
        now += POLL * 2;
        assert_eq!(w.poll(now), None, "reported once");
    }

    #[test]
    fn the_watch_is_rate_limited() {
        let path = tmp("rate");
        std::fs::write(&path, "a").unwrap();
        let mut w = Watch::new(path.clone());
        std::fs::write(&path, "b").unwrap();
        let t = std::fs::metadata(&path).unwrap().modified().unwrap() + Duration::from_secs(2);
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(t).unwrap();
        assert_eq!(w.poll(Instant::now()), None, "looked at too soon after it started");
        assert_eq!(w.poll(Instant::now() + POLL * 2).as_deref(), Some("b"));
    }

    const FLOOR: &str = r#""camera":{"position":[0,1.7,6],"target":[0,1,0]},"spawns":[{"id":"s","position":[-6,0,0],"yaw_deg":90}]"#;

    fn scene_text(extra_objects: &str) -> String {
        format!(r#"{{{FLOOR},"objects":[{{"id":"floor","type":"plane","size":[40,20],"position":[0,0.01,0]}}{extra_objects}]}}"#)
    }

    /// Writes `text` and moves the file's modification time forward so the poll sees a new save.
    fn save(path: &std::path::Path, text: &str, step: u64) {
        std::fs::write(path, text).unwrap();
        let t = SystemTime::now() + Duration::from_secs(step * 5);
        std::fs::File::options().write(true).open(path).unwrap().set_modified(t).unwrap();
    }

    fn started(path: &std::path::Path) -> App {
        let scene = red_engine2::load_scene(path).unwrap();
        let mut app = App::new(scene, path.to_path_buf(), Some(Character::Human), None, None);
        app.headless = true;
        app.start_game(Character::Human);
        app
    }

    /// One frame after the poll interval has passed.
    fn frame(app: &mut App) {
        std::thread::sleep(POLL + Duration::from_millis(60));
        app.update(1.0 / 60.0);
    }

    const WALL: &str = r#",{"id":"w","type":"wall","from":[2,-4],"to":[2,4],"height":2.5,"thickness":0.2}"#;

    #[test]
    fn a_saved_scene_replaces_the_running_one_and_the_player_stays_where_they_are() {
        let path = tmp("apply");
        std::fs::write(&path, scene_text("")).unwrap();
        let mut app = started(&path);
        assert!(app.reload_watch.is_some(), "an offline single-player game watches its file");
        app.physics_pos = Vec2::new(0.5, 0.25);
        app.camera.yaw = 1.0;
        app.camera.pitch = -0.25;
        let (objects, colliders) = (app.scene.objects.len(), app.colliders.len());
        save(&path, &scene_text(WALL), 1);
        frame(&mut app);
        assert_eq!(app.reloads, 1);
        assert_eq!(app.scene.objects.len(), objects + 1, "the wall is in the running scene");
        assert!(app.colliders.len() > colliders, "and in the collision world the player walks in ({} -> {})", colliders, app.colliders.len());
        assert_eq!((app.physics_pos, app.camera.yaw, app.camera.pitch), (Vec2::new(0.5, 0.25), 1.0, -0.25), "the player keeps their place and view");
        assert!(app.notice.as_ref().is_some_and(|(t, _)| t == "RELOADED"));
        assert_eq!(app.scene.objects[app.player_object_index].id, build_player_object(Character::Human).id, "the player's body is rebuilt in the new scene");
    }

    #[test]
    fn an_invalid_save_keeps_the_old_content_running_and_a_fixed_save_applies() {
        let path = tmp("invalid");
        std::fs::write(&path, scene_text(WALL)).unwrap();
        let mut app = started(&path);
        let (objects, colliders) = (app.scene.objects.len(), app.colliders.len());
        // A typo in a field name: `validate` says so with a did-you-mean fix.
        save(&path, &scene_text(r#",{"id":"b","type":"box","size":[1,1,1],"positon":[0,0,0]}"#), 1);
        frame(&mut app);
        assert_eq!(app.reloads, 0, "nothing was applied");
        let errs = app.reload_failed.clone().expect("the failure is recorded");
        assert!(errs.iter().any(|e| e.contains("positon")), "the diagnostic is validate's: {errs:?}");
        assert_eq!((app.scene.objects.len(), app.colliders.len()), (objects, colliders), "the old content is untouched");
        assert!(app.notice.as_ref().is_some_and(|(t, _)| t.starts_with("RELOAD FAILED")));
        // Not even JSON.
        save(&path, "{", 2);
        frame(&mut app);
        assert!(app.reload_failed.is_some() && app.reloads == 0);
        // Fixed: applies, and the failure is cleared.
        save(&path, &scene_text(&format!(r#"{WALL},{{"id":"b","type":"box","size":[1,1,1],"position":[0,0.5,3]}}"#)), 3);
        frame(&mut app);
        assert_eq!(app.reloads, 1);
        assert!(app.reload_failed.is_none());
        assert_eq!(app.scene.objects.len(), objects + 1);
    }

    #[test]
    fn hot_reload_is_off_online_and_in_split_screen_and_before_the_game_starts() {
        let path = tmp("off");
        std::fs::write(&path, scene_text("")).unwrap();
        let scene = red_engine2::load_scene(&path).unwrap();
        let app = App::new(scene, path.clone(), Some(Character::Human), None, None);
        assert!(!app.hot_reload_allowed(), "on the connect form there is no game to swap");
        let mut app = started(&path);
        assert!(app.hot_reload_allowed());
        app.net_server = Some("127.0.0.1:27015".parse().unwrap());
        assert!(!app.hot_reload_allowed(), "online the server's map hash would reject a different map");
        app.net_server = None;
        let objects = app.scene.objects.len();
        save(&path, &scene_text(WALL), 1);
        app.want_players = 2;
        frame(&mut app);
        assert_eq!((app.reloads, app.scene.objects.len()), (0, objects), "split screen is not reloaded");
    }

    /// `cargo test --bin re2 measure_reload -- --ignored --nocapture`: how long applying a save takes against starting the same scene from nothing (in process: no window,
    /// no device, no process start; a real restart also pays those, so the restart figure is a lower bound).
    #[test]
    #[ignore = "a measurement, not a check"]
    fn measure_reload_against_a_cold_start() {
        for name in ["examples/test_lab.json", "examples/house.json", "examples/office.json"] {
            let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
            let dir = std::env::temp_dir().join(format!("re2_reload_measure_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("scene.json");
            std::fs::copy(&src, &path).unwrap();
            let mut app = None;
            let mut cold = Vec::new();
            for _ in 0..3 {
                let t = Instant::now();
                app = Some(started(&path));
                cold.push(t.elapsed().as_millis());
            }
            let mut app = app.unwrap();
            let mut warm = Vec::new();
            for step in 1..=3u64 {
                let text = std::fs::read_to_string(&src).unwrap().replacen("\"objects\"", "\"x-touch\":0,\"objects\"", 1) + &" ".repeat(step as usize);
                save(&path, &text, step);
                std::thread::sleep(POLL + Duration::from_millis(60));
                let t = Instant::now();
                app.update(1.0 / 60.0);
                warm.push(t.elapsed().as_millis());
            }
            println!("{name}: cold start {cold:?} ms, hot reload {warm:?} ms (+ up to {} ms for the poll), reloads applied {}", POLL.as_millis(), app.reloads);
        }
    }
}
