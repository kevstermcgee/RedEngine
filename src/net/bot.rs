//! A headless client: the real networking, prediction and interpolation stack driven by a scripted
//! behaviour instead of a keyboard. It is how multiplayer is *proved* without a window (tests, the
//! `red_bot` binary), and it shares [`ClientWorld`] with the graphical client so both see the same
//! static map.

use crate::collide::{collect_box_colliders_except, collect_ground_candidates_except, Collider2D, GroundCandidates};
use crate::net::client::{ConnState, NetClient, NetEvent, TICK_SECS};
use crate::net::interp::{PlayerPose, PropPose};
use crate::net::predict::Predictor;
use crate::net::protocol::character_from_wire;
use crate::net::protocol::PlayerSnap;
use crate::physics::loose_props;
use crate::player::Character;
use crate::schema::Scene;
use crate::sim::flow::Phase;
use crate::sim::player::{PlayerInput, PlayerState};
use glam::Vec2;
use std::net::SocketAddr;
use std::path::Path;
use std::time::{Duration, Instant};

/// What a client needs of the map to predict its own movement and to place props it is told about.
pub struct ClientWorld {
    /// Static colliders (loose props excluded: they move, the server owns them).
    pub colliders: Vec<Collider2D>,
    /// Static ground candidates.
    pub ground: GroundCandidates,
    /// Prop id (as used in snapshots) -> index into `scene.objects`.
    pub prop_objects: Vec<usize>,
    /// Hash of the map file text, sent when joining.
    pub map_hash: u32,
}

impl ClientWorld {
    /// Loads the scene at `path` and builds the client's view of it. Prop ids follow
    /// `physics::loose_props`, exactly as on the server, so the scene must not yet contain the
    /// local player's own body (add that after).
    pub fn load(path: &Path) -> Result<(Scene, ClientWorld), String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let scene = crate::schema::parse_scene(&text).map_err(|e| e.join("; "))?;
        let loose = loose_props(&scene, None);
        let set = loose.iter().map(|(i, _)| *i).collect();
        let hash = crate::net::map_hash(&text);
        let world = ClientWorld::from_parts(&scene, &set, loose.iter().map(|(i, _)| *i).collect(), hash);
        Ok((scene, world))
    }

    fn from_parts(scene: &Scene, loose: &std::collections::HashSet<usize>, prop_objects: Vec<usize>, map_hash: u32) -> ClientWorld {
        ClientWorld { colliders: collect_box_colliders_except(scene, loose), ground: collect_ground_candidates_except(scene, loose), prop_objects, map_hash }
    }
}

/// A scripted way of moving.
#[derive(Debug, Clone)]
pub enum Behavior {
    /// Stand still.
    Idle,
    /// Walk straight ahead on a heading (degrees; 0 = -Z, 90 = +X).
    Forward {
        /// Heading, degrees.
        yaw_deg: f32,
        /// Hold sprint.
        sprint: bool,
    },
    /// Walk forward while turning at a constant rate (walks a circle).
    Circle {
        /// Degrees per second.
        turn_deg_per_sec: f32,
    },
    /// Walk through `(x, z)` points in order, then stop.
    Waypoints {
        /// The route.
        points: Vec<(f32, f32)>,
        /// Hold sprint.
        sprint: bool,
    },
}

/// One observation of the bot's world.
#[derive(Debug, Clone)]
pub struct BotFrame {
    /// Seconds since the bot started.
    pub t: f64,
    /// Connection state.
    pub conn: ConnState,
    /// Our own drawn (predicted) position.
    pub me: Vec2,
    /// Our predicted state.
    pub me_state: Option<PlayerState>,
    /// Other players as drawn (interpolated).
    pub remote: Vec<(u8, PlayerPose)>,
    /// Props as drawn (interpolated).
    pub props: Vec<(u16, PropPose)>,
    /// Smoothed round-trip time, ms.
    pub rtt_ms: f32,
}

/// The headless client.
pub struct Bot {
    /// The network connection.
    pub client: NetClient,
    /// Local prediction (`None` until welcomed).
    pub predictor: Option<Predictor>,
    /// The static map.
    pub world: ClientWorld,
    /// What it does.
    pub behavior: Behavior,
    started: Instant,
    next_tick: Instant,
    waypoint: usize,
    yaw: f32,
    character: Character,
    /// Every event seen, for tests: `(seconds, description)`.
    pub events: Vec<(f64, String)>,
    /// Action buttons held on every input the bot sends (`PlayerInput::flags` bits: 8 interact, 16 attack, 32 reload,
    /// 64 switch). The server acts on the press, so set a button for a few ticks, then clear it.
    pub buttons: u8,
    /// Look pitch the bot sends, radians (negative = down): tests aim at low props with it.
    pub pitch: f32,
    /// Press Ready whenever the match is in the lobby or showing results (how a bot takes part in a match flow and asks for a rematch).
    pub auto_ready: bool,
}

impl Bot {
    /// Starts joining `server` as `character` on the map in `world`. `resume_token` is `0` for a fresh join.
    pub fn new(server: SocketAddr, character: Character, world: ClientWorld, behavior: Behavior, resume_token: u64) -> std::io::Result<Bot> {
        let code = if character == Character::Rat { 1 } else { 0 };
        Self::with_client(NetClient::connect(server, code, world.map_hash, resume_token)?, character, world, behavior)
    }

    /// Like [`Bot::new`] with a client the caller configured (a join key, a name).
    pub fn with_client(client: NetClient, character: Character, world: ClientWorld, behavior: Behavior) -> std::io::Result<Bot> {
        let now = Instant::now();
        Ok(Bot {
            client,
            predictor: None,
            world,
            behavior,
            started: now,
            next_tick: now,
            waypoint: 0,
            yaw: 0.0,
            character,
            events: Vec::new(),
            buttons: 0,
            pitch: 0.0,
            auto_ready: false,
        })
    }

    fn own_state(&self, s: &PlayerSnap) -> PlayerState {
        PlayerState { pos: Vec2::new(s.pos[0], s.pos[2]), foot_y: s.pos[1], vy: s.vy, yaw: s.yaw, pitch: s.pitch, character: character_from_wire(s.character) }
    }

    fn decide(&mut self, st: &PlayerState, dt: f32) -> PlayerInput {
        match &self.behavior {
            Behavior::Idle => PlayerInput { yaw: st.yaw, ..Default::default() },
            Behavior::Forward { yaw_deg, sprint } => PlayerInput { forward: 1, sprint: *sprint, yaw: yaw_deg.to_radians(), ..Default::default() },
            Behavior::Circle { turn_deg_per_sec } => {
                self.yaw += turn_deg_per_sec.to_radians() * dt;
                PlayerInput { forward: 1, yaw: self.yaw, ..Default::default() }
            }
            Behavior::Waypoints { points, sprint } => {
                while self.waypoint < points.len() && (Vec2::new(points[self.waypoint].0, points[self.waypoint].1) - st.pos).length() < 0.3 {
                    self.waypoint += 1;
                }
                match points.get(self.waypoint) {
                    None => PlayerInput { yaw: st.yaw, ..Default::default() },
                    Some(&(x, z)) => {
                        let d = Vec2::new(x, z) - st.pos;
                        PlayerInput { forward: 1, sprint: *sprint, yaw: d.x.atan2(-d.y), ..Default::default() }
                    }
                }
            }
        }
    }

    /// Drives the connection and runs every input tick that is due. Call often.
    pub fn pump(&mut self, now: Instant) {
        for ev in self.client.poll(now) {
            let t = now.duration_since(self.started).as_secs_f64();
            match ev {
                NetEvent::Connected(w) if !w.in_round => {
                    self.events.push((t, format!("connected as player {} (watching: not in a round)", w.player_id)));
                }
                NetEvent::Connected(w) => {
                    let st = PlayerState {
                        pos: Vec2::new(w.spawn[0], w.spawn[2]),
                        foot_y: w.spawn[1],
                        vy: 0.0,
                        yaw: w.spawn[3],
                        pitch: 0.0,
                        character: character_from_wire(w.character),
                    };
                    match &mut self.predictor {
                        Some(p) => p.teleport(st),
                        None => self.predictor = Some(Predictor::new(st)),
                    }
                    self.yaw = w.spawn[3];
                    self.character = st.character;
                    self.next_tick = now;
                    self.waypoint = 0;
                    self.events.push((t, format!("connected as player {} at ({:.2}, {:.2})", w.player_id, w.spawn[0], w.spawn[2])));
                }
                NetEvent::PhaseChanged { phase, round } => self.events.push((t, format!("phase {} round {round}", phase.name()))),
                NetEvent::Snapshot { own: Some(own), ack_input_seq } => {
                    let server_state = self.own_state(&own);
                    if let Some(p) = &mut self.predictor {
                        p.reconcile(server_state, ack_input_seq, &self.world.colliders, &self.world.ground);
                    }
                }
                NetEvent::Snapshot { own: None, .. } => {}
                NetEvent::Disconnected => self.events.push((t, "disconnected (server silent)".into())),
                NetEvent::Rejected(r) => self.events.push((t, format!("rejected: {r:?}"))),
                NetEvent::ServerBye => self.events.push((t, "server said bye".into())),
            }
        }
        if self.auto_ready
            && self.client.state() == ConnState::Connected
            && !self.client.is_ready()
            && matches!(self.client.phase(), Phase::Waiting | Phase::Results)
        {
            self.client.set_ready(true, now);
        }
        if self.client.state() != ConnState::Connected || !self.client.in_round() {
            self.next_tick = now;
            return;
        }
        let tick = Duration::from_secs_f64(TICK_SECS);
        let mut ran = 0;
        while now >= self.next_tick && ran < 8 {
            let Some(state) = self.predictor.as_ref().map(|p| p.state) else {
                self.next_tick = now; // connected but no snapshot yet: nothing to predict from
                return;
            };
            let mut input = self.decide(&state, TICK_SECS as f32);
            input = input.with_flags(input.flags() | self.buttons);
            input.pitch = self.pitch;
            let Some(p) = self.predictor.as_mut() else { return };
            input.seq = p.next_seq();
            p.apply_local(input, &self.world.colliders, &self.world.ground);
            self.client.send_input(input, now);
            self.next_tick += tick;
            ran += 1;
        }
        if ran == 8 {
            self.next_tick = now + tick;
        }
    }

    /// What the bot sees right now.
    pub fn frame(&self, now: Instant) -> BotFrame {
        let view = self.client.view(now);
        BotFrame {
            t: now.duration_since(self.started).as_secs_f64(),
            conn: self.client.state(),
            me: self.predictor.as_ref().map_or(Vec2::ZERO, |p| p.visual_pos()),
            me_state: self.predictor.as_ref().map(|p| p.state),
            remote: view.players,
            props: view.props,
            rtt_ms: self.client.stats().rtt_ms,
        }
    }

    /// Runs for `duration` at roughly 250 pumps per second, calling `on_frame` after each; stops early
    /// if it returns `false`.
    pub fn run(&mut self, duration: Duration, mut on_frame: impl FnMut(&BotFrame) -> bool) {
        let end = Instant::now() + duration;
        loop {
            let now = Instant::now();
            if now >= end {
                break;
            }
            self.pump(now);
            let f = self.frame(now);
            if !on_frame(&f) {
                break;
            }
            std::thread::sleep(Duration::from_millis(3));
        }
    }
}
