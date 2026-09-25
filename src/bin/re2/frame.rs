//! The per-frame game loop: fixed-step physics and input sampling (the same `sim::player::step_player` the server runs), the
//! variable-rate `update` (camera, avatar, scene sync), drawing, and starting a game from the menu.

use super::*;

impl App {
    /// What the player is asking for this tick, from the held keys (or the `RE2_AUTOWALK` debug script).
    pub(crate) fn build_input(&mut self) -> PlayerInput {
        if let Some(mode) = self.autowalk.clone() {
            // "circle" or "circle:DEGREES_PER_SECOND" turns while walking; anything else walks straight.
            if let Some(rate) = mode.strip_prefix("circle") {
                let dps = rate.trim_start_matches(':').parse::<f32>().unwrap_or(40.0);
                self.camera.yaw += dps.to_radians() * FIXED_DT;
            }
            return PlayerInput { forward: 1, sprint: false, yaw: self.camera.yaw, pitch: self.camera.pitch, ..Default::default() };
        }
        let held = |a: KeyCode, b: KeyCode| self.keys.contains(&a) || self.keys.contains(&b);
        let axis = |pos: bool, neg: bool| pos as i8 - neg as i8;
        PlayerInput {
            seq: 0,
            forward: axis(held(KeyCode::KeyW, KeyCode::ArrowUp), held(KeyCode::KeyS, KeyCode::ArrowDown)),
            strafe: axis(held(KeyCode::KeyD, KeyCode::ArrowRight), held(KeyCode::KeyA, KeyCode::ArrowLeft)),
            jump: std::mem::take(&mut self.jump_queued),
            sprint: self.sprint_held,
            crouch: held(KeyCode::ControlLeft, KeyCode::ControlRight),
            yaw: self.camera.yaw,
            pitch: self.camera.pitch,
            interact: self.take_pulse(0),
            attack: self.take_pulse(1),
            switch_weapon: self.take_pulse(2),
            reload: self.take_pulse(3),
        }
    }

    /// One fixed-size (`FIXED_DT`) physics step: movement/collision + jump/gravity, sampling
    /// currently-held input fresh (input state doesn't change within a rendered frame between
    /// steps). Snapshots the pre-step planar position/foot height into `prev_physics_pos`/
    /// `prev_foot_y` first, so `update` can interpolate between them for the actual rendered
    /// frame instead of drawing exactly on whichever physics step boundary just landed.
    pub(crate) fn fixed_step_physics(&mut self) {
        self.prev_physics_pos = self.physics_pos;
        self.prev_foot_y = self.foot_y;
        if self.online_frozen() {
            self.last_move_speed = 0.0; // a lobby, a countdown or the results: the body stands still and nothing is sent
            return;
        }

        // Movement is the shared pure function `sim::player::step_player` (the single-player game, the
        // authoritative server and a client's prediction all run it). Online, the predictor owns the state
        // and also sends the input to the server.
        let input = self.build_input();
        let mut st = PlayerState {
            pos: self.physics_pos,
            foot_y: self.foot_y,
            vy: self.vertical_velocity,
            yaw: self.camera.yaw,
            pitch: self.camera.pitch,
            character: self.character,
        };
        self.last_move_speed = 0.0;
        if let Some(net) = self.net.as_mut() {
            if let Some(s2) = net.step_local(input, Instant::now()) {
                st = s2;
                self.last_move_speed = net.last_speed;
            }
        } else {
            // Offline the player can stand on (and jump off) a loose prop: its top is an extra floor under the feet.
            let extra_floor = self.props.as_ref().and_then(|p| p.floor_under(st.pos, st.foot_y, self.body.radius, 0.35));
            self.last_move_speed = step_player_on(&mut st, &input, &self.colliders, &self.ground, extra_floor);
        }
        self.physics_pos = st.pos;
        self.foot_y = st.foot_y;
        self.vertical_velocity = st.vy;

        // Loose props: the player's body shoves what it walks into, then the world steps.
        let d = (self.physics_pos - self.prev_physics_pos) / FIXED_DT;
        self.player_vel = Vec3::new(d.x, 0.0, d.y);
        if let Some(props) = &mut self.props {
            props.set_player(Vec3::new(self.physics_pos.x, self.foot_y, self.physics_pos.y), self.body.radius, self.body.body_height);
            props.step();
        }

        self.fixed_step_combat();
    }

    /// The player's eye at the latest completed tick (the origin of this tick's swings and shots).
    /// Unlike `self.eye` it does not depend on the render frame.
    pub(crate) fn tick_eye(&self) -> Vec3 {
        Vec3::new(self.physics_pos.x, self.foot_y + self.eye_height, self.physics_pos.y)
    }

    pub(crate) fn update(&mut self, dt: f32) {
        // Online, the connection must be serviced even when the window is not focused (or the server
        // would time us out), so the simulation keeps ticking; offline it pauses like before.
        if !self.grabbed && self.net.is_none() {
            return;
        }
        let mut server_weapon = None;
        if let Some(net) = self.net.as_mut() {
            let now = Instant::now();
            net.poll(now);
            server_weapon = net.own.map(|o| Weapon::from_wire(o.weapon));
            if let Some(st) = net.teleport.take() {
                // Joined, or resumed after a reconnect: go where the server says.
                self.physics_pos = st.pos;
                self.prev_physics_pos = st.pos;
                self.foot_y = st.foot_y;
                self.prev_foot_y = st.foot_y;
                self.vertical_velocity = 0.0;
                self.camera.yaw = st.yaw;
            }
            // Reconciliation may have nudged the predicted state; carry the difference through so the
            // interpolation below does not lurch (the visual offset is added back after it).
            if let Some(st) = net.state() {
                let delta = st.pos - self.physics_pos;
                self.prev_physics_pos += delta;
                self.physics_pos = st.pos;
                self.foot_y = st.foot_y;
                self.vertical_velocity = st.vy;
            }
            if self.net_title_at.elapsed().as_secs_f32() > 1.0 {
                self.net_title_at = Instant::now();
                if let Some(window) = &self.window {
                    window.set_title(&format!("Red Engine 2 — {} — {} — {}", self.scene_path.display(), self.character.name(), net.status));
                }
            }
        }

        self.sync_online_ui();

        // Online, the weapon in hand is whatever the server says (it owns weapons, health and pick-ups).
        if let Some(w) = server_weapon {
            self.weapon = w;
        }

        // Accumulate real time and drain it in fixed-size chunks (the standard "fix your
        // timestep" pattern) — capped so a long stall (window drag, debugger pause) resumes from
        // where it left off instead of trying to replay minutes of physics in one frame.
        self.clock.push_time(dt);
        while self.clock.next_tick().is_some() {
            self.fixed_step_physics();
        }
        let alpha = self.clock.alpha();
        let mut planar_pos = self.prev_physics_pos.lerp(self.physics_pos, alpha);
        if let Some(net) = &self.net {
            planar_pos += net.visual_offset();
        }
        let foot_y = self.prev_foot_y + (self.foot_y - self.prev_foot_y) * alpha;

        let crouching = self.keys.contains(&KeyCode::ControlLeft) || self.keys.contains(&KeyCode::ControlRight);
        let forward_held = self.keys.contains(&KeyCode::KeyW) || self.keys.contains(&KeyCode::ArrowUp);
        let back_held = self.keys.contains(&KeyCode::KeyS) || self.keys.contains(&KeyCode::ArrowDown);
        let sprinting = self.sprint_held && forward_held && !back_held && !crouching && self.body.sprint_speed > self.body.walk_speed;

        // Crouch: blend the eye height toward its target instead of snapping, so the camera
        // doesn't jump-cut when Ctrl is pressed/released.
        let target_eye_height = if crouching { self.body.crouch_eye } else { self.body.stand_eye };
        let blend = (dt / CROUCH_TRANSITION_TIME).min(1.0);
        self.eye_height += (target_eye_height - self.eye_height) * blend;

        // Update the player's own body (position/facing/pose) from the interpolated
        // (pre-third-person-pullback) planar position, then place the camera: directly at the
        // eye in first person, or pulled back behind/above it in third person. This order
        // matters — the body must be placed before `self.camera.position` is potentially
        // overwritten by the third-person pullback below.
        let body_yaw_deg = 180.0 - self.camera.yaw.to_degrees();
        self.update_player_body(planar_pos, body_yaw_deg, self.last_move_speed, dt);

        let anchor = Vec3::new(planar_pos.x, foot_y + self.eye_height, planar_pos.y);
        self.eye = anchor;
        // Cosmetic timers run on render time; everything that decides a hit is in `fixed_step_combat`.
        self.since_shot += dt;
        self.flash_left = (self.flash_left - dt).max(0.0);
        // The animation reads mirrors of the tick-based swing/switch state, smoothed by `alpha`.
        self.swing_timer = self.swing.elapsed_secs(alpha);
        self.switching = self.switch.elapsed_secs(alpha);
        self.camera.position = match self.view_mode {
            ViewMode::FirstPerson => anchor,
            ViewMode::ThirdPerson => {
                let desired = anchor - self.camera.forward() * self.body.third_person_distance + Vec3::Y * self.body.third_person_lift;
                let active = colliders_on_floor(&self.colliders, foot_y);
                let cam_radius = THIRD_PERSON_CAM_RADIUS.min(self.body.radius * 0.7);
                let clamped = resolve_collision(Vec2::new(desired.x, desired.z), cam_radius, &active);
                Vec3::new(clamped.x, desired.y, clamped.y)
            }
        };

        // Sprint FOV kick, blended the same way as the crouch height.
        let target_fov = if sprinting { BASE_FOV_DEG + SPRINT_FOV_BOOST_DEG } else { BASE_FOV_DEG };
        let fov_blend = (dt / FOV_TRANSITION_TIME).min(1.0);
        self.fov_deg += (target_fov - self.fov_deg) * fov_blend;
        self.camera.fov_deg = self.fov_deg;

        // Loose props: keep a carried one in front of the player, write every prop's physics pose into
        // the scene, and see what the crosshair could pick up.
        if let Some(props) = &mut self.props {
            if let Some(h) = props.held() {
                let pose = props.hold_pose(h, anchor, self.camera.forward(), self.body.radius, self.body.hold_drop, foot_y);
                props.set_held_pose(pose);
            }
            props.sync_scene(&mut self.scene);
            self.pickup_target = props.pick_target(anchor, self.camera.forward(), self.body.pickup_reach, &self.body.carry);
        }

        // What's the crosshair aimed at, within bat reach? (Drives the crosshair's gold "you
        // could hit this" state.) Tested against the objects' real shapes, not bounding boxes, so
        // it is gold only where a swing would actually connect. Only the human has a bat. The ray
        // starts at the player's eye (`anchor`), not the camera: in third person the camera hangs
        // metres behind the player, and testing from there "hit" things behind them.
        let reach = if self.shown_weapon() == Weapon::Revolver { REVOLVER_RANGE } else { MELEE_REACH };
        self.target_index = if self.body.has_bat && !self.carrying() { self.probe(anchor, reach).map(|(o, _, _)| o) } else { None };

        if let Some(t) = self.freeze_shot {
            self.since_shot = t;
            self.flash_left = if t < MUZZLE_FLASH_TIME { MUZZLE_FLASH_TIME * 0.9 } else { 0.0 };
        }
        if let Some(t) = self.freeze_swing {
            self.swing_timer = Some(t);
        }
        if let Some(net) = self.net.as_mut() {
            // Other players and the server's props, interpolated, into the scene the renderer draws.
            net.update_scene(&mut self.scene, Instant::now(), dt);
        }
    }

    pub(crate) fn draw(&mut self) {
        let weapon_transform = self.weapon_transform();
        let carrying = self.carrying();
        let shown_weapon = self.shown_weapon();
        let muzzle_flash = (self.flash_left / MUZZLE_FLASH_TIME).clamp(0.0, 1.0);
        let Some(gpu) = self.gpu.as_mut() else { return };
        let Some(live) = gpu.live.as_mut() else { return };
        let Some((surface_tex, reconfigure)) = acquire_frame(&gpu.surface, &gpu.device, &gpu.config) else { return };
        let view = surface_tex.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let t = if self.scene.duration > 0.0 { self.start.elapsed().as_secs_f32() % self.scene.duration } else { 0.0 };
        let opts = FrameOptions { crosshair: true, viewmodel: !carrying, pickup: self.pickup_target.is_some(), weapon: shown_weapon, muzzle_flash };
        live.render_ex(
            &gpu.device,
            &gpu.queue,
            &self.scene,
            t,
            &self.camera,
            &view,
            self.target_index.is_some(),
            weapon_transform,
            self.hand_prop_transform,
            opts,
        );
        gpu.queue.present(surface_tex);
        if reconfigure {
            gpu.surface.configure(&gpu.device, &gpu.config);
        }
    }

    /// One frame of the launch menu: the turning models behind, the text and panels over them.
    pub(crate) fn menu_frame(&mut self) {
        let Some(gpu) = self.gpu.as_mut() else { return };
        let Some(menu_live) = gpu.menu.as_mut() else { return };
        let Some((surface_tex, reconfigure)) = acquire_frame(&gpu.surface, &gpu.device, &gpu.config) else { return };
        let (w, h) = (gpu.config.width, gpu.config.height);
        let t = self.start.elapsed().as_secs_f32();
        menu::animate(&mut self.menu_scene, w as f32 / h as f32, t, self.character);
        let key = (w, h, self.character);
        if self.menu_painted != Some(key) {
            let map = self.scene_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            menu_live.overlay.set(&gpu.device, &gpu.queue, w, h, &menu::paint(w, h, self.character, &map));
            self.menu_painted = Some(key);
        }
        let view = surface_tex.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let hidden = Mat4::from_scale(Vec3::splat(HIDDEN_SCALE));
        menu_live.render_ex(
            &gpu.device,
            &gpu.queue,
            &self.menu_scene,
            t,
            &menu::menu_camera(),
            &view,
            false,
            hidden,
            hidden,
            FrameOptions { crosshair: false, viewmodel: false, pickup: false, ..FrameOptions::default() },
        );
        gpu.queue.present(surface_tex);
        if reconfigure {
            gpu.surface.configure(&gpu.device, &gpu.config);
        }
    }

    /// Leaves the menu: adds the chosen character's body to the map, builds everything that
    /// depends on it (colliders, hit shapes, the renderer) and captures the mouse.
    pub(crate) fn start_game(&mut self, who: Character) {
        self.character = who;
        self.body = who.body();
        self.player_object_index = self.scene.objects.len();
        self.scene.objects.push(build_player_object(who));
        // Loose props (chairs, crates, apples...) live in the rigid-body world, not in the static
        // collider lists: they move.
        let online = self.net_server.is_some();
        let (props, loose) = if self.pending_net.is_some() || (self.net_server.is_some() && self.net_world.is_some()) {
            // Online: the server owns the props; this client only draws them and predicts its own walking. The session comes from the
            // connect form (already joined) or is made here from `--connect` (with `--key` and `--name`).
            let (mut session, joined) = match self.pending_net.take() {
                Some(mut s) => {
                    let spawn = s.teleport.take();
                    (s, Ok(spawn))
                }
                None => {
                    let (Some(addr), Some(world)) = (self.net_server, self.net_world.take()) else { fail_online("no server to join") };
                    let mut cfg = red_engine2::net::client::ClientConfig::new(addr, if who == Character::Rat { 1 } else { 0 }, world.map_hash, 0);
                    cfg.join_key = self.join_key.clone();
                    cfg.name = self.player_name.clone();
                    let mut s = match NetSession::connect_with(cfg, world) {
                        Ok(s) => s,
                        Err(e) => fail_online(&format!("cannot open a network socket: {e}")),
                    };
                    let joined = s.wait_connected(5.0);
                    (s, joined)
                }
            };
            let loose: HashSet<usize> = session.world.prop_objects.iter().copied().collect();
            self.colliders = session.world.colliders.clone();
            self.ground = session.world.ground.clone();
            session.add_avatar_pool(&mut self.scene); // before the renderer takes its meshes from the scene
            match joined {
                Ok(Some(st)) => {
                    self.physics_pos = st.pos;
                    self.prev_physics_pos = st.pos;
                    self.foot_y = st.foot_y;
                    self.prev_foot_y = st.foot_y;
                    self.camera.yaw = st.yaw;
                    println!("Joined as player {} at ({:.1}, {:.1}).", session.client.my_id().unwrap_or(0), st.pos.x, st.pos.y);
                }
                Ok(None) => println!("Joined as player {}: in the lobby (the round places you when it starts).", session.client.my_id().unwrap_or(0)),
                Err(e) => fail_online(&e),
            }
            self.net = Some(session);
            (None, loose)
        } else {
            let props = PropWorld::new(&self.scene, Some(self.player_object_index));
            let loose = props.movable_indices();
            println!("{} loose props (pick up with E).", loose.len());
            self.colliders = collect_box_colliders_except(&self.scene, &loose);
            self.ground = collect_ground_candidates_except(&self.scene, &loose);
            (Some(props), loose)
        };
        let _ = online;
        // The player's own body is in `scene.objects` so the renderer can draw it, but the bat must
        // never be able to hit it (e.g. looking down at your own feet), so it is skipped here.
        let player_index = self.player_object_index;
        self.hit_shapes = collect_hit_shapes_where(&self.scene, |i| i != player_index && !loose.contains(&i));
        self.props = props;
        self.eye_height = self.body.stand_eye;
        self.camera.position.y = self.body.stand_eye;
        self.camera.near = self.body.near_plane;
        if self.debug_third_person {
            self.view_mode = ViewMode::ThirdPerson;
        }
        // Debug: `RE2_WEAPON=revolver` starts with the revolver in hand (for screenshots).
        if who == Character::Human && std::env::var("RE2_WEAPON").is_ok_and(|v| v.eq_ignore_ascii_case("revolver")) {
            self.weapon = Weapon::Revolver;
        }
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.live = Some(LiveRenderer::new(&gpu.device, gpu.config.format, &self.scene, gpu.config.width, gpu.config.height));
            gpu.menu = None;
        }
        self.phase = Phase::Playing;
        println!("Playing as {}.", who.name());
        if let Some(window) = &self.window {
            window.set_title(&format!("Red Engine 2 — {} — {}", self.scene_path.display(), who.name()));
        }
        self.last_frame = Instant::now();
        self.net_title_at = Instant::now();
        self.set_grab(true);
    }
}
