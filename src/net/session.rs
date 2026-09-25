//! The networked-game glue for the graphical client, kept free of any window or GPU type so it is
//! testable: a [`NetSession`] owns the connection, the local player's prediction and the pool of
//! scene objects used to draw other players, and each frame it updates the *scene* — remote avatars
//! and props — from the interpolated network view. The graphical binary only has to call it.
//!
//! Avatars are pre-created (a fixed pool of hidden humans and rats) before the renderer is built,
//! because the renderer takes its meshes from the scene at creation; a joining player just claims one.

use crate::characters::{human_object, rat_object};
use crate::net::bot::ClientWorld;
use crate::net::client::{ClientConfig, ConnState, NetClient, NetEvent};
use crate::net::interp::PlayerPose;
use crate::net::predict::Predictor;
use crate::net::protocol::character_from_wire;
use crate::net::protocol::{PlayerSnap, MAX_PLAYERS_PER_SNAPSHOT};
use crate::physics::set_object_pose;
use crate::player::Character;
use crate::schema::{Object, ObjectKind, Scene};
use crate::sim::player::{PlayerInput, PlayerState};
use crate::track::Track;
use glam::{Vec2, Vec3};
use std::net::SocketAddr;
use std::time::Instant;

/// Scale that makes an object effectively invisible (the renderer has no per-object visibility flag).
pub const HIDDEN_SCALE: f32 = 0.0005;

const WALK_CYCLES_PER_SEC_AT_WALK_SPEED: f32 = 1.6;
const HIP_SWING_DEG: f32 = 28.0;
const KNEE_LIFT_DEG: f32 = 45.0;
const KNEE_REST_DEG: f32 = 4.0;
const SHOULDER_SWING_DEG: f32 = 20.0;
const IDLE_SWAY_DEG: f32 = 1.4;
const RAT_GAIT_RAD_PER_M: f32 = 5.0;

struct Avatar {
    object_index: usize,
    character: Character,
    /// Which remote player currently wears it.
    used_by: Option<u8>,
    phase: f32,
}

/// A connection to a server plus everything the client keeps for it.
pub struct NetSession {
    /// The connection.
    pub client: NetClient,
    /// Prediction of the local player (`None` until welcomed).
    pub predictor: Option<Predictor>,
    /// The static map as the client sees it.
    pub world: ClientWorld,
    avatars: Vec<Avatar>,
    started: Instant,
    /// Set when a Welcome arrives (first join or resume): the state to teleport the camera to.
    pub teleport: Option<PlayerState>,
    /// Latest one-line description for the window title.
    pub status: String,
    /// Speed of the local player at the last predicted tick (m/s), for the local walk animation.
    pub last_speed: f32,
    /// The server's latest word about the local player (weapon in hand, hit points, what they carry).
    pub own: Option<PlayerSnap>,
    /// Whether a `Welcome` has arrived (also true in a lobby, where there is no body to place yet).
    pub joined: bool,
}

impl NetSession {
    /// Starts joining `server`. Call [`add_avatar_pool`](Self::add_avatar_pool) on the scene, then
    /// [`wait_connected`](Self::wait_connected).
    pub fn connect(server: SocketAddr, character: Character, world: ClientWorld, resume_token: u64) -> std::io::Result<NetSession> {
        let code = if character == Character::Rat { 1 } else { 0 };
        Self::connect_with(ClientConfig::new(server, code, world.map_hash, resume_token), world)
    }

    /// Like [`NetSession::connect`] with a join key and a name (`cfg.map_hash` should be `world.map_hash`).
    pub fn connect_with(cfg: ClientConfig, world: ClientWorld) -> std::io::Result<NetSession> {
        let client = NetClient::connect_with(cfg)?;
        Ok(NetSession {
            client,
            predictor: None,
            world,
            avatars: Vec::new(),
            started: Instant::now(),
            teleport: None,
            status: "connecting...".into(),
            last_speed: 0.0,
            own: None,
            joined: false,
        })
    }

    /// Adds hidden avatar objects (8 humans, 8 rats) to `scene`. Do this before the renderer is created.
    pub fn add_avatar_pool(&mut self, scene: &mut Scene) {
        for who in [Character::Human, Character::Rat] {
            for k in 0..MAX_PLAYERS_PER_SNAPSHOT {
                let mut o: Object = match who {
                    Character::Human => human_object(&format!("net_human_{k}")),
                    Character::Rat => rat_object(&format!("net_rat_{k}")),
                };
                o.scale = Track::constant(Vec3::splat(HIDDEN_SCALE));
                o.collide = false;
                self.avatars.push(Avatar { object_index: scene.objects.len(), character: who, used_by: None, phase: 0.0 });
                scene.objects.push(o);
            }
        }
    }

    /// Polls until welcomed (or refused / `timeout_secs` passes). Returns the spawn state, or `None` when the server put us in a lobby
    /// (no body yet: the first round's `Welcome` will place us), or a message saying what went wrong and what to do.
    pub fn wait_connected(&mut self, timeout_secs: f64) -> Result<Option<PlayerState>, String> {
        let end = Instant::now() + std::time::Duration::from_secs_f64(timeout_secs);
        while Instant::now() < end {
            self.poll(Instant::now());
            match self.client.state() {
                ConnState::Connected => {
                    if let Some(st) = self.teleport.take() {
                        return Ok(Some(st));
                    }
                    if self.joined {
                        return Ok(None);
                    }
                }
                ConnState::Rejected(r) => return Err(format!("the server refused us: {}", r.explain())),
                _ => {}
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Err(format!("no answer from the server within {timeout_secs} s (is it running, and is the address right?)"))
    }

    /// The local player's current predicted state.
    pub fn state(&self) -> Option<PlayerState> {
        self.predictor.as_ref().map(|p| p.state)
    }

    fn own_state(s: &PlayerSnap) -> PlayerState {
        PlayerState { pos: Vec2::new(s.pos[0], s.pos[2]), foot_y: s.pos[1], vy: s.vy, yaw: s.yaw, pitch: s.pitch, character: character_from_wire(s.character) }
    }

    /// Receives network traffic; applies welcomes (sets [`teleport`](Self::teleport)) and reconciles
    /// prediction with each snapshot.
    pub fn poll(&mut self, now: Instant) {
        for ev in self.client.poll(now) {
            match ev {
                NetEvent::Connected(w) if !w.in_round => self.joined = true,
                NetEvent::Connected(w) => {
                    self.joined = true;
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
                    self.teleport = Some(st);
                }
                NetEvent::Snapshot { own: Some(own), ack_input_seq } => {
                    if let Some(p) = &mut self.predictor {
                        p.reconcile(Self::own_state(&own), ack_input_seq, &self.world.colliders, &self.world.ground);
                    }
                    self.own = Some(own);
                }
                _ => {}
            }
        }
        self.status = match self.client.state() {
            ConnState::Connected => {
                let s = self.client.stats();
                format!("online: player {}, ping {:.0} ms, {} others", self.client.my_id().unwrap_or(0), s.rtt_ms, self.client.view(now).players.len())
            }
            ConnState::Connecting => "connecting...".into(),
            ConnState::Reconnecting => "connection lost - reconnecting...".into(),
            ConnState::Rejected(r) => format!("refused: {r:?}"),
            ConnState::Closed => "disconnected".into(),
        };
    }

    /// One simulation tick of the local player: predict it immediately and send it to the server.
    /// Returns the new predicted state, or `None` while not connected.
    pub fn step_local(&mut self, mut input: PlayerInput, now: Instant) -> Option<PlayerState> {
        if self.client.state() != ConnState::Connected || !self.client.in_round() {
            return None; // in a lobby, a countdown or the results, or watching: the local body does not move
        }
        let p = self.predictor.as_mut()?;
        input.seq = p.next_seq();
        self.last_speed = p.apply_local(input, &self.world.colliders, &self.world.ground);
        self.client.send_input(input, now);
        Some(p.state)
    }

    /// The correction still fading out, to add to the drawn position of the local player.
    pub fn visual_offset(&self) -> Vec2 {
        self.predictor.as_ref().map_or(Vec2::ZERO, |p| p.visual_pos() - p.state.pos)
    }

    /// Updates the scene from the interpolated network view: remote players wear pooled avatar
    /// objects (position, facing, walk cycle) and props take the server's poses. Unused avatars are hidden.
    pub fn update_scene(&mut self, scene: &mut Scene, now: Instant, dt: f32) {
        let view = self.client.view(now);
        let idle_t = now.duration_since(self.started).as_secs_f32();
        // Free avatars whose player left.
        for a in &mut self.avatars {
            if let Some(id) = a.used_by {
                if !view.players.iter().any(|(pid, _)| *pid == id) {
                    a.used_by = None;
                    scene.objects[a.object_index].scale = Track::constant(Vec3::splat(HIDDEN_SCALE));
                }
            }
        }
        for (id, pose) in &view.players {
            let ch = character_from_wire(pose.character);
            let idx = match self.avatars.iter().position(|a| a.used_by == Some(*id) && a.character == ch) {
                Some(i) => i,
                None => match self.avatars.iter().position(|a| a.used_by.is_none() && a.character == ch) {
                    Some(i) => {
                        self.avatars[i].used_by = Some(*id);
                        self.avatars[i].phase = 0.0;
                        i
                    }
                    None => continue,
                },
            };
            let a = &mut self.avatars[idx];
            pose_avatar(&mut scene.objects[a.object_index], ch, pose, &mut a.phase, dt, idle_t);
        }
        for (id, pose) in &view.props {
            if let Some(&obj) = self.world.prop_objects.get(*id as usize) {
                set_object_pose(&mut scene.objects[obj], pose.pos, pose.rot);
            }
        }
    }

    /// Leaves politely.
    pub fn disconnect(&mut self) {
        self.client.disconnect();
    }
}

impl Drop for NetSession {
    fn drop(&mut self) {
        self.client.disconnect();
    }
}

/// Places one avatar object at a remote player's pose and animates it: a walk cycle driven by the
/// distance covered (`speed`), an idle sway when still.
fn pose_avatar(o: &mut Object, who: Character, pose: &PlayerPose, phase: &mut f32, dt: f32, idle_t: f32) {
    // Same convention as the local player's body: rotation = 180 - yaw.
    let yaw_deg = 180.0 - pose.yaw.to_degrees();
    o.position = Track::constant(pose.pos);
    o.rotation = Track::constant(Vec3::new(0.0, yaw_deg, 0.0));
    o.scale = Track::constant(Vec3::ONE);
    match (&mut o.kind, who) {
        (ObjectKind::Humanoid(h), Character::Human) => {
            let walk = crate::player::Character::Human.body().walk_speed;
            let (spine_x, l_hip, r_hip, l_knee, r_knee, l_sh, r_sh) = if pose.speed > 0.05 {
                *phase += dt * pose.speed * (WALK_CYCLES_PER_SEC_AT_WALK_SPEED / walk) * std::f32::consts::TAU;
                let ph = *phase;
                (
                    3.0 * (ph * 2.0).sin(),
                    HIP_SWING_DEG * ph.sin(),
                    -HIP_SWING_DEG * ph.sin(),
                    KNEE_REST_DEG + (KNEE_LIFT_DEG * (-ph).sin()).max(0.0),
                    KNEE_REST_DEG + (KNEE_LIFT_DEG * ph.sin()).max(0.0),
                    -SHOULDER_SWING_DEG * ph.sin(),
                    SHOULDER_SWING_DEG * ph.sin(),
                )
            } else {
                (IDLE_SWAY_DEG * (idle_t * 1.1).sin(), 0.0, 0.0, KNEE_REST_DEG, KNEE_REST_DEG, 0.0, 0.0)
            };
            h.pose.spine = Track::constant(Vec3::new(spine_x, 0.0, 0.0));
            h.pose.head = Track::constant(Vec3::new(pose.pitch.to_degrees().clamp(-60.0, 60.0) * -0.5, 0.0, 0.0));
            h.pose.l_hip = Track::constant(Vec3::new(l_hip, 0.0, 0.0));
            h.pose.r_hip = Track::constant(Vec3::new(r_hip, 0.0, 0.0));
            h.pose.l_knee = Track::constant(l_knee);
            h.pose.r_knee = Track::constant(r_knee);
            h.pose.l_shoulder = Track::constant(Vec3::new(l_sh, 0.0, -6.0));
            h.pose.r_shoulder = Track::constant(Vec3::new(r_sh, 0.0, 6.0));
            h.pose.l_elbow = Track::constant(8.0);
            h.pose.r_elbow = Track::constant(KNEE_REST_DEG);
        }
        (ObjectKind::Rat(r), Character::Rat) => {
            *phase += dt * pose.speed * RAT_GAIT_RAD_PER_M;
            let stride = (pose.speed / crate::player::Character::Rat.body().sprint_speed).clamp(0.0, 1.0);
            r.gait = Track::constant(*phase);
            r.stride = Track::constant(stride);
            r.sway = Track::constant(idle_t * 1.3);
        }
        _ => {}
    }
}

/// Every avatar object currently visible in `scene` (for tests): objects named `net_*` that are not hidden.
pub fn visible_avatars(scene: &Scene) -> Vec<&Object> {
    scene.objects.iter().filter(|o| o.id.starts_with("net_") && o.scale.sample(0.0).x > 0.5).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::server::{Server, ServerConfig};
    use crate::sim::match_sim::MatchSim;
    use crate::sim::spawns::parse_spawns;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// The graphical client's whole networked path, minus the window: it joins a real server, sees
    /// another client's avatar appear in the scene (and disappear when they leave), and the other
    /// client's moving prop shows up posed in the scene.
    #[test]
    fn a_networked_session_draws_the_other_player_and_moved_props_into_the_scene() {
        use crate::net::bot::{Behavior, Bot};
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json");
        let text = std::fs::read_to_string(&path).unwrap();
        let scene0 = crate::schema::parse_scene(&text).unwrap();
        let mut spawns = parse_spawns(&text).unwrap();
        spawns.retain(|s| s.group == "props");
        let sim = MatchSim::new(&scene0, spawns);
        let mut server = Server::bind(ServerConfig::new("127.0.0.1:0".parse().unwrap(), crate::net::map_hash(&text)), sim).unwrap();
        server.set_logger(|_| {});
        let addr: SocketAddr = format!("127.0.0.1:{}", server.local_addr().unwrap().port()).parse().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let handle = std::thread::spawn(move || {
            server.run(&s2);
            server
        });

        // The "graphical" client (no window): joins first, stands still at the first prop spawn.
        let (mut scene, world) = ClientWorld::load(&path).unwrap();
        let mut session = NetSession::connect(addr, Character::Human, world, 0).unwrap();
        session.add_avatar_pool(&mut scene);
        session.wait_connected(4.0).expect("joined");
        assert!(visible_avatars(&scene).is_empty(), "nobody else is here yet");

        // A bot joins second (walks forward into a barrel), then leaves.
        let (_s, bw) = ClientWorld::load(&path).unwrap();
        let mut bot = Bot::new(addr, Character::Rat, bw, Behavior::Forward { yaw_deg: 0.0, sprint: false }, 0).unwrap();
        let mut saw_avatar = false;
        let mut moved_prop_posed = false;
        let end = Instant::now() + Duration::from_millis(3500);
        let authored: Vec<Vec3> = session.world.prop_objects.iter().map(|&o| scene.objects[o].position.sample(0.0)).collect();
        while Instant::now() < end {
            let now = Instant::now();
            bot.pump(now);
            session.poll(now);
            session.update_scene(&mut scene, now, 0.01);
            saw_avatar |= visible_avatars(&scene).len() == 1;
            moved_prop_posed |=
                session.world.prop_objects.iter().enumerate().any(|(i, &o)| (scene.objects[o].position.sample(0.0) - authored[i]).length() > 0.05);
            std::thread::sleep(Duration::from_millis(4));
        }
        assert!(saw_avatar, "the other player's avatar appeared in the scene");
        assert!(moved_prop_posed, "a prop moved by the other player was posed in the scene");
        let visible = visible_avatars(&scene);
        assert!(
            visible.iter().all(|o| o.id.starts_with("net_rat_")),
            "the bot is a rat, so a rat avatar is used: {:?}",
            visible.iter().map(|o| &o.id).collect::<Vec<_>>()
        );

        // The other player leaves: the avatar is hidden again. Normally at once (the Bye); if the Bye is lost (or the bot had just timed out and
        // has no session key to sign it with) the server's 3 s timeout is the fallback, so allow for that under a loaded machine.
        bot.client.disconnect();
        let end = Instant::now() + Duration::from_millis(6000);
        while Instant::now() < end && !visible_avatars(&scene).is_empty() {
            let now = Instant::now();
            session.poll(now);
            session.update_scene(&mut scene, now, 0.01);
            std::thread::sleep(Duration::from_millis(4));
        }
        assert!(visible_avatars(&scene).is_empty(), "the avatar of a player who left is hidden");
        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
    }
}
