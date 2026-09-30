//! One match from the client's side: connecting, the lobby, the match itself and its end.
//!
//! The server owns every rule. This file sends buttons (`PlayerInput`), predicts the local body exactly as the server moves it, keeps the camera,
//! the weapon in hand and its animation, plays sounds for what the server reports, draws the world, and runs the killcam. Nothing here decides
//! who is hurt.

use super::app::GpuCtx;
use super::input::Controls;
use glam::{Mat4, Vec2, Vec3};
use red_engine2::arsenal::Class;
use red_engine2::audio::Audio;
use red_engine2::avatar::recoil_kick;
use red_engine2::feel::{Cue, Feel, FxParams, Own};
use red_engine2::firearms;
use red_engine2::hit::{collect_hit_shapes_where, raycast_shapes, HitShape};
use red_engine2::killcam::{Playback, LENGTH_SECS};
use red_engine2::net::bot::ClientWorld;
use red_engine2::net::client::{ClientConfig, ConnState};
use red_engine2::net::interp::View;
use red_engine2::net::protocol::{ArenaSnap, FxSnap, OwnKit, RosterEntry, FLAG_PROTECTED, ROSTER_BOT, ROSTER_IN_ROUND};
use red_engine2::net::session::NetSession;
use red_engine2::scene_pool::HIDDEN_SCALE;
use red_engine2::schema::Scene;
use red_engine2::sfx::{KitSounds, Listener, SoundBank};
use red_engine2::shooter_world::ShooterWorld;
use red_engine2::sim::clock::TickClock;
use red_engine2::sim::flow::Phase;
use red_engine2::sim::ordnance::FxKind;
use red_engine2::sim::player::{PlayerInput, PlayerState};
use red_engine2::stats::Stats;
use red_engine2::streaks::Streaks;
use red_engine2::ui::killchain as ui;
use red_engine2::viewer::{viewmodel_transform, FpsCamera, FrameOptions, LiveRenderer};
use red_engine2::weapons::{Weapon, MUZZLE_FLASH_TIME};
use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

/// The horizontal field of view the game plays at, degrees (what shooters mean by "FOV 90"): the vertical angle follows the window's shape.
pub const FOV_DEG: f32 = 90.0;

/// The vertical field of view that gives the horizontal one `horizontal_deg` in a window of `aspect` (width / height).
pub fn vertical_fov(horizontal_deg: f32, aspect: f32) -> f32 {
    2.0 * ((0.5 * horizontal_deg.to_radians()).tan() / aspect.max(0.2)).atan().to_degrees()
}
const MOUSE_SENSITIVITY: f32 = 0.0022;
const PAD_LOOK_RATE: f32 = 3.0;
const ADS_SECS: f32 = 0.13;
const CROUCH_SECS: f32 = 0.11;
const PUNCH_RECOVERY: f32 = 7.5;
const SWITCH_SECS: f32 = 0.36;
const BODY_FLASH_SECS: f32 = MUZZLE_FLASH_TIME;

/// Everything a frame needs from the program around the match.
pub struct Env<'a> {
    /// Sound device, if there is one.
    pub audio: Option<&'a Audio>,
    /// The prototype's sounds (shots, feedback, footsteps).
    pub sounds: &'a SoundBank,
    /// The loadout's sounds.
    pub kit_sounds: &'a KitSounds,
    /// The player's lifetime record.
    pub stats: &'a mut Stats,
    /// The window's width over its height.
    pub aspect: f32,
}

/// Where a match is, as the player experiences it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Waiting for the server to answer.
    Connecting,
    /// Choosing a team and getting ready.
    Lobby,
    /// The countdown before a round: the body is frozen.
    Countdown,
    /// Fighting.
    Playing,
    /// A round is running and we are not in it yet.
    Watching,
    /// The round is over.
    Results,
    /// The server is gone.
    Lost,
}

/// A replay of our death from the killer's eyes.
struct Killcam {
    playback: Option<Playback>,
    elapsed: f32,
    killer: u8,
    weapon: Weapon,
    headshot: bool,
    last_shots: Option<u8>,
    flash: f32,
}

/// One match.
pub struct Game {
    scene: Scene,
    live: LiveRenderer,
    /// The connection and everything the client keeps for it.
    pub net: NetSession,
    world: Option<ShooterWorld>,
    streaks: Streaks,
    hit_shapes: Vec<HitShape>,
    feel: Feel,
    clock: TickClock,
    started: Instant,
    // the local body
    phys: Vec2,
    prev_phys: Vec2,
    foot_y: f32,
    prev_foot_y: f32,
    vy: f32,
    velocity: Vec2,
    eye_height: f32,
    last_speed: f32,
    // the view
    yaw: f32,
    pitch: f32,
    punch: Vec2,
    camera: FpsCamera,
    ads: f32,
    scope_level: u8,
    fov: f32,
    // the weapon in hand
    weapon: Weapon,
    last_weapon: Option<Weapon>,
    draw_timer: f32,
    since_shot: f32,
    flash_left: f32,
    melee_timer: f32,
    throw_timer: f32,
    // fire prediction
    attack_prev: bool,
    next_fire: f32,
    pending_shots: u32,
    last_server_shots: Option<u8>,
    local_reload_until: f32,
    clock_secs: f32,
    // death and the killcam
    dead_seen: bool,
    killcam: Option<Killcam>,
    // sound bookkeeping
    last_reload_left: u16,
    pending_bolt: Option<f32>,
    reload_blend: f32,
    last_kit: Option<OwnKit>,
    last_throwing: [bool; 12],
    step_count: u32,
    // stats bookkeeping
    last_kills: Option<u8>,
    last_hits: Option<u8>,
    last_headshots: Option<u8>,
    last_phase: Option<Phase>,
    streak: u32,
    last_attack_weapon: Weapon,
    counted_round: Option<u16>,
    // screens
    /// The HUD / lobby overlay was painted for this content.
    pub painted: Option<u64>,
    /// Whether this machine hosts the match (the lobby button says START).
    pub hosting: bool,
    /// Join codes to show in the lobby when hosting.
    pub join_codes: Vec<String>,
    /// A line to show in the lobby (connection trouble, UPnP news).
    pub notice: Option<String>,
    /// `Esc` menu open.
    pub paused: bool,
    my_name: String,
    scoped_now: bool,
    debug_frames: u64,
}

/// A connection request.
pub struct Connection {
    /// Server address.
    pub cfg: ClientConfig,
    /// The wanted team (`0` = any).
    pub team: u8,
}

impl Game {
    /// Loads the map, joins the server and builds the renderer.
    pub fn start(gpu: &GpuCtx, map: &Path, conn: Connection, hosting: bool, name: &str) -> Result<Game, String> {
        let (mut scene, world) = ClientWorld::load(map)?;
        let mut cfg = conn.cfg;
        cfg.map_hash = world.map_hash;
        cfg.name = name.to_string();
        let base = scene.objects.len();
        let loose: HashSet<usize> = world.prop_objects.iter().copied().collect();
        let mut net = NetSession::connect_with(cfg, world).map_err(|e| format!("cannot open a network socket: {e}"))?;
        let spawn = net.wait_connected(6.0)?;
        net.add_avatar_pool(&mut scene);
        let shooter = ShooterWorld::add_to_scene(&mut scene);
        let streaks = Streaks::new(red_engine2::streaks::add_pool(&mut scene));
        let hit_shapes = collect_hit_shapes_where(&scene, |i| i < base && !loose.contains(&i));
        let live = LiveRenderer::new_loadout(&gpu.device, gpu.format, &scene, gpu.size.0, gpu.size.1);
        let (x, z, y, yaw) = spawn.map_or((0.0, 0.0, 0.0, 0.0), |s| (s.pos.x, s.pos.y, s.foot_y, s.yaw));
        let mut camera = FpsCamera::new(Vec3::new(x, y + 1.7, z), yaw.to_degrees());
        camera.fov_deg = vertical_fov(FOV_DEG, gpu.size.0 as f32 / gpu.size.1.max(1) as f32);
        camera.near = 0.05;
        camera.far = 400.0;
        let started = Instant::now();
        let mut game = Game {
            scene,
            live,
            net,
            world: shooter,
            streaks,
            hit_shapes,
            feel: Feel::new(),
            clock: TickClock::default(),
            started,
            phys: Vec2::new(x, z),
            prev_phys: Vec2::new(x, z),
            foot_y: y,
            prev_foot_y: y,
            vy: 0.0,
            velocity: Vec2::ZERO,
            eye_height: 1.7,
            last_speed: 0.0,
            yaw,
            pitch: 0.0,
            punch: Vec2::ZERO,
            camera,
            ads: 0.0,
            scope_level: 0,
            fov: FOV_DEG,
            weapon: Weapon::Pistol,
            last_weapon: None,
            draw_timer: 0.0,
            since_shot: 10.0,
            flash_left: 0.0,
            melee_timer: 0.0,
            throw_timer: 0.0,
            attack_prev: false,
            next_fire: 0.0,
            pending_shots: 0,
            last_server_shots: None,
            local_reload_until: 0.0,
            clock_secs: 0.0,
            dead_seen: false,
            killcam: None,
            last_reload_left: 0,
            pending_bolt: None,
            reload_blend: 0.0,
            last_kit: None,
            last_throwing: [false; 12],
            step_count: 0,
            last_kills: None,
            last_hits: None,
            last_headshots: None,
            last_phase: None,
            streak: 0,
            last_attack_weapon: Weapon::Pistol,
            counted_round: None,
            painted: None,
            hosting,
            join_codes: Vec::new(),
            notice: None,
            paused: false,
            my_name: name.to_string(),
            scoped_now: false,
            debug_frames: 0,
        };
        if conn.team != 0 {
            game.net.client.set_team(conn.team, Instant::now());
        }
        Ok(game)
    }

    /// The renderer (the overlay is painted into it).
    pub fn live_mut(&mut self) -> &mut LiveRenderer {
        &mut self.live
    }

    /// Resizes the renderer's targets.
    pub fn resize(&mut self, gpu: &GpuCtx) {
        self.live.resize(&gpu.device, gpu.size.0, gpu.size.1);
        self.painted = None;
    }

    // ---- state queries ---------------------------------------------------------------------------------------------------------

    /// Where the match is.
    pub fn stage(&self) -> Stage {
        match self.net.client.state() {
            ConnState::Connecting => Stage::Connecting,
            ConnState::Rejected(_) | ConnState::Closed => Stage::Lost,
            ConnState::Reconnecting => Stage::Lost,
            ConnState::Connected => match self.net.client.status().map(|s| s.phase) {
                None => Stage::Connecting,
                Some(Phase::Waiting) => Stage::Lobby,
                Some(Phase::Countdown) => Stage::Countdown,
                Some(Phase::Playing) if self.net.client.in_round() => Stage::Playing,
                Some(Phase::Playing) => Stage::Watching,
                Some(Phase::Results) => Stage::Results,
            },
        }
    }

    fn own_kit(&self) -> Option<OwnKit> {
        self.net.client.arena().and_then(|a| a.own)
    }

    fn arena(&self) -> Option<&ArenaSnap> {
        self.net.client.arena()
    }

    fn own_dead(&self) -> bool {
        self.net.own.is_some_and(|o| o.flags & 4 != 0)
    }

    /// Whether the local body is held still.
    fn frozen(&self) -> bool {
        self.stage() != Stage::Playing || self.paused
    }

    /// Why the connection ended, in words.
    pub fn lost_reason(&self) -> String {
        match self.net.client.state() {
            ConnState::Rejected(r) => r.explain().to_string(),
            ConnState::Reconnecting => "CONNECTION LOST - TRYING TO RECONNECT".to_string(),
            _ => "DISCONNECTED".to_string(),
        }
    }

    /// Whether the cursor should be free (a menu or overlay has the window).
    pub fn wants_cursor(&self) -> bool {
        self.paused || !matches!(self.stage(), Stage::Playing | Stage::Countdown) || self.killcam.is_some()
    }

    /// Whether the window should show the scoreboard overlay of the moment.
    fn roster(&self) -> Vec<RosterEntry> {
        self.net.client.status().map(|s| s.roster.clone()).unwrap_or_default()
    }

    /// The lobby's content.
    pub fn lobby_view(&self) -> ui::LobbyView {
        let st = self.net.client.status();
        let secs = st.and_then(|s| (s.phase == Phase::Countdown && s.ticks_left != u32::MAX).then(|| s.ticks_left.div_ceil(60)));
        ui::LobbyView {
            roster: self.roster(),
            me: self.net.client.my_id().unwrap_or(255),
            ready: self.net.client.is_ready(),
            kill_limit: st.map_or(0, |s| s.kill_limit),
            time_limit_secs: st.map_or(0, |s| s.time_limit_secs),
            countdown: secs,
            join_address: self.join_codes.first().cloned(),
            message: self.notice.clone(),
            hosting: self.hosting,
            map: String::new(),
        }
    }

    /// The scoreboard / results content.
    pub fn board_view(&self, over: bool) -> ui::BoardView {
        let st = self.net.client.status();
        let roster = self.roster();
        let me = self.net.client.my_id().unwrap_or(255);
        let waiting = roster.iter().filter(|e| e.flags & ROSTER_BOT == 0 && e.flags & 1 == 0 && e.id != me).count();
        ui::BoardView {
            team: roster.iter().find(|e| e.id == me).map_or(0, |e| e.team),
            roster,
            me,
            team_score: st.map_or([0, 0], |s| s.team_score),
            winner_team: st.map_or(0, |s| s.winner_team),
            reason: st.map_or(String::new(), |s| match s.end_code {
                0 => s.end_text.clone(),
                1 => "time up".to_string(),
                2 => "kill limit".to_string(),
                3 => "abandoned".to_string(),
                _ => String::new(),
            }),
            ready: self.net.client.is_ready(),
            waiting_for: waiting,
            over,
        }
    }

    /// The HUD's content.
    pub fn hud_view(&self) -> ui::HudView {
        let st = self.net.client.status();
        let kit = self.own_kit();
        let mut view = ui::HudView {
            hp: self.net.own.map_or(100, |o| o.hp as u32),
            weapon: self.weapon.name().to_string(),
            team_score: st.map_or([0, 0], |s| s.team_score),
            team: self.roster().iter().find(|e| Some(e.id) == self.net.client.my_id()).map_or(0, |e| e.team),
            kill_limit: st.map_or(0, |s| s.kill_limit),
            secs_left: st.and_then(|s| (s.phase == Phase::Playing && s.ticks_left != u32::MAX).then(|| s.ticks_left.div_ceil(60))),
            ..Default::default()
        };
        if let Some(k) = kit {
            if self.weapon.is_gun() {
                let sel = (k.sel as usize).min(1);
                let (_, loaded, reserve) = k.guns[sel];
                view.loaded = Some(loaded.saturating_sub(self.pending_shots.min(loaded as u32) as u16));
                view.reserve = reserve;
                view.reloading = k.reload_left > 0 || self.local_reload_until > self.clock_secs;
            }
            for g in k.grenades {
                if g > 0 {
                    view.grenades.push(match Weapon::from_wire(g - 1) {
                        Weapon::Frag => 'F',
                        Weapon::Flash => 'B',
                        Weapon::Smoke => 'S',
                        _ => 'I',
                    });
                }
            }
        }
        if self.stage() == Stage::Countdown {
            let secs = st.map_or(0, |s| s.ticks_left.div_ceil(60));
            view.center = Some(if secs > 0 { secs.to_string() } else { "GO".to_string() });
        }
        view.prompt = self.pickup_prompt();
        view
    }

    /// The killcam's caption (`None` when not showing one).
    pub fn killcam_view(&self) -> Option<ui::KillcamView> {
        let kc = self.killcam.as_ref()?;
        let roster = self.roster();
        let killer = roster.iter().find(|e| e.id == kc.killer);
        let me = self.net.client.my_id();
        Some(ui::KillcamView {
            killer: if Some(kc.killer) == me { "YOURSELF".to_string() } else { killer.map_or("SOMEONE".to_string(), |e| e.name.clone()) },
            killer_team: killer.map_or(0, |e| e.team),
            weapon: kc.weapon.name().to_string(),
            headshot: kc.headshot,
            respawn_secs: self.net.client.respawn_in_secs().ceil() as u32,
            replay: kc.playback.is_some(),
        })
    }

    /// Whether the dead player's replay is showing.
    pub fn in_killcam(&self) -> bool {
        self.killcam.is_some()
    }

    /// A pickup in reach that pressing use would take.
    fn pickup_prompt(&self) -> Option<String> {
        let arena = self.arena()?;
        let eye = Vec3::new(self.phys.x, self.foot_y, self.phys.y);
        let near = |p: Vec3| (Vec2::new(p.x, p.z) - self.phys).length() < 2.0 && (p.y - eye.y).abs() < 1.6;
        let cfg = self.scene.shooter.as_ref()?;
        let mut best: Option<(f32, String)> = None;
        for (i, spot) in cfg.pickups.iter().enumerate() {
            if arena.pickups[i / 8 % arena.pickups.len()] & (1 << (i % 8)) == 0 || !near(spot.at) {
                continue;
            }
            let d = (Vec2::new(spot.at.x, spot.at.z) - self.phys).length();
            let name = spot.weapon.map_or("ammunition".to_string(), |w| w.name().to_string());
            if best.as_ref().is_none_or(|b| d < b.0) {
                best = Some((d, name));
            }
        }
        for d in &arena.dropped {
            let p = Vec3::from(d.pos);
            if near(p) {
                let dist = (Vec2::new(p.x, p.z) - self.phys).length();
                if best.as_ref().is_none_or(|b| dist < b.0) {
                    best = Some((dist, Weapon::from_wire(d.weapon).name().to_string()));
                }
            }
        }
        best.map(|b| b.1)
    }

    // ---- per frame -------------------------------------------------------------------------------------------------------------

    /// Advances the match by `dt` seconds of real time.
    pub fn frame(&mut self, dt: f32, controls: &mut Controls, env: &mut Env) {
        let now = Instant::now();
        self.debug_frames += 1;
        self.net.poll(now);
        self.clock_secs += dt;
        self.sync_from_server(env);
        let stage = self.stage();
        if stage == Stage::Playing || stage == Stage::Countdown {
            env.stats.time_in_matches_secs += if stage == Stage::Playing && !self.paused { dt as f64 } else { 0.0 };
        }
        // Looking.
        let grabbed = !self.wants_cursor();
        if grabbed {
            let zoom = (0.5 * self.fov.to_radians()).tan() / (0.5 * FOV_DEG.to_radians()).tan();
            let (dx, dy) = controls.mouse;
            self.yaw += dx * MOUSE_SENSITIVITY * zoom;
            self.pitch = (self.pitch - dy * MOUSE_SENSITIVITY * zoom).clamp(-1.5, 1.5);
            let (sx, sy) = controls.look_stick();
            let curve = |v: f32| v * v.abs();
            self.yaw += curve(sx) * PAD_LOOK_RATE * dt * zoom;
            self.pitch = (self.pitch + curve(sy) * PAD_LOOK_RATE * dt * zoom).clamp(-1.5, 1.5);
        }
        // Aim: hold to aim; a scope's RMB click cycles its zoom levels.
        let spec = self.weapon.kit();
        let scoped_weapon = self.weapon.is_gun() && spec.scoped;
        let mut want_ads = false;
        if grabbed && self.weapon.is_gun() && !self.own_dead() {
            if scoped_weapon {
                if controls.aim_edge() {
                    let levels = if spec.zoom2 > 0.0 { 2 } else { 1 };
                    self.scope_level = if self.scope_level >= levels { 0 } else { self.scope_level + 1 };
                }
                want_ads = self.scope_level > 0;
            } else {
                self.scope_level = 0;
                want_ads = controls.aiming();
            }
            let reloading = self.net.client.arena().and_then(|a| a.own).is_some_and(|k| k.reload_left > 0) || self.local_reload_until > self.clock_secs;
            if reloading || self.draw_timer > 0.0 {
                want_ads = false;
                if scoped_weapon {
                    self.scope_level = 0;
                }
            }
        } else {
            self.scope_level = 0;
        }
        let target = if want_ads { 1.0 } else { 0.0 };
        self.ads += (target - self.ads) * (dt / ADS_SECS).min(1.0);
        // Fixed ticks.
        self.clock.push_time(dt);
        while self.clock.next_tick().is_some() {
            self.fixed_tick(controls, want_ads, env);
        }
        let alpha = self.clock.alpha();
        let mut planar = self.prev_phys + (self.phys - self.prev_phys) * alpha;
        planar += self.net.visual_offset();
        let foot = self.prev_foot_y + (self.foot_y - self.prev_foot_y) * alpha;
        let crouching = controls.crouching() && !self.own_dead();
        let body = red_engine2::player::Character::Human.body();
        let target_eye = if self.own_dead() { 0.35 } else if crouching { body.crouch_eye } else { body.stand_eye };
        self.eye_height += (target_eye - self.eye_height) * (dt / CROUCH_SECS).min(1.0);
        // Timers that only the picture uses.
        self.since_shot += dt;
        self.flash_left = (self.flash_left - dt).max(0.0);
        self.draw_timer = (self.draw_timer - dt).max(0.0);
        self.melee_timer = (self.melee_timer - dt).max(0.0);
        self.throw_timer = (self.throw_timer - dt).max(0.0);
        let decay = (-PUNCH_RECOVERY * dt).exp();
        self.punch *= decay;
        // The view.
        let (yaw, pitch) = (self.yaw + self.punch.y, (self.pitch + self.punch.x).clamp(-1.52, 1.52));
        self.camera.yaw = yaw;
        self.camera.pitch = pitch;
        self.camera.position = Vec3::new(planar.x, foot + self.eye_height, planar.y);
        let zoom = if self.weapon.is_gun() {
            match (spec.scoped, self.scope_level) {
                (true, 1) => spec.zoom,
                (true, 2) => spec.zoom2,
                (true, _) => 1.0,
                _ => spec.zoom.max(1.0),
            }
        } else {
            1.0
        };
        let aim_fov = 2.0 * ((0.5 * FOV_DEG.to_radians()).tan() / zoom).atan().to_degrees();
        let target_fov = FOV_DEG + (aim_fov - FOV_DEG) * self.ads;
        self.fov += (target_fov - self.fov) * (dt / 0.06).min(1.0);
        self.camera.fov_deg = vertical_fov(self.fov, env.aspect);
        self.scoped_now = scoped_weapon && self.ads > 0.93 && !self.own_dead();
        // The killcam takes the camera.
        self.killcam_frame(dt, env);
        // The scene.
        if self.killcam.is_none() {
            self.net.update_scene(&mut self.scene, now, dt);
        }
        let fresh: Vec<FxSnap> = match self.world.as_mut() {
            Some(w) => {
                let arena = self.net.client.arena();
                match (&self.killcam, arena) {
                    (Some(kc), Some(a)) if kc.playback.is_some() => {
                        let mut a = a.clone();
                        a.fx.clear();
                        a.projectiles.clear();
                        w.update(&mut self.scene, Some(&a), dt)
                    }
                    _ => w.update(&mut self.scene, arena, dt),
                }
            }
            None => Vec::new(),
        };
        self.streaks.update(&mut self.scene, dt);
        self.audio_frame(dt, &fresh, env);
        self.feel.tick(dt);
        let cues = self.feel.take_cues();
        for cue in cues {
            self.play_cue(&cue, env);
        }
        controls.end_frame();
    }

    /// Reads what the server says about us into the local copies.
    fn sync_from_server(&mut self, env: &mut Env) {
        // A new round or a respawn places us.
        if let Some(st) = self.net.teleport.take() {
            self.place(st);
        }
        if let Some(st) = self.net.state() {
            let delta = st.pos - self.phys;
            self.prev_phys += delta;
            self.phys = st.pos;
            self.foot_y = st.foot_y;
            self.vy = st.vy;
            self.velocity = st.velocity;
        }
        let kit = self.own_kit();
        if let Some(k) = kit {
            let w = k.current_weapon().unwrap_or(Weapon::Knife);
            if Some(w) != self.last_weapon {
                if self.last_weapon.is_some() {
                    self.draw_timer = SWITCH_SECS;
                    self.scope_level = 0;
                }
                self.last_weapon = Some(w);
            }
            self.weapon = w;
            if k.reload_left == 0 && self.local_reload_until > self.clock_secs + 0.05 && self.last_reload_left > 0 {
                self.local_reload_until = 0.0;
            }
            // The server's count of our shots confirms the ones we predicted.
            if let Some(o) = self.net.own {
                if let Some(last) = self.last_server_shots {
                    let delta = o.shots.wrapping_sub(last) as u32;
                    self.pending_shots = self.pending_shots.saturating_sub(delta);
                }
                self.last_server_shots = Some(o.shots);
            }
        }
        // The local copy of the dead flag drives the killcam.
        let dead = self.own_dead();
        if dead && !self.dead_seen {
            self.dead_seen = true;
            env.stats.deaths += 1;
            self.streak = 0;
            self.start_killcam();
            self.scope_level = 0;
            self.pending_shots = 0;
        } else if !dead && self.dead_seen {
            self.dead_seen = false;
            self.killcam = None;
            if let Some(o) = self.net.own {
                self.yaw = o.yaw;
                self.pitch = 0.0;
                self.punch = Vec2::ZERO;
            }
        }
        self.track_stats(env);
        // The feel state machine.
        let happened = self.net.take_happened();
        let headshots_now = kit.map(|k| k.headshots);
        if let Some(own) = self.own_for_feel() {
            for h in &happened {
                self.feel.on_happened(h, &own);
            }
            self.feel.observe_own(&own);
        }
        for h in &happened {
            env.stats.shots_hit += h.hits as u64;
            let head_delta = match (headshots_now, self.last_headshots) {
                (Some(now), Some(before)) => now.wrapping_sub(before),
                _ => 0,
            };
            for n in 0..h.kills {
                let w = self.last_attack_weapon;
                let class = w.class();
                env.stats.add_kill(w.name(), n < head_delta, class == Class::Melee, matches!(class, Class::Grenade | Class::Launcher));
                self.streak += 1;
                env.stats.best_streak = env.stats.best_streak.max(self.streak);
            }
        }
        if headshots_now.is_some() {
            self.last_headshots = headshots_now;
        }
        if let Some(st) = self.net.client.status() {
            let me = self.net.client.my_id();
            let won = st.winner_team != 0 && self.roster().iter().any(|e| Some(e.id) == me && e.team == st.winner_team);
            self.feel.observe_match(st.phase, (st.ticks_left != u32::MAX && st.phase == Phase::Playing).then(|| st.ticks_left.div_ceil(60)), won);
        }
    }

    fn place(&mut self, st: PlayerState) {
        self.phys = st.pos;
        self.prev_phys = st.pos;
        self.foot_y = st.foot_y;
        self.prev_foot_y = st.foot_y;
        self.vy = 0.0;
        self.velocity = Vec2::ZERO;
        self.yaw = st.yaw;
        self.pitch = 0.0;
        self.punch = Vec2::ZERO;
    }

    fn own_for_feel(&self) -> Option<Own> {
        let own = self.net.own?;
        Some(Own { hp: own.hp as u32, dead: own.flags & 4 != 0, protected: own.flags & FLAG_PROTECTED != 0, weapon: own.weapon, last_rung: false, ladder: false })
    }

    /// One fixed simulation tick: sends our input and predicts our body and our own shot's look.
    fn fixed_tick(&mut self, controls: &mut Controls, want_ads: bool, env: &mut Env) {
        self.prev_phys = self.phys;
        self.prev_foot_y = self.foot_y;
        let dead = self.own_dead();
        let yaw = self.yaw + self.punch.y;
        let pitch = (self.pitch + self.punch.x).clamp(-1.52, 1.52);
        if self.frozen() {
            self.last_speed = 0.0;
            self.attack_prev = false;
            // Keep a stale press from firing when the screen goes away.
            let _ = controls.tick(yaw, pitch, false);
            return;
        }
        controls.apply_wheel();
        let mut input = controls.tick(yaw, pitch, want_ads && !dead);
        if dead || self.killcam.is_some() {
            input = PlayerInput { seq: 0, yaw, pitch, ..Default::default() };
        }
        self.predict_attack(&input, env);
        if let Some(st) = self.net.step_local(input, Instant::now()) {
            self.phys = st.pos;
            self.foot_y = st.foot_y;
            self.vy = st.vy;
            self.velocity = st.velocity;
            self.last_speed = self.net.last_speed;
        }
        self.feel.observe_motion(self.last_speed, 1.0 / 60.0, self.vy, self.vy == 0.0, None, false);
        if input.reload && self.weapon.is_gun() && !dead {
            self.start_local_reload();
        }
    }

    fn start_local_reload(&mut self) {
        let Some(k) = self.own_kit() else { return };
        let sel = (k.sel as usize).min(1);
        if k.sel > 1 || self.local_reload_until > self.clock_secs {
            return;
        }
        let (_, loaded, reserve) = k.guns[sel];
        let spec = self.weapon.kit();
        if reserve > 0 && (loaded as u32).saturating_sub(self.pending_shots) < spec.mag as u32 {
            self.local_reload_until = self.clock_secs + spec.reload;
        }
    }

    /// What our own trigger does, locally and at once: recoil, flash, the tracer and the sound. The server decides what it hit.
    fn predict_attack(&mut self, input: &PlayerInput, env: &mut Env) {
        let edge = input.attack && !self.attack_prev;
        self.attack_prev = input.attack;
        if !input.attack || self.own_dead() || self.draw_timer > 0.0 {
            return;
        }
        let Some(k) = self.own_kit() else { return };
        let spec = self.weapon.kit();
        let now = self.clock_secs;
        if now < self.next_fire {
            return;
        }
        match spec.class {
            Class::Melee => {
                if edge || spec.auto {
                    self.next_fire = now + spec.cooldown;
                    self.melee_timer = spec.cooldown.min(0.4);
                    self.last_attack_weapon = self.weapon;
                    env.stats.shots_fired += 1;
                    if let Some(a) = env.audio {
                        a.play_at(&env.kit_sounds.blade_swish, 0.6, 0.0);
                    }
                }
            }
            Class::Grenade => {
                if edge {
                    self.next_fire = now + 0.9;
                    self.throw_timer = 0.3;
                    self.last_attack_weapon = self.weapon;
                    env.stats.shots_fired += 1;
                    if let Some(a) = env.audio {
                        a.play_at(&env.kit_sounds.throw, 0.6, 0.0);
                    }
                }
            }
            _ => {
                if !(edge || spec.auto) || k.sel > 1 {
                    return;
                }
                let loaded = k.guns[k.sel as usize].1 as u32;
                let reloading = k.reload_left > 0 || self.local_reload_until > now;
                if reloading && !(spec.per_shell && loaded > self.pending_shots) {
                    return;
                }
                if loaded <= self.pending_shots {
                    // A dry click.
                    if edge {
                        self.next_fire = now + 0.25;
                        if let Some(a) = env.audio {
                            a.play_at(&env.sounds.draw, 0.5, 0.0);
                        }
                    }
                    return;
                }
                self.local_reload_until = 0.0;
                self.pending_shots += 1;
                self.next_fire = now + spec.cooldown;
                self.since_shot = 0.0;
                self.flash_left = BODY_FLASH_SECS;
                self.last_attack_weapon = self.weapon;
                env.stats.shots_fired += 1;
                // Recoil: the view kicks up and a little sideways; it recovers as you let go.
                let side = ((self.pending_shots as f32 * 12.9898).sin() * 43758.547).fract() - 0.5;
                self.punch.x += spec.kick.to_radians() * if spec.auto { 0.75 } else { 1.0 };
                self.punch.y += spec.kick.to_radians() * 0.22 * side;
                self.punch.x = self.punch.x.min(0.35);
                self.feel.own_shot(self.weapon.wire(), self.camera.position);
                if spec.class != Class::Launcher {
                    self.draw_own_shot(spec.pellets, spec.spread_hip.max(0.3));
                }
                if spec.per_shell || spec.cooldown > 0.8 {
                    // A pump or a bolt works after the shot.
                    self.pending_bolt = Some(now + 0.35);
                }
            }
        }
    }
}

impl Game {
    // ---- stats -----------------------------------------------------------------------------------------------------------------

    /// Counts the end of a match in the player's record.
    fn track_stats(&mut self, env: &mut Env) {
        let Some(st) = self.net.client.status() else { return };
        let phase = st.phase;
        if phase == Phase::Results && self.last_phase != Some(Phase::Results) && self.counted_round != Some(st.round) {
            self.counted_round = Some(st.round);
            let me = self.net.client.my_id();
            let roster = &st.roster;
            let mine = roster.iter().find(|e| Some(e.id) == me);
            if let Some(mine) = mine.filter(|m| m.flags & ROSTER_IN_ROUND != 0 || m.score > 0) {
                env.stats.rounds_played += 1;
                match (st.winner_team, mine.team) {
                    (0, _) => env.stats.rounds_drawn += 1,
                    (w, t) if w == t => env.stats.rounds_won += 1,
                    _ => env.stats.rounds_lost += 1,
                }
                env.stats.best_match_kills = env.stats.best_match_kills.max(mine.score as u32);
            }
            self.streak = 0;
        }
        self.last_phase = Some(phase);
    }

    // ---- the killcam -----------------------------------------------------------------------------------------------------------

    fn start_killcam(&mut self) {
        let me = self.net.client.my_id().unwrap_or(255);
        let (killer, weapon, headshot) = match self.own_kit() {
            Some(k) if k.killed_by != 255 => (k.killed_by, Weapon::from_wire(k.killed_weapon), k.killed_head),
            _ => (me, self.weapon, false),
        };
        let recorder = self.net.client.recorder();
        let playback = if killer != me { recorder.newest().and_then(|t| recorder.playback(killer, t)) } else { None };
        self.killcam = Some(Killcam { playback, elapsed: 0.0, killer, weapon, headshot, last_shots: None, flash: 0.0 });
    }

    /// While the replay runs the camera is the killer's and the world is drawn as it was.
    fn killcam_frame(&mut self, dt: f32, env: &mut Env) {
        let dead = self.own_dead();
        let Some(kc) = self.killcam.as_mut() else { return };
        if !dead {
            self.killcam = None;
            return;
        }
        kc.elapsed += dt;
        kc.flash = (kc.flash - dt).max(0.0);
        let Some(playback) = kc.playback.as_ref() else { return };
        let Some(shot) = playback.at(kc.elapsed.min(LENGTH_SECS as f32)) else { return };
        let body = red_engine2::player::Character::Human.body();
        let eye = if shot.killer.crouching { body.crouch_eye } else { body.stand_eye };
        self.camera.position = shot.killer.pos + Vec3::Y * eye;
        self.camera.yaw = shot.killer.yaw;
        self.camera.pitch = shot.killer.pitch;
        self.camera.fov_deg = vertical_fov(FOV_DEG, env.aspect);
        self.fov = FOV_DEG;
        let weapon = Weapon::from_wire(shot.killer.weapon);
        if let Some(last) = kc.last_shots {
            if shot.killer.shots != last && weapon.is_gun() {
                kc.flash = BODY_FLASH_SECS;
                if let Some(a) = env.audio {
                    a.play_at(env.sounds.gun(weapon.wire()), 0.8, 0.0);
                }
            }
        }
        kc.last_shots = Some(shot.killer.shots);
        let others: Vec<(u8, red_engine2::net::interp::PlayerPose)> = shot.players.iter().filter(|(id, _)| *id != kc.killer).cloned().collect();
        let present = others.len();
        let idle_t = self.started.elapsed().as_secs_f32();
        self.net.apply_view(&View { players: others, props: Vec::new() }, present, None, &mut self.scene, dt, idle_t);
    }

    // ---- shots you can see -----------------------------------------------------------------------------------------------------

    /// Tracers for our own shot: one per pellet from just ahead of the gun to what the crosshair ray met.
    fn draw_own_shot(&mut self, pellets: u8, cone_deg: f32) {
        let eye = self.camera.position;
        let forward = self.camera.forward();
        let muzzle = eye + forward * 0.55 + self.camera.right() * 0.16 - self.camera.up() * 0.14;
        let me = self.net.client.my_id();
        self.draw_shots(pellets, cone_deg * if self.ads > 0.5 { 0.3 } else { 1.0 }, muzzle, eye, forward, me, true);
    }

    fn draw_remote_shot(&mut self, weapon: Weapon, at: Vec3, yaw: f32, pitch: f32, shooter: u8) {
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        let dir = Vec3::new(sy * cp, sp, -cy * cp);
        let eye = at + Vec3::Y * 1.65;
        let muzzle = at + Vec3::Y * 1.3 + dir * 0.7;
        let k = weapon.kit();
        if k.class == Class::Launcher || !weapon.is_gun() {
            return;
        }
        self.draw_shots(k.pellets, k.spread_hip, muzzle, eye, dir, Some(shooter), false);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_shots(&mut self, pellets: u8, cone_deg: f32, muzzle: Vec3, eye: Vec3, dir: Vec3, shooter: Option<u8>, own: bool) {
        let reach = 120.0;
        let mut bodies: Vec<red_engine2::net::session::RemoteBody> = self.net.bodies().to_vec();
        if !own {
            if let Some(id) = self.net.client.my_id() {
                bodies.push(red_engine2::net::session::RemoteBody {
                    id,
                    pos: Vec3::new(self.phys.x, self.foot_y, self.phys.y),
                    dead: self.own_dead(),
                    character: red_engine2::player::Character::Human,
                });
            }
        }
        bodies.retain(|b| Some(b.id) != shooter);
        let tracer = if own { Vec3::new(1.0, 0.95, 0.65) } else { Vec3::new(1.0, 0.6, 0.3) };
        let seed = (self.clock.ticks_run() as u32).wrapping_mul(2654435761);
        for pellet in 0..pellets.max(1) {
            let n = seed.wrapping_add(pellet as u32 * 40503);
            let (u, v) = (((n >> 3) & 0xffff) as f32 / 65535.0, ((n >> 11) & 0xffff) as f32 / 65535.0);
            let d = red_engine2::sim::kit::scatter(dir, if pellets > 1 { cone_deg } else { cone_deg * 0.4 }, u, v);
            let wall = raycast_shapes(eye, d, reach, &self.hit_shapes).map(|h| h.distance);
            let body = red_engine2::net::session::nearest_body_on_ray(&bodies, eye, d, reach).map(|(_, dist)| dist);
            let (dist, spark) = match (wall, body) {
                (Some(w), Some(b)) if b < w => (b, Some(Vec3::new(1.0, 0.15, 0.1))),
                (Some(w), _) => (w, Some(Vec3::new(1.0, 0.85, 0.4))),
                (None, Some(b)) => (b, Some(Vec3::new(1.0, 0.15, 0.1))),
                (None, None) => (reach, None),
            };
            let end = eye + d * dist;
            self.streaks.tracer(muzzle, end, tracer);
            if let Some(color) = spark {
                self.streaks.spark(end, color);
            }
        }
    }

    // ---- sound -----------------------------------------------------------------------------------------------------------------

    fn listener(&self) -> Listener {
        Listener { eye: self.camera.position.to_array(), yaw: self.camera.yaw }
    }

    fn audio_frame(&mut self, dt: f32, fresh: &[FxSnap], env: &mut Env) {
        let Some(audio) = env.audio else { return };
        for f in fresh {
            let at = Vec3::from(f.pos);
            let (gain, pan) = red_engine2::sfx::spatial(self.camera.position.to_array(), self.camera.yaw, [at.x, at.y, at.z]);
            let clip = match FxKind::from_wire(f.kind) {
                Some(FxKind::Blast) => &env.kit_sounds.explosion,
                Some(FxKind::FlashPop) => &env.kit_sounds.flash_pop,
                Some(FxKind::SmokePop) => &env.kit_sounds.smoke_pop,
                Some(FxKind::FirePop) => &env.kit_sounds.fire_burst,
                None => continue,
            };
            audio.play_at(clip, (gain * 1.1).min(1.0), pan);
        }
        if let Some(k) = self.own_kit() {
            // A reload the server started by itself (an empty gun): play it too.
            if k.reload_left > 0 && self.last_reload_left == 0 && self.local_reload_until <= self.clock_secs {
                self.local_reload_until = self.clock_secs + self.weapon.kit().reload;
            }
            if (k.reload_left > 0 && self.last_reload_left == 0) || (self.local_reload_until > self.clock_secs && self.reload_blend < 0.05 && k.reload_left == 0) {
                let spec = self.weapon.kit();
                audio.play_at(if spec.per_shell { &env.kit_sounds.reload_shell } else { &env.kit_sounds.reload_mag }, 0.55, 0.0);
            }
            self.last_reload_left = k.reload_left;
            if let Some(old) = self.last_kit {
                let held = |k: &OwnKit| {
                    let mut v: Vec<u8> = k.guns.iter().filter(|g| g.0 > 0).map(|g| g.0).collect();
                    v.extend(k.grenades.iter().filter(|g| **g > 0));
                    v.push(k.melee + 1);
                    v
                };
                let before = held(&old);
                if held(&k).iter().any(|w| !before.contains(w)) {
                    audio.play_at(&env.kit_sounds.pickup, 0.6, 0.0);
                }
            }
            self.last_kit = Some(k);
            // The flashbang's ring for whoever it caught.
        }
        if let Some(t) = self.pending_bolt {
            if self.clock_secs >= t {
                self.pending_bolt = None;
                audio.play_at(&env.kit_sounds.bolt, 0.45, 0.0);
            }
        }
        // Other players throwing grenades.
        let view = self.net.client.view(Instant::now());
        for (id, pose) in &view.players {
            let i = (*id as usize).min(11);
            if pose.throwing() && !self.last_throwing[i] {
                let (gain, pan) = red_engine2::sfx::spatial(self.camera.position.to_array(), self.camera.yaw, [pose.pos.x, pose.pos.y + 1.3, pose.pos.z]);
                audio.play_at(&env.kit_sounds.throw, gain * 0.7, pan);
            }
            self.last_throwing[i] = pose.throwing();
        }
        let _ = dt;
    }

    fn play_cue(&mut self, cue: &Cue, env: &mut Env) {
        let in_replay = self.killcam.as_ref().is_some_and(|k| k.playback.is_some());
        if let Cue::Shot { weapon, at, yaw, pitch, shooter, own: false } = *cue {
            if in_replay {
                return; // the replay plays its own
            }
            self.draw_remote_shot(Weapon::from_wire(weapon), at, yaw, pitch, shooter);
        }
        let Some(audio) = env.audio else { return };
        let played = env.sounds.play(cue, self.listener(), self.step_count);
        if matches!(cue, Cue::Step) {
            self.step_count = self.step_count.wrapping_add(1);
        }
        if played.gain > 0.01 && !(in_replay && !matches!(cue, Cue::Death | Cue::Respawn)) {
            audio.play_at(played.clip, played.gain, played.pan);
        }
    }

    // ---- drawing ---------------------------------------------------------------------------------------------------------------

    /// Paints `rgba` (a full-window HUD or menu image) over the frame; `None` hides it.
    pub fn set_overlay(&mut self, gpu: &GpuCtx, rgba: Option<&[u8]>) {
        match rgba {
            Some(px) => self.live.overlay.set(&gpu.device, &gpu.queue, gpu.size.0, gpu.size.1, px),
            None => self.live.overlay.hide(),
        }
    }

    /// Whether the scope picture is showing.
    pub fn scoped(&self) -> bool {
        self.scoped_now && self.killcam.is_none()
    }

    fn body_under_crosshair(&self) -> bool {
        let eye = self.camera.position;
        let dir = self.camera.forward();
        let reach = 120.0;
        let wall = raycast_shapes(eye, dir, reach, &self.hit_shapes).map_or(f32::INFINITY, |h| h.distance);
        let my_team = self.roster().iter().find(|e| Some(e.id) == self.net.client.my_id()).map_or(0, |e| e.team);
        let roster = self.roster();
        self.net.player_in_sight(eye, dir, reach).is_some_and(|(id, d)| d < wall && roster.iter().find(|e| e.id == id).is_none_or(|e| e.team != my_team || my_team == 0))
    }

    /// Draws one frame into `target`.
    pub fn draw(&mut self, gpu: &GpuCtx, target: &wgpu::TextureView) {
        let in_replay = self.killcam.as_ref().is_some_and(|k| k.playback.is_some());
        let dead = self.own_dead();
        let playing = matches!(self.stage(), Stage::Playing | Stage::Countdown);
        // The viewmodel: ours, or the killer's in a replay.
        let (weapon, skin, flash, aiming) = if in_replay {
            let kc = self.killcam.as_ref();
            let shot_weapon = kc.map_or(Weapon::Knife, |k| k.playback.as_ref().and_then(|p| p.at(k.elapsed.min(LENGTH_SECS as f32))).map_or(k.weapon, |s| Weapon::from_wire(s.killer.weapon)));
            let (team, aim) = kc
                .and_then(|k| k.playback.as_ref().and_then(|p| p.at(k.elapsed.min(LENGTH_SECS as f32))))
                .map_or((0, false), |s| (s.killer.team(), s.killer.aiming()));
            (shot_weapon, team, kc.map_or(0.0, |k| k.flash / BODY_FLASH_SECS), if aim { 1.0 } else { 0.0 })
        } else {
            let team = self.net.client.arena().and_then(|a| a.own).map_or(0, |o| o.team);
            (self.weapon, team, self.flash_left / BODY_FLASH_SECS, self.ads)
        };
        let reloading = !in_replay && (self.own_kit().is_some_and(|k| k.reload_left > 0) || self.local_reload_until > self.clock_secs);
        self.reload_blend += ((if reloading { 1.0 } else { 0.0 }) - self.reload_blend) * 0.2;
        let dip = (self.draw_timer / SWITCH_SECS).clamp(0.0, 1.0).max(self.reload_blend * 0.75);
        let kick = match weapon.class() {
            Class::Melee => {
                let total = weapon.kit().cooldown.min(0.4);
                let p = 1.0 - (self.melee_timer / total).clamp(0.0, 1.0);
                if self.melee_timer > 0.0 { (p * std::f32::consts::PI).sin() } else { 0.0 }
            }
            Class::Grenade => {
                let p = 1.0 - (self.throw_timer / 0.3).clamp(0.0, 1.0);
                if self.throw_timer > 0.0 { (p * std::f32::consts::PI).sin() } else { 0.0 }
            }
            _ => recoil_kick(self.since_shot),
        };
        let (offset, rotation) = if weapon == Weapon::Bat { (Vec3::new(0.12, -0.14, 0.34), Mat4::IDENTITY) } else { firearms::held_pose(weapon, aiming, kick, dip) };
        let weapon_tf = viewmodel_transform(&self.camera, offset, rotation);
        let scoped = self.scoped();
        let mut fx: FxParams = if in_replay { FxParams::default() } else { self.feel.fx(self.camera.yaw) };
        if let Some(k) = self.own_kit().filter(|k| k.flash_left > 0 && !in_replay) {
            let total = (k.flash_total as f32 / 10.0).max(0.5);
            let left = k.flash_left as f32 / 60.0;
            let a = (left / total).clamp(0.0, 1.0).powf(0.45);
            fx.flash = [1.0, 1.0, 1.0, fx.flash[3].max(a * 0.97)];
        }
        let enemy = playing && !dead && self.ads < 0.5 && self.body_under_crosshair();
        let opts = FrameOptions {
            crosshair: playing && !dead && !scoped && self.ads < 0.45 && !in_replay,
            viewmodel: !scoped && (playing || in_replay) && (!dead || in_replay),
            pickup: false,
            weapon,
            muzzle_flash: flash.clamp(0.0, 1.0),
            fx,
            enemy,
            skin,
        };
        let hidden_tf = Mat4::from_scale(Vec3::splat(HIDDEN_SCALE));
        let Game { live, net, scene, streaks, world, .. } = self;
        let mut hidden: Vec<&str> = Vec::new();
        hidden.extend(net.hidden_objects());
        hidden.extend(net.hidden_avatar_ids(scene));
        hidden.extend(streaks.hidden_ids(scene));
        if let Some(w) = world.as_ref() {
            hidden.extend(w.hidden_ids(scene));
        }
        live.set_hidden_objects(hidden);
        live.set_remote_hands(net.remote_hands());
        let t = self.started.elapsed().as_secs_f32();
        self.live.render_ex(&gpu.device, &gpu.queue, &self.scene, t, &self.camera, target, false, weapon_tf, hidden_tf, opts);
    }

    /// Leaves the match politely.
    pub fn leave(&mut self) {
        self.net.disconnect();
    }

    /// The name this player uses.
    pub fn name(&self) -> &str {
        &self.my_name
    }

    /// Points the view somewhere (degrees; a script's `look` step).
    pub fn set_look(&mut self, yaw_deg: Option<f32>, pitch_deg: Option<f32>) {
        if let Some(y) = yaw_deg {
            self.yaw = y.to_radians();
        }
        if let Some(p) = pitch_deg {
            self.pitch = p.to_radians().clamp(-1.5, 1.5);
        }
    }

    /// One line of the client's state (a script's `state` step).
    pub fn debug_state(&self) -> String {
        let kit = self.own_kit();
        format!(
            "stage {:?} weapon {} hp {} pos ({:.1}, {:.1}, {:.1}) yaw {:.0} kit {:?} dropped {} flyers {} pending_shots {} killcam {}",
            self.stage(),
            self.weapon.name(),
            self.net.own.map_or(0, |o| o.hp),
            self.phys.x,
            self.foot_y,
            self.phys.y,
            self.yaw.to_degrees(),
            kit.map(|k| (k.sel, k.guns, k.reload_left)),
            self.world.as_ref().map_or(0, |w| w.dropped_drawn()),
            self.world.as_ref().map_or(0, |w| w.flyers_drawn()),
            self.pending_shots,
            self.killcam.is_some()
        )
    }

    /// Frames run (for the debug overlay and tests).
    pub fn frames(&self) -> u64 {
        self.debug_frames
    }
}
