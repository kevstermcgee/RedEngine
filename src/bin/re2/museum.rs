//! A museum scene (`museum` block) in the live client: the visit ([`red_engine2::museum::Museum`]) fed with the player every frame, its flights into and out
//! of paintings driving the camera, its rooms' lamps lighting the scene, its sounds, its music steered to the wing, its labels, its pause menu and its map.
//! Words are set in the player's own serif ([`red_engine2::ui::serif`]).

use super::*;
use red_engine2::museum::exhibit::Kind;
use red_engine2::museum::map::{key_glyph, MapView};
use red_engine2::museum::sound::{touch, Sounds};
use red_engine2::museum::state::{yaw_of, Event, Place, Target};
use red_engine2::museum::{Museum, Progress, RoomId};
use red_engine2::ui::serif::{Face, Image, Serif};

/// A button of the museum's pause menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Button {
    Resume,
    Map,
    Music,
    Sound,
    Steps,
    StepOut,
    Fullscreen,
    Quit,
}

/// The museum's part of the client.
pub(crate) struct MuseumClient {
    pub(crate) visit: Museum,
    serif: Serif,
    sounds: Option<Sounds>,
    /// The player pressed interact since the last frame.
    pub(crate) interact: bool,
    /// The big words in the middle of the screen (a room's name, a painting's title) and how long they have shown.
    title: Option<(String, String, f32)>,
    /// A line of news at the bottom (a key found, a locked door).
    toast: Option<(String, f32)>,
    target: Option<Target>,
    /// How far the label has faded in (0..1).
    label: f32,
    pub(crate) map_open: bool,
    map_view: MapView,
    drag: Option<(f32, f32)>,
    hover: Option<Button>,
    hud_key: Option<String>,
    save_in: f32,
    step: u32,
}

const GOLD: [u8; 4] = [242, 204, 128, 255];
const INK: [u8; 4] = [244, 236, 220, 255];
const DIM: [u8; 4] = [196, 186, 168, 235];

fn progress_path(key: &str) -> Option<std::path::PathBuf> {
    red_engine2::settings::game_state_path(key, "museum.json")
}

impl App {
    /// Starts the visit when the scene is a museum: the saved one, or a new museum from this launch's seed.
    pub(crate) fn start_museum(&mut self) {
        let Some(spec) = self.scene.museum.clone() else { return };
        let saved = progress_path(&self.settings_key)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| Progress::from_json(&v))
            .filter(|_| std::env::var("RE2_MUSEUM_FRESH").is_err());
        let seed = saved.as_ref().map_or_else(|| spec.seed.value(None), |p| p.seed);
        let visit = Museum::new(spec, seed, saved);
        println!("museum seed {seed}: {} keys found, {} rooms discovered", visit.keys(), visit.rooms_found());
        let serif = Serif::load();
        if let Some(src) = &serif.source {
            println!("museum type: {src}");
        }
        let start = visit.start();
        let sounds = self.audio.is_some().then(Sounds::render);
        self.museum = Some(MuseumClient {
            visit,
            serif,
            sounds,
            interact: false,
            title: None,
            toast: None,
            target: None,
            label: 0.0,
            map_open: false,
            map_view: MapView { centre: glam::Vec2::ZERO, zoom: 90.0 },
            drag: None,
            hover: None,
            hud_key: None,
            save_in: 30.0,
            step: 0,
        });
        self.museum_teleport(start.at, start.yaw);
        self.museum_room_title(true);
        self.museum_steer_music();
        if let Some(m) = self.museum.as_mut() {
            self.colliders = m.visit.colliders().to_vec();
            m.visit.take_colliders_changed();
        }
    }

    fn museum_teleport(&mut self, at: Vec3, yaw: f32) {
        self.physics_pos = Vec2::new(at.x, at.z);
        self.prev_physics_pos = self.physics_pos;
        self.foot_y = at.y;
        self.prev_foot_y = at.y;
        self.vertical_velocity = 0.0;
        self.horizontal_velocity = Vec2::ZERO;
        self.camera.yaw = yaw;
        self.camera.pitch = 0.0;
        self.eye = Vec3::new(at.x, at.y + self.eye_height, at.z);
        self.camera.position = self.eye;
    }

    /// The music follows the wing the player is in.
    fn museum_steer_music(&mut self) {
        let Some(m) = self.museum.as_ref() else { return };
        let theme = &m.visit.spec.collection.themes[m.visit.theme_of(m.visit.room)];
        if theme.music.is_empty() {
            return;
        }
        let music = theme.music.clone();
        if let Some(a) = self.ambient.as_mut() {
            a.steer_journey(&music);
        }
    }

    fn museum_room_title(&mut self, first: bool) {
        let Some(m) = self.museum.as_mut() else { return };
        let room = m.visit.layout.room(m.visit.room);
        let theme = &m.visit.spec.collection.themes[room.theme];
        let wing = if room.theme == room.wing_theme {
            theme.wing.clone()
        } else {
            format!("{}, visiting {}", theme.name, m.visit.spec.collection.themes[room.wing_theme].wing)
        };
        let line = if room.theme == room.wing_theme { theme.name.clone() } else { String::new() };
        let sub = if line.is_empty() { wing } else { format!("{wing}  \u{00b7}  {line}") };
        m.title = Some((format!("Gallery {}", m.visit.room.number()), if first { sub } else { sub.clone() }, 0.0));
    }

    fn museum_play(&self, clip: &[f32], gain: f32) {
        if let Some(audio) = &self.audio {
            if self.sfx_on {
                audio.play_at(clip, gain, 0.0);
            }
        }
    }

    /// A soft step on the museum's floor (or nothing, when footsteps are off). Returns whether the museum handled it.
    pub(crate) fn museum_step(&mut self) -> bool {
        let Some(m) = self.museum.as_mut() else { return false };
        if !self.settings.steps {
            return true;
        }
        m.step = m.step.wrapping_add(1);
        let i = (m.step % 3) as usize;
        let gain = if m.visit.place == Place::Halls { 0.22 } else { 0.16 };
        if let (Some(s), Some(audio)) = (&m.sounds, &self.audio) {
            if self.sfx_on {
                audio.play_at(&s.steps[i], gain, if m.step % 2 == 0 { -0.08 } else { 0.08 });
            }
        }
        true
    }

    /// One frame of the visit, after the player has moved.
    pub(crate) fn museum_frame(&mut self, dt: f32) {
        let Some(m) = self.museum.as_mut() else { return };
        if self.paused || self.card.is_some() {
            return;
        }
        let eye = self.eye;
        let look = self.camera.forward();
        let foot = Vec3::new(self.physics_pos.x, self.foot_y, self.physics_pos.y);
        let interact = std::mem::take(&mut m.interact);
        let (events, teleport) = m.visit.update(dt, eye, look, foot, interact);
        if let Some(tp) = teleport {
            self.museum_teleport(tp.at, tp.yaw);
        }
        let Some(m) = self.museum.as_mut() else { return };
        if let Some(pose) = m.visit.pose() {
            self.camera.position = pose.eye;
            self.camera.yaw = yaw_of(pose.look);
            self.camera.pitch = pose.look.y.clamp(-1.0, 1.0).asin();
        }
        self.fov_deg = m.visit.fov();
        self.camera.fov_deg = self.fov_deg;
        if m.visit.take_colliders_changed() {
            self.colliders = m.visit.colliders().to_vec();
        }
        self.scene.lights = red_engine2::museum::lamps_as_lights(&m.visit.lamps(self.camera.position, 13));
        // What the player looks at, faded in and out.
        let aim = if m.visit.free() { m.visit.aim(eye, look) } else { None };
        if aim.is_some() && aim != m.target {
            m.label = 0.0;
        }
        if aim.is_some() {
            m.target = aim;
            m.label = (m.label + dt * 3.0).min(1.0);
        } else {
            m.label = (m.label - dt * 2.5).max(0.0);
            if m.label <= 0.0 {
                m.target = None;
            }
        }
        if let Some((_, _, age)) = m.title.as_mut() {
            *age += dt;
            if *age > 6.0 {
                m.title = None;
            }
        }
        if let Some((_, age)) = m.toast.as_mut() {
            *age += dt;
            if *age > 6.0 {
                m.toast = None;
            }
        }
        m.save_in -= dt;
        let save_now = m.save_in <= 0.0;
        for e in events {
            self.museum_event(e);
        }
        if save_now {
            self.museum_save();
        }
    }

    fn museum_save(&mut self) {
        let Some(m) = self.museum.as_mut() else { return };
        m.save_in = 30.0;
        if let Some(path) = progress_path(&self.settings_key) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&path, m.visit.progress.to_json().to_string()) {
                eprintln!("could not save the museum: {e}");
            }
        }
    }

    fn museum_event(&mut self, e: Event) {
        println!("museum: {e:?}");
        self.fresh_events.push(format!("museum:{e:?}"));
        let Some(m) = self.museum.as_mut() else { return };
        match e {
            Event::EnteredRoom { first, wing_changed, .. } => {
                if first || wing_changed {
                    self.museum_room_title(first);
                }
                if wing_changed {
                    self.museum_steer_music();
                }
                if first {
                    self.museum_save();
                }
            }
            Event::SteppingIn { .. } => {
                if let Some(clip) = m.sounds.as_ref().map(|s| s.step_in.clone()) {
                    self.museum_play(&clip, 0.55);
                }
                if let Some(audio) = &self.audio {
                    audio.duck_music(red_engine2::mixer::Duck { depth: 0.5, attack: 0.4, release: 2.5 }, 1.2);
                }
            }
            Event::Arrived { depth, title } => {
                let deep = match depth {
                    1 => "inside the painting".to_string(),
                    2 => "two paintings deep".to_string(),
                    n => format!("{n} paintings deep"),
                };
                m.title = Some((title, deep, 0.0));
            }
            Event::SteppingOut { .. } => {
                if let Some(clip) = m.sounds.as_ref().map(|s| s.step_out.clone()) {
                    self.museum_play(&clip, 0.5);
                }
            }
            Event::Returned { .. } => {
                m.title = None;
            }
            Event::Touched { room, index } => {
                let piece = m.visit.contents_if_built(room).and_then(|k| k.exhibits.get(index as usize).cloned());
                if let Some(e) = piece {
                    if let Some(f) = e.family {
                        let clip = touch(f, e.art.seed);
                        self.museum_play(&clip, 0.5);
                    }
                }
            }
            Event::Revealing { room, index } => {
                let name = m.visit.contents_if_built(room).and_then(|k| k.exhibits.get(index as usize).map(|e| e.plaque.title.clone())).unwrap_or_default();
                m.toast = Some((format!("Something stirs inside \u{201c}{name}\u{201d}\u{2026}"), 0.0));
            }
            Event::KeyFound { room, keys } => {
                let doors = if m.visit.place == Place::Halls { "Its doors are opening." } else { "Its doors will be open when you return." };
                m.toast = Some((format!("The key of Gallery {}. {doors}  ({keys} found)", room.number()), 0.0));
                if let Some(clip) = m.sounds.as_ref().map(|s| s.key.clone()) {
                    self.museum_play(&clip, 0.6);
                }
                self.museum_save();
            }
            Event::DoorOpening { edge } => {
                let d = edge.centre().distance(self.eye);
                if let Some(clip) = m.sounds.as_ref().map(|s| s.door.clone()) {
                    self.museum_play(&clip, (0.5 / (1.0 + d * 0.08)).max(0.08));
                }
            }
            Event::Locked { room } => {
                m.toast = Some((format!("Locked. Gallery {} still keeps its key in one of its pieces.", room.number()), 0.0));
                if let Some(clip) = m.sounds.as_ref().map(|s| s.locked.clone()) {
                    self.museum_play(&clip, 0.5);
                }
            }
        }
    }

    /// Draws the museum's labels, counters and titles (and the start card) over the frame, when anything about them changed.
    pub(crate) fn sync_museum_hud(&mut self) {
        let Some((w, h)) = self.window_size() else { return };
        let card = self.card_content().map(|(c, button)| (c.title.clone(), c.text.clone(), c.button.clone(), button));
        let card_layout = self.card_layout_now(w, h);
        let Some(m) = self.museum.as_mut() else { return };
        let q = |x: f32| (x * 20.0).round() as i32;
        let title_alpha = m.title.as_ref().map_or(0.0, |(_, _, age)| fade(*age, 0.8, 4.4, 1.4));
        let toast_alpha = m.toast.as_ref().map_or(0.0, |(_, age)| fade(*age, 0.3, 4.5, 1.2));
        let key = format!(
            "{w}x{h}|{}|{}|{:?}|{}|{}|{:?}|{}|{:?}|{}|{:?}|{}",
            m.visit.keys(),
            m.visit.rooms_found(),
            m.visit.room,
            q(title_alpha),
            q(toast_alpha),
            m.target,
            q(m.label),
            card.as_ref().map(|c| c.0.clone()),
            m.visit.depth(),
            m.title.as_ref().map(|t| t.0.clone()),
            m.visit.free()
        );
        if m.hud_key.as_deref() == Some(key.as_str()) {
            return;
        }
        m.hud_key = Some(key);
        let mut px = vec![0u8; (w * h * 4) as usize];
        {
            let mut img = Image { w, h, px: &mut px };
            let s = (h as f32 / 1080.0).max(0.55);
            let serif = &m.serif;
            if let Some((title, text, button, _)) = &card {
                // The start card: the museum's name, how to play, the way in.
                img.rect(0, 0, w as i32, h as i32, [6, 7, 12, 200]);
                serif.draw_centred(&mut img, w as f32 * 0.5, h as f32 * 0.26, title, 96.0 * s, Face::Italic, GOLD);
                let lines = serif.wrap(text, 26.0 * s, Face::Regular, (w as f32 * 0.5).max(420.0 * s));
                let lh = serif.line_height(26.0 * s);
                for (i, l) in lines.iter().enumerate() {
                    serif.draw_centred(&mut img, w as f32 * 0.5, h as f32 * 0.43 + i as f32 * lh, l, 26.0 * s, Face::Regular, INK);
                }
                if let (Some(b), Some(layout)) = (button, &card_layout) {
                    if let Some(r) = layout
                        .widgets
                        .iter()
                        .find(|wd| wd.text.is_some() && layout.button_at(((wd.rect.0 + wd.rect.2) / 2) as f32, ((wd.rect.1 + wd.rect.3) / 2) as f32).is_some())
                        .map(|wd| wd.rect)
                    {
                        img.rect(r.0, r.1, r.2, r.3, [30, 24, 16, 230]);
                        img.frame(r.0, r.1, r.2, r.3, 2, GOLD);
                        serif.draw_centred(&mut img, (r.0 + r.2) as f32 * 0.5, (r.1 + r.3) as f32 * 0.5 - 16.0 * s, b, 30.0 * s, Face::Italic, GOLD);
                    }
                }
                serif.draw_centred(
                    &mut img,
                    w as f32 * 0.5,
                    h as f32 * 0.9,
                    "Mouse looks  \u{00b7}  W A S D walks  \u{00b7}  E steps in, or touches  \u{00b7}  M map  \u{00b7}  Esc pause",
                    18.0 * s,
                    Face::Regular,
                    DIM,
                );
            } else {
                museum_hud(m, &mut img, w, h, s, title_alpha, toast_alpha);
            }
        }
        if let Some(gpu) = self.gpu.as_mut() {
            if let Some(live) = gpu.live.as_mut() {
                live.overlay.set(&gpu.device, &gpu.queue, w, h, &px);
            }
        }
    }

    /// Hands this frame's visit to the renderer.
    pub(crate) fn museum_sync_gpu(&mut self) {
        let t = self.scene_time();
        let (Some(m), Some(gpu)) = (self.museum.as_ref(), self.gpu.as_mut()) else { return };
        if let Some(live) = gpu.live.as_mut() {
            live.sync_museum(&gpu.device, &m.visit, t, self.camera.position);
            live.set_game_params(m.visit.shader_params());
        }
    }

    /// The painting's colour washing over the screen during a flight.
    pub(crate) fn museum_wash(&self, fx: red_engine2::feel::FxParams) -> red_engine2::feel::FxParams {
        let Some(m) = self.museum.as_ref() else { return fx };
        let (w, c) = m.visit.wash();
        if w <= 0.0 {
            return fx;
        }
        red_engine2::feel::FxParams { flash: [c[0], c[1], c[2], w * 0.92], ..fx }
    }

    // ---- The pause menu and the map ----------------------------------------------------------------------------------------------------

    fn museum_buttons(&self, w: u32, h: u32) -> Vec<(Button, String, red_engine2::ui::Rect)> {
        let Some(m) = self.museum.as_ref() else { return Vec::new() };
        let s = (h as f32 / 1080.0).max(0.55);
        let mut rows: Vec<(Button, String)> = vec![(Button::Resume, "Return to the museum".into()), (Button::Map, "The plan of the museum".into())];
        if m.visit.place != Place::Halls {
            rows.push((Button::StepOut, "Step out of the painting".into()));
        }
        rows.push((Button::Music, format!("Music: {}", if self.music_on { "on" } else { "off" })));
        rows.push((Button::Sound, format!("Sounds: {}", if self.sfx_on { "on" } else { "off" })));
        rows.push((Button::Steps, format!("Footsteps: {}", if self.settings.steps { "on" } else { "off" })));
        rows.push((Button::Fullscreen, "Full screen  (F)".into()));
        rows.push((Button::Quit, "Leave the museum".into()));
        let bw = (420.0 * s) as i32;
        let bh = (52.0 * s) as i32;
        let gap = (12.0 * s) as i32;
        let total = rows.len() as i32 * (bh + gap);
        let x0 = w as i32 / 2 - bw / 2;
        let y0 = (h as i32 - total) / 2 + (70.0 * s) as i32;
        rows.into_iter().enumerate().map(|(i, (b, label))| (b, label, (x0, y0 + i as i32 * (bh + gap), x0 + bw, y0 + i as i32 * (bh + gap) + bh))).collect()
    }

    /// The museum's pause menu, or its map.
    pub(crate) fn museum_repaint_pause(&mut self) {
        let Some((w, h)) = self.window_size() else { return };
        let buttons = self.museum_buttons(w, h);
        let music_on = self.music_on;
        let player = (Vec3::new(self.physics_pos.x, self.foot_y, self.physics_pos.y), self.camera.yaw);
        let t = self.play_secs;
        let Some(m) = self.museum.as_mut() else { return };
        let mut px = vec![0u8; (w * h * 4) as usize];
        {
            let mut img = Image { w, h, px: &mut px };
            let s = (h as f32 / 1080.0).max(0.55);
            if m.map_open {
                // In a painting the map shows where its room is.
                let at = if m.visit.place == Place::Halls { player } else { (m.visit.room.centre(), player.1) };
                red_engine2::museum::map::paint(&m.visit, &m.serif, &mut img, &m.map_view, at, t);
                m.serif.draw_centred(
                    &mut img,
                    w as f32 * 0.5,
                    h as f32 - 40.0 * s,
                    "Drag to look around  \u{00b7}  scroll to zoom  \u{00b7}  M or Esc to close",
                    18.0 * s,
                    Face::Regular,
                    DIM,
                );
            } else {
                img.rect(0, 0, w as i32, h as i32, [6, 7, 12, 190]);
                let top = buttons.first().map_or(h as i32 / 3, |b| b.2 .1);
                m.serif.draw_centred(&mut img, w as f32 * 0.5, top as f32 - 190.0 * s, "Frameless", 72.0 * s, Face::Italic, GOLD);
                let line = format!(
                    "{} {} found  \u{00b7}  {} {} discovered  \u{00b7}  Gallery {}",
                    m.visit.keys(),
                    if m.visit.keys() == 1 { "key" } else { "keys" },
                    m.visit.rooms_found(),
                    if m.visit.rooms_found() == 1 { "room" } else { "rooms" },
                    m.visit.room.number()
                );
                m.serif.draw_centred(&mut img, w as f32 * 0.5, top as f32 - 92.0 * s, &line, 24.0 * s, Face::Regular, INK);
                for (b, label, r) in &buttons {
                    let hot = m.hover == Some(*b);
                    img.rect(r.0, r.1, r.2, r.3, if hot { [52, 40, 24, 235] } else { [22, 20, 26, 220] });
                    img.frame(r.0, r.1, r.2, r.3, if hot { 2 } else { 1 }, if hot { GOLD } else { [120, 104, 80, 255] });
                    m.serif.draw_centred(
                        &mut img,
                        (r.0 + r.2) as f32 * 0.5,
                        r.1 as f32 + (r.3 - r.1) as f32 * 0.5 - 15.0 * s,
                        label,
                        26.0 * s,
                        Face::Italic,
                        if hot { GOLD } else { INK },
                    );
                }
                let _ = music_on;
                m.serif.draw_centred(&mut img, w as f32 * 0.5, h as f32 - 50.0 * s, "Esc returns to the museum", 18.0 * s, Face::Regular, DIM);
            }
        }
        self.set_window_overlay(w, h, Some(&px));
    }

    /// Opens the map (over the pause menu), or closes it.
    pub(crate) fn museum_toggle_map(&mut self) {
        if self.museum.as_ref().is_some_and(|m| m.map_open) {
            if let Some(m) = self.museum.as_mut() {
                m.map_open = false;
            }
            self.leave_pause();
        } else {
            self.museum_open_map();
        }
    }

    fn museum_open_map(&mut self) {
        let player = Vec3::new(self.physics_pos.x, self.foot_y, self.physics_pos.y);
        let Some((_, h)) = self.window_size() else { return };
        if let Some(m) = self.museum.as_mut() {
            let at = if m.visit.place == Place::Halls { player } else { m.visit.room.centre() };
            m.map_view = MapView::around(at, h);
            m.map_open = true;
        }
        if !self.paused {
            self.enter_pause();
        } else {
            self.museum_repaint_pause();
        }
    }

    /// The museum's own handling of window events; true when it used the event.
    pub(crate) fn museum_window_event(&mut self, event: &WindowEvent, event_loop: &ActiveEventLoop) -> bool {
        if self.museum.is_none() || self.phase != Phase::Playing {
            return false;
        }
        let map_open = self.museum.as_ref().is_some_and(|m| m.map_open);
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return false };
                if event.state != ElementState::Pressed || event.repeat {
                    return self.paused;
                }
                if self.paused {
                    match code {
                        KeyCode::Escape | KeyCode::KeyM if map_open => {
                            if let Some(m) = self.museum.as_mut() {
                                m.map_open = false;
                            }
                            if code == KeyCode::KeyM {
                                self.leave_pause();
                            } else {
                                self.museum_repaint_pause();
                            }
                        }
                        KeyCode::KeyM => self.museum_open_map(),
                        KeyCode::Escape => self.leave_pause(),
                        _ => {}
                    }
                    return true;
                }
                if self.card.is_none() && code == KeyCode::KeyM && self.grabbed {
                    self.museum_open_map();
                    return true;
                }
                false
            }
            WindowEvent::CursorMoved { position, .. } if self.paused => {
                let (x, y) = (position.x as f32, position.y as f32);
                if let (Some((dx0, dy0)), true) = (self.museum.as_ref().and_then(|m| m.drag), map_open) {
                    if let Some(m) = self.museum.as_mut() {
                        m.map_view.centre -= glam::Vec2::new(x - dx0, y - dy0) / m.map_view.zoom;
                        m.drag = Some((x, y));
                    }
                    self.museum_repaint_pause();
                }
                self.cursor = (x, y);
                if !map_open {
                    let hover = self.window_size().and_then(|(w, h)| {
                        self.museum_buttons(w, h)
                            .into_iter()
                            .find(|(_, _, r)| x >= r.0 as f32 && x < r.2 as f32 && y >= r.1 as f32 && y < r.3 as f32)
                            .map(|b| b.0)
                    });
                    if self.museum.as_ref().is_some_and(|m| m.hover != hover) {
                        if let Some(m) = self.museum.as_mut() {
                            m.hover = hover;
                        }
                        self.museum_repaint_pause();
                    }
                }
                true
            }
            WindowEvent::MouseWheel { delta, .. } if self.paused && map_open => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                if let Some(m) = self.museum.as_mut() {
                    m.map_view.zoom = (m.map_view.zoom * 1.15f32.powf(lines)).clamp(20.0, 260.0);
                }
                self.museum_repaint_pause();
                true
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } if self.paused => {
                if map_open {
                    if let Some(m) = self.museum.as_mut() {
                        m.drag = (*state == ElementState::Pressed).then_some(self.cursor);
                    }
                    return true;
                }
                if *state != ElementState::Pressed {
                    return true;
                }
                let (x, y) = self.cursor;
                let hit = self.window_size().and_then(|(w, h)| {
                    self.museum_buttons(w, h).into_iter().find(|(_, _, r)| x >= r.0 as f32 && x < r.2 as f32 && y >= r.1 as f32 && y < r.3 as f32).map(|b| b.0)
                });
                match hit {
                    Some(Button::Resume) => self.leave_pause(),
                    Some(Button::Map) => self.museum_open_map(),
                    Some(Button::Music) => {
                        self.toggle_music();
                        self.museum_repaint_pause();
                    }
                    Some(Button::Sound) => {
                        self.toggle_sfx();
                        self.museum_repaint_pause();
                    }
                    Some(Button::Steps) => {
                        self.settings.steps = !self.settings.steps;
                        let _ = red_engine2::settings::save(&self.settings_key, &self.settings);
                        self.museum_repaint_pause();
                    }
                    Some(Button::StepOut) => {
                        let back = self.museum.as_mut().and_then(|m| m.visit.return_to_halls());
                        if let Some(tp) = back {
                            self.museum_teleport(tp.at, tp.yaw);
                            if let Some(m) = self.museum.as_mut() {
                                self.colliders = m.visit.colliders().to_vec();
                                m.visit.take_colliders_changed();
                                m.title = None;
                            }
                        }
                        self.leave_pause();
                    }
                    Some(Button::Fullscreen) => self.toggle_fullscreen(),
                    Some(Button::Quit) => {
                        self.museum_save();
                        event_loop.exit();
                    }
                    None => {}
                }
                true
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } if self.grabbed && self.card.is_none() => {
                if let Some(m) = self.museum.as_mut() {
                    m.interact = true;
                }
                true
            }
            _ => false,
        }
    }

    /// The box of a museum thing a script names (`museum:N`, `museum:frame:N`, `museum:way-back`, `museum:key`; see `Driver::locate`).
    pub(crate) fn museum_bounds(&self, what: &str) -> Option<(Vec3, Vec3)> {
        let m = self.museum.as_ref()?;
        let around = |c: Vec3, r: Vec3| Some((c - r, c + r));
        if what == "key" {
            return around(m.visit.key_at()?, Vec3::splat(0.25));
        }
        if what == "way-back" {
            let p = m.visit.pocket.as_ref()?;
            return around(red_engine2::museum::state::POCKET_ORIGIN + p.exit + Vec3::Y * 1.3, Vec3::new(0.8, 1.2, 0.1));
        }
        if let Some(side) = what.strip_prefix("door:") {
            let side = match side {
                "north" => red_engine2::museum::Side::North,
                "east" => red_engine2::museum::Side::East,
                "south" => red_engine2::museum::Side::South,
                "west" => red_engine2::museum::Side::West,
                _ => return None,
            };
            let e = red_engine2::museum::EdgeId::of(m.visit.room, side);
            return m.visit.layout.has_door(e).then(|| (e.centre() + Vec3::new(-0.3, 0.0, -0.3), e.centre() + Vec3::new(0.3, 2.0, 0.3)));
        }
        if let Some(n) = what.strip_prefix("frame:") {
            let f = m.visit.pocket.as_ref()?.frames.get(n.parse::<usize>().ok()?)?;
            return around(red_engine2::museum::state::POCKET_ORIGIN + f.at, Vec3::new(f.size.x * 0.5, f.size.y * 0.5, 0.1));
        }
        let k = m.visit.contents_if_built(m.visit.room)?;
        let e = k.exhibits.get(what.parse::<usize>().ok()?)?;
        let f = e.focus();
        let r = match e.kind {
            Kind::Sculpture | Kind::Installation => Vec3::new(e.size.x, e.size.y * 0.5, e.size.x),
            _ => Vec3::new(e.size.x * 0.5, e.size.y * 0.5, e.size.x * 0.5) * Vec3::new(e.facing.z.abs().max(0.1), 1.0, e.facing.x.abs().max(0.1)),
        };
        around(f, r)
    }

    /// The state dump's museum part.
    pub(crate) fn museum_dump(&self) -> serde_json::Value {
        let Some(m) = self.museum.as_ref() else { return serde_json::Value::Null };
        let target = m.target.as_ref().map(|t| format!("{t:?}"));
        serde_json::json!({
            "seed": m.visit.layout.seed,
            "room": [m.visit.room.x, m.visit.room.z],
            "gallery": m.visit.room.number(),
            "place": match &m.visit.place { Place::Halls => "halls".to_string(), Place::Inside(p) => format!("inside {}/{}{}", p.room.number(), p.exhibit, p.nest.iter().map(|n| format!("/{n}")).collect::<String>()) },
            "keys": m.visit.keys(),
            "rooms": m.visit.rooms_found(),
            "entered": m.visit.progress.entered,
            "deepest": m.visit.progress.deepest,
            "flying": !m.visit.free(),
            "target": target,
            "title": m.title.as_ref().map(|t| format!("{} / {}", t.0, t.1)),
            "toast": m.toast.as_ref().map(|t| t.0.clone()),
            "key_floats_at": m.visit.key_at().map(|k| [k.x, k.y, k.z]),
            "fov": m.visit.fov(),
        })
    }
}

/// 0 → 1 over `rise` seconds, held until `hold`, then → 0 over `fall`.
fn fade(age: f32, rise: f32, hold: f32, fall: f32) -> f32 {
    if age < rise {
        (age / rise).clamp(0.0, 1.0)
    } else if age < hold {
        1.0
    } else {
        (1.0 - (age - hold) / fall).clamp(0.0, 1.0)
    }
}

fn alpha(c: [u8; 4], a: f32) -> [u8; 4] {
    [c[0], c[1], c[2], (c[3] as f32 * a.clamp(0.0, 1.0)) as u8]
}

/// The museum's screen during play: counters, the room or painting title, news, the label of what is looked at, a point to aim with.
fn museum_hud(m: &MuseumClient, img: &mut Image<'_>, w: u32, h: u32, s: f32, title_alpha: f32, toast_alpha: f32) {
    let serif = &m.serif;
    let (wf, hf) = (w as f32, h as f32);
    // ---- Counters, top left ----
    let x = 34.0 * s;
    let y = 30.0 * s;
    key_glyph(img, x + 10.0 * s, y + 14.0 * s, 30.0 * s);
    let keys = format!("{}", m.visit.keys());
    serif.draw(img, x + 34.0 * s + 1.0, y + 1.0, &keys, 26.0 * s, Face::Regular, [0, 0, 0, 150]);
    serif.draw(img, x + 34.0 * s, y, &keys, 26.0 * s, Face::Regular, GOLD);
    let rooms = format!("{} {}", m.visit.rooms_found(), if m.visit.rooms_found() == 1 { "room" } else { "rooms" });
    serif.draw(img, x + 80.0 * s + 1.0, y + 4.0 * s + 1.0, &rooms, 20.0 * s, Face::Italic, [0, 0, 0, 150]);
    serif.draw(img, x + 80.0 * s, y + 4.0 * s, &rooms, 20.0 * s, Face::Italic, INK);
    let place = match &m.visit.place {
        Place::Halls => format!("Gallery {}", m.visit.room.number()),
        Place::Inside(p) => format!("Gallery {}, {} deep", p.room.number(), p.depth()),
    };
    serif.draw(img, x + 1.0, y + 38.0 * s + 1.0, &place, 17.0 * s, Face::Regular, [0, 0, 0, 140]);
    serif.draw(img, x, y + 38.0 * s, &place, 17.0 * s, Face::Regular, DIM);
    // ---- A point to aim with ----
    if m.visit.free() {
        let (cx, cy) = (wf * 0.5, hf * 0.5);
        if m.target.is_some() {
            img.disc(cx, cy, 5.0 * s, [255, 236, 200, 60]);
            img.disc(cx, cy, 2.2 * s, [255, 236, 200, 220]);
        } else {
            img.disc(cx, cy, 1.8 * s, [255, 255, 255, 120]);
        }
    }
    // ---- The title in the middle ----
    if let Some((big, small, _)) = &m.title {
        if title_alpha > 0.0 {
            let y = hf * 0.24;
            serif.draw_centred(img, wf * 0.5 + 2.0, y + 2.0, big, 64.0 * s, Face::Italic, alpha([0, 0, 0, 160], title_alpha));
            serif.draw_centred(img, wf * 0.5, y, big, 64.0 * s, Face::Italic, alpha(GOLD, title_alpha));
            serif.draw_centred(img, wf * 0.5 + 1.0, y + 84.0 * s + 1.0, small, 26.0 * s, Face::Regular, alpha([0, 0, 0, 160], title_alpha));
            serif.draw_centred(img, wf * 0.5, y + 84.0 * s, small, 26.0 * s, Face::Regular, alpha(INK, title_alpha));
        }
    }
    // ---- News ----
    if let Some((text, _)) = &m.toast {
        if toast_alpha > 0.0 {
            let y = hf * 0.5 + 60.0 * s;
            let tw = serif.measure(text, 24.0 * s, Face::Italic);
            img.rect(
                (wf * 0.5 - tw * 0.5 - 24.0 * s) as i32,
                (y - 10.0 * s) as i32,
                (wf * 0.5 + tw * 0.5 + 24.0 * s) as i32,
                (y + 40.0 * s) as i32,
                alpha([10, 8, 6, 170], toast_alpha),
            );
            serif.draw_centred(img, wf * 0.5, y, text, 24.0 * s, Face::Italic, alpha(GOLD, toast_alpha));
        }
    }
    // ---- The label of what is looked at ----
    let Some(target) = &m.target else { return };
    let a = m.label;
    if a <= 0.0 {
        return;
    }
    let (head, byline, medium, story, prompt) = label_of(m, target);
    let pw = (wf * 0.56).clamp(520.0 * s, 1000.0 * s).min(wf - 40.0);
    let x0 = wf * 0.5 - pw * 0.5;
    let pad = 26.0 * s;
    let story_lines = serif.wrap(&story, 19.0 * s, Face::Regular, pw - pad * 2.0);
    let story_lines: Vec<String> = story_lines.into_iter().take(6).collect();
    let lh = serif.line_height(19.0 * s);
    let mut ph = pad * 2.0
        + 40.0 * s
        + if byline.is_empty() { 0.0 } else { 30.0 * s }
        + if medium.is_empty() { 0.0 } else { 26.0 * s }
        + story_lines.len() as f32 * lh
        + if prompt.is_empty() { 0.0 } else { 44.0 * s };
    if !story_lines.is_empty() {
        ph += 12.0 * s;
    }
    let y0 = hf - ph - 34.0 * s;
    img.rect(x0 as i32, y0 as i32, (x0 + pw) as i32, (y0 + ph) as i32, alpha([12, 10, 9, 205], a));
    img.frame(x0 as i32, y0 as i32, (x0 + pw) as i32, (y0 + ph) as i32, 1, alpha([150, 124, 84, 255], a));
    let mut y = y0 + pad;
    serif.draw(img, x0 + pad, y, &head, 32.0 * s, Face::Italic, alpha(GOLD, a));
    y += 40.0 * s;
    if !byline.is_empty() {
        serif.draw(img, x0 + pad, y, &byline, 21.0 * s, Face::Regular, alpha(INK, a));
        y += 30.0 * s;
    }
    if !medium.is_empty() {
        serif.draw(img, x0 + pad, y, &medium, 17.0 * s, Face::Italic, alpha(DIM, a));
        y += 26.0 * s;
    }
    if !story_lines.is_empty() {
        y += 12.0 * s;
        for l in &story_lines {
            serif.draw(img, x0 + pad, y, l, 19.0 * s, Face::Regular, alpha(INK, a * 0.92));
            y += lh;
        }
    }
    if !prompt.is_empty() {
        y += 14.0 * s;
        serif.draw(img, x0 + pad, y, &prompt, 21.0 * s, Face::Italic, alpha(GOLD, a));
    }
}

/// The words of a label: heading, artist and year, medium, story, what E does.
fn label_of(m: &MuseumClient, t: &Target) -> (String, String, String, String, String) {
    match t {
        Target::Exhibit { room, index } => {
            let Some(e) = m.visit.contents_if_built(*room).and_then(|k| k.exhibits.get(*index as usize)) else { return Default::default() };
            let p = &e.plaque;
            let prompt = match e.kind {
                Kind::Painting | Kind::Triptych => "E  \u{2014}  step into the painting",
                Kind::Tapestry => "E  \u{2014}  step into the tapestry",
                Kind::Sculpture | Kind::Installation => "E  \u{2014}  touch it",
            };
            (p.title.clone(), format!("{}, {}", p.artist, p.year), p.medium.clone(), p.story.clone(), prompt.to_string())
        }
        Target::Nested { index } => {
            let Some(f) = m.visit.pocket.as_ref().and_then(|p| p.frames.get(*index as usize)) else { return Default::default() };
            let p = &f.plaque;
            let deeper = if m.visit.depth() + 1 >= red_engine2::museum::exhibit::MAX_DEPTH {
                "E  \u{2014}  step into it (it goes no deeper)"
            } else {
                "E  \u{2014}  step into it"
            };
            (p.title.clone(), format!("{}, {}", p.artist, p.year), p.medium.clone(), p.story.clone(), deeper.to_string())
        }
        Target::WayBack => (
            "The way back".into(),
            String::new(),
            String::new(),
            "A frame of light, the colour of the room you came from.".into(),
            "E  \u{2014}  step out of the painting".into(),
        ),
        Target::Note { room } => {
            let text = m.visit.contents_if_built(*room).map(|k| k.note.text.clone()).unwrap_or_default();
            ("A note from the curator".into(), format!("Gallery {}", room.number()), String::new(), text, String::new())
        }
        Target::LockedDoor { room, .. } => (
            "Locked".into(),
            String::new(),
            String::new(),
            format!(
                "The doors of Gallery {} open when its key is found. Every room keeps its key in one of its pieces; the curator's note by the door may help.",
                room.number()
            ),
            String::new(),
        ),
    }
}

impl MuseumClient {
    /// Makes the next frame repaint the labels (after the pause menu covered them).
    pub(crate) fn forget_hud(&mut self) {
        self.hud_key = None;
    }

    /// Whether a room was visited (for tests).
    #[allow(dead_code)]
    pub(crate) fn visited(&self, r: RoomId) -> bool {
        self.visit.progress.visited.contains(&r)
    }
}
