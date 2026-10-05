//! Local co-op: up to four players on one screen, each a full player of the same game (ADR 2026-10-04 "Split-screen co-op").
//!
//! `App` was written for one player and keeps that player's state as flat fields (`self.camera`, `self.physics_pos`, `self.weapon` ...), and nearly everything it
//! does reads and writes them. Rather than rewrite five thousand lines, a guest's state lives here in a [`PlayerCtx`] and is **swapped in**: to run player 3, the
//! app exchanges its per-player fields with theirs, runs the very code the first player runs (movement, the tick, the camera, the body, the pad), and exchanges
//! them back. Player 1 is the app itself; one player is the case of no guests, where nothing is swapped and nothing changes.
//!
//! The pieces that are one per game stay on `App` (the scene, the rules, the clock, the props, the audio); the pieces that are one per player move with the context.
//! A test builds the context from the app, swaps it in and out, and checks that nothing is lost, so a field added to one side and forgotten on the other fails.

use super::shots::ShotRecord;
use super::*;
use red_engine2::splitscreen::{self, Device};

/// Everything about one local player that is not shared by the game.
pub(crate) struct PlayerCtx {
    /// Which player: 0 is the first, who is the app itself.
    pub slot: usize,
    /// What drives them.
    pub device: Device,
    pub camera: FpsCamera,
    pub character: Character,
    pub body: BodySpec,
    pub keys: HashSet<KeyCode>,
    pub pad: red_engine2::controller::Sample,
    pub sprint_held: bool,
    pub jump_queued: bool,
    pub attack_queued: bool,
    pub attack_held: bool,
    pub ads_held: bool,
    pub ads_blend: f32,
    pub net_pulse: [u8; 4],
    pub switch_queued: Option<i32>,
    pub weapon: Weapon,
    pub switching: Option<(Weapon, f32)>,
    pub scroll_accum: f32,
    pub ammo: Ammo,
    pub shot_cd: Cooldown,
    pub swing: MeleeSwing,
    pub switch: WeaponSwitch,
    pub since_shot: f32,
    pub flash_left: f32,
    pub eye: Vec3,
    pub swing_timer: Option<f32>,
    pub target_index: Option<usize>,
    pub pickup_target: Option<usize>,
    pub physics_pos: Vec2,
    pub prev_physics_pos: Vec2,
    pub foot_y: f32,
    pub prev_foot_y: f32,
    pub vertical_velocity: f32,
    pub horizontal_velocity: Vec2,
    pub last_move_speed: f32,
    pub eye_height: f32,
    pub fov_deg: f32,
    pub view_mode: ViewMode,
    pub player_object_index: usize,
    pub walk_phase: f32,
    pub hand_prop_transform: Mat4,
    pub flashlight_on: bool,
}

/// Exchanges every per-player field of `$app` with the same field of `$ctx`. The pattern names every field of [`PlayerCtx`] with no `..`, so a field added to the
/// struct and not to this list does not compile.
macro_rules! swap_player {
    ($app:expr, $ctx:expr; $($field:ident),* $(,)?) => {
        let PlayerCtx { $($field),* } = $ctx;
        $( std::mem::swap(&mut $app.$field, $field); )*
    };
}

impl App {
    /// Exchanges this app's per-player state with `ctx`'s (the same call twice puts everything back).
    pub(crate) fn swap_player(&mut self, ctx: &mut PlayerCtx) {
        swap_player!(
            self, ctx; slot, device, camera, character, body, keys, pad, sprint_held, jump_queued, attack_queued, attack_held, ads_held, ads_blend, net_pulse, switch_queued, weapon,
            switching, scroll_accum, ammo, shot_cd, swing, switch, since_shot, flash_left, eye, swing_timer, target_index, pickup_target, physics_pos, prev_physics_pos,
            foot_y, prev_foot_y, vertical_velocity, horizontal_velocity, last_move_speed, eye_height, fov_deg, view_mode, player_object_index, walk_phase,
            hand_prop_transform, flashlight_on
        );
    }

    /// A copy of this app's per-player state, as a context for a new player in `slot` (position and body are set by the caller).
    pub(crate) fn player_ctx_like_me(&self, slot: usize, device: Device) -> PlayerCtx {
        PlayerCtx {
            slot,
            device,
            camera: self.camera,
            character: self.character,
            body: self.body,
            keys: HashSet::new(),
            pad: Default::default(),
            sprint_held: false,
            jump_queued: false,
            attack_queued: false,
            attack_held: false,
            ads_held: false,
            ads_blend: 0.0,
            net_pulse: [0; 4],
            switch_queued: None,
            weapon: self.weapon,
            switching: None,
            scroll_accum: 0.0,
            ammo: self.ammo,
            shot_cd: Cooldown::default(),
            swing: MeleeSwing::default(),
            switch: WeaponSwitch::default(),
            since_shot: RECOIL_TIME,
            flash_left: 0.0,
            eye: self.eye,
            swing_timer: None,
            target_index: None,
            pickup_target: None,
            physics_pos: self.physics_pos,
            prev_physics_pos: self.physics_pos,
            foot_y: self.foot_y,
            prev_foot_y: self.foot_y,
            vertical_velocity: 0.0,
            horizontal_velocity: Vec2::ZERO,
            last_move_speed: 0.0,
            eye_height: self.body.stand_eye,
            fov_deg: self.scene.player.fov_deg,
            view_mode: self.view_mode,
            player_object_index: self.player_object_index,
            walk_phase: 0.0,
            hand_prop_transform: Mat4::from_scale(Vec3::splat(HIDDEN_SCALE)),
            flashlight_on: false,
        }
    }

    /// How many players share this screen.
    pub(crate) fn local_player_count(&self) -> usize {
        1 + self.locals.len()
    }

    /// Runs `f` as local player `slot` (0 is the app itself): that player's state is swapped in for the call and out again after it.
    pub(crate) fn as_player<R>(&mut self, slot: usize, f: impl FnOnce(&mut App) -> R) -> R {
        if slot == 0 || slot > self.locals.len() {
            return f(self);
        }
        let Some(mut ctx) = self.locals[slot - 1].take() else { return f(self) };
        self.swap_player(&mut ctx);
        let out = f(self);
        self.swap_player(&mut ctx);
        self.locals[slot - 1] = Some(ctx);
        out
    }

    /// Runs `f` once for each local player, first to last.
    pub(crate) fn for_each_player(&mut self, mut f: impl FnMut(&mut App, usize)) {
        for slot in 0..self.local_player_count() {
            self.as_player(slot, |app| f(app, slot));
        }
    }

    /// Adds the guests: bodies in the scene (before the renderer is built), a starting place and a device each. `players` counts the first player.
    pub(crate) fn add_local_guests(&mut self, players: usize, pads: usize, pads_only: bool) -> Result<(), String> {
        let players = players.clamp(1, splitscreen::MAX_LOCAL_PLAYERS);
        if players == 1 {
            return Ok(());
        }
        let devices = if self.headless {
            (0..players).map(|i| if i == 0 { Device::KeyboardMouse } else { Device::Pad(i - 1) }).collect()
        } else {
            splitscreen::assign(players, pads, pads_only).map_err(|e| e.to_string())?
        };
        self.device = devices[0];
        // Say, before the second player finds out, what in this scene a guest cannot do (see `splitscreen::GUEST_LIMITS`).
        for note in splitscreen::guest_limits_for(&self.scene) {
            eprintln!("note: split-screen: {note}");
        }
        // Colours of the guests' jumpers, so players can tell each other apart at a glance.
        const JUMPERS: [&str; 3] = ["#3a7fd5", "#3fae5a", "#e4b72f"];
        for slot in 1..players {
            let mut body = build_player_object(self.character);
            body.id = format!("player_body_{}", slot + 1);
            if let ObjectKind::Humanoid(h) = &mut body.kind {
                h.material.color = Track::constant(red_engine2::color::parse_hex_to_linear(JUMPERS[(slot - 1) % JUMPERS.len()]).unwrap_or(Vec3::ONE));
            }
            let index = self.scene.objects.len();
            self.scene.objects.push(body);
            let mut ctx = self.player_ctx_like_me(slot, devices[slot]);
            ctx.player_object_index = index;
            // Players start in a ring round the first player's place, facing the same way (a scene with a spawn per player uses those).
            let around = std::f32::consts::TAU * slot as f32 / players as f32;
            let place = self
                .spawns
                .get(slot)
                .map(|s| Vec2::new(s.position[0], s.position[2]))
                .unwrap_or_else(|| self.physics_pos + Vec2::new(around.cos(), around.sin()) * 1.4);
            ctx.physics_pos = place;
            ctx.prev_physics_pos = place;
            if let Some(s) = self.spawns.get(slot) {
                ctx.foot_y = s.position[1];
                ctx.prev_foot_y = s.position[1];
                ctx.camera.yaw = s.yaw_deg.to_radians();
            }
            self.locals.push(Some(ctx));
        }
        Ok(())
    }

    /// Where the guests' places on the ground are, once the ground exists: put every guest's feet on it.
    pub(crate) fn settle_guests(&mut self) {
        let ground = self.ground.clone();
        for ctx in self.locals.iter_mut().flatten() {
            let h = ground.terrain_height_at(ctx.physics_pos).unwrap_or(ctx.foot_y);
            ctx.foot_y = ctx.foot_y.max(h);
            ctx.prev_foot_y = ctx.foot_y;
        }
    }
}

/// Pixels between the views.
const GUTTER: u32 = 4;

/// One player's frame, collected from their context.
pub(crate) struct SplitFrame {
    pub cameras: Vec<red_engine2::app::ViewCamera>,
    pub layers: Vec<red_engine2::viewer::FpsLayers>,
    pub hidden: Vec<Vec<String>>,
}

impl App {
    /// The first-person layers (crosshair, the weapon in hand, the effects) for the player swapped in.
    fn fps_layers(&self) -> red_engine2::viewer::FpsLayers {
        let hud = &self.scene.hud;
        let peaceful = self.scene.player.mode.is_peaceful();
        red_engine2::viewer::FpsLayers {
            crosshair_highlighted: self.target_index.is_some(),
            weapon_transform: self.weapon_transform(),
            hand_prop_transform: self.hand_prop_transform,
            opts: FrameOptions {
                crosshair: hud.shows_crosshair(),
                viewmodel: !peaceful && !self.carrying() && !self.own_dead(),
                pickup: self.pickup_target.is_some(),
                weapon: self.shown_weapon(),
                muzzle_flash: (self.flash_left / MUZZLE_FLASH_TIME).clamp(0.0, 1.0),
                fx: self.fx_now(),
                enemy: self.aim_enemy && hud.shows_combat(),
                skin: 0,
            },
        }
    }

    /// Every local player's camera, layers and hidden objects for this frame.
    pub(crate) fn collect_split_frame(&mut self) -> SplitFrame {
        let mut frame = SplitFrame { cameras: Vec::new(), layers: Vec::new(), hidden: Vec::new() };
        self.for_each_player(|app, _| {
            frame.cameras.push(app.camera.view());
            frame.layers.push(app.fps_layers());
            let own = (app.view_mode == ViewMode::FirstPerson).then_some(app.player_object_index);
            frame.hidden.push(frame::hidden_ids(&app.net, &app.rules, &app.streaks, &app.scene, own).into_iter().map(str::to_string).collect());
        });
        frame
    }

    /// Builds the compositor for the local players now on the screen and sizes the renderer to one view. Returns the size the renderer should be built with.
    pub(crate) fn split_for(&self, device: &wgpu::Device, format: wgpu::TextureFormat, window: (u32, u32)) -> Option<red_engine2::split_gpu::SplitScreen> {
        (self.local_player_count() > 1).then(|| red_engine2::split_gpu::SplitScreen::new(device, format, window, self.local_player_count(), GUTTER))
    }

    /// Paints each player's own HUD at the size of their view (the rules' variables, what they are looking at), and the shared card over the whole window.
    pub(crate) fn sync_split_hud(&mut self, w: u32, h: u32) {
        let Some((vw, vh)) = self.split.as_ref().map(|s| s.view_size()) else { return };
        let players = self.local_player_count();
        self.split_hud_painted.resize(players, None);
        // The card (a story page, a shop) belongs to the game, not to one player.
        let card = self.card_layout_now(w, h).map(|l| l.paint().px);
        let key = format!("{:?}|{}x{}", self.card, w, h);
        if self.rule_hud_painted.as_ref().map(|p| p.2.as_str()) != Some(key.as_str()) {
            self.rule_hud_painted = Some((w, h, key));
            self.set_window_overlay(w, h, card.as_deref());
        }
        for slot in 0..players {
            let old = self.split_hud_painted[slot].clone();
            let (fingerprint, px) = self.as_player(slot, |app| {
                let vars = app.rules.vars();
                let event = app.rule_event.as_deref();
                let outcome = app.rules.ended();
                let inspected = app.target_index.and_then(|i| app.scene.objects.get(i)).map(|o| o.id.as_str()).unwrap_or("");
                let fingerprint = format!("{vars:?}|{event:?}|{outcome:?}|{inspected}|{vw}x{vh}");
                if old.as_deref() == Some(fingerprint.as_str()) {
                    return (fingerprint, None);
                }
                let layout = app.rules_overlay(vw, vh, &vars, event, outcome);
                let px = (!layout.widgets.is_empty()).then(|| layout.paint().px);
                (fingerprint, Some(px))
            });
            let (Some(px), Some(gpu), Some(split)) = (px, self.gpu.as_ref(), self.split.as_mut()) else { continue };
            split.set_hud(&gpu.device, &gpu.queue, slot, px.as_deref());
            self.split_hud_painted[slot] = Some(fingerprint);
        }
    }

    /// Draws every local player's view into the window.
    pub(crate) fn draw_split(&mut self) {
        let frame = self.collect_split_frame();
        let t = self.scene_time();
        let (Some(gpu), Some(split)) = (self.gpu.as_mut(), self.split.as_mut()) else { return };
        let Some(live) = gpu.live.as_mut() else { return };
        let Some(surface) = gpu.surface.as_ref() else { return };
        let Some((surface_tex, reconfigure)) = window::acquire_frame(surface, &gpu.device, &gpu.config) else { return };
        let view = surface_tex.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let views: Vec<red_engine2::split_gpu::PlayerView> = (0..frame.cameras.len())
            .map(|i| red_engine2::split_gpu::PlayerView { camera: frame.cameras[i], layers: Some(frame.layers[i]), hidden: &frame.hidden[i] })
            .collect();
        split.render(live, &gpu.device, &gpu.queue, &self.scene, t, &views, &view);
        gpu.queue.present(surface_tex);
        if reconfigure {
            surface.configure(&gpu.device, &gpu.config);
        }
    }

    /// Where every local player is, for the state dump (`null` for a single-player game).
    pub(crate) fn players_state(&self) -> serde_json::Value {
        use serde_json::json;
        if self.local_player_count() < 2 {
            return serde_json::Value::Null;
        }
        let first = json!({
            "slot": 1, "device": format!("{:?}", self.device), "character": self.character.name(),
            "pos": [self.physics_pos.x, self.foot_y, self.physics_pos.y], "yaw_deg": self.camera.yaw.to_degrees().rem_euclid(360.0), "weapon": self.shown_weapon().name(),
        });
        let guests = self.locals.iter().flatten().map(|c| {
            json!({
                "slot": c.slot + 1, "device": format!("{:?}", c.device), "character": c.character.name(),
                "pos": [c.physics_pos.x, c.foot_y, c.physics_pos.y], "yaw_deg": c.camera.yaw.to_degrees().rem_euclid(360.0), "weapon": c.weapon.name(),
            })
        });
        serde_json::Value::Array(std::iter::once(first).chain(guests).collect())
    }

    /// The picture of the shared screen: every player's view, composed as the window composes them, saved as a PNG.
    pub(crate) fn capture_split(&mut self, name: &str) -> Result<ShotRecord, String> {
        if self.split.is_none() {
            return Err("this game has a single player: start the client with --players N for a split picture".into());
        }
        let frame = self.collect_split_frame();
        let t = self.scene_time();
        let path = self.shots.path_for(name);
        let Some(gpu) = self.gpu.as_mut() else { return Err("there is no GPU adapter to draw with".into()) };
        let (Some(live), Some(split)) = (gpu.live.as_mut(), self.split.as_mut()) else { return Err("the game has not started yet".into()) };
        let (w, h) = (gpu.config.width, gpu.config.height);
        if !self.shots.capture.as_ref().is_some_and(|c| c.matches(gpu.config.format, w, h)) {
            self.shots.capture = Some(red_engine2::capture::Capture::new(&gpu.device, gpu.config.format, w, h).map_err(|e| e.to_string())?);
        }
        let capture = self.shots.capture.as_ref().ok_or("no capture target")?;
        let views: Vec<_> = (0..frame.cameras.len())
            .map(|i| red_engine2::split_gpu::PlayerView { camera: frame.cameras[i], layers: Some(frame.layers[i]), hidden: &frame.hidden[i] })
            .collect();
        split.render(live, &gpu.device, &gpu.queue, &self.scene, t, &views, capture.view());
        let image = capture.read_rgba(&gpu.device, &gpu.queue).map_err(|e| e.to_string())?;
        red_engine2::capture::save_png(&image, &path).map_err(|e| e.to_string())?;
        Ok(ShotRecord { name: name.to_string(), file: path, secs: self.play_secs, camera: "split".into(), flat: red_engine2::capture::flat_fraction(&image) })
    }

    /// Routes a window-sized image (the pause menu, a card, the map list) over everything: the split screen's own overlay, or the renderer's.
    pub(crate) fn set_window_overlay(&mut self, w: u32, h: u32, rgba: Option<&[u8]>) {
        let Some(gpu) = self.gpu.as_mut() else { return };
        if let Some(split) = self.split.as_mut() {
            match rgba {
                Some(px) => split.global.set(&gpu.device, &gpu.queue, w, h, px),
                None => split.global.hide(),
            }
        } else if let Some(live) = gpu.live.as_mut() {
            match rgba {
                Some(px) => live.overlay.set(&gpu.device, &gpu.queue, w, h, px),
                None => live.overlay.hide(),
            }
        }
    }

    /// Notes where every player's ears are, once a frame (a swapped-in guest cannot see the others' state, which is parked).
    pub(crate) fn note_listeners(&mut self) {
        if self.locals.is_empty() {
            return;
        }
        let me = red_engine2::mixer::Listener { pos: self.eye.to_array(), yaw: self.camera.yaw };
        let guests = self.locals.iter().flatten().map(|c| red_engine2::mixer::Listener { pos: c.eye.to_array(), yaw: c.camera.yaw });
        self.listeners = std::iter::once(me).chain(guests).collect();
    }

    /// How the sound the swapped-in guest makes is heard: the gain scale and the pan, against everyone else.
    pub(crate) fn guest_sound(&self) -> (f32, f32) {
        let others: Vec<red_engine2::mixer::Listener> = self.listeners.iter().enumerate().filter(|(i, _)| *i != self.slot).map(|(_, l)| *l).collect();
        splitscreen::guest_mix(&others, self.eye.to_array())
    }

    /// Reads the other players' gamepads and applies them (look, jump, use ...); a pad that was unplugged pauses the game and says whose it was.
    pub(crate) fn poll_guest_pads(&mut self, dt: f32) {
        if self.locals.is_empty() {
            return;
        }
        let (samples, lost) = self.pads.poll(self.focused);
        if !lost.is_empty() && !self.paused {
            let mut names = Vec::new();
            for pad in &lost {
                // `lost` is the position the pad had; the devices were assigned by position when the game started.
                let mut devices = vec![self.device];
                devices.extend(self.locals.iter().flatten().map(|c| c.device));
                names.extend(splitscreen::owners_of(&devices, *pad).into_iter().map(|p| format!("player {}", p + 1)));
            }
            if !names.is_empty() {
                self.pause_message = Some(format!("{}'s gamepad was unplugged: reconnect it and press resume", names.join(" and ")));
                self.enter_pause();
            }
        }
        for slot in 1..self.local_player_count() {
            let Some(Device::Pad(n)) = self.locals[slot - 1].as_ref().map(|c| c.device) else { continue };
            let sample = samples.get(n).copied().unwrap_or_default();
            let active = self.grabbed && !self.paused && self.card.is_none();
            self.as_player(slot, |app| {
                app.pad = if active { sample } else { Default::default() };
                if active {
                    app.apply_pad_gameplay(dt);
                }
            });
        }
    }
}
