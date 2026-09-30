//! The networked-game glue for the graphical client, kept free of any window or GPU type so it is
//! testable: a [`NetSession`] owns the connection, the local player's prediction and the pool of
//! scene objects used to draw other players, and each frame it updates the *scene* — remote avatars
//! and props — from the interpolated network view. The graphical binary only has to call it.
//!
//! Avatars are pre-created (a fixed pool of hidden bodies, one set for every body somebody in the game can wear, sized by [`avatar_plan`]) before the
//! renderer is built, because the renderer takes its meshes from the scene at creation; a joining player just claims one.
//!
//! **Nothing is skipped silently.** Every lookup that can fail is counted and shown: a remote player with no avatar to wear (`undrawn`), one drawn in another
//! body (`standins`), one the server left out (`hidden_by_interest`), one the interpolation cannot pose yet (`unposed`), a prop nobody knows (`unknown_props`).
//! [`NetSession::stats`] has them for this frame, [`NetSession::counters`] for the whole session, [`NetSession::take_warnings`] says it in words once.

use crate::avatar::{animate, AvatarAnim, RemoteHand};
use crate::characters::character_object;
use crate::net::bot::ClientWorld;
use crate::net::client::{ClientConfig, ConnState, NetClient, NetEvent};
use crate::net::interp::View;
use crate::net::predict::Predictor;
use crate::net::protocol::character_from_wire;
use crate::net::protocol::{PlayerSnap, RosterEntry, MAX_PLAYERS_PER_SNAPSHOT, ROSTER_IN_ROUND};
use crate::physics::set_object_pose;
use crate::player::Character;
use crate::scene_pool::{PoolStats, ScenePool};
use crate::schema::{Object, Scene};
use crate::sim::player::{PlayerInput, PlayerState};
use glam::{Vec2, Vec3};
use std::net::SocketAddr;
use std::time::Instant;

pub use crate::scene_pool::HIDDEN_SCALE;

/// Avatars kept ready for each body that only bots can wear (see [`avatar_plan`]). A roster seldom repeats a body more than twice and the
/// pool cannot grow once the renderer is built, so four is a comfortable margin; a fifth wearer gets a stand-in of another costume, never nothing.
pub const BOT_BODY_POOL: usize = 4;

/// How a client prepares avatars for a scene, per body, in [`Character::ALL`] order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvatarPlan {
    /// Avatars to prepare.
    pub pool: [usize; 8],
    /// The most other players who can be wearing the body at once (humans where they may pick it, and the scene's bots, roster first, then the generated ones).
    pub needed: [usize; 8],
}

impl AvatarPlan {
    /// The bodies where fewer avatars are prepared than a match can need (a stand-in of another costume will be drawn for the rest).
    pub fn short(&self) -> Vec<Character> {
        Character::ALL.iter().enumerate().filter(|(i, _)| self.pool[*i] < self.needed[*i]).map(|(_, c)| *c).collect()
    }
}

fn body_index(who: Character) -> usize {
    Character::ALL.iter().position(|c| *c == who).unwrap_or(0)
}

/// Sizes the avatar pool for `scene`: [`MAX_PLAYERS_PER_SNAPSHOT`] for every body a human can wear (any, or the one `player.humans_play_as` forces),
/// and for the others as many as the scene's bots wear (`bots.roster` bodies, then the generated fighters in rotation), at least [`BOT_BODY_POOL`] for
/// the fighting bodies when the scene has bots at all (a server can fill a match with bots the map never names). A body nobody can wear costs nothing.
pub fn avatar_plan(scene: &Scene) -> AvatarPlan {
    let forced = scene.player.character;
    let bots = &scene.bots;
    let mut bot_bodies = [0usize; 8];
    for k in 0..bots.fill {
        bot_bodies[body_index(bots.spec(k).character)] += 1;
    }
    let (mut pool, mut needed) = ([0usize; 8], [0usize; 8]);
    let team_match = scene.shooter.is_some();
    for (i, who) in Character::ALL.iter().enumerate() {
        // A team match shows soldiers only (one uniform per team); every other match never draws one.
        let soldier = matches!(who, Character::Ridgeback | Character::Nightfall);
        if soldier != team_match {
            continue;
        }
        if soldier {
            pool[i] = crate::sim::shooter::MAX_TEAM + 1;
            needed[i] = crate::sim::shooter::MAX_TEAM;
            continue;
        }
        let human = forced.is_none_or(|f| f == *who);
        pool[i] = if human { MAX_PLAYERS_PER_SNAPSHOT } else { bot_bodies[i].max(if bots.fill > 0 && *who != Character::Rat { BOT_BODY_POOL } else { 0 }) };
        needed[i] = (bot_bodies[i] + if human { MAX_PLAYERS_PER_SNAPSHOT - 1 } else { 0 }).min(MAX_PLAYERS_PER_SNAPSHOT - 1);
    }
    AvatarPlan { pool, needed }
}

/// One body's avatars: the scene objects, and who wears each.
struct AvatarPool {
    body: Character,
    pool: ScenePool,
    used_by: Vec<Option<u8>>,
    anim: Vec<AvatarAnim>,
}

/// Adds the hidden avatars [`avatar_plan`] asks for to `scene` (before the renderer is built).
fn build_avatar_pools(scene: &mut Scene) -> Vec<AvatarPool> {
    let plan = avatar_plan(scene);
    let mut pools = Vec::new();
    for (i, who) in Character::ALL.iter().enumerate() {
        let count = plan.pool[i];
        if count == 0 {
            continue;
        }
        let pool = ScenePool::add(scene, count, |k| {
            let mut o: Object = character_object(*who, &format!("net_{who:?}_{k}").to_lowercase());
            o.collide = false;
            o
        });
        pools.push(AvatarPool { body: *who, pool, used_by: vec![None; count], anim: (0..count).map(|_| AvatarAnim::default()).collect() });
    }
    pools
}

/// What a client would really have: builds the avatar pool for the scene in `scene_json` exactly as [`NetSession::add_avatar_pool`] does and counts the avatars made per
/// body, in [`Character::ALL`] order. `game check` compares it with what the scene's bots wear.
pub fn avatars_made(scene_json: &str) -> Result<[usize; 6], Vec<String>> {
    let mut scene = crate::schema::parse_scene(scene_json)?;
    let mut made = [0usize; 6];
    for pool in build_avatar_pools(&mut scene) {
        made[body_index(pool.body)] = pool.pool.capacity();
    }
    Ok(made)
}

/// A remote player as drawn this frame: enough to tell whether the crosshair is on them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RemoteBody {
    /// Their player id.
    pub id: u8,
    /// Where they stand (x, foot_y, z), as drawn (interpolated).
    pub pos: Vec3,
    /// Dead, waiting to respawn.
    pub dead: bool,
    /// What they look like (sets the size of their body).
    pub character: Character,
}

/// A remote player and the avatar object that draws them this frame (for reports and tests).
#[derive(Debug, Clone, PartialEq)]
pub struct DrawnPlayer {
    /// Their player id.
    pub id: u8,
    /// The body they wear in the game.
    pub body: Character,
    /// The body of the avatar that stands in for them (differs from `body` for a stand-in).
    pub avatar_body: Character,
    /// The id of the scene object that draws them.
    pub avatar: String,
    /// Where they are drawn.
    pub pos: Vec3,
    /// Dead, lying where they fell.
    pub dead: bool,
}

/// The nearest living body a ray from `eye` along the unit vector `dir` meets within `reach`, and how far along it: `(player id, distance)`.
pub fn nearest_body_on_ray(bodies: &[RemoteBody], eye: Vec3, dir: Vec3, reach: f32) -> Option<(u8, f32)> {
    bodies
        .iter()
        .filter(|b| !b.dead)
        .filter_map(|b| {
            let body = b.character.body();
            crate::sim::interact::ray_cylinder(eye, dir, reach, Vec2::new(b.pos.x, b.pos.z), body.radius, b.pos.y, body.body_height).map(|d| (b.id, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// What the client did with the other players this frame; every way one can fail to be drawn has a number.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteStats {
    /// Other players the interpolation has a pose for.
    pub in_view: usize,
    /// ...of which have an avatar to draw them.
    pub drawn: usize,
    /// ...of which wear an avatar of another body (the pool held none of theirs).
    pub standins: usize,
    /// ...of which have NO avatar: invisible. Should be zero; each frame it is not is a bug someone will report as "I can't see the enemies".
    pub undrawn: usize,
    /// The ids of the undrawn players.
    pub undrawn_ids: Vec<u8>,
    /// Other players the snapshots mention but the interpolation has no pose for yet.
    pub unposed: usize,
    /// Other players the roster says are in the round.
    pub roster_others: usize,
    /// In the round, but neither in the view nor merely late: the server did not send them (interest management, or a lost packet).
    pub hidden_by_interest: usize,
    /// Avatars prepared, and worn right now.
    pub pool_capacity: usize,
    /// Avatars worn right now.
    pub pool_in_use: usize,
}

/// Counters over the whole session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionCounters {
    /// Frames the scene was updated.
    pub frames: u64,
    /// Frames on which some remote player had no avatar.
    pub undrawn_frames: u64,
    /// Frames on which some remote player wore a stand-in.
    pub standin_frames: u64,
    /// Frames on which the server left a player of the round out.
    pub hidden_frames: u64,
    /// Prop poses received for a prop the map does not have.
    pub unknown_props: u64,
}

/// A connection to a server plus everything the client keeps for it.
pub struct NetSession {
    /// The connection.
    pub client: NetClient,
    /// Prediction of the local player (`None` until welcomed).
    pub predictor: Option<Predictor>,
    /// The static map as the client sees it.
    pub world: ClientWorld,
    avatars: Vec<AvatarPool>,
    /// The kart models, hazards and item boxes, in a race match.
    fleet: Option<crate::net::fleet::KartFleet>,
    started: Instant,
    /// Set when a Welcome arrives (first join or resume): the state to teleport the camera to.
    pub teleport: Option<PlayerState>,
    /// Latest one-line description for the window title.
    pub status: String,
    /// Speed of the local player at the last predicted tick (m/s), for the local walk animation.
    pub last_speed: f32,
    /// The server's latest word about the local player (weapon in hand, hit points, what they carry).
    pub own: Option<PlayerSnap>,
    /// The race as of the newest snapshot, in a race match (the HUD reads the countdown and clock; the prediction reads whether the light is green).
    pub race: Option<crate::net::protocol::RaceSnap>,
    /// Whether a `Welcome` has arrived (also true in a lobby, where there is no body to place yet).
    pub joined: bool,
    /// Things the snapshots reported since the last [`take_happened`](Self::take_happened): shots heard, hits landed, damage taken, kills.
    happened: Vec<crate::net::happenings::Happenings>,
    /// The weapons other players hold this frame (set by [`update_scene`](Self::update_scene)): where each is and how it is held.
    remote_hands: Vec<RemoteHand>,
    bodies: Vec<RemoteBody>,
    drawn: Vec<DrawnPlayer>,
    stats: RemoteStats,
    counters: SessionCounters,
    warnings: Vec<String>,
    /// Players already warned about as undrawn (one warning each, until they are drawn again).
    warned: Vec<u8>,
}

impl NetSession {
    /// Starts joining `server`. Call [`add_avatar_pool`](Self::add_avatar_pool) on the scene, then
    /// [`wait_connected`](Self::wait_connected).
    pub fn connect(server: SocketAddr, character: Character, world: ClientWorld, resume_token: u64) -> std::io::Result<NetSession> {
        let code = crate::net::protocol::character_to_wire(character);
        Self::connect_with(ClientConfig::new(server, code, world.map_hash, resume_token), world)
    }

    /// Like [`NetSession::connect`] with a join key and a name (`cfg.map_hash` should be `world.map_hash`).
    pub fn connect_with(cfg: ClientConfig, world: ClientWorld) -> std::io::Result<NetSession> {
        let mut client = NetClient::connect_with(cfg)?;
        client.set_movement_profile(world.player_tuning, &world.jump_pads);
        Ok(NetSession {
            client,
            predictor: None,
            world,
            avatars: Vec::new(),
            fleet: None,
            started: Instant::now(),
            teleport: None,
            status: "connecting...".into(),
            last_speed: 0.0,
            own: None,
            race: None,
            joined: false,
            happened: Vec::new(),
            remote_hands: Vec::new(),
            bodies: Vec::new(),
            drawn: Vec::new(),
            stats: RemoteStats::default(),
            counters: SessionCounters::default(),
            warnings: Vec::new(),
            warned: Vec::new(),
        })
    }

    /// The happenings reported since the last call, oldest first (a client turns them into sounds, hit markers and damage flashes).
    pub fn take_happened(&mut self) -> Vec<crate::net::happenings::Happenings> {
        std::mem::take(&mut self.happened)
    }

    /// The weapons other players hold (for the renderer to draw), as of the last [`update_scene`](Self::update_scene).
    pub fn remote_hands(&self) -> &[RemoteHand] {
        &self.remote_hands
    }

    /// The other players as drawn this frame.
    pub fn bodies(&self) -> &[RemoteBody] {
        &self.bodies
    }

    /// The other players as drawn this frame, with the avatar object that draws each (reports, tests).
    pub fn drawn_players(&self) -> &[DrawnPlayer] {
        &self.drawn
    }

    /// What happened to the other players this frame: how many were drawn, stood in for, undrawn, hidden by interest management.
    pub fn stats(&self) -> &RemoteStats {
        &self.stats
    }

    /// Counters over the whole session.
    pub fn counters(&self) -> SessionCounters {
        self.counters
    }

    /// The sentences that say a failure in words, once each (the client prints them): a player nobody can see, and why.
    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    /// Per body: the avatars prepared and how they were used (`failed_claims` is the number of times a player found none).
    pub fn avatar_pool_stats(&self) -> Vec<(Character, PoolStats)> {
        self.avatars.iter().map(|a| (a.body, a.pool.stats())).collect()
    }

    /// The ids of the avatar objects nobody wears: the renderer must not draw them (`LiveRenderer::set_hidden_objects`).
    pub fn hidden_avatar_ids<'a>(&'a self, scene: &'a Scene) -> impl Iterator<Item = &'a str> + 'a {
        self.avatars.iter().flat_map(move |a| a.pool.hidden_ids(scene)).chain(self.fleet.iter().flat_map(move |f| f.hidden_ids(scene)))
    }

    /// The nearest living remote player a ray from `eye` along `dir` meets within `reach`, as they are drawn, and how far: what the crosshair turns
    /// red for. The shot itself is the server's call (it judges it against the same picture, ADR 0053).
    pub fn player_in_sight(&self, eye: Vec3, dir: Vec3, reach: f32) -> Option<(u8, f32)> {
        nearest_body_on_ray(&self.bodies, eye, dir, reach)
    }

    /// Adds hidden avatars for every body somebody in this game can wear ([`avatar_plan`]): the bodies its humans may pick (all of them, or the one
    /// `player.humans_play_as` forces) and, when the scene has bots, the fighting bodies they wear whatever the humans are (a roster gives each bot a body of
    /// its own: cowboy, wizard, alien, robot). A body nobody can wear costs nothing.
    pub fn add_avatar_pool(&mut self, scene: &mut Scene) {
        // A race match draws karts, not people: the fleet replaces the character avatars.
        self.fleet = crate::net::fleet::KartFleet::build(scene);
        self.avatars = if self.fleet.is_some() { Vec::new() } else { build_avatar_pools(scene) };
    }

    /// The local kart as the chase camera needs it (where the prediction has it, drawn position included), in a race once the first snapshot has arrived.
    pub fn kart_view(&self) -> Option<crate::kart_camera::KartView> {
        let p = self.predictor.as_ref()?;
        let kart = p.kart()?;
        let driver = crate::sim::kart::Driver::from_wire(self.own.as_ref()?.kart.as_ref()?.driver)?;
        Some(crate::kart_camera::KartView {
            pos: p.visual_pos(),
            foot_y: p.state.foot_y,
            heading: p.state.yaw,
            velocity: p.state.velocity,
            top_speed: driver.spec().top_speed,
            drifting: kart.drift_dir != 0,
            boosting: kart.boost_ticks > 0,
        })
    }

    /// What the race HUD shows, from the newest snapshot and the local prediction (so a used item or a pulled boost shows at once, not a round trip later).
    pub fn race_hud(&self) -> Option<crate::ui::race::RaceHud> {
        use crate::ui::race::{Finish, RaceHud};
        let (fleet, race) = (self.fleet.as_ref()?, self.race.as_ref()?);
        let snap = self.own.as_ref()?.kart.as_ref()?;
        let driver = crate::sim::kart::Driver::from_wire(snap.driver)?;
        let mine = self.predictor.as_ref().and_then(|p| p.kart().copied()).unwrap_or_else(|| snap.to_state());
        let tier = if mine.drift_dir == 0 {
            0
        } else if mine.drift_charge >= 3.2 {
            3
        } else if mine.drift_charge >= 2.0 {
            2
        } else {
            u8::from(mine.drift_charge >= 1.0)
        };
        let standings = if race.phase == 2 {
            fleet
                .standings()
                .into_iter()
                .filter_map(|(d, place, finished)| {
                    let name = crate::sim::kart::Driver::from_wire(d)?.name().to_string();
                    Some((place, name, if finished { Finish::Done } else { Finish::DidNotFinish }))
                })
                .collect()
        } else {
            Vec::new()
        };
        Some(RaceHud {
            phase: race.phase,
            countdown_secs: race.countdown_ticks as f32 / crate::sim::clock::TICK_RATE_HZ as f32,
            race_secs: race.race_tick as f32 / crate::sim::clock::TICK_RATE_HZ as f32,
            lap: if snap.finished { fleet.laps() } else { snap.lap.saturating_add(1).min(fleet.laps()) },
            laps: fleet.laps(),
            place: snap.place,
            racers: fleet.standings().len().max(1) as u8,
            speed: self.last_speed,
            item: mine.item,
            shielded: mine.shield_ticks > 0,
            boosting: mine.boost_ticks > 0,
            drift_tier: tier,
            ability_cooldown: (driver.spec().ability == crate::sim::kart::Ability::Build)
                .then(|| mine.ability_cooldown as f32 / crate::sim::clock::TICK_RATE_HZ as f32),
            finished: snap.finished,
            standings,
        })
    }

    /// Whether this match is a kart race (the scene has a `race` block): the client then drives with the chase camera and the race HUD.
    pub fn is_race(&self) -> bool {
        self.fleet.is_some()
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
        PlayerState {
            pos: Vec2::new(s.pos[0], s.pos[2]),
            foot_y: s.pos[1],
            vy: s.vy,
            velocity: Vec2::from_array(s.velocity),
            yaw: s.yaw,
            pitch: s.pitch,
            character: character_from_wire(s.character),
        }
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
                        velocity: Vec2::ZERO,
                        yaw: w.spawn[3],
                        pitch: 0.0,
                        character: character_from_wire(w.character),
                    };
                    match &mut self.predictor {
                        Some(p) => p.teleport(st),
                        None => {
                            let mut predictor = Predictor::new(st);
                            predictor.set_course(self.world.race.clone());
                            self.predictor = Some(predictor);
                        }
                    }
                    self.teleport = Some(st);
                }
                NetEvent::Snapshot { own: Some(own), ack_input_seq, race } => {
                    self.race = race;
                    if let Some(p) = &mut self.predictor {
                        p.reconcile_snapshot(
                            Self::own_state(&own),
                            own.kart.as_ref(),
                            ack_input_seq,
                            &self.world.colliders,
                            &self.world.ground,
                            self.world.player_tuning,
                            &self.world.jump_pads,
                        );
                    }
                    self.own = Some(own);
                }
                NetEvent::Happened(h) if self.happened.len() < 64 => self.happened.push(h),
                _ => {}
            }
        }
        let collision_disabled = self.client.rule_state().map(|state| state.collision_disabled.clone()).unwrap_or_default();
        self.world.set_collision_disabled(&collision_disabled);
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
        // In a race the light decides whether the input counts (the server ignores it until green, so the prediction must too).
        let can_drive = self.race.as_ref().is_none_or(|r| r.phase == 1);
        self.last_speed = p.apply_local_auto(input, can_drive, &self.world.colliders, &self.world.ground, self.world.player_tuning, &self.world.jump_pads);
        self.client.send_input(input, now);
        Some(p.state)
    }

    /// The correction still fading out, to add to the drawn position of the local player.
    pub fn visual_offset(&self) -> Vec2 {
        self.predictor.as_ref().map_or(Vec2::ZERO, |p| p.visual_pos() - p.state.pos)
    }

    /// Object ids hidden by the newest authoritative rule-state snapshot. Wire indices refer to
    /// the shared parsed scene; invalid indices are ignored defensively.
    pub fn hidden_objects(&self) -> Vec<&str> {
        self.client
            .rule_state()
            .into_iter()
            .flat_map(|state| state.hidden.iter())
            .filter_map(|&i| self.world.rule_object_ids.get(i as usize).map(String::as_str))
            .collect()
    }

    /// The pooled avatar player `id` wears this frame, as `(body pool, slot)`. A player keeps theirs while it is of their body; a newcomer (or one who
    /// changed body) takes a free avatar of their own body, and when the pool holds none, a stand-in: any free avatar of the same rig (the rat's, or the
    /// people's). That is the last resort for a body the scene did not announce (a server filling the match with bots the map never mentions): another
    /// costume is better than an enemy nobody can see. `None` when even that is not to be had (counted as a failed claim of their body's pool). Leaving an
    /// avatar hides it.
    fn claim_avatar(&mut self, id: u8, body: Character, scene: &mut Scene) -> Option<(usize, usize)> {
        let is_rat = |c: Character| c == Character::Rat;
        let worn = self.avatars.iter().enumerate().find_map(|(p, a)| a.used_by.iter().position(|u| *u == Some(id)).map(|s| (p, s)));
        if let Some((p, _)) = worn {
            if self.avatars[p].body == body {
                return worn;
            }
        }
        let free_of = |pools: &[AvatarPool], fits: &dyn Fn(Character) -> bool| pools.iter().position(|a| fits(a.body) && a.used_by.iter().any(|u| u.is_none()));
        if let Some(p) = free_of(&self.avatars, &|c| c == body) {
            return self.wear(id, p, worn, scene);
        }
        if let Some((p, _)) = worn {
            if is_rat(self.avatars[p].body) == is_rat(body) {
                return worn; // keeps the stand-in they already have
            }
        }
        if let Some(p) = free_of(&self.avatars, &|c| is_rat(c) == is_rat(body)) {
            return self.wear(id, p, worn, scene);
        }
        if let Some(w) = worn {
            self.release_avatar(w, scene);
        }
        if let Some(a) = self.avatars.iter_mut().find(|a| a.body == body) {
            let _ = a.pool.claim(); // full: the pool counts the failed claim
        }
        None
    }

    fn release_avatar(&mut self, (p, s): (usize, usize), scene: &mut Scene) {
        let a = &mut self.avatars[p];
        a.used_by[s] = None;
        a.pool.release(scene, s);
    }

    /// Makes `id` wear a free avatar of pool `p`, freeing the one they wore before. `None` if the pool turns out to have no free slot after all: the
    /// caller always checks first, so this is a defensive fallback, not an expected outcome.
    fn wear(&mut self, id: u8, p: usize, worn: Option<(usize, usize)>, scene: &mut Scene) -> Option<(usize, usize)> {
        if let Some(w) = worn {
            self.release_avatar(w, scene);
        }
        let a = &mut self.avatars[p];
        let slot = a.pool.claim()?;
        a.used_by[slot] = Some(id);
        a.anim[slot] = AvatarAnim::default();
        Some((p, slot))
    }

    /// Updates the scene from the interpolated network view: remote players wear pooled avatar
    /// objects (position, facing, walk cycle) and props take the server's poses. Unused avatars are hidden.
    pub fn update_scene(&mut self, scene: &mut Scene, now: Instant, dt: f32) {
        let view = self.client.view(now);
        let me = self.client.my_id();
        let present = self.client.remote_world().present_others(me);
        let roster: Option<Vec<RosterEntry>> = self.client.status().map(|s| s.roster.clone());
        let idle_t = now.duration_since(self.started).as_secs_f32();
        self.apply_view(&view, present, roster.as_deref().map(|r| (r, me)), scene, dt, idle_t);
        // The local kart is not in the interpolated view of the others: draw it where the prediction has it.
        if let (Some(fleet), Some(p), Some(own)) = (self.fleet.as_mut(), self.predictor.as_ref(), self.own.as_ref()) {
            if let Some(kart) = own.kart.as_ref() {
                let at = p.visual_pos();
                fleet.place_kart(scene, kart.driver, Vec3::new(at.x, p.state.foot_y, at.y), p.state.yaw, kart);
            }
        }
    }

    /// The part of [`update_scene`](Self::update_scene) that needs no connection: draws `view` into `scene` and accounts for every player and prop in it.
    /// `present` is how many other players the snapshots mention; `roster` the match's roster and our own id, when known.
    pub fn apply_view(&mut self, view: &View, present: usize, roster: Option<(&[RosterEntry], Option<u8>)>, scene: &mut Scene, dt: f32, idle_t: f32) {
        self.remote_hands.clear();
        self.bodies.clear();
        self.drawn.clear();
        if let Some(fleet) = self.fleet.as_mut() {
            fleet.begin_frame(scene);
            if let Some(race) = &self.race {
                fleet.set_boxes_ready(race.boxes_ready);
                fleet.place_hazards(scene, &race.hazards, idle_t);
            }
        }
        // Free avatars whose player left.
        let gone: Vec<(usize, usize)> = self
            .avatars
            .iter()
            .enumerate()
            .flat_map(|(p, a)| a.used_by.iter().enumerate().filter_map(move |(s, u)| u.map(|id| (p, s, id))))
            .filter(|(_, _, id)| !view.players.iter().any(|(pid, _)| pid == id))
            .map(|(p, s, _)| (p, s))
            .collect();
        for g in gone {
            self.release_avatar(g, scene);
        }
        let mut stats = RemoteStats { in_view: view.players.len(), ..RemoteStats::default() };
        for (id, pose) in &view.players {
            // In a race a player is a kart: the fleet has a fixed model for each driver.
            if let (Some(fleet), Some(kart)) = (self.fleet.as_mut(), pose.kart.as_ref()) {
                if fleet.place_kart(scene, kart.driver, pose.pos, pose.yaw, kart) {
                    stats.drawn += 1;
                } else {
                    stats.undrawn += 1;
                    stats.undrawn_ids.push(*id);
                }
                continue;
            }
            let body = character_from_wire(pose.character);
            let Some((p, s)) = self.claim_avatar(*id, body, scene) else {
                stats.undrawn += 1;
                stats.undrawn_ids.push(*id);
                if !self.warned.contains(id) {
                    self.warned.push(*id);
                    let held: Vec<String> = self.avatars.iter().map(|a| format!("{:?} x{}", a.body, a.pool.capacity())).collect();
                    self.warnings.push(format!(
                        "player {id} wears {body:?} and the avatar pool has nothing free for them (pool: {}): they are INVISIBLE. The scene's `player.humans_play_as`/`bots` block \
                         should announce every body in play (`red_engine2 game check` verifies it)",
                        held.join(", ")
                    ));
                }
                continue;
            };
            self.warned.retain(|w| w != id);
            let avatar_body = self.avatars[p].body;
            if avatar_body != body {
                stats.standins += 1;
            }
            stats.drawn += 1;
            self.bodies.push(RemoteBody { id: *id, pos: pose.pos, dead: pose.dead, character: body });
            let object_index = self.avatars[p].pool.object_index(s);
            self.drawn.push(DrawnPlayer { id: *id, body, avatar_body, avatar: scene.objects[object_index].id.clone(), pos: pose.pos, dead: pose.dead });
            let a = &mut self.avatars[p];
            if let Some(hand) = animate(&mut scene.objects[object_index], avatar_body, pose, &mut a.anim[s], dt, idle_t) {
                self.remote_hands.push(hand);
            }
        }
        for (id, pose) in &view.props {
            match self.world.prop_objects.get(*id as usize) {
                Some(&obj) => set_object_pose(&mut scene.objects[obj], pose.pos, pose.rot),
                None => self.counters.unknown_props += 1,
            }
        }
        stats.unposed = present.saturating_sub(view.players.len());
        if let Some((entries, me)) = roster {
            stats.roster_others = entries.iter().filter(|e| Some(e.id) != me && e.flags & ROSTER_IN_ROUND != 0).count();
            stats.hidden_by_interest = stats.roster_others.saturating_sub(view.players.len() + stats.unposed);
        }
        stats.pool_capacity = self.avatars.iter().map(|a| a.pool.capacity()).sum();
        stats.pool_in_use = self.avatars.iter().map(|a| a.pool.stats().in_use).sum();
        self.counters.frames += 1;
        self.counters.undrawn_frames += u64::from(stats.undrawn > 0);
        self.counters.standin_frames += u64::from(stats.standins > 0);
        self.counters.hidden_frames += u64::from(stats.hidden_by_interest > 0);
        self.stats = stats;
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

/// Every avatar object currently visible in `scene` (for tests): objects named `net_*` that are not hidden.
pub fn visible_avatars(scene: &Scene) -> Vec<&Object> {
    scene.objects.iter().filter(|o| o.id.starts_with("net_") && o.scale.sample(0.0).x > 0.5).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::interp::PlayerPose;
    use crate::net::server::{Server, ServerConfig};
    use crate::sim::match_sim::MatchSim;
    use crate::sim::spawns::parse_spawns;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn the_crosshair_finds_the_nearest_living_body_on_the_ray() {
        let body = |id, x, z, dead| RemoteBody { id, pos: Vec3::new(x, 0.0, z), dead, character: Character::Human };
        let bodies = [body(1, 0.0, -10.0, false), body(2, 0.0, -5.0, false), body(3, 0.0, -3.0, true), body(4, 4.0, -5.0, false)];
        let eye = Vec3::new(0.0, 1.7, 0.0);
        assert_eq!(
            nearest_body_on_ray(&bodies, eye, Vec3::NEG_Z, 80.0).map(|(id, _)| id),
            Some(2),
            "the nearer of two in line, and the dead one does not count"
        );
        let (_, d) = nearest_body_on_ray(&bodies, eye, Vec3::NEG_Z, 80.0).unwrap();
        assert!((4.0..5.5).contains(&d), "the front of a body 5 m away: {d}");
        assert_eq!(nearest_body_on_ray(&bodies, eye, Vec3::NEG_Z, 4.0), None, "out of reach");
        assert_eq!(nearest_body_on_ray(&bodies, eye, Vec3::X, 80.0), None, "looking at nobody");
        // A shot that skims high over a head misses; the body is 1.8 m tall.
        assert_eq!(nearest_body_on_ray(&bodies, Vec3::new(0.0, 2.5, 0.0), Vec3::NEG_Z, 80.0), None);
    }

    /// A networked session's whole path, minus the window: it joins a real server, sees
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
        assert_eq!((session.stats().undrawn, session.counters().undrawn_frames), (0, 0), "nobody was left undrawn: {:?}", session.stats());

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

    /// `examples/test_lab.json` made into the kind of game an arena is: `forced` is the body its humans are given (`player.humans_play_as`) and `bots`
    /// the scene's `bots` block. Returns where the variant was written (the temp directory) and its text.
    fn lab_variant(tag: &str, forced: Option<&str>, bots: Option<serde_json::Value>) -> (std::path::PathBuf, String) {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json");
        let mut root: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(source).unwrap()).unwrap();
        if let Some(who) = forced {
            root["player"]["character"] = who.into();
        }
        if let Some(bots) = bots {
            root["bots"] = bots;
        }
        let text = serde_json::to_string(&root).unwrap();
        // Tests run on parallel threads of one process: a name of the tag and the process alone would let them overwrite each other's file.
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("re_session_{tag}_{}_{}.json", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
        std::fs::write(&path, &text).unwrap();
        (path, text)
    }

    /// A server for `text` on a free loopback port (the "duel" spawns, room for a few players), run on a thread; `fill` overrides the map's bot fill.
    fn serve(text: &str, fill: Option<usize>) -> (SocketAddr, Arc<AtomicBool>, std::thread::JoinHandle<Server>) {
        let scene = crate::schema::parse_scene(text).unwrap();
        let mut spawns = parse_spawns(text).unwrap();
        spawns.retain(|s| s.group == "duel");
        let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), crate::net::map_hash(text));
        cfg.bot_fill = fill;
        let mut server = Server::bind(cfg, MatchSim::new(&scene, spawns)).unwrap();
        server.set_logger(|_| {});
        let addr: SocketAddr = format!("127.0.0.1:{}", server.local_addr().unwrap().port()).parse().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let handle = std::thread::spawn(move || {
            server.run(&stopped);
            server
        });
        (addr, stop, handle)
    }

    /// Joins `addr` as the graphical client does (a Human, the pool added before the first frame) and returns the ids of the avatars drawn once
    /// `expected` of them are up, sorted; it then keeps drawing for a moment, so a player who took a second body would show. The session's stats
    /// over that time come with it.
    fn drawn_avatars(path: &std::path::Path, addr: SocketAddr, expected: usize) -> (Vec<String>, RemoteStats, SessionCounters) {
        let (mut scene, world) = ClientWorld::load(path).unwrap();
        let mut session = NetSession::connect(addr, Character::Human, world, 0).unwrap();
        session.add_avatar_pool(&mut scene);
        session.wait_connected(4.0).expect("joined");
        let mut frame = |scene: &mut Scene| {
            let now = Instant::now();
            session.poll(now);
            session.update_scene(scene, now, 0.01);
            std::thread::sleep(Duration::from_millis(4));
        };
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end && visible_avatars(&scene).len() < expected {
            frame(&mut scene);
        }
        for _ in 0..75 {
            frame(&mut scene);
        }
        let mut ids: Vec<String> = visible_avatars(&scene).iter().map(|o| o.id.clone()).collect();
        ids.sort();
        (ids, session.stats().clone(), session.counters())
    }

    #[test]
    fn bots_in_bodies_the_humans_are_not_forced_to_are_drawn() {
        // The situation of an arena game: every human is a Human, and the roster's bots are a wizard, a cowboy and a robot.
        let bots = serde_json::json!({"fill": 4, "roster": [
            {"name": "Wiz", "character": "wizard"}, {"name": "Cow", "character": "cowboy"}, {"name": "Rob", "character": "robot"}]});
        let (path, text) = lab_variant("roster", Some("human"), Some(bots));
        let (addr, stop, handle) = serve(&text, None);
        let (ids, stats, counters) = drawn_avatars(&path, addr, 3);
        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
        let _ = std::fs::remove_file(&path);
        let of = |body: &str| ids.iter().filter(|id| id.starts_with(&format!("net_{body}_"))).count();
        assert_eq!(ids.len(), 3, "the three bots are drawn, one body each: {ids:?}");
        assert_eq!((of("wizard"), of("cowboy"), of("robot")), (1, 1, 1), "each in its own costume: {ids:?}");
        // The count is the assertion: "3 fighters means 3 drawn, none undrawn, none stood in for".
        assert_eq!((stats.in_view, stats.drawn, stats.undrawn, stats.standins), (3, 3, 0, 0), "{stats:?}");
        assert_eq!((counters.undrawn_frames, counters.standin_frames), (0, 0), "{counters:?}");
    }

    #[test]
    fn a_body_the_pool_lacks_is_stood_in_for_rather_than_left_undrawn() {
        // A server that fills the match with bots the map never mentions (`red_server --fill`) wears them in bodies this client did not prepare.
        let (path, text) = lab_variant("standin", Some("human"), None);
        let (addr, stop, handle) = serve(&text, Some(4));
        let (ids, stats, counters) = drawn_avatars(&path, addr, 3);
        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(ids.len(), 3, "three bots, three bodies: {ids:?}");
        assert!(ids.iter().all(|id| id.starts_with("net_human_")), "in the one body there is: {ids:?}");
        assert_eq!((stats.drawn, stats.undrawn), (3, 0), "{stats:?}");
        assert_eq!(stats.standins, 3, "and the stand-ins are counted, not hidden: {stats:?}");
        assert!(counters.standin_frames > 0);
    }

    #[test]
    fn the_pool_holds_every_body_somebody_can_wear() {
        let pool = |forced: Option<&str>, bots: bool| {
            let bots = bots.then(|| serde_json::json!({"fill": 4}));
            let (path, _) = lab_variant("pool", forced, bots);
            let (mut scene, world) = ClientWorld::load(&path).unwrap();
            let mut session = NetSession::connect("127.0.0.1:9".parse().unwrap(), Character::Human, world, 0).unwrap();
            session.add_avatar_pool(&mut scene);
            let _ = std::fs::remove_file(&path);
            let of = |who: Character| scene.objects.iter().filter(|o| o.id.starts_with(&format!("net_{who:?}_").to_lowercase())).count();
            Character::ALL.map(of)
        };
        let n = MAX_PLAYERS_PER_SNAPSHOT;
        // [Human, Rat, Wizard, Cowboy, Alien, Robot]
        assert_eq!(pool(None, false), [n, n, n, n, n, n, 0, 0], "nobody is forced: any body can turn up");
        assert_eq!(pool(Some("human"), false), [n, 0, 0, 0, 0, 0, 0, 0], "everybody is a Human and there are no bots: one body is enough");
        assert_eq!(
            pool(Some("human"), true),
            [n, 0, BOT_BODY_POOL, BOT_BODY_POOL, BOT_BODY_POOL, BOT_BODY_POOL, 0, 0],
            "bots wear the fighting bodies, never the rat"
        );
        assert_eq!(
            pool(Some("rat"), true),
            [BOT_BODY_POOL, n, BOT_BODY_POOL, BOT_BODY_POOL, BOT_BODY_POOL, BOT_BODY_POOL, 0, 0],
            "a rat game's bots still fight as people"
        );
    }

    fn scene_with(forced: Option<&str>, bots: Option<serde_json::Value>) -> Scene {
        let (path, text) = lab_variant("plan", forced, bots);
        let _ = std::fs::remove_file(path);
        crate::schema::parse_scene(&text).unwrap()
    }

    #[test]
    fn the_plan_counts_the_bodies_a_roster_asks_for_and_never_under_provisions_them() {
        // Seven cowboys in a roster: more than the floor of four, so the pool grows to fit them.
        let roster: Vec<serde_json::Value> = (0..7).map(|k| serde_json::json!({"name": format!("C{k}"), "character": "cowboy"})).collect();
        let plan = avatar_plan(&scene_with(Some("human"), Some(serde_json::json!({"fill": 8, "roster": roster}))));
        let idx = |c: Character| Character::ALL.iter().position(|x| *x == c).unwrap();
        assert_eq!(plan.pool[idx(Character::Cowboy)], 7, "{plan:?}");
        assert_eq!(plan.needed[idx(Character::Cowboy)], 7);
        assert_eq!(plan.pool[idx(Character::Wizard)], BOT_BODY_POOL, "an unnamed body keeps the floor for a server that fills with generated bots");
        assert!(plan.short().is_empty(), "the pool always covers what the scene needs: {:?}", plan.short());
        // No bots, nothing forced: humans can be anything.
        let open = avatar_plan(&scene_with(None, None));
        assert_eq!(open.pool, [MAX_PLAYERS_PER_SNAPSHOT, MAX_PLAYERS_PER_SNAPSHOT, MAX_PLAYERS_PER_SNAPSHOT, MAX_PLAYERS_PER_SNAPSHOT, MAX_PLAYERS_PER_SNAPSHOT, MAX_PLAYERS_PER_SNAPSHOT, 0, 0]);
        assert_eq!(open.needed, [MAX_PLAYERS_PER_SNAPSHOT - 1, MAX_PLAYERS_PER_SNAPSHOT - 1, MAX_PLAYERS_PER_SNAPSHOT - 1, MAX_PLAYERS_PER_SNAPSHOT - 1, MAX_PLAYERS_PER_SNAPSHOT - 1, MAX_PLAYERS_PER_SNAPSHOT - 1, 0, 0]);
    }

    /// A session with no server, for driving `apply_view` with views made by hand.
    fn offline_session(forced: Option<&str>, bots: Option<serde_json::Value>) -> (NetSession, Scene) {
        let (path, _) = lab_variant("offline", forced, bots);
        let (mut scene, world) = ClientWorld::load(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let mut session = NetSession::connect("127.0.0.1:9".parse().unwrap(), Character::Human, world, 0).unwrap();
        session.add_avatar_pool(&mut scene);
        (session, scene)
    }

    fn pose(character: u8, x: f32) -> PlayerPose {
        PlayerPose {
            pos: Vec3::new(x, 0.0, 0.0),
            yaw: 0.0,
            pitch: 0.0,
            speed: 0.0,
            character,
            crouching: false,
            swinging: false,
            dead: false,
            weapon: 0,
            held: crate::net::protocol::NO_PROP,
            hp: 100,
            shots: 0,
            protected: false,
            kart: None,
            extra: 0,
        }
    }

    fn race_session() -> (NetSession, Scene) {
        let text = r#"{"camera":{"position":[0,30,60],"target":[0,0,0]},
            "zones":[{"id":"line","rect":[-6,-41,6,-39]},{"id":"east","rect":[39,-6,41,6]},{"id":"south","rect":[-6,39,6,41]},{"id":"west","rect":[-41,-6,-39,6]}],
            "race":{"gates":["line","east","south","west"]},"spawns":[{"id":"a","position":[-20,0,-41],"yaw_deg":90}],
            "objects":[{"id":"floor","type":"plane","size":[100,100]}]}"#;
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path =
            std::env::temp_dir().join(format!("re2_race_session_{}_{}.json", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        std::fs::write(&path, text).unwrap();
        let (mut scene, world) = ClientWorld::load(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let mut session = NetSession::connect("127.0.0.1:9".parse().unwrap(), Character::Human, world, 0).unwrap();
        session.add_avatar_pool(&mut scene);
        (session, scene)
    }

    fn kart_pose(driver: u8, x: f32, yaw: f32) -> PlayerPose {
        PlayerPose { yaw, kart: Some(crate::net::protocol::KartSnap { driver, ..Default::default() }), ..pose(0, x) }
    }

    #[test]
    fn a_race_view_draws_each_player_as_a_kart_of_their_driver_and_no_people() {
        use crate::sim::kart::Driver;
        let (mut session, mut scene) = race_session();
        assert!(session.is_race());
        assert!(session.avatar_pool_stats().is_empty(), "a race needs no character avatars");
        let players = vec![
            (1, kart_pose(Driver::Bunny.wire(), 3.0, 0.0)),
            (2, kart_pose(Driver::Bear.wire(), 6.0, std::f32::consts::FRAC_PI_2)),
            (3, kart_pose(Driver::Beaver.wire(), 9.0, 0.0)),
        ];
        session.apply_view(&View { players, props: vec![] }, 3, None, &mut scene, 0.016, 0.0);
        let s = session.stats();
        assert_eq!((s.drawn, s.undrawn), (3, 0), "{s:?}");
        let find = |id: &str| scene.objects.iter().find(|o| o.id == id).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(find("kart_bunny").position.sample(0.0).x, 3.0);
        assert_eq!(find("kart_bear").position.sample(0.0).x, 6.0);
        assert!((find("kart_bear").rotation.sample(0.0).y + 90.0).abs() < 1e-3, "the Bear heads east");
        let hidden: Vec<&str> = session.hidden_avatar_ids(&scene).collect();
        assert!(hidden.contains(&"kart_duck") && hidden.contains(&"kart_wolf"), "drivers who are not racing are not drawn");
        assert!(!hidden.contains(&"kart_bunny") && !hidden.contains(&"kart_beaver"));
        // A driver who leaves is hidden again next frame.
        session.apply_view(&View { players: vec![(1, kart_pose(Driver::Bunny.wire(), 4.0, 0.0))], props: vec![] }, 1, None, &mut scene, 0.016, 0.0);
        let hidden: Vec<&str> = session.hidden_avatar_ids(&scene).collect();
        assert!(hidden.contains(&"kart_bear") && !hidden.contains(&"kart_bunny"));
    }

    #[test]
    fn a_race_snapshot_puts_hazards_on_the_track_and_takes_item_boxes_off_it() {
        use crate::net::protocol::{HazardSnap, RaceSnap};
        let (mut session, mut scene) = race_session();
        session.race = Some(RaceSnap { phase: 1, hazards: vec![HazardSnap { kind: 1, owner: 0, pos: [5.0, 6.0] }], ..Default::default() });
        session.apply_view(&View::default(), 0, None, &mut scene, 0.016, 0.0);
        let plank = scene.objects.iter().find(|o| o.id == "plank_0").unwrap();
        assert_eq!((plank.position.sample(0.0).x, plank.position.sample(0.0).z), (5.0, 6.0));
        assert!(!session.hidden_avatar_ids(&scene).any(|id| id == "plank_0"));
    }

    #[test]
    fn the_race_hud_and_the_chase_view_come_from_the_prediction_and_the_newest_snapshot() {
        use crate::net::predict::Predictor;
        use crate::net::protocol::{KartSnap, PlayerSnap, RaceSnap};
        use crate::sim::kart::{Driver, Item};
        let (mut session, mut scene) = race_session();
        assert!(session.race_hud().is_none() && session.kart_view().is_none(), "nothing to show before the first snapshot");
        session.race = Some(RaceSnap { phase: 1, race_tick: 630, ..Default::default() });
        let snap = KartSnap {
            driver: Driver::Beaver.wire(),
            lap: 1,
            place: 3,
            item: Item::Mushroom.wire(),
            boost_ticks: 10,
            drift_dir: 1,
            drift_charge_ms: 2300,
            ability_cooldown: 120,
            ..Default::default()
        };
        session.own = Some(PlayerSnap {
            id: 0,
            character: 0,
            flags: 0,
            pos: [1.0, 0.0, 2.0],
            yaw: 1.5,
            pitch: 0.0,
            speed: 20.0,
            vy: 0.0,
            velocity: [20.0, 0.0],
            weapon: 0,
            held: crate::net::protocol::NO_PROP,
            hp: 100,
            shots: 0,
            extra: 0,
            kart: Some(snap),
        });
        let mut predictor = Predictor::new(PlayerState::spawn(1.0, 2.0, 0.0, 90.0, Character::Human));
        predictor.enable_kart(snap.to_state(), Driver::Beaver.spec());
        session.predictor = Some(predictor);
        session.last_speed = 21.0;
        // One frame draws the others, which is where the standings come from.
        let rival = PlayerPose {
            kart: Some(KartSnap { driver: Driver::Bear.wire(), place: 1, finished: true, ..Default::default() }),
            ..kart_pose(Driver::Bear.wire(), 3.0, 0.0)
        };
        session.apply_view(&View { players: vec![(1, rival)], props: vec![] }, 1, None, &mut scene, 0.016, 0.0);
        // ... and update_scene then places the local kart, from the prediction.
        session.fleet.as_mut().unwrap().place_kart(&mut scene, snap.driver, Vec3::new(1.0, 0.0, 2.0), 1.5, &snap);
        let hud = session.race_hud().expect("a race HUD");
        assert_eq!((hud.phase, hud.lap, hud.laps, hud.place), (1, 2, 3, 3));
        assert_eq!((hud.item, hud.boosting, hud.shielded, hud.drift_tier), (Item::Mushroom, true, false, 2));
        assert_eq!(hud.ability_cooldown, Some(2.0), "the Beaver's Build cooldown, in seconds");
        assert!((hud.race_secs - 10.5).abs() < 1e-3 && (hud.speed - 21.0).abs() < 1e-6);
        assert_eq!(hud.racers, 2, "the rival and us");
        assert!(hud.standings.is_empty(), "the standings table is for the end of the race");
        let view = session.kart_view().expect("a chase view");
        assert!((view.heading - 90f32.to_radians()).abs() < 1e-5 && view.drifting && view.boosting);
        assert_eq!(view.top_speed, Driver::Beaver.spec().top_speed);
        // The race ends: the standings list everyone drawn, best place first, saying who finished.
        session.race = Some(RaceSnap { phase: 2, ..Default::default() });
        let hud = session.race_hud().unwrap();
        assert_eq!(hud.standings.len(), 2);
        assert_eq!((hud.standings[0].0, hud.standings[0].1.as_str(), hud.standings[0].2), (1, "Bear", crate::ui::race::Finish::Done), "best place first");
        assert_eq!((hud.standings[1].0, hud.standings[1].1.as_str(), hud.standings[1].2), (3, "Beaver", crate::ui::race::Finish::DidNotFinish));
    }

    fn roster_entry(id: u8, in_round: bool) -> RosterEntry {
        RosterEntry { id, team: 0, flags: if in_round { ROSTER_IN_ROUND } else { 0 }, character: 0, ping_ms: 0, score: 0, name: format!("p{id}") }
    }

    #[test]
    fn a_player_the_pool_cannot_dress_is_counted_and_warned_about_once_not_skipped_in_silence() {
        // Only Humans are prepared (no bots, Human forced). Somebody arrives as a rat: no rat avatar, and the rat rig cannot be borrowed from a person's.
        let (mut session, mut scene) = offline_session(Some("human"), None);
        let view = View { players: vec![(1, pose(0, 1.0)), (2, pose(1, 2.0))], props: vec![] };
        session.apply_view(&view, 2, None, &mut scene, 0.016, 0.0);
        let stats = session.stats().clone();
        assert_eq!((stats.in_view, stats.drawn, stats.undrawn, stats.undrawn_ids.as_slice()), (2, 1, 1, &[2u8][..]), "{stats:?}");
        assert_eq!(visible_avatars(&scene).len(), 1, "only the player who could be dressed is on screen: {}", visible_avatars(&scene).len());
        let warnings = session.take_warnings();
        assert_eq!(warnings.len(), 1, "one warning, in words: {warnings:?}");
        assert!(warnings[0].contains("player 2") && warnings[0].contains("INVISIBLE") && warnings[0].contains("Human x12"), "{}", warnings[0]);
        session.apply_view(&view, 2, None, &mut scene, 0.016, 0.0);
        assert!(session.take_warnings().is_empty(), "not repeated every frame");
        let c = session.counters();
        assert_eq!((c.frames, c.undrawn_frames), (2, 2));
        let pools = session.avatar_pool_stats();
        assert_eq!(pools.len(), 1, "only the Human pool exists");
        // The rat is not a Human pool member, so no pool of its body counted the miss; the counters above are the record.
        assert_eq!(session.drawn_players().len(), 1);
        assert_eq!(session.drawn_players()[0].avatar, "net_human_0");
    }

    #[test]
    fn a_stand_in_is_counted_and_a_full_pool_of_the_right_body_counts_its_failed_claim() {
        // One wizard prepared for the roster's single wizard (floor 4 for fighting bodies); five wizards arrive: four in wizard bodies, the fifth a stand-in.
        let (mut session, mut scene) = offline_session(Some("human"), Some(serde_json::json!({"fill": 2, "roster": [{"name": "Wiz", "character": "wizard"}]})));
        let wizard = 2; // wire code of Character::Wizard
        let players: Vec<(u8, PlayerPose)> = (1..=5).map(|id| (id, pose(wizard, id as f32))).collect();
        session.apply_view(&View { players, props: vec![] }, 5, None, &mut scene, 0.016, 0.0);
        let s = session.stats();
        assert_eq!((s.drawn, s.standins, s.undrawn), (5, 1, 0), "{s:?}");
        let drawn = session.drawn_players();
        assert_eq!(drawn.iter().filter(|d| d.avatar_body == Character::Wizard).count(), 4);
        assert_eq!(drawn.iter().filter(|d| d.avatar_body == Character::Human).count(), 1, "the fifth wears a person's costume: {drawn:?}");
        // Somebody leaves: their avatar is free again and hidden.
        session.apply_view(&View { players: (1..=3).map(|id| (id, pose(wizard, id as f32))).collect(), props: vec![] }, 3, None, &mut scene, 0.016, 0.0);
        assert_eq!(session.stats().pool_in_use, 3);
        let hidden = session.hidden_avatar_ids(&scene).count();
        assert_eq!(hidden, session.stats().pool_capacity - 3, "every unworn avatar is listed for the renderer to skip");
    }

    #[test]
    fn interest_management_shows_up_as_a_count_of_players_in_the_round_that_were_not_sent() {
        let (mut session, mut scene) = offline_session(None, None);
        let roster: Vec<RosterEntry> = vec![roster_entry(0, true), roster_entry(1, true), roster_entry(2, true), roster_entry(3, true), roster_entry(4, false)];
        // We are player 0. The roster says 3 others are in the round; the view holds one of them: two are hidden. Player 4 is in the lobby and counts for nothing.
        let view = View { players: vec![(1, pose(0, 1.0))], props: vec![] };
        session.apply_view(&view, 1, Some((&roster, Some(0))), &mut scene, 0.016, 0.0);
        let s = session.stats();
        assert_eq!((s.roster_others, s.in_view, s.hidden_by_interest, s.unposed), (3, 1, 2, 0), "{s:?}");
        assert_eq!(session.counters().hidden_frames, 1);
        // A player the snapshots mention but the interpolation cannot pose yet is `unposed`, not hidden.
        session.apply_view(&view, 3, Some((&roster, Some(0))), &mut scene, 0.016, 0.0);
        let s = session.stats();
        assert_eq!((s.unposed, s.hidden_by_interest), (2, 0), "{s:?}");
    }

    #[test]
    fn a_prop_pose_for_a_prop_the_map_lacks_is_counted() {
        let (mut session, mut scene) = offline_session(None, None);
        let bogus = crate::net::interp::PropPose { pos: Vec3::ZERO, rot: glam::Quat::IDENTITY };
        let known = session.world.prop_objects.len() as u16;
        session.apply_view(&View { players: vec![], props: vec![(known + 5, bogus)] }, 0, None, &mut scene, 0.016, 0.0);
        assert_eq!(session.counters().unknown_props, 1);
    }
}
