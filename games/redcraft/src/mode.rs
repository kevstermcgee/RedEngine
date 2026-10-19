//! RedCraft in the live client, as a game mode of the engine's client ([`CraftMode`], a [`GameMode`]): the title screen over a slowly turning
//! world, the world being played ([`Play`]) fed with the player's keys, mouse and gamepad, the camera at the body's eyes, the screens and the HUD
//! painted over the frame, the sounds, the shader's numbers (the aimed block, the daylight) and the saves. It reaches the client only through its
//! [`ModeContext`].
//!
//! Controls, as the original's: WASD to walk, the mouse to look, Space to jump (twice quickly to fly in creative), Shift to sneak, Ctrl to sprint,
//! the left button to mine, the right to place and use, 1-9 and the wheel for the hotbar, E for the inventory, F3 for the debug lines, Esc to
//! pause. A gamepad: the left stick walks, the right looks, A jumps, the right trigger mines, the left places, the bumpers step the hotbar, X
//! opens the inventory, B sneaks.
//!
//! `REDCRAFT_WORLD=NAME` skips the title and plays that world (made with `REDCRAFT_SEED` and `REDCRAFT_MODE` when new): a headless script plays
//! the game that way, pressing `key:` names [`GameMode::script_key`] lists.

use crate::block::Block;
use crate::item::Item;
use crate::play::{Event, Input, Opened, Play};
use crate::save::GameMode as Mode;
use crate::screens::{click_slot, Screen, Title};
use crate::sound::Sounds;
use crate::ui::{Button, Hit, IconCache, Painted};
use glam::{Mat4, Vec3};
use red_engine2::client::{Answer, GameMode, ModeContext};
use red_engine2::controller::button as pad;
use red_engine2::feel::FxParams;
use std::path::PathBuf;
use std::time::Instant;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};

mod paint;
mod script;

const SPLASHES: &[&str] = &[
    "Punch a tree!",
    "Made of blocks!",
    "Now with caves!",
    "Red Engine inside!",
    "Also try Honeybound!",
    "Also try Frameless!",
    "Mind the creepers... there are none!",
    "Diamonds below 16!",
    "Torches keep the dark away!",
    "Every world is new!",
];

/// The seed of the world turning behind the title.
const PANORAMA_SEED: u32 = 2;

/// Settings kept between runs.
#[derive(Clone, Copy, Debug)]
struct Options {
    view: i32,
    auto_jump: bool,
}

fn options_path(key: &str) -> Option<PathBuf> {
    red_engine2::settings::game_state_path(key, "redcraft-options.json")
}

fn load_options(key: &str) -> Options {
    let v: Option<serde_json::Value> = options_path(key).and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok());
    Options {
        view: v.as_ref().and_then(|v| v.get("view")).and_then(|v| v.as_i64()).map_or(10, |n| n as i32).clamp(4, 16),
        auto_jump: v.as_ref().and_then(|v| v.get("auto_jump")).and_then(|v| v.as_bool()).unwrap_or(true),
    }
}

struct Game {
    key: String,
    sounds: Option<Sounds>,
    icons: IconCache,
    screen: Screen,
    play: Option<Play>,
    panorama: Option<crate::stream::Terrain>,
    pano_angle: f32,
    opts: Options,
    painted: Option<Painted>,
    painted_key: u64,
    mouse: (f32, f32),
    mine_held: bool,
    use_pressed: bool,
    use_held: bool,
    script_mine: bool,
    debug: bool,
    held_name: Option<(String, f32)>,
    message: Option<(String, f32)>,
    last_selected: Option<Item>,
    clock: Instant,
    since_start: f32,
    splash: &'static str,
    worlds: Vec<(PathBuf, crate::save::Level)>,
    creative_items: Vec<Item>,
    sound_n: u32,
    fps: f32,
    /// The world switched: the layer drops what it holds.
    clear_layer: bool,
    bob: f32,
    view_fov: f32,
    /// Meshes of held items, by item and the light they were lit with.
    hand_meshes: std::collections::HashMap<(Item, u8, u8), (std::sync::Arc<red_engine2::gpu::GpuMesh>, bool)>,
    /// Unit cubes in a mob's colours, by colour and light.
    part_meshes: std::collections::HashMap<(u8, u8, u8), std::sync::Arc<red_engine2::gpu::GpuMesh>>,
}

/// RedCraft as a game mode of the client.
#[derive(Default)]
pub struct CraftMode(Option<Box<Game>>);

impl CraftMode {
    pub fn new() -> CraftMode {
        CraftMode(None)
    }
}

fn workers() -> usize {
    crate::stream::live_workers()
}

impl Game {
    fn new(key: String, audio: bool) -> Game {
        let opts = load_options(&key);
        let splash = SPLASHES[(crate::save::now_secs() % SPLASHES.len() as u64) as usize];
        Game {
            key,
            sounds: audio.then(Sounds::make),
            icons: IconCache::default(),
            screen: Screen::Title(Title::Main),
            play: None,
            panorama: None,
            pano_angle: 0.6,
            opts,
            painted: None,
            painted_key: 0,
            mouse: (0.0, 0.0),
            mine_held: false,
            use_pressed: false,
            use_held: false,
            script_mine: false,
            debug: false,
            held_name: None,
            message: None,
            last_selected: None,
            clock: Instant::now(),
            since_start: 0.0,
            splash,
            worlds: Vec::new(),
            creative_items: Item::all(),
            sound_n: 0,
            fps: 60.0,
            clear_layer: false,
            bob: 0.0,
            view_fov: 72.0,
            hand_meshes: std::collections::HashMap::new(),
            part_meshes: std::collections::HashMap::new(),
        }
    }

    fn save_options(&self) {
        if let Some(p) = options_path(&self.key) {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(p, serde_json::json!({"view": self.opts.view, "auto_jump": self.opts.auto_jump}).to_string());
        }
    }

    fn worlds_dir(&self) -> PathBuf {
        crate::save::worlds_dir(&self.key).unwrap_or_else(|| std::env::temp_dir().join("redcraft-worlds"))
    }

    fn refresh_worlds(&mut self) {
        self.worlds = crate::save::list(&self.worlds_dir());
    }

    fn play_sound(&mut self, cx: &ModeContext, clip: Clip, gain: f32) {
        let Some(s) = &self.sounds else { return };
        self.sound_n = self.sound_n.wrapping_add(1);
        let n = self.sound_n;
        let samples: &[f32] = match clip {
            Clip::Dig(m) => s.dig(m, n),
            Clip::Broke(m) => s.broke(m, n),
            Clip::Place(m) => s.place(m, n),
            Clip::Step(m) => s.step(m, n),
            Clip::Hurt => &s.hurt,
            Clip::Pop => &s.pop,
            Clip::Eat => &s.eat,
            Clip::Splash => &s.splash,
            Clip::Click => &s.click,
            Clip::Swing => &s.swing,
            Clip::Pig => &s.pig,
            Clip::Zombie => &s.zombie,
            Clip::Fuse => &s.fuse,
            Clip::Explode => &s.explode,
        };
        let pan = ((n % 7) as f32 - 3.0) * 0.04;
        cx.play(samples, gain, pan);
    }

    /// Starts playing `play`, leaving the title.
    fn enter(&mut self, cx: &mut ModeContext, play: Play, time: Option<f32>) {
        let mut play = play;
        play.body.auto_jump = self.opts.auto_jump;
        play.terrain.view = self.opts.view;
        self.play = Some(play);
        self.panorama = None;
        self.clear_layer = true;
        self.screen = Screen::Hud;
        cx.set_time_of_day(time.unwrap_or(0.29));
        if cx.paused() {
            cx.leave_pause();
        }
    }

    fn create_world(&mut self, cx: &mut ModeContext, name: &str, seed: Option<u32>, mode: Mode) {
        let base = self.worlds_dir();
        let mut slug = crate::save::slug(name);
        let mut n = 2;
        while base.join(&slug).exists() {
            slug = format!("{}-{n}", crate::save::slug(name));
            n += 1;
        }
        let seed = seed.unwrap_or_else(|| (crate::save::now_secs() as u32).wrapping_mul(2654435761) & 0x00ff_ffff);
        let play = Play::create(base.join(slug), name, seed, mode, self.opts.view, workers());
        self.enter(cx, play, None);
    }

    fn open_world(&mut self, cx: &mut ModeContext, dir: PathBuf) {
        match crate::save::load(&dir) {
            Ok((level, chunks)) => {
                let time = level.time;
                let (yaw, pitch) = (level.yaw, level.pitch);
                let play = Play::open(dir, level, chunks, self.opts.view, workers());
                self.enter(cx, play, Some(time));
                cx.set_camera(Vec3::ZERO, yaw, pitch);
            }
            Err(e) => self.message = Some((format!("Could not open the world: {e}"), 1.0)),
        }
    }

    fn save(&mut self, cx: &ModeContext) {
        let (yaw, pitch) = cx.camera_angles();
        if let Some(p) = self.play.as_mut() {
            if let Err(e) = p.save(yaw, pitch) {
                eprintln!("redcraft: could not save the world: {e}");
            }
        }
    }

    fn back_to_title(&mut self, cx: &mut ModeContext) {
        self.save(cx);
        self.play = None;
        self.clear_layer = true;
        self.screen = Screen::Title(Title::Main);
        self.refresh_worlds();
        if !cx.paused() {
            cx.enter_pause();
        }
    }

    /// Opens a screen with slots (the cursor comes free).
    fn open(&mut self, cx: &mut ModeContext, s: Screen) {
        self.mine_held = false;
        self.use_held = false;
        self.screen = s;
        if !cx.paused() {
            cx.enter_pause();
        }
    }

    /// Closes whatever screen is up and goes back to playing.
    fn close(&mut self, cx: &mut ModeContext) {
        if let Some(p) = self.play.as_mut() {
            p.return_carried();
        }
        self.screen = Screen::Hud;
        if cx.paused() {
            cx.leave_pause();
        }
    }

    fn input(&self, cx: &ModeContext) -> Input {
        let k = |c: KeyCode| cx.key_held(c);
        let pd = cx.pad();
        let axis = |pos: bool, neg: bool, stick: f32| ((pos as i32 - neg as i32) as f32 + stick).clamp(-1.0, 1.0);
        let (yaw, pitch) = cx.camera_angles();
        Input {
            forward: axis(k(KeyCode::KeyW) || k(KeyCode::ArrowUp), k(KeyCode::KeyS) || k(KeyCode::ArrowDown), pd.movement.y),
            strafe: axis(k(KeyCode::KeyD) || k(KeyCode::ArrowRight), k(KeyCode::KeyA) || k(KeyCode::ArrowLeft), pd.movement.x),
            jump: k(KeyCode::Space) || pd.down(pad::JUMP),
            sneak: k(KeyCode::ShiftLeft) || k(KeyCode::ShiftRight) || pd.down(pad::CROUCH),
            sprint: k(KeyCode::ControlLeft) || k(KeyCode::ControlRight) || pd.down(pad::SPRINT),
            yaw,
            pitch,
            mine: self.mine_held || self.script_mine || pd.triggers.y > 0.5 || pd.down(pad::FIRE),
            use_pressed: self.use_pressed || pd.hit(pad::AIM),
            use_held: self.use_held || pd.triggers.x > 0.5 || pd.down(pad::AIM),
        }
    }

    fn select(&mut self, slot: usize) {
        if let Some(p) = self.play.as_mut() {
            p.inv.selected = slot.min(8);
        }
    }

    fn events(&mut self, cx: &mut ModeContext) {
        let Some(p) = self.play.as_mut() else { return };
        let evs = p.take_events();
        for e in evs {
            match e {
                Event::Dig(m) => self.play_sound(cx, Clip::Dig(m), 0.5),
                Event::Broke(m) => self.play_sound(cx, Clip::Broke(m), 0.8),
                Event::Place(m) => self.play_sound(cx, Clip::Place(m), 0.7),
                Event::Step(m) => self.play_sound(cx, Clip::Step(m), 0.35),
                Event::Hurt => self.play_sound(cx, Clip::Hurt, 0.8),
                Event::Pop => self.play_sound(cx, Clip::Pop, 0.4),
                Event::Eat => self.play_sound(cx, Clip::Eat, 0.6),
                Event::Splash => self.play_sound(cx, Clip::Splash, 0.6),
                Event::Click => self.play_sound(cx, Clip::Click, 0.5),
                Event::Swing => self.play_sound(cx, Clip::Swing, 0.3),
                Event::Oink => self.play_sound(cx, Clip::Pig, 0.45),
                Event::Groan => self.play_sound(cx, Clip::Zombie, 0.5),
                Event::Fuse => self.play_sound(cx, Clip::Fuse, 0.7),
                Event::Explode => {
                    self.play_sound(cx, Clip::Explode, 1.0);
                    cx.duck_music(red_engine2::mixer::Duck { depth: 0.6, attack: 0.05, release: 1.5 }, 0.8);
                }
                Event::MobHurt(k) => {
                    self.play_sound(cx, Clip::Swing, 0.4);
                    self.play_sound(cx, if k == crate::entities::Kind::Pig { Clip::Pig } else { Clip::Hurt }, 0.45);
                }
                Event::Death => {
                    self.screen = Screen::Death;
                    if !cx.paused() {
                        cx.enter_pause();
                    }
                }
                Event::Open(o) => {
                    let s = match o {
                        Opened::Table(at) => Screen::Table(at),
                        Opened::Furnace(at) => Screen::Furnace(at),
                        Opened::Chest(at) => Screen::Chest(at),
                    };
                    self.open(cx, s);
                }
                Event::Dawn(d) => self.message = Some((format!("Day {}", d + 1), 1.0)),
            }
        }
    }

    /// Hands the renderer this frame's world: its meshes, and the shader's numbers.
    fn feed_gpu(&mut self, cx: &mut ModeContext) {
        let eye = cx.camera_position();
        let tod = cx.time_of_day().unwrap_or(0.4);
        let day = crate::tool::daylight(tod * 24.0);
        let origin = red_engine2::render::render_origin(eye);
        let clear = std::mem::take(&mut self.clear_layer);
        let updates = if let Some(p) = self.play.as_mut() { Some(p.terrain.take_updates()) } else { self.panorama.as_mut().map(|t| t.take_updates()) };
        let mut params = [[0.0f32; 4]; 8];
        params[2][0] = day;
        if let Some(p) = &self.play {
            if let (Some(t), Screen::Hud) = (p.target, &self.screen) {
                let stage = p.mining.filter(|m| m.pos == t.pos && m.needs > 0.0).map_or(0.0, |m| (m.done / m.needs * 10.0).floor().clamp(0.0, 9.0) + 1.0);
                let c = t.pos.as_vec3() - origin;
                params[0] = [c.x, c.y, c.z, stage];
                params[1][0] = 1.0;
            }
        }
        // The held item, lit by where the player stands.
        let (yaw, pitch) = cx.camera_angles();
        let held = self.play.as_ref().filter(|p| p.ready && p.dead.is_none() && matches!(self.screen, Screen::Hud)).and_then(|p| {
            let item = p.inv.held_item()?;
            let c = eye.floor().as_ivec3();
            let (sky, glow) = p.terrain.world.light(c.x, c.y, c.z);
            Some((item, sky, glow, p.swing))
        });
        let bob = self.bob;
        let Some((device, live)) = cx.renderer() else { return };
        let mut models = Vec::new();
        if let Some((item, sky, glow, swing)) = held {
            let entry = self.hand_meshes.entry((item, sky, glow)).or_insert_with(|| {
                let (m, painted) = crate::hand::item_mesh(item, sky as f32 / 15.0, glow as f32 / 15.0);
                (std::sync::Arc::new(red_engine2::gpu::GpuMesh::upload(device, &m)), painted)
            });
            let shape = item.block().map(|b| b.info().shape);
            let cube = matches!(shape, Some(crate::block::Shape::Cube | crate::block::Shape::Cactus | crate::block::Shape::Liquid));
            let mut world = crate::hand::hand_transform(eye, yaw, pitch, swing, bob, cube);
            // A torch or a flower is a small model in its cell: held smaller.
            if matches!(shape, Some(crate::block::Shape::Torch(_) | crate::block::Shape::Plant(_))) {
                world *=
                    Mat4::from_translation(Vec3::new(0.5, 0.3, 0.5)) * Mat4::from_scale(Vec3::splat(0.7)) * Mat4::from_translation(Vec3::new(-0.5, -0.3, -0.5));
            }
            models.push(crate::gpu::Model { mesh: entry.0.clone(), world, painted: entry.1, emissive: Vec3::ZERO });
        }
        if let Some(p) = &self.play {
            let t = self.since_start;
            for e in p.entities.iter().filter(|e| (e.body.pos - eye).length() < 80.0) {
                let c = (e.body.pos + Vec3::Y * 0.5).floor().as_ivec3();
                let (sky, glow) = p.terrain.world.light(c.x, c.y, c.z);
                let mut emissive = Vec3::new(0.6, 0.0, 0.0) * e.hurt;
                if e.burning {
                    emissive += Vec3::new(0.8, 0.35, 0.05) * (0.6 + 0.4 * (t * 17.0 + e.id as f32).sin());
                }
                if e.fuse > 0.0 && ((e.fuse * if e.kind == crate::entities::Kind::Tnt { 4.0 } else { 6.0 }) as i32) % 2 == 1 {
                    emissive += Vec3::splat(0.8);
                }
                if e.dying >= 0.0 {
                    emissive += Vec3::new(0.5, 0.0, 0.0);
                }
                match e.kind {
                    crate::entities::Kind::Item(st) => {
                        let entry = self.hand_meshes.entry((st.item, sky, glow)).or_insert_with(|| {
                            let (m, painted) = crate::hand::item_mesh(st.item, sky as f32 / 15.0, glow as f32 / 15.0);
                            (std::sync::Arc::new(red_engine2::gpu::GpuMesh::upload(device, &m)), painted)
                        });
                        let cube = st.item.block().is_some();
                        let size = if cube { 0.25 } else { 0.4 };
                        let bob = (e.age * 2.5 + e.id as f32).sin() * 0.06 + 0.12;
                        let world = Mat4::from_translation(e.body.pos + Vec3::Y * bob)
                            * Mat4::from_rotation_y(e.age * 1.6 + e.id as f32)
                            * Mat4::from_scale(Vec3::splat(size))
                            * Mat4::from_translation(Vec3::new(-0.5, 0.0, -0.5));
                        models.push(crate::gpu::Model { mesh: entry.0.clone(), world, painted: entry.1, emissive });
                        // A stack shows as two.
                        if st.count > 1 {
                            let w2 = world * Mat4::from_translation(Vec3::new(0.25, 0.3, 0.2));
                            models.push(crate::gpu::Model { mesh: entry.0.clone(), world: w2, painted: entry.1, emissive });
                        }
                    }
                    crate::entities::Kind::Tnt => {
                        let item = Item::of(Block::TNT);
                        let entry = self.hand_meshes.entry((item, sky, glow)).or_insert_with(|| {
                            let (m, painted) = crate::hand::item_mesh(item, sky as f32 / 15.0, glow as f32 / 15.0);
                            (std::sync::Arc::new(red_engine2::gpu::GpuMesh::upload(device, &m)), painted)
                        });
                        let swell = 1.0 + (1.0 - (e.fuse / 0.6).min(1.0)) * 0.15;
                        let world = Mat4::from_translation(e.body.pos)
                            * Mat4::from_scale(Vec3::splat(0.98 * swell))
                            * Mat4::from_translation(Vec3::new(-0.5, 0.0, -0.5));
                        models.push(crate::gpu::Model { mesh: entry.0.clone(), world, painted: true, emissive });
                    }
                    _ => {
                        for (tex, world) in crate::entities::pose(e) {
                            let mesh = self.part_meshes.entry((tex, sky, glow)).or_insert_with(|| {
                                std::sync::Arc::new(red_engine2::gpu::GpuMesh::upload(
                                    device,
                                    &crate::entities::cube_mesh(tex, sky as f32 / 15.0, glow as f32 / 15.0),
                                ))
                            });
                            models.push(crate::gpu::Model { mesh: mesh.clone(), world, painted: true, emissive });
                        }
                    }
                }
            }
        }
        if let Some(p) = &self.play {
            for q in p.particles.iter().filter(|q| (q.pos - eye).length() < 24.0) {
                let c = q.pos.floor().as_ivec3();
                let (sky, glow) = p.terrain.world.light(c.x, c.y, c.z);
                let mesh = self.part_meshes.entry((q.tex, sky, glow)).or_insert_with(|| {
                    std::sync::Arc::new(red_engine2::gpu::GpuMesh::upload(device, &crate::entities::cube_mesh(q.tex, sky as f32 / 15.0, glow as f32 / 15.0)))
                });
                let s = q.size * (1.0 - (q.age / q.life).powi(3));
                let world = Mat4::from_translation(q.pos)
                    * Mat4::from_rotation_y(q.life * 7.0)
                    * Mat4::from_scale(Vec3::splat(s))
                    * Mat4::from_translation(Vec3::splat(-0.5));
                models.push(crate::gpu::Model { mesh: mesh.clone(), world, painted: true, emissive: Vec3::ZERO });
            }
        }
        if self.hand_meshes.len() > 512 {
            self.hand_meshes.clear();
        }
        if self.part_meshes.len() > 1024 {
            self.part_meshes.clear();
        }
        live.set_game_params(params);
        if let Some(layer) = live.layer_mut::<crate::gpu::VoxelLayer>() {
            if clear {
                layer.clear();
            }
            if let Some(u) = updates {
                layer.push(u);
            }
            layer.set_models(models);
        }
    }

    /// A click (left or right) at the mouse on the screen that is up.
    fn click(&mut self, cx: &mut ModeContext, right: bool, shift: bool) -> Answer {
        let Some(hit) = self.painted.as_ref().and_then(|p| p.at(self.mouse.0, self.mouse.1)) else {
            // Outside every panel: a carried stack is let go of (back into the inventory).
            return Answer::Used;
        };
        let screen = self.screen.clone();
        match hit {
            Hit::Slot(r) => {
                if let Some(p) = self.play.as_mut() {
                    click_slot(p, &screen, r, right, shift, &self.creative_items);
                }
            }
            Hit::Button(b) => {
                self.play_sound(cx, Clip::Click, 0.5);
                return self.button(cx, b);
            }
            Hit::Panel => {}
        }
        Answer::Used
    }

    fn button(&mut self, cx: &mut ModeContext, b: Button) -> Answer {
        match b {
            Button::Resume => self.close(cx),
            Button::Music => cx.toggle_music(),
            Button::Sound => cx.toggle_sfx(),
            Button::View => {
                let steps = [4, 6, 8, 10, 12, 16];
                let i = steps.iter().position(|v| *v == self.opts.view).unwrap_or(3);
                self.opts.view = steps[(i + 1) % steps.len()];
                if let Some(p) = self.play.as_mut() {
                    p.terrain.view = self.opts.view;
                }
                self.save_options();
            }
            Button::AutoJump => {
                self.opts.auto_jump = !self.opts.auto_jump;
                if let Some(p) = self.play.as_mut() {
                    p.body.auto_jump = self.opts.auto_jump;
                }
                self.save_options();
            }
            Button::Fullscreen => cx.toggle_fullscreen(),
            Button::QuitTitle | Button::Title => self.back_to_title(cx),
            Button::QuitGame => {
                self.save(cx);
                return Answer::Quit;
            }
            Button::Continue => {
                if let Some((dir, _)) = self.worlds.first().cloned() {
                    self.open_world(cx, dir);
                }
            }
            Button::NewWorld => {
                let name = format!("World {}", self.worlds.len() + 1);
                self.screen = Screen::Title(Title::NewWorld { name, seed: String::new(), creative: false, focus: Button::NameField });
            }
            Button::LoadWorld => self.screen = Screen::Title(Title::Worlds { page: 0 }),
            Button::World(i) => {
                if let Some((dir, _)) = self.worlds.get(i).cloned() {
                    self.open_world(cx, dir);
                }
            }
            Button::Create => {
                if let Screen::Title(Title::NewWorld { name, seed, creative, .. }) = self.screen.clone() {
                    let seed = seed_of(&seed);
                    let name = if name.trim().is_empty() { "World".to_string() } else { name.trim().to_string() };
                    self.create_world(cx, &name, seed, if creative { Mode::Creative } else { Mode::Survival });
                }
            }
            Button::Cancel => self.screen = Screen::Title(Title::Main),
            Button::ModeToggle => {
                if let Screen::Title(Title::NewWorld { creative, .. }) = &mut self.screen {
                    *creative = !*creative;
                }
            }
            Button::NameField | Button::SeedField => {
                if let Screen::Title(Title::NewWorld { focus, .. }) = &mut self.screen {
                    *focus = b;
                }
            }
            Button::Respawn => {
                if let Some(p) = self.play.as_mut() {
                    p.respawn();
                }
                self.close(cx);
            }
            Button::PageUp | Button::PageDown => {
                let d: i32 = if b == Button::PageUp { -1 } else { 1 };
                match &mut self.screen {
                    Screen::Creative { page } => *page = (*page as i32 + d).max(0) as usize,
                    Screen::Title(Title::Worlds { page }) => *page = (*page as i32 + d).max(0) as usize,
                    _ => {}
                }
            }
        }
        Answer::Used
    }

    /// Typing into the new-world form.
    fn type_key(&mut self, event: &winit::event::KeyEvent) -> bool {
        let Screen::Title(Title::NewWorld { name, seed, focus, .. }) = &mut self.screen else { return false };
        let field = if *focus == Button::SeedField { seed } else { name };
        match &event.logical_key {
            Key::Named(NamedKey::Backspace) => {
                field.pop();
            }
            Key::Named(NamedKey::Tab) => {
                *focus = if *focus == Button::NameField { Button::SeedField } else { Button::NameField };
            }
            _ => {
                if let Some(t) = &event.text {
                    for ch in t.chars() {
                        if !ch.is_control() && field.chars().count() < 28 {
                            field.push(ch);
                        }
                    }
                }
            }
        }
        true
    }

    fn update_camera(&mut self, cx: &mut ModeContext, dt: f32) {
        let (yaw, pitch) = cx.camera_angles();
        let Some(p) = &self.play else { return };
        let alpha = (p.acc / crate::play::TICK).clamp(0.0, 1.0);
        let mut eye = p.eye(alpha);
        // A gentle bob while walking on the ground.
        let speed = glam::Vec2::new(p.body.vel.x, p.body.vel.z).length();
        if p.body.on_ground && speed > 0.5 {
            self.bob += dt * speed * 1.9;
        }
        let k = (speed / 5.0).min(1.0) * if p.body.on_ground { 1.0 } else { 0.0 };
        eye += Vec3::Y * (self.bob * 2.0).sin().abs() * 0.045 * k;
        let feet = p.body.pos;
        cx.teleport(feet, yaw);
        cx.set_camera(eye, yaw, pitch);
        let want = 72.0 + if p.body.sprinting { 8.0 } else { 0.0 } + if p.body.flying && p.body.sprinting { 6.0 } else { 0.0 };
        self.view_fov += (want - self.view_fov) * (dt * 10.0).min(1.0);
        cx.set_fov(self.view_fov);
    }

    fn step_panorama(&mut self, cx: &mut ModeContext, dt: f32) {
        let t = self.panorama.get_or_insert_with(|| crate::stream::Terrain::new(crate::world::World::new(PANORAMA_SEED), 6, workers()));
        let (x, y, z) = t.world.gen.spawn_point();
        let eye = Vec3::new(x as f32 + 0.5, y as f32 + 16.0, z as f32 + 0.5);
        t.update(eye);
        self.pano_angle += dt * 0.035;
        cx.set_camera(eye, self.pano_angle, -0.16);
        cx.set_fov(80.0);
    }
}

#[derive(Clone, Copy)]
enum Clip {
    Dig(crate::block::Sound),
    Broke(crate::block::Sound),
    Place(crate::block::Sound),
    Step(crate::block::Sound),
    Hurt,
    Pop,
    Eat,
    Splash,
    Click,
    Swing,
    Pig,
    Zombie,
    Fuse,
    Explode,
}

/// Where the haze begins and how thick it grows for a world that reaches `view` chunks: clear for the nearer half, nine tenths hazed at the edge.
pub fn haze_for(view: i32) -> (f32, f32) {
    let edge = view as f32 * 16.0;
    let start = edge * 0.5;
    (start, (10.0f32).ln() / (edge - start))
}

/// A seed from what was typed: a number as it is, words by their hash, nothing for a random one.
pub fn seed_of(text: &str) -> Option<u32> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    Some(match t.parse::<i64>() {
        Ok(n) => (n as u32) & 0x00ff_ffff,
        Err(_) => t.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619)) & 0x00ff_ffff,
    })
}

impl GameMode for CraftMode {
    fn name(&self) -> &'static str {
        "redcraft"
    }

    fn start(&mut self, cx: &mut ModeContext) {
        let mut g = Box::new(Game::new(cx.settings_key().to_string(), cx.has_audio()));
        g.refresh_worlds();
        if let Ok(name) = std::env::var("REDCRAFT_WORLD") {
            let dir = g.worlds_dir().join(crate::save::slug(&name));
            if dir.join("level.json").exists() {
                g.open_world(cx, dir);
            } else {
                let seed = std::env::var("REDCRAFT_SEED").ok().and_then(|s| seed_of(&s));
                let mode = if std::env::var("REDCRAFT_MODE").as_deref() == Ok("creative") { Mode::Creative } else { Mode::Survival };
                let play = Play::create(dir, &name, seed.unwrap_or(1), mode, g.opts.view, workers());
                g.enter(cx, play, None);
            }
        } else {
            cx.set_time_of_day(0.33);
            cx.enter_pause();
        }
        self.0 = Some(g);
    }

    fn frame(&mut self, cx: &mut ModeContext, dt: f32) {
        let Some(g) = self.0.as_mut() else { return };
        if g.play.is_none() {
            return;
        }
        if !matches!(g.screen, Screen::Hud) {
            // A screen is up: the world waits, the view stays where it was.
            g.update_camera(cx, 0.0);
            return;
        }
        let input = g.input(cx);
        g.use_pressed = false;
        // The hotbar from the pad's bumpers.
        let pd = cx.pad();
        if pd.hit(pad::NEXT) || pd.hit(pad::PREVIOUS) {
            let d = if pd.hit(pad::NEXT) { 1 } else { 8 };
            let sel = g.play.as_ref().map_or(0, |p| (p.inv.selected + d) % 9);
            g.select(sel);
        }
        if pd.hit(pad::INTERACT) {
            let s = if g.play.as_ref().is_some_and(Play::creative) { Screen::Creative { page: 0 } } else { Screen::Inventory };
            g.open(cx, s);
            return;
        }
        if let Some(tod) = cx.time_of_day() {
            if let Some(p) = g.play.as_mut() {
                p.set_time(tod);
                p.daylight = crate::tool::daylight(tod * 24.0);
            }
        }
        if let Some(p) = g.play.as_mut() {
            p.advance(&input, dt);
        }
        g.events(cx);
        // The name of what is in hand and a message fade with the game's time.
        if let Some((_, a)) = g.held_name.as_mut() {
            *a -= dt;
        }
        g.held_name = g.held_name.take().filter(|(_, a)| *a > 0.0).map(|(n, a)| (n, a.min(1.0)));
        if let Some((_, a)) = g.message.as_mut() {
            *a -= dt * 0.4;
        }
        g.message = g.message.take().filter(|(_, a)| *a > 0.0);
        if g.play.as_ref().is_some_and(|p| p.save_in <= 0.0) {
            g.save(cx);
        }
        g.update_camera(cx, dt);
    }

    fn input_frozen(&self) -> bool {
        true
    }

    fn owns_hud(&self) -> bool {
        true
    }

    fn owns_pause(&self) -> bool {
        true
    }

    fn repaint_pause(&mut self, cx: &mut ModeContext) {
        if let Some(g) = self.0.as_mut() {
            // The client paused (Esc, the window lost focus): show the game menu, and keep the world safe.
            if matches!(g.screen, Screen::Hud) && g.play.is_some() {
                g.screen = Screen::Pause;
                g.save(cx);
            }
            g.painted_key = 0;
            g.paint(cx);
        }
    }

    fn left_pause(&mut self) {
        if let Some(g) = self.0.as_mut() {
            if g.screen.has_slots() || matches!(g.screen, Screen::Pause) {
                if let Some(p) = g.play.as_mut() {
                    p.return_carried();
                }
                g.screen = Screen::Hud;
            }
        }
    }

    fn sync_gpu(&mut self, cx: &mut ModeContext) {
        let Some(g) = self.0.as_mut() else { return };
        let now = Instant::now();
        let dt = now.duration_since(g.clock).as_secs_f32().min(0.25);
        g.clock = now;
        g.since_start += dt;
        if dt > 0.0 {
            g.fps += (1.0 / dt - g.fps) * 0.05;
        }
        // Keep the screen in step with the client's pause (Esc toggles it there).
        if !cx.paused() && (g.screen.has_slots() || g.screen == Screen::Pause) {
            {
                if let Some(p) = g.play.as_mut() {
                    p.return_carried();
                }
                g.screen = Screen::Hud;
            }
        }
        if matches!(g.screen, Screen::Title(_)) {
            g.step_panorama(cx, dt);
        } else if !cx.paused() {
            if let Some(p) = g.play.as_mut() {
                if !p.ready {
                    // Stream while the first chunks load, before the body is placed.
                    p.terrain.update(p.body.pos);
                }
            }
        }
        // What is in hand: its name for a moment when it changes.
        if let Some(p) = &g.play {
            let held = p.inv.held_item();
            if held != g.last_selected {
                g.last_selected = held;
                g.held_name = held.map(|i| (i.title().to_string(), 2.5));
            }
        }
        // The haze hides the world's edge wherever the render distance puts it.
        let view = if g.play.is_some() { g.opts.view } else { 6 };
        let (start, density) = haze_for(view);
        cx.set_haze(start, density);
        g.feed_gpu(cx);
        g.paint(cx);
    }

    fn window_event(&mut self, cx: &mut ModeContext, event: &WindowEvent) -> Answer {
        let Some(g) = self.0.as_mut() else { return Answer::Pass };
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                g.mouse = (position.x as f32, position.y as f32);
                if !matches!(g.screen, Screen::Hud) {
                    g.paint(cx);
                }
                Answer::Pass
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = *state == ElementState::Pressed;
                if !matches!(g.screen, Screen::Hud) {
                    if pressed {
                        let shift = cx.key_held(KeyCode::ShiftLeft) || cx.key_held(KeyCode::ShiftRight);
                        let a = g.click(cx, *button == MouseButton::Right, shift);
                        g.paint(cx);
                        return a;
                    }
                    return Answer::Used;
                }
                if !cx.grabbed() {
                    return Answer::Pass;
                }
                match button {
                    MouseButton::Left => g.mine_held = pressed,
                    MouseButton::Right => {
                        g.use_held = pressed;
                        if pressed {
                            g.use_pressed = true;
                        }
                    }
                    MouseButton::Middle if pressed => {
                        // Pick block (creative): the aimed block into the hand.
                        if let Some(p) = g.play.as_mut() {
                            if p.creative() {
                                if let Some(t) = p.target {
                                    p.inv.pick(Item::of(t.block.base()));
                                }
                            }
                        }
                    }
                    _ => {}
                }
                Answer::Used
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y,
                    MouseScrollDelta::PixelDelta(p) => (p.y / 40.0) as f32,
                };
                if dy == 0.0 {
                    return Answer::Used;
                }
                match &mut g.screen {
                    Screen::Hud => {
                        if let Some(p) = g.play.as_mut() {
                            let step = if dy > 0.0 { 8 } else { 1 };
                            p.inv.selected = (p.inv.selected + step) % 9;
                        }
                    }
                    Screen::Creative { page } => {
                        let pages = g.creative_items.len().div_ceil(63);
                        *page = if dy > 0.0 { page.saturating_sub(1) } else { (*page + 1).min(pages.saturating_sub(1)) };
                        g.paint(cx);
                    }
                    _ => {}
                }
                Answer::Used
            }
            WindowEvent::KeyboardInput { event: key, .. } => {
                if key.state != ElementState::Pressed {
                    return if matches!(g.screen, Screen::Hud) { Answer::Pass } else { Answer::Used };
                }
                let PhysicalKey::Code(code) = key.physical_key else { return Answer::Pass };
                match &g.screen {
                    Screen::Title(t) => {
                        let t = t.clone();
                        if code == KeyCode::Escape {
                            g.screen = Screen::Title(Title::Main);
                        } else if code == KeyCode::Enter && matches!(t, Title::NewWorld { .. }) {
                            return g.button(cx, Button::Create);
                        } else if matches!(t, Title::NewWorld { .. }) {
                            g.type_key(key);
                        }
                        g.paint(cx);
                        Answer::Used
                    }
                    Screen::Hud => {
                        let Some(p) = g.play.as_mut() else { return Answer::Pass };
                        match code {
                            KeyCode::Digit1
                            | KeyCode::Digit2
                            | KeyCode::Digit3
                            | KeyCode::Digit4
                            | KeyCode::Digit5
                            | KeyCode::Digit6
                            | KeyCode::Digit7
                            | KeyCode::Digit8
                            | KeyCode::Digit9 => {
                                let n = format!("{code:?}").chars().last().and_then(|c| c.to_digit(10)).unwrap_or(1) as usize;
                                p.inv.selected = n - 1;
                                Answer::Used
                            }
                            KeyCode::KeyE => {
                                let s = if p.creative() { Screen::Creative { page: 0 } } else { Screen::Inventory };
                                g.open(cx, s);
                                Answer::Used
                            }
                            KeyCode::KeyQ => {
                                // Throwing what is in hand: one (all of it with Ctrl).
                                let all = cx.key_held(KeyCode::ControlLeft);
                                let (yaw, pitch) = cx.camera_angles();
                                p.throw(all, yaw, pitch);
                                Answer::Used
                            }
                            KeyCode::F3 => {
                                g.debug = !g.debug;
                                Answer::Used
                            }
                            KeyCode::ShiftLeft | KeyCode::ShiftRight | KeyCode::ControlLeft | KeyCode::ControlRight => {
                                // Ours (sneak, sprint): the client's crouch and sprint never see them... but scripts and `key_held` do.
                                Answer::Pass
                            }
                            _ => Answer::Pass,
                        }
                    }
                    _ => {
                        if code == KeyCode::Escape || (code == KeyCode::KeyE && g.screen.has_slots()) {
                            if g.screen != Screen::Death {
                                g.close(cx);
                            }
                            return Answer::Used;
                        }
                        Answer::Used
                    }
                }
            }
            _ => Answer::Pass,
        }
    }

    fn script_key(&mut self, cx: &mut ModeContext, name: &str) -> bool {
        self.0.as_mut().is_some_and(|g| g.script(cx, name))
    }

    fn footstep(&mut self, _cx: &mut ModeContext) -> bool {
        true
    }

    fn fx(&self, fx: FxParams) -> FxParams {
        let Some(p) = self.0.as_ref().and_then(|g| g.play.as_ref()) else { return fx };
        let mut fx = fx;
        if p.body.head_in_water {
            fx.flash = [0.05, 0.16, 0.45, 0.42];
        } else if p.body.in_lava {
            fx.flash = [0.9, 0.35, 0.05, 0.7];
        }
        if p.hurt_flash > 0.0 {
            fx.vignette = [0.8, 0.05, 0.05, p.hurt_flash * 0.6];
        }
        if p.dead.is_some() {
            fx.flash = [0.4, 0.0, 0.0, 0.35];
        }
        fx
    }

    fn dump(&self) -> serde_json::Value {
        self.0.as_ref().map_or(serde_json::Value::Null, |g| g.dump_json())
    }

    fn closing(&mut self, cx: &mut ModeContext) {
        if let Some(g) = self.0.as_mut() {
            g.save(cx);
        }
    }

    fn typing(&self) -> bool {
        self.0.as_ref().is_some_and(|g| matches!(g.screen, Screen::Title(Title::NewWorld { .. })))
    }
}

/// The block the player is standing in (for tests).
pub fn feet_block(p: &Play) -> Block {
    p.terrain.world.block_at(p.body.pos.floor().as_ivec3())
}
