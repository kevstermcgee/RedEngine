//! The per-frame game loop: fixed-step physics and input sampling (the same `sim::player::step_player` the server runs), the
//! variable-rate `update` (camera, avatar, scene sync), drawing, and starting a game from the menu.

use super::*;
use red_engine2::streaks::Streaks;

/// The ids of the scene objects the renderer must not draw this frame: what the game rules (or, online, the server's rule state) hide, the avatars nobody wears, the
/// tracer boxes showing nothing, and `own_body` (the player's own body in first person). A pooled object scaled to a speck is still a draw call in both passes; a hidden
/// one is none. A free function over the fields it reads so the caller can hold the list while it borrows the GPU state.
pub(crate) fn hidden_ids<'a>(
    net: &'a Option<NetSession>,
    rules: &'a RulesEngine,
    streaks: &'a Option<Streaks>,
    scene: &'a Scene,
    own_body: Option<usize>,
) -> Vec<&'a str> {
    let mut ids: Vec<&str> = Vec::new();
    match net {
        Some(net) => {
            ids.extend(net.hidden_objects());
            ids.extend(net.hidden_avatar_ids(scene));
        }
        None => ids.extend(rules.hidden()),
    }
    if let Some(s) = streaks {
        ids.extend(s.hidden_ids(scene));
    }
    if let Some(o) = own_body.and_then(|i| scene.objects.get(i)) {
        ids.push(o.id.as_str());
    }
    ids
}

/// Id of the synthesized flashlight light [`App::sync_flashlight`] maintains — never authored in a scene's own JSON.
const FLASHLIGHT_ID: &str = "__flashlight";
/// How far ahead of the eye the light sits, metres (clear of the player's own head/body).
const FLASHLIGHT_OFFSET: f32 = 0.3;
/// How far the flashlight reaches, metres.
const FLASHLIGHT_RANGE: f32 = 9.0;
/// Brightness when on; 0 when off (no object removal needed, it is simply dark).
const FLASHLIGHT_INTENSITY: f32 = 6.0;

/// The flashlight's light for this frame: a warm `Point` light at `eye + forward * FLASHLIGHT_OFFSET`, dark when `on` is
/// false. A `Point`, not a cone: the engine has no spotlight kind (see the flashlight ADR). No shadow casting — one more
/// shadow-casting light recomputed every frame is a real cost the atmosphere does not need to pay for a small game.
pub(crate) fn flashlight_light(eye: Vec3, forward: Vec3, on: bool) -> Light {
    Light {
        id: FLASHLIGHT_ID.to_string(),
        kind: LightKind::Point { position: Track::constant(eye + forward * FLASHLIGHT_OFFSET), range: FLASHLIGHT_RANGE },
        color: Track::constant(Vec3::new(1.0, 0.92, 0.75)),
        intensity: Track::constant(if on { FLASHLIGHT_INTENSITY } else { 0.0 }),
        cast_shadows: false,
        shadow_radius: 0.0,
        shadow_center: Vec3::ZERO,
        shadow_follow: false,
    }
}

/// Inserts or updates the flashlight light in `lights` in place: a pure helper behind [`App::sync_flashlight`],
/// factored out so it is testable against a plain `Vec<Light>` rather than a whole live `App`.
fn sync_flashlight_into(lights: &mut Vec<Light>, eye: Vec3, forward: Vec3, on: bool) {
    let light = flashlight_light(eye, forward, on);
    match lights.iter_mut().find(|l| l.id == FLASHLIGHT_ID) {
        Some(existing) => *existing = light,
        None => lights.push(light),
    }
}

impl App {
    /// The scene's animation time for this frame: seconds into a looping animation (`duration`), or, in a scene with a `clock`, the game's own seconds
    /// (fixed steps run, so a paused game keeps its sunset and a headless script can live through a whole day in a moment).
    pub(crate) fn scene_time(&self) -> f32 {
        if self.scene.clock.is_some() {
            self.clock.ticks_run() as f32 * FIXED_DT
        } else if self.scene.duration > 0.0 {
            self.start.elapsed().as_secs_f32() % self.scene.duration
        } else {
            0.0
        }
    }

    /// Keeps the synthesized flashlight light (if `scene.flashlight`) following the camera: inserts it once, then
    /// updates it in place every frame. A no-op for a scene that never asked for a flashlight.
    pub(crate) fn sync_flashlight(&mut self) {
        if !self.scene.flashlight {
            return;
        }
        sync_flashlight_into(&mut self.scene.lights, self.eye, self.camera.forward(), self.flashlight_on);
    }

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
                self.aim_at_nearest_visible();
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
        if self.net.as_ref().is_some_and(|n| n.is_race()) {
            return self.build_kart_input();
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
            aim: false,
            drop: false,
            select: 0,
        }
    }

    /// What the driver is asking for in a kart race, from the keys and the gamepad (`controller::kart_controls`): W/S or up/down throttle and brake, A/D or
    /// left/right steer, Space hop and drift (held), the mouse button or F the item, E the driver's ability. The pad's analog stick and triggers win when
    /// they are being used.
    fn build_kart_input(&mut self) -> PlayerInput {
        let pad = red_engine2::controller::kart_controls(&self.pad);
        let held = |a: KeyCode, b: KeyCode| self.keys.contains(&a) || self.keys.contains(&b);
        let axis = |pos: bool, neg: bool| pos as i8 - neg as i8;
        let analog = pad.active();
        let key_forward = axis(held(KeyCode::KeyW, KeyCode::ArrowUp), held(KeyCode::KeyS, KeyCode::ArrowDown));
        let key_steer = axis(held(KeyCode::KeyD, KeyCode::ArrowRight), held(KeyCode::KeyA, KeyCode::ArrowLeft));
        PlayerInput {
            seq: 0,
            forward: if analog { (pad.throttle * 127.0).round() as i8 } else { key_forward },
            strafe: if analog { (pad.steer * 127.0).round() as i8 } else { key_steer },
            analog,
            jump: pad.hop || self.keys.contains(&KeyCode::Space),
            interact: pad.ability || self.take_pulse(0),
            attack: pad.item || self.keys.contains(&KeyCode::KeyF) || self.take_pulse(1),
            ..Default::default()
        }
    }

    /// One fixed-size (`FIXED_DT`) physics step: movement/collision + jump/gravity, sampling
    /// currently-held input fresh (input state doesn't change within a rendered frame between
    /// steps). Snapshots the pre-step planar position/foot height into `prev_physics_pos`/
    /// `prev_foot_y` first, so `update` can interpolate between them for the actual rendered
    /// frame instead of drawing exactly on whichever physics step boundary just landed.
    pub(crate) fn fixed_step_physics(&mut self) {
        self.fixed_step_player();
        // The other local players take the same step with their own state; the rules then see all of them where they have got to.
        for slot in 1..self.local_player_count() {
            self.as_player(slot, |app| app.fixed_step_player());
        }
        if self.net.is_none() {
            self.fixed_step_rules();
        }
    }

    /// One tick of the player swapped in: their movement, the props they shove, their feel and their combat.
    fn fixed_step_player(&mut self) {
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
        // In a race `attack` uses the item (the kart step handles it): it must not swing a bat or fire a gun as well.
        let attack_now = input.attack && !self.net.as_ref().is_some_and(|n| n.is_race());
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
        if let Some(props) = self.props.as_mut().filter(|_| self.slot == 0) {
            props.set_player(Vec3::new(self.physics_pos.x, self.foot_y, self.physics_pos.y), self.body.radius, self.body.body_height);
            props.step();
        }

        self.feel.observe_motion(self.last_move_speed, FIXED_DT, self.vertical_velocity, self.vertical_velocity == 0.0, self.pad_launch, self.sprint_held);
        self.fixed_step_combat(attack_now);
    }

    /// Saves the `persist` variables when any of them has changed since the last save.
    fn persist_vars(&mut self) {
        let now: std::collections::BTreeMap<String, f64> = self.rules.persisted().into_iter().collect();
        if !now.is_empty() && now != self.saved_vars {
            if let Err(e) = red_engine2::settings::save_vars(&self.settings_key, &now) {
                eprintln!("could not save the game's progress: {e}");
            }
            self.saved_vars = now;
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
        self.persist_vars();
        // A scene with a clock tells its rules when the sun comes up and goes down (`when: {event: "sunrise"}`).
        if let Some(clock) = &self.scene.clock {
            if let Some(event) = self.sun_watch.at(clock, tick as f32 * FIXED_DT) {
                self.rules.inject(tick, event, None);
            }
        }
        if self.rules.ended().is_some() {
            return;
        }
        let mut players = vec![RulePlayer {
            slot: 0,
            pos: Vec3::new(self.physics_pos.x, self.foot_y, self.physics_pos.y),
            radius: self.body.radius,
            height: self.body.body_height,
            character: self.character,
            team: 0,
        }];
        for ctx in self.locals.iter().flatten() {
            players.push(RulePlayer {
                slot: ctx.slot,
                pos: Vec3::new(ctx.physics_pos.x, ctx.foot_y, ctx.physics_pos.y),
                radius: ctx.body.radius,
                height: ctx.body.body_height,
                character: ctx.character,
                team: 0,
            });
        }
        let collision_before: Vec<String> = self.rules.collision_disabled().map(str::to_string).collect();
        // The same prop view the authoritative simulation feeds its rules (built only when a rule looks at props).
        let mut prop_views: Vec<red_engine2::sim::rules_run::RuleProp> = Vec::new();
        if let (true, Some(props)) = (self.rules.needs_props(), self.props.as_ref()) {
            prop_views.extend((0..props.props().len()).map(|k| red_engine2::sim::rules_run::RuleProp::of(props, k)));
        }
        for effect in self.rules.step_props(tick, &players, &prop_views) {
            match effect {
                red_engine2::sim::rules_run::Effect::Teleport { slot, target } => {
                    let target = match target {
                        Target::Point(p) => Some(p),
                        Target::Spawn(id) => self.spawns.iter().find(|s| s.id == id).map(|s| Vec3::from(s.position)),
                    };
                    if let Some(p) = target {
                        if slot == 0 {
                            self.physics_pos = Vec2::new(p.x, p.z);
                            self.prev_physics_pos = self.physics_pos;
                            self.foot_y = p.y;
                            self.prev_foot_y = p.y;
                            self.vertical_velocity = 0.0;
                            self.horizontal_velocity = Vec2::ZERO;
                        } else if let Some(Some(ctx)) = self.locals.get_mut(slot - 1) {
                            ctx.physics_pos = Vec2::new(p.x, p.z);
                            ctx.prev_physics_pos = ctx.physics_pos;
                            ctx.foot_y = p.y;
                            ctx.prev_foot_y = p.y;
                            ctx.vertical_velocity = 0.0;
                            ctx.horizontal_velocity = Vec2::ZERO;
                        }
                    }
                }
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
                red_engine2::sim::rules_run::Effect::Reset { prop } => {
                    if let Some(props) = self.props.as_mut() {
                        props.reset_prop(prop);
                    }
                }
                red_engine2::sim::rules_run::Effect::Place { prop, at } => {
                    if let Some(props) = self.props.as_mut() {
                        props.place_prop(prop, at);
                    }
                }
            }
        }
        if !self.rules.collision_disabled().eq(collision_before.iter().map(String::as_str)) {
            self.rebuild_collision_world();
        }
        for event in self.rules.take_new_events() {
            self.fresh_events.push(event.name.clone());
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

    /// Everything about the player swapped in that follows the render frame rather than the tick: where they are drawn between two ticks, their eye and crouch, their
    /// body and its walk, their camera (first or third person), field of view, what they could pick up and what the crosshair is on.
    fn update_player_view(&mut self, dt: f32, alpha: f32) {
        // Between two physics states, the short way round (a looping world's seam is crossed between two ticks now and then).
        let mut planar_pos = self.prev_physics_pos + self.scene.player.expanse.delta(self.prev_physics_pos, self.physics_pos) * alpha;
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
        self.update_player_body(planar_pos, foot_y, body_yaw_deg, self.last_move_speed, dt);

        let anchor = Vec3::new(planar_pos.x, foot_y + self.eye_height, planar_pos.y);
        self.eye = anchor;
        if self.slot == 0 {
            self.sync_flashlight();
        }
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
                // In a generated world the trees are walls for the camera too, and it never dips under the hill behind the character.
                let walls = match self.ground.procgen() {
                    Some(world) => world.colliders_near(Vec2::new(desired.x, desired.z), 4.0, &self.colliders),
                    None => self.colliders.clone(),
                };
                let active = colliders_on_floor(&walls, foot_y);
                let cam_radius = THIRD_PERSON_CAM_RADIUS.min(self.body.radius * 0.7);
                let clamped = resolve_collision(Vec2::new(desired.x, desired.z), cam_radius, &active);
                let floor = self.ground.terrain_height_at(clamped).map_or(f32::NEG_INFINITY, |h| h + 0.4);
                Vec3::new(clamped.x, desired.y.max(floor), clamped.y)
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
        // A kart race is seen from behind and above the kart: the chase camera decides where the camera is and what it looks at.
        if let Some(view) = self.net.as_ref().and_then(|n| n.kart_view()) {
            let pose = self.chase.update(&view, dt);
            self.camera.position = pose.eye;
            self.camera.yaw = pose.yaw;
            self.camera.pitch = pose.pitch;
            self.camera.fov_deg = pose.fov_deg;
        }

        // Loose props: keep a carried one in front of the player, write every prop's physics pose into
        // the scene, and see what the crosshair could pick up.
        if let Some(props) = self.props.as_mut().filter(|_| self.slot == 0) {
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
    }

    /// The screen effects for this frame: the fight's (when the scene shows combat) under the opening fade from black.
    pub(crate) fn fx_now(&self) -> red_engine2::feel::FxParams {
        let fx = if self.scene.hud.shows_combat() { self.feel.fx(self.camera.yaw) } else { Default::default() };
        let secs = self.scene.player.fade_in;
        if secs <= 0.0 {
            return fx;
        }
        // Black until play begins (behind a start card), then eased clear.
        let t = (self.fade_age / secs).clamp(0.0, 1.0);
        fx.with_black(1.0 - t * t * (3.0 - 2.0 * t))
    }

    pub(crate) fn update(&mut self, dt: f32) {
        // Online, the connection must be serviced even when the window is not focused (or the server
        // would time us out), so the simulation keeps ticking; offline it pauses like before.
        // A start or end card is up: the game waits for its button (the clock still runs for scripts and pictures).
        if self.card.is_some() && self.net.is_none() {
            self.frame_no += 1;
            self.play_secs += dt;
            self.sync_rule_hud();
            return;
        }
        if !self.grabbed && self.net.is_none() {
            return;
        }
        self.frame_no += 1;
        self.play_secs += dt;
        self.fade_age += dt;
        self.fps_avg += (1.0 / dt.max(1e-4) - self.fps_avg) * 0.1;
        if self.debug_hud && self.frame_no % 15 == 1 {
            self.debug_text = self.debug_lines(); // four times a second: the overlay is repainted only when its text changes
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
        self.sync_card();
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
        self.note_listeners();
        while self.clock.next_tick().is_some() {
            self.fixed_step_physics();
        }
        let alpha = self.clock.alpha();
        self.update_player_view(dt, alpha);
        for slot in 1..self.local_player_count() {
            self.as_player(slot, |app| app.update_player_view(dt, alpha));
        }
        self.update_ambient(dt);
        if let Some(audio) = &self.audio {
            audio.tick();
        }
        if let Some(net) = self.net.as_mut() {
            // Other players and the server's props, interpolated, into the scene the renderer draws.
            net.update_scene(&mut self.scene, Instant::now(), dt);
            // A player nobody can see is a bug to report at once, in words, not one to find by looking at a screenshot.
            for warning in net.take_warnings() {
                eprintln!("warning: {warning}");
                self.failures.push(warning);
            }
        }
        if let Some(streaks) = self.streaks.as_mut() {
            streaks.update(&mut self.scene, dt);
        }
    }

    pub(crate) fn draw(&mut self) {
        // Pictures asked for (`--shot-at`, F12, a script) are drawn first, offscreen: they need neither a visible window nor focus, and the window's own frame below
        // may well not be presented at all (a minimised or occluded window).
        self.take_due_shots();
        if self.split.is_some() {
            self.draw_split();
            return;
        }
        let weapon_transform = self.weapon_transform();
        let carrying = self.carrying();
        let dead = self.own_dead();
        let shown_weapon = self.shown_weapon();
        let muzzle_flash = (self.flash_left / MUZZLE_FLASH_TIME).clamp(0.0, 1.0);
        let hud = &self.scene.hud;
        let peaceful = self.scene.player.mode.is_peaceful();
        let fx = self.fx_now();
        let enemy = self.aim_enemy && hud.shows_combat();
        let (show_crosshair, show_viewmodel) = (hud.shows_crosshair(), !peaceful);
        let racing = self.net.as_ref().is_some_and(|n| n.is_race());
        let hidden = hidden_ids(
            &self.net,
            &self.rules,
            &self.streaks,
            &self.scene,
            (self.view_mode == ViewMode::FirstPerson || racing).then_some(self.player_object_index),
        );
        let t = self.scene_time();
        let Some(gpu) = self.gpu.as_mut() else { return };
        let Some(live) = gpu.live.as_mut() else { return };
        live.set_hidden_objects(hidden);
        if let Some(net) = self.net.as_ref().filter(|_| !peaceful) {
            live.set_remote_hands(net.remote_hands());
        }
        let Some(surface) = gpu.surface.as_ref() else { return }; // a headless run has nothing to present
        let Some((surface_tex, reconfigure)) = acquire_frame(surface, &gpu.device, &gpu.config) else { return };
        let view = surface_tex.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let opts = FrameOptions {
            crosshair: show_crosshair,
            viewmodel: show_viewmodel && !carrying && !dead,
            pickup: self.pickup_target.is_some(),
            weapon: shown_weapon,
            muzzle_flash,
            fx,
            enemy,
            skin: 0,
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
            surface.configure(&gpu.device, &gpu.config);
        }
    }

    /// Leaves the connect form (or skips it): adds the player's body to the map, builds everything that
    /// depends on it (colliders, hit shapes, the renderer) and captures the mouse.
    pub(crate) fn start_game(&mut self, who: Character) {
        self.character = who;
        self.body = who.body();
        self.player_object_index = self.scene.objects.len();
        self.scene.objects.push(build_player_object(who));
        // Local co-op: the other players' bodies join the scene now, before the renderer takes its meshes from it.
        if self.want_players > 1 && self.net_server.is_none() && self.pending_net.is_none() {
            if let Err(e) = self.add_local_guests(self.want_players, self.pads.count(), self.pads_only) {
                eprintln!("split-screen: {e}");
                std::process::exit(2);
            }
        }
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
                    cfg.transport = self.transport.for_server(addr).unwrap_or_else(|e| fail_online(&e));
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
            // The rules name props by object id; bind them to this world's prop numbers once, as `MatchSim` does.
            self.rules.bind_props(|id| self.scene.objects.iter().position(|o| o.id == id).and_then(|i| props.prop_of_object(i)));
            println!("{} loose props (pick up with E).", loose.len());
            self.physical = Some(PhysicalWorld::new(&self.scene, &loose));
            self.rebuild_collision_world();
            (Some(props), loose)
        };
        let _ = online;
        // The player's own body is in `scene.objects` so the renderer can draw it, but the bat must
        // never be able to hit it (e.g. looking down at your own feet), so it is skipped here.
        let bodies: Vec<usize> = std::iter::once(self.player_object_index).chain(self.locals.iter().flatten().map(|c| c.player_object_index)).collect();
        self.hit_shapes = collect_hit_shapes_where(&self.scene, |i| !bodies.contains(&i) && !loose.contains(&i));
        self.props = props;
        self.settle_guests();
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
        let split = self.gpu.as_ref().and_then(|gpu| self.split_for(&gpu.device, gpu.config.format, (gpu.config.width, gpu.config.height)));
        let players = self.local_player_count();
        if let Some(gpu) = self.gpu.as_mut() {
            // With several players on the screen the renderer is sized for one view; the compositor puts the views in the window.
            let (w, h) = split.as_ref().map_or((gpu.config.width, gpu.config.height), |s| s.view_size());
            // A peaceful scene has no weapons: none of their meshes are built or uploaded.
            let mut live = if self.scene.player.mode.is_peaceful() {
                LiveRenderer::world(&gpu.device, gpu.config.format, &self.scene, w, h)
            } else {
                LiveRenderer::new(&gpu.device, gpu.config.format, &self.scene, w, h)
            };
            if players > 1 {
                live.set_view_distance(red_engine2::splitscreen::view_distance(players));
            }
            gpu.live = Some(live);
            gpu.backdrop = None;
        }
        self.split = split;
        self.phase = Phase::Playing;
        self.rule_hud_painted = None;
        self.split_hud_painted.clear();
        self.start_music();
        println!("Playing as {}.", who.name());
        if let Some(window) = &self.window {
            window.set_title(&format!("Red Engine 2 — {} — {}", self.scene_path.display(), who.name()));
        }
        self.last_frame = Instant::now();
        self.net_title_at = Instant::now();
        self.set_grab(true);
        self.open_start_card();
    }

    /// Brings `colliders` and `ground` to what [`PhysicalWorld`] says exists now: the scene's generated world, and every object whose collision the rules have not switched off.
    fn rebuild_collision_world(&mut self) {
        let Some(world) = self.physical.as_mut() else { return };
        world.set_collision_disabled(self.rules.collision_disabled());
        self.colliders = world.colliders().to_vec();
        self.ground = world.ground().clone();
    }
}

#[cfg(test)]
mod flashlight_tests {
    use super::*;

    #[test]
    fn the_flashlight_sits_ahead_of_the_eye_and_is_dark_when_off() {
        let eye = Vec3::new(1.0, 2.0, 3.0);
        let forward = Vec3::new(0.0, 0.0, -1.0);
        let on = flashlight_light(eye, forward, true);
        assert_eq!(on.id, FLASHLIGHT_ID);
        let LightKind::Point { position, range } = on.kind else { panic!("the flashlight is a Point light, never a Directional one") };
        assert_eq!(position.sample(0.0), eye + forward * FLASHLIGHT_OFFSET);
        assert_eq!(range, FLASHLIGHT_RANGE);
        assert!(on.intensity.sample(0.0) > 0.0);
        assert!(!on.cast_shadows, "no shadow-casting cost for a light recomputed every frame");

        let off = flashlight_light(eye, forward, false);
        assert_eq!(off.intensity.sample(0.0), 0.0);
    }

    #[test]
    fn sync_flashlight_into_inserts_once_then_updates_the_same_light_in_place() {
        let mut lights: Vec<Light> = Vec::new();
        let forward = Vec3::new(0.0, 0.0, -1.0);
        sync_flashlight_into(&mut lights, Vec3::new(0.0, 1.7, 0.0), forward, true);
        assert_eq!(lights.iter().filter(|l| l.id == FLASHLIGHT_ID).count(), 1);

        let eye = Vec3::new(5.0, 1.7, 0.0);
        sync_flashlight_into(&mut lights, eye, forward, true);
        assert_eq!(lights.iter().filter(|l| l.id == FLASHLIGHT_ID).count(), 1, "the second call must update, not duplicate");
        let light = lights.iter().find(|l| l.id == FLASHLIGHT_ID).unwrap();
        let LightKind::Point { position, .. } = &light.kind else { panic!("expected a Point light") };
        assert_eq!(position.sample(0.0), eye + forward * FLASHLIGHT_OFFSET);
    }

    #[test]
    fn sync_flashlight_into_leaves_other_lights_alone() {
        let mut lights = vec![Light {
            id: "lamp".into(),
            kind: LightKind::Point { position: Track::constant(Vec3::ZERO), range: 5.0 },
            color: Track::constant(Vec3::ONE),
            intensity: Track::constant(1.0),
            cast_shadows: false,
            shadow_radius: 0.0,
            shadow_center: Vec3::ZERO,
            shadow_follow: false,
        }];
        sync_flashlight_into(&mut lights, Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0), true);
        assert_eq!(lights.len(), 2);
        assert!(lights.iter().any(|l| l.id == "lamp"), "the authored light must survive untouched");
    }
}
