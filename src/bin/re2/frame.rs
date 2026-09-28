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
            if self.autoaim {
                // The nearest other player with nothing solid in between: a sentry that stands still and turns until it sees one.
                let eye = self.tick_eye();
                let bodies = self.net.as_ref().map(|n| n.bodies().to_vec()).unwrap_or_default();
                let visible = bodies.into_iter().filter(|b| !b.dead).filter(|b| {
                    let d = b.pos + Vec3::Y - eye;
                    raycast_shapes(eye, d.normalize_or_zero(), d.length(), &self.hit_shapes).is_none()
                });
                if let Some(b) = visible.min_by(|a, c| (a.pos - eye).length().total_cmp(&(c.pos - eye).length())) {
                    let d = b.pos + Vec3::Y - eye;
                    self.camera.yaw = d.x.atan2(-d.z);
                    self.camera.pitch = d.y.atan2(Vec2::new(d.x, d.z).length()).clamp(-1.2, 1.2);
                }
            }
            let attack = (self.autofire || self.autoaim) && self.clock.ticks_run() % 12 < 6;
            return PlayerInput {
                forward: i8::from(!self.autoaim),
                sprint: false,
                yaw: self.camera.yaw,
                pitch: self.camera.pitch,
                attack,
                ..Default::default()
            };
        }
        let held = |a: KeyCode, b: KeyCode| self.keys.contains(&a) || self.keys.contains(&b);
        let axis = |pos: bool, neg: bool| pos as i8 - neg as i8;
        let forward = axis(held(KeyCode::KeyW, KeyCode::ArrowUp), held(KeyCode::KeyS, KeyCode::ArrowDown));
        let strafe = axis(held(KeyCode::KeyD, KeyCode::ArrowRight), held(KeyCode::KeyA, KeyCode::ArrowLeft));
        let analog = self.grabbed && forward == 0 && strafe == 0 && self.pad.movement.length_squared() > 0.0;
        PlayerInput {
            seq: 0,
            forward: if analog { (self.pad.movement.y * 127.0).round() as i8 } else { forward },
            strafe: if analog { (self.pad.movement.x * 127.0).round() as i8 } else { strafe },
            analog,
            jump: std::mem::take(&mut self.jump_queued),
            sprint: self.sprint_held || self.pad.down(red_engine2::controller::button::SPRINT),
            crouch: held(KeyCode::ControlLeft, KeyCode::ControlRight) || self.pad.down(red_engine2::controller::button::CROUCH),
            yaw: self.camera.yaw,
            pitch: self.camera.pitch,
            interact: self.take_pulse(0),
            attack: self.take_pulse(1) || ((self.attack_held || self.pad.down(red_engine2::controller::button::FIRE)) && self.weapon.automatic()),
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
        // Dead: the body lies where it fell. The server ignores our movement then, and predicting it would only pull us back.
        let input = if self.own_dead() { PlayerInput { yaw: input.yaw, pitch: input.pitch, ..Default::default() } } else { input };
        let attack_now = input.attack;
        let mut st = PlayerState {
            pos: self.physics_pos,
            foot_y: self.foot_y,
            vy: self.vertical_velocity,
            velocity: self.horizontal_velocity,
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
            self.last_move_speed = step_player_on_tuned(&mut st, &input, &self.colliders, &self.ground, extra_floor, self.scene.player, &self.scene.jump_pads);
        }
        self.physics_pos = st.pos;
        self.foot_y = st.foot_y;
        self.vertical_velocity = st.vy;
        self.horizontal_velocity = st.velocity;

        // Loose props: the player's body shoves what it walks into, then the world steps.
        let d = (self.physics_pos - self.prev_physics_pos) / FIXED_DT;
        self.player_vel = Vec3::new(d.x, 0.0, d.y);
        if let Some(props) = &mut self.props {
            props.set_player(Vec3::new(self.physics_pos.x, self.foot_y, self.physics_pos.y), self.body.radius, self.body.body_height);
            props.step();
        }

        self.feel.observe_motion(self.last_move_speed, FIXED_DT, self.vertical_velocity, self.vertical_velocity == 0.0, self.pad_launch, self.sprint_held);
        self.fixed_step_combat(attack_now);
        if self.net.is_none() {
            self.fixed_step_rules();
        }
    }

    /// Runs scene rules for the offline player and applies their world-facing effects. This is
    /// deliberately the existing [`RulesEngine`], not a presentation-side copy of game logic.
    fn fixed_step_rules(&mut self) {
        if !self.rules.has_rules() {
            return;
        }
        let tick = self.clock.ticks_run();
        if tick >= self.rule_event_until {
            self.rule_event = None;
        }
        if self.rules.ended().is_some() {
            return;
        }
        let player = RulePlayer {
            slot: 0,
            pos: Vec3::new(self.physics_pos.x, self.foot_y, self.physics_pos.y),
            radius: self.body.radius,
            height: self.body.body_height,
            character: self.character,
        };
        let collision_before: Vec<String> = self.rules.collision_disabled().map(str::to_string).collect();
        for effect in self.rules.step(tick, &[player]) {
            match effect {
                red_engine2::sim::rules_run::Effect::Teleport { slot: 0, target } => {
                    let target = match target {
                        Target::Point(p) => Some(p),
                        Target::Spawn(id) => self.spawns.iter().find(|s| s.id == id).map(|s| Vec3::from(s.position)),
                    };
                    if let Some(p) = target {
                        self.physics_pos = Vec2::new(p.x, p.z);
                        self.prev_physics_pos = self.physics_pos;
                        self.foot_y = p.y;
                        self.prev_foot_y = p.y;
                        self.vertical_velocity = 0.0;
                        self.horizontal_velocity = Vec2::ZERO;
                    }
                }
                red_engine2::sim::rules_run::Effect::Teleport { .. } => {}
                red_engine2::sim::rules_run::Effect::Impulse { object, dir, speed } => {
                    let object_index = self.scene.objects.iter().position(|o| o.id == object);
                    if let (Some(props), Some(object_index)) = (self.props.as_mut(), object_index) {
                        if let Some(prop) = props.prop_of_object(object_index) {
                            let at = props.prop_pose(prop).w_axis.truncate();
                            let impulse = props.mass(prop) * speed;
                            props.strike_impulse(prop, dir.normalize_or_zero(), at, impulse);
                        }
                    }
                }
            }
        }
        if !self.rules.collision_disabled().eq(collision_before.iter().map(String::as_str)) {
            self.rebuild_collision_world();
        }
        for event in self.rules.take_new_events() {
            if !event.name.starts_with("end:") {
                self.rule_event = Some(event.name);
                self.rule_event_until = tick + 120;
            }
        }
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
                self.horizontal_velocity = Vec2::ZERO;
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
                self.horizontal_velocity = st.velocity;
            }
            if self.net_title_at.elapsed().as_secs_f32() > 1.0 {
                self.net_title_at = Instant::now();
                if let Some(window) = &self.window {
                    window.set_title(&format!("Red Engine 2 — {} — {} — {}", self.scene_path.display(), self.character.name(), net.status));
                }
            }
        }

        self.feedback_frame(dt);
        self.sync_online_ui();
        self.sync_rule_hud();

        // Online, the weapon in hand is whatever the server says (it owns weapons, health and pick-ups).
        if let Some(w) = server_weapon {
            if w != self.weapon && self.net_weapon_synced {
                // The server changed it (a rung up the ladder, a respawn): lower the old weapon and raise the new one.
                self.switch.start(self.weapon);
                self.swing.cancel();
            }
            self.weapon = w;
            self.net_weapon_synced = true;
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

        let crouching =
            self.keys.contains(&KeyCode::ControlLeft) || self.keys.contains(&KeyCode::ControlRight) || self.pad.down(red_engine2::controller::button::CROUCH);
        let forward_held = self.keys.contains(&KeyCode::KeyW) || self.keys.contains(&KeyCode::ArrowUp) || self.pad.movement.y > 0.0;
        let back_held = self.keys.contains(&KeyCode::KeyS) || self.keys.contains(&KeyCode::ArrowDown);
        let sprinting = (self.sprint_held || self.pad.down(red_engine2::controller::button::SPRINT))
            && forward_held
            && !back_held
            && !crouching
            && self.scene.player.sprint_speed > self.scene.player.walk_speed;

        // Crouch: blend the eye height toward its target instead of snapping, so the camera
        // doesn't jump-cut when Ctrl is pressed/released.
        let dead = self.own_dead();
        let target_eye_height = if dead {
            feedback::DEAD_EYE_HEIGHT
        } else if crouching {
            self.body.crouch_eye
        } else {
            self.body.stand_eye
        };
        let blend = (dt / if dead { 0.4 } else { CROUCH_TRANSITION_TIME }).min(1.0);
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

        // Aim-down-sights and sprint FOV transitions use the same smooth presentation path.
        let wants_ads = (self.ads_held || self.pad.down(red_engine2::controller::button::AIM))
            && self.shown_weapon().is_firearm()
            && !self.carrying()
            && self.view_mode == ViewMode::FirstPerson;
        let ads_target = if wants_ads { 1.0 } else { 0.0 };
        self.ads_blend += (ads_target - self.ads_blend) * (dt / ADS_TRANSITION_TIME).min(1.0);
        let hip_fov = if sprinting { self.scene.player.fov_deg + SPRINT_FOV_BOOST_DEG } else { self.scene.player.fov_deg };
        let aim_fov = 2.0 * ((0.5 * self.scene.player.fov_deg.to_radians()).tan() / self.shown_weapon().aim_magnification()).atan().to_degrees();
        let target_fov = hip_fov + (aim_fov - hip_fov) * self.ads_blend;
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
        let reach = self.shown_weapon().firearm().map_or(MELEE_REACH, |s| s.range);
        // Online with a firearm the crosshair means something else: red when an enemy is in the sights and nothing solid is in front of them (gold on
        // any wall within 80 m would be on nearly all the time).
        let online_gun = self.net.is_some() && self.shown_weapon().is_firearm();
        self.target_index = if self.body.has_bat && !self.carrying() && !online_gun { self.probe(anchor, reach).map(|(o, _, _)| o) } else { None };
        self.aim_enemy = false;
        if online_gun && !self.carrying() && !self.own_dead() {
            let dir = self.camera.forward();
            let wall = raycast_shapes(anchor, dir, reach, &self.hit_shapes).map_or(f32::INFINITY, |h| h.distance);
            self.aim_enemy = self.net.as_ref().and_then(|n| n.player_in_sight(anchor, dir, reach)).is_some_and(|(_, d)| d < wall);
        }

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
        if let Some(streaks) = self.streaks.as_mut() {
            streaks.update(&mut self.scene, dt);
        }
    }

    pub(crate) fn draw(&mut self) {
        let weapon_transform = self.weapon_transform();
        let carrying = self.carrying();
        let dead = self.own_dead();
        let shown_weapon = self.shown_weapon();
        let muzzle_flash = (self.flash_left / MUZZLE_FLASH_TIME).clamp(0.0, 1.0);
        let fx = self.feel.fx(self.camera.yaw);
        let enemy = self.aim_enemy;
        let Some(gpu) = self.gpu.as_mut() else { return };
        let Some(live) = gpu.live.as_mut() else { return };
        if let Some(net) = &self.net {
            live.set_hidden_objects(net.hidden_objects());
            live.set_remote_hands(net.remote_hands());
        } else {
            live.set_hidden_objects(self.rules.hidden());
        }
        let Some((surface_tex, reconfigure)) = acquire_frame(&gpu.surface, &gpu.device, &gpu.config) else { return };
        let view = surface_tex.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let t = if self.scene.duration > 0.0 { self.start.elapsed().as_secs_f32() % self.scene.duration } else { 0.0 };
        let opts = FrameOptions {
            crosshair: true,
            viewmodel: !carrying && !dead,
            pickup: self.pickup_target.is_some(),
            weapon: shown_weapon,
            muzzle_flash,
            fx,
            enemy,
        };
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
                    let mut cfg = red_engine2::net::client::ClientConfig::new(addr, red_engine2::net::protocol::character_to_wire(who), world.map_hash, 0);
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
            self.collider_groups = collect_box_colliders_grouped_except(&self.scene, &loose);
            self.ground_groups = collect_ground_candidates_grouped_except(&self.scene, &loose);
            self.collision_object_ids = self.scene.objects.iter().map(|object| object.id.clone()).collect();
            self.rebuild_collision_world();
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
        // Debug: `RE2_WEAPON=<name>` starts with that weapon in hand (for screenshots and game launchers).
        if who != Character::Rat {
            if let Ok(wanted) = std::env::var("RE2_WEAPON") {
                if let Some(weapon) =
                    Weapon::ALL.iter().copied().find(|w| w.name().eq_ignore_ascii_case(&wanted) || format!("{w:?}").eq_ignore_ascii_case(&wanted))
                {
                    self.weapon = weapon;
                }
            }
        }
        // The pool of glowing boxes that draws tracers and sparks joins the scene now: the renderer takes its meshes from the scene as it is built.
        self.streaks = Some(red_engine2::streaks::Streaks::new(red_engine2::streaks::add_pool(&mut self.scene)));
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.live = Some(LiveRenderer::new(&gpu.device, gpu.config.format, &self.scene, gpu.config.width, gpu.config.height));
            gpu.menu = None;
        }
        self.phase = Phase::Playing;
        self.rule_hud_painted = None;
        println!("Playing as {}.", who.name());
        if let Some(window) = &self.window {
            window.set_title(&format!("Red Engine 2 — {} — {}", self.scene_path.display(), who.name()));
        }
        self.last_frame = Instant::now();
        self.net_title_at = Instant::now();
        self.set_grab(true);
    }

    fn rebuild_collision_world(&mut self) {
        self.colliders.clear();
        self.ground = GroundCandidates::default();
        for (i, id) in self.collision_object_ids.iter().enumerate() {
            if self.rules.collision_disabled().any(|disabled| disabled == id) {
                continue;
            }
            self.colliders.extend_from_slice(&self.collider_groups[i]);
            self.ground.append(&self.ground_groups[i]);
        }
    }
}
