//! A visit: where the player is (a room or a world inside a painting), what they have found, what they are looking at, the doors, and the flight into
//! and out of a painting. The client feeds it the player's eye and the interact button every frame and turns its [`Event`]s into sound and words; the
//! renderer draws its [`Museum::models`]. Pure and headless: the tests walk a whole visit with no window.

use super::build::{door_collider, edge_model, pocket_model, room_model, Model, DOOR_PASSABLE};
use super::exhibit::{furnish, pocket, Contents, Homes, Kind, Pocket, PocketPath};
use super::layout::{EdgeId, Layout, RoomId, Side, CELL};
use super::spec::MuseumSpec;
use glam::{Vec2, Vec3};
use red_engine2::collide::Collider2D;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// Where the player is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    Halls,
    Inside(PocketPath),
}

/// What the player is looking at, near enough to read or use.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A piece in a room.
    Exhibit { room: RoomId, index: u8 },
    /// A painting inside the world the player is in.
    Nested { index: u8 },
    /// The frame that leads back out.
    WayBack,
    /// The room's curator's note.
    Note { room: RoomId },
    /// A door that is still locked: the room on the player's side has not given up its key.
    LockedDoor { edge: EdgeId, room: RoomId },
}

/// Something that happened this frame, for sound and words.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The player walked into a room (`first`: never before).
    EnteredRoom { room: RoomId, first: bool, theme: usize, wing_changed: bool },
    /// The flight into a painting began.
    SteppingIn { depth: usize },
    /// The player arrived inside a painting's world.
    Arrived { depth: usize, title: String },
    /// The flight back out began.
    SteppingOut { depth: usize },
    /// The player is back where the painting hangs.
    Returned { depth: usize },
    /// A piece was touched (a sculpture, an installation).
    Touched { room: RoomId, index: u8 },
    /// A sculpture is giving up its key.
    Revealing { room: RoomId, index: u8 },
    /// A key was found: the room's doors will open.
    KeyFound { room: RoomId, keys: usize },
    /// A door began to swing open.
    DoorOpening { edge: EdgeId },
    /// The player tried a locked door.
    Locked { room: RoomId },
}

/// What has been found (saved between launches).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Progress {
    pub seed: u32,
    /// Rooms walked into.
    pub visited: BTreeSet<RoomId>,
    /// Rooms whose key has been found.
    pub solved: BTreeSet<RoomId>,
    /// Paintings stepped into.
    pub entered: u32,
    /// The deepest world reached.
    pub deepest: u32,
    /// The room the player was last in.
    pub last: RoomId,
}

impl Progress {
    pub fn to_json(&self) -> serde_json::Value {
        let rooms = |s: &BTreeSet<RoomId>| s.iter().map(|r| serde_json::json!([r.x, r.z])).collect::<Vec<_>>();
        serde_json::json!({
            "seed": self.seed,
            "visited": rooms(&self.visited),
            "solved": rooms(&self.solved),
            "entered": self.entered,
            "deepest": self.deepest,
            "last": [self.last.x, self.last.z],
        })
    }

    pub fn from_json(v: &serde_json::Value) -> Option<Progress> {
        let rooms = |k: &str| -> BTreeSet<RoomId> {
            v.get(k)
                .and_then(|a| a.as_array())
                .map(|a| a.iter().filter_map(|p| Some(RoomId::new(p.get(0)?.as_i64()? as i32, p.get(1)?.as_i64()? as i32))).collect())
                .unwrap_or_default()
        };
        let last = v.get("last").and_then(|p| Some(RoomId::new(p.get(0)?.as_i64()? as i32, p.get(1)?.as_i64()? as i32))).unwrap_or_default();
        Some(Progress {
            seed: v.get("seed")?.as_u64()? as u32,
            visited: rooms("visited"),
            solved: rooms("solved"),
            entered: v.get("entered").and_then(|n| n.as_u64()).unwrap_or(0) as u32,
            deepest: v.get("deepest").and_then(|n| n.as_u64()).unwrap_or(0) as u32,
            last,
        })
    }
}

/// The camera during a flight (the client draws from here instead of the player's eye).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub eye: Vec3,
    pub look: Vec3,
    pub fov_deg: f32,
}

/// Where the player goes when a flight lands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Teleport {
    /// Feet.
    pub at: Vec3,
    pub yaw: f32,
}

#[derive(Debug, Clone)]
enum Flight {
    /// Into the painting at `canvas` (centre, facing, size), toward `to`.
    In { from: Pose, canvas: (Vec3, Vec3, Vec2), to: PocketPath, t: f32, swapped: bool, palette: [f32; 3] },
    /// Out of the world, back to where the painting hangs.
    Out { t: f32, swapped: bool, back: Return },
}

#[derive(Debug, Clone)]
struct Return {
    place: Place,
    /// Where the player stands when they come back out, and the painting's canvas (to fly back from).
    stand: Teleport,
    canvas: (Vec3, Vec3, Vec2),
}

/// Seconds the flight into a painting takes (into the canvas, then the world opens).
pub const FLIGHT_IN: f32 = 2.6;
/// Seconds the flight out takes.
pub const FLIGHT_OUT: f32 = 2.0;
/// The share of a flight spent before the swap.
const SWAP_AT: f32 = 0.55;
/// The player's normal field of view.
pub const FOV: f32 = 75.0;
/// Where the worlds inside paintings are: at the origin, on the same invisible floor as the museum, which is neither drawn nor solid while the player is inside one.
pub const POCKET_ORIGIN: Vec3 = Vec3::ZERO;
/// Metres from which a piece can be read and used.
pub const REACH: f32 = 4.2;

/// One visit to the museum.
pub struct Museum {
    pub spec: Arc<MuseumSpec>,
    pub layout: Layout,
    homes: Homes,
    contents: HashMap<RoomId, Arc<Contents>>,
    rooms: HashMap<RoomId, Arc<Model>>,
    edges: HashMap<EdgeId, Arc<Model>>,
    pocket_model: Option<(PocketPath, Arc<Model>)>,
    pub pocket: Option<Arc<Pocket>>,
    pub progress: Progress,
    pub place: Place,
    /// The room the player is in (in a painting: the room it hangs in).
    pub room: RoomId,
    returns: Vec<Return>,
    flight: Option<Flight>,
    /// Door openness, 0 shut .. 1 open.
    doors: HashMap<EdgeId, f32>,
    /// A sculpture giving up its key: room, exhibit, seconds since it began.
    revealing: Option<(RoomId, u8, f32)>,
    touched: HashMap<(RoomId, u8), f32>,
    pub time: f32,
    /// The key taken in this world (it no longer floats there).
    key_taken: bool,
    colliders: Vec<Collider2D>,
    colliders_dirty: bool,
    /// Bumped whenever the set of models changes (the renderer re-reads [`Self::models`]).
    pub generation: u64,
    pending: Vec<Event>,
}

/// How long a sculpture takes to give up its key.
pub const REVEAL_SECS: f32 = 3.0;

impl Museum {
    /// A visit from `progress` (a fresh one for `seed` when `None`).
    pub fn new(spec: Arc<MuseumSpec>, seed: u32, progress: Option<Progress>) -> Museum {
        let progress = progress.filter(|p| p.seed == seed).unwrap_or(Progress { seed, ..Default::default() });
        let layout = Layout::new(seed, spec.collection.themes.len(), spec.start_theme);
        let homes = Homes::place(&layout, &spec.collection);
        let room = progress.last;
        let mut m = Museum {
            spec,
            layout,
            homes,
            contents: HashMap::new(),
            rooms: HashMap::new(),
            edges: HashMap::new(),
            pocket_model: None,
            pocket: None,
            progress,
            place: Place::Halls,
            room,
            returns: Vec::new(),
            flight: None,
            doors: HashMap::new(),
            revealing: None,
            touched: HashMap::new(),
            time: 0.0,
            key_taken: false,
            colliders: Vec::new(),
            colliders_dirty: true,
            generation: 0,
            pending: Vec::new(),
        };
        m.progress.visited.insert(room);
        // Doors that are already open start open.
        m.load_around(room, usize::MAX);
        let edges: Vec<EdgeId> = m.edges.keys().copied().collect();
        for e in edges {
            if m.door_unlocked(e) {
                m.doors.insert(e, 1.0);
            }
        }
        m.rebuild_colliders();
        m
    }

    /// Where a new visit (or a continued one) starts: the middle of the last room, facing its first door.
    pub fn start(&self) -> Teleport {
        let r = self.layout.room(self.room);
        let side = Side::ALL.into_iter().find(|s| r.has_door(*s)).unwrap_or(Side::North);
        let at = self.room.centre() - side.out() * (r.wall_length(side.opposite()) * 0.0 + 2.0);
        Teleport { at, yaw: yaw_of(side.out()) }
    }

    /// Stands the visit where `eye` is (a still picture): the room around it and everything through its doors, built now.
    pub fn settle_at(&mut self, eye: Vec3) {
        if self.place != Place::Halls {
            return;
        }
        let r = RoomId::at(eye);
        if r != self.room || self.rooms.is_empty() {
            self.room = r;
        }
        let g = self.generation;
        self.load_around(r, usize::MAX);
        if g != self.generation {
            let edges: Vec<EdgeId> = self.edges.keys().copied().collect();
            for e in edges {
                if self.door_unlocked(e) {
                    self.doors.insert(e, 1.0);
                }
            }
            self.rebuild_colliders();
        }
    }

    /// The furnished room `id` (built once, kept).
    pub fn contents(&mut self, id: RoomId) -> Arc<Contents> {
        if let Some(c) = self.contents.get(&id) {
            return c.clone();
        }
        let room = self.layout.room(id);
        let c = Arc::new(furnish(&self.layout, &self.spec.collection, &self.homes, &room));
        self.contents.insert(id, c.clone());
        c
    }

    /// The furnished room `id` if it has been built.
    pub fn contents_if_built(&self, id: RoomId) -> Option<&Arc<Contents>> {
        self.contents.get(&id)
    }

    /// Whether a room's key has been found.
    pub fn solved(&self, r: RoomId) -> bool {
        self.progress.solved.contains(&r)
    }

    /// A door opens once the key of either room it joins has been found.
    pub fn door_unlocked(&self, e: EdgeId) -> bool {
        let (a, b) = e.rooms();
        self.solved(a) || self.solved(b)
    }

    /// How far a door is open.
    pub fn door_openness(&self, e: EdgeId) -> f32 {
        self.doors.get(&e).copied().unwrap_or(0.0)
    }

    /// Keys found.
    pub fn keys(&self) -> usize {
        self.progress.solved.len()
    }

    /// Rooms discovered.
    pub fn rooms_found(&self) -> usize {
        self.progress.visited.len()
    }

    /// Builds the models of `centre`, the rooms through its doors and theirs (at most `budget` new rooms now; the rest on later frames).
    fn load_around(&mut self, centre: RoomId, budget: usize) {
        let mut want: Vec<RoomId> = vec![centre];
        for (_, n) in self.layout.neighbours(centre) {
            want.push(n);
        }
        let first: Vec<RoomId> = want.clone();
        for r in first.iter().skip(1) {
            for (_, n) in self.layout.neighbours(*r) {
                if !want.contains(&n) {
                    want.push(n);
                }
            }
        }
        let mut built = 0;
        let mut changed = false;
        for r in &want {
            if self.rooms.contains_key(r) {
                continue;
            }
            if built >= budget {
                break;
            }
            let k = self.contents(*r);
            self.rooms.insert(*r, Arc::new(room_model(&self.spec.collection, &k)));
            built += 1;
            changed = true;
        }
        // Doorways of every loaded room.
        let loaded: Vec<RoomId> = self.rooms.keys().copied().collect();
        for r in &loaded {
            for s in Side::ALL {
                let e = EdgeId::of(*r, s);
                if self.layout.has_door(e) && !self.edges.contains_key(&e) {
                    self.edges.insert(e, Arc::new(edge_model(e)));
                    let open = if self.door_unlocked(e) { 1.0 } else { 0.0 };
                    self.doors.entry(e).or_insert(open);
                    changed = true;
                }
            }
        }
        // Forget rooms far away (the furnished contents stay: they are small and the map wants them).
        let before = self.rooms.len();
        self.rooms.retain(|r, _| (r.x - centre.x).abs() + (r.z - centre.z).abs() <= 3);
        let rooms = &self.rooms;
        self.edges.retain(|e, _| {
            let (a, b) = e.rooms();
            rooms.contains_key(&a) || rooms.contains_key(&b)
        });
        if changed || before != self.rooms.len() {
            self.generation += 1;
            self.colliders_dirty = true;
        }
    }

    /// Everything to draw now: the rooms and doorways around the player, or the world they are inside.
    pub fn models(&self) -> Vec<(ModelKey, Arc<Model>)> {
        match (&self.place, &self.pocket_model) {
            (Place::Inside(_), Some((path, m))) => vec![(ModelKey::Pocket(path.clone()), m.clone())],
            _ => {
                self.rooms.iter().map(|(k, m)| (ModelKey::Room(*k), m.clone())).chain(self.edges.iter().map(|(k, m)| (ModelKey::Edge(*k), m.clone()))).collect()
            }
        }
    }

    /// The lamps nearest the eye (at most `n`).
    pub fn lamps(&self, eye: Vec3, n: usize) -> Vec<super::build::Lamp> {
        let mut all: Vec<super::build::Lamp> = self.models().iter().flat_map(|(_, m)| m.lamps.clone()).collect();
        all.sort_by(|a, b| a.at.distance_squared(eye).total_cmp(&b.at.distance_squared(eye)));
        all.truncate(n);
        all
    }

    /// What the player bumps into: walls, plinths, shut doors (only near them).
    pub fn colliders(&self) -> &[Collider2D] {
        &self.colliders
    }

    /// Whether [`Self::colliders`] changed since the last call (and forgets that it did).
    pub fn take_colliders_changed(&mut self) -> bool {
        std::mem::take(&mut self.colliders_dirty)
    }

    fn rebuild_colliders(&mut self) {
        let mut out = Vec::new();
        match &self.place {
            Place::Inside(_) => {
                if let Some((_, m)) = &self.pocket_model {
                    out.extend(m.colliders.iter().copied());
                }
            }
            Place::Halls => {
                for (r, m) in &self.rooms {
                    if (r.x - self.room.x).abs() + (r.z - self.room.z).abs() <= 1 {
                        out.extend(m.colliders.iter().copied());
                    }
                }
                for (e, m) in &self.edges {
                    let (a, b) = e.rooms();
                    if a == self.room || b == self.room {
                        out.extend(m.colliders.iter().copied());
                        if self.door_openness(*e) < DOOR_PASSABLE {
                            out.push(door_collider(*e));
                        }
                    }
                }
            }
        }
        self.colliders = out;
        self.colliders_dirty = true;
    }

    /// The camera while flying into or out of a painting.
    pub fn pose(&self) -> Option<Pose> {
        match &self.flight {
            Some(Flight::In { from, canvas, t, swapped, .. }) => {
                let k = (t / FLIGHT_IN).clamp(0.0, 1.0);
                if !swapped {
                    // Glide up to the canvas until it fills the view.
                    let u = smooth(k / SWAP_AT);
                    let (c, f, size) = *canvas;
                    let fov_end = 2.0 * ((size.y * 0.5) / 0.9).atan().to_degrees();
                    let close = c + f * 0.9;
                    Some(Pose {
                        eye: from.eye.lerp(close, u),
                        look: from.look.lerp(-f, u).normalize_or(-f),
                        fov_deg: from.fov_deg + (fov_end.clamp(25.0, 70.0) - from.fov_deg) * u,
                    })
                } else {
                    None
                }
            }
            Some(Flight::Out { t, swapped: true, back }) => {
                // Back out of the canvas, drawing away from it to where the player stood.
                let k = ((t / FLIGHT_OUT - SWAP_AT) / (1.0 - SWAP_AT)).clamp(0.0, 1.0);
                let u = smooth(k);
                let (c, f, _) = back.canvas;
                let stand_eye = back.stand.at + Vec3::Y * 1.6;
                let look = (c - stand_eye).normalize_or(-f);
                Some(Pose { eye: (c + f * 0.9).lerp(stand_eye, u), look: (-f).lerp(look, u).normalize_or(-f), fov_deg: self.fov() })
            }
            _ => None,
        }
    }

    /// The field of view now (it opens out as a world appears, and narrows as one leaves it).
    pub fn fov(&self) -> f32 {
        match &self.flight {
            Some(Flight::In { canvas, t, swapped: true, .. }) => {
                let k = ((t / FLIGHT_IN - SWAP_AT) / (1.0 - SWAP_AT)).clamp(0.0, 1.0);
                let fov_start = (2.0 * ((canvas.2.y * 0.5) / 0.9).atan().to_degrees()).clamp(25.0, 70.0);
                fov_start + (FOV - fov_start) * smooth(k)
            }
            Some(Flight::Out { t, swapped, .. }) => {
                let k = (t / FLIGHT_OUT).clamp(0.0, 1.0);
                if !swapped {
                    FOV - (FOV - 32.0) * smooth(k / SWAP_AT)
                } else {
                    32.0 + (FOV - 32.0) * smooth((k - SWAP_AT) / (1.0 - SWAP_AT))
                }
            }
            Some(Flight::In { .. }) => self.pose().map_or(FOV, |p| p.fov_deg),
            None => FOV,
        }
    }

    /// 0..1..0 over a flight: how far the picture has taken over the screen (the client washes the view with the painting's colour; the sky ripples).
    pub fn wash(&self) -> (f32, [f32; 3]) {
        match &self.flight {
            Some(Flight::In { t, palette, .. }) => {
                let k = (t / FLIGHT_IN).clamp(0.0, 1.0);
                let w = if k < SWAP_AT { smooth((k - SWAP_AT * 0.7) / (SWAP_AT * 0.3)) } else { 1.0 - smooth((k - SWAP_AT) / 0.25) };
                (w, *palette)
            }
            Some(Flight::Out { t, .. }) => {
                let k = (t / FLIGHT_OUT).clamp(0.0, 1.0);
                let w = if k < SWAP_AT { smooth((k - SWAP_AT * 0.5) / (SWAP_AT * 0.5)) } else { 1.0 - smooth((k - SWAP_AT) / 0.3) };
                (w, [1.0, 0.95, 0.85])
            }
            None => (0.0, [1.0; 3]),
        }
    }

    /// Whether the player may move (not during a flight).
    pub fn free(&self) -> bool {
        self.flight.is_none()
    }

    /// The eight numbers the game's shaders read (`game_param(i)`): `[0]` place (0 halls, 1 inside a painting), style, seed, how far the world has opened
    /// (0..1); `[1..5]` the world's palette (rgb) with the depth in `[1].w`; `[5]` the room's light (rgb) and its theme index; `[6]` the museum's time,
    /// the wash, whether the key floats here, the number of keys found.
    pub fn shader_params(&self) -> [[f32; 4]; 8] {
        let mut p = [[0.0f32; 4]; 8];
        let theme = &self.spec.collection.themes[self.layout.room(self.room).theme];
        p[5] = [theme.light[0], theme.light[1], theme.light[2], self.layout.room(self.room).theme as f32];
        if let (Place::Inside(path), Some(pk)) = (&self.place, &self.pocket) {
            let open = match &self.flight {
                Some(Flight::In { t, .. }) => ((t / FLIGHT_IN - SWAP_AT) / (1.0 - SWAP_AT)).clamp(0.0, 1.0),
                Some(Flight::Out { t, swapped: false, .. }) => 1.0 - (t / FLIGHT_OUT / SWAP_AT).clamp(0.0, 1.0),
                _ => 1.0,
            };
            p[0] = [1.0, pk.art.style as f32, pk.art.seed, open];
            for i in 0..4 {
                p[1 + i] = [pk.art.palette[i][0], pk.art.palette[i][1], pk.art.palette[i][2], 0.0];
            }
            p[1][3] = path.depth() as f32;
        }
        let (wash, _) = self.wash();
        p[6] = [self.time, wash, if self.pocket.as_ref().is_some_and(|p| p.key.is_some()) && !self.key_taken { 1.0 } else { 0.0 }, self.keys() as f32];
        p
    }

    /// Where the floating key is now (world space), when one is: in a world, or rising out of a sculpture.
    pub fn key_at(&self) -> Option<Vec3> {
        if let (Place::Inside(_), Some(p)) = (&self.place, &self.pocket) {
            if self.key_taken {
                return None;
            }
            return p.key.map(|k| POCKET_ORIGIN + k + Vec3::Y * 0.12 * (self.time * 1.3).sin());
        }
        let (room, index, t) = self.revealing?;
        let k = self.contents.get(&room)?;
        let e = k.exhibits.get(index as usize)?;
        let top = match e.kind {
            Kind::Installation => e.at - Vec3::Y * (e.size.y + 0.3),
            _ => e.at + Vec3::Y * (e.size.y + 0.25),
        };
        let rise = smooth(t / REVEAL_SECS);
        Some(top + Vec3::Y * 0.6 * rise + e.facing * 0.4 * rise)
    }

    /// How a sculpture is being stirred right now (seconds since touched, how far it has opened for its key).
    pub fn stir_of(&self, room: RoomId, index: u8) -> (f32, f32) {
        let since = self.touched.get(&(room, index)).map_or(1e3, |t| self.time - t);
        let reveal = match self.revealing {
            Some((r, i, t)) if r == room && i == index => smooth(t / (REVEAL_SECS * 0.6)),
            _ if self.solved(room) && self.contents.get(&room).is_some_and(|k| k.key.depth == 0 && k.key.exhibit == index) => 1.0,
            _ => 0.0,
        };
        (since, reveal)
    }

    /// What the player at `eye` looking along `dir` is aiming at, within reach.
    pub fn aim(&self, eye: Vec3, dir: Vec3) -> Option<Target> {
        let dir = dir.normalize_or(Vec3::NEG_Z);
        let mut best: Option<(f32, Target)> = None;
        let mut consider = |d: f32, t: Target| {
            if best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                best = Some((d, t));
            }
        };
        match (&self.place, &self.pocket) {
            (Place::Inside(_), Some(p)) => {
                for f in &p.frames {
                    if let Some(d) = ray_rect(eye, dir, POCKET_ORIGIN + f.at, f.facing, f.size + Vec2::splat(0.3)) {
                        if d < REACH + f.size.x * 0.5 {
                            consider(d, Target::Nested { index: f.index });
                        }
                    }
                }
                if let Some(d) = ray_rect(eye, dir, POCKET_ORIGIN + p.exit + Vec3::Y * 1.3, -Vec3::Z, Vec2::new(1.9, 2.7)) {
                    if d < REACH + 1.0 {
                        consider(d, Target::WayBack);
                    }
                }
            }
            _ => {
                let here = self.contents.get(&self.room);
                if let Some(k) = here {
                    for e in &k.exhibits {
                        let d = match e.kind {
                            Kind::Painting | Kind::Triptych | Kind::Tapestry => ray_rect(eye, dir, e.at, e.facing, e.size + Vec2::splat(0.2)),
                            Kind::Sculpture => ray_cylinder(eye, dir, e.at - Vec3::Y * e.plinth, e.size.x * 1.15 + 0.1, e.plinth + e.size.y + 0.1),
                            Kind::Installation => ray_cylinder(eye, dir, e.at - Vec3::Y * (e.size.y + 0.3), e.size.x, e.size.y + 0.3),
                        };
                        if let Some(d) = d {
                            let reach = if e.kind.enterable() {
                                REACH + e.size.x * 0.4
                            } else {
                                REACH + e.size.x * 0.5 + if e.kind == Kind::Installation { e.size.y } else { 0.0 }
                            };
                            if d < reach {
                                consider(d, Target::Exhibit { room: self.room, index: e.index });
                            }
                        }
                    }
                    if let Some(d) = ray_rect(eye, dir, k.note.at, k.note.facing, Vec2::new(0.8, 1.0)) {
                        if d < 3.0 {
                            consider(d, Target::Note { room: self.room });
                        }
                    }
                }
                for s in Side::ALL {
                    let e = EdgeId::of(self.room, s);
                    if !self.layout.has_door(e) || self.door_openness(e) > 0.05 {
                        continue;
                    }
                    let c = e.centre() + Vec3::Y * 1.6;
                    let toward = c - eye;
                    if toward.length() < 3.6 && toward.normalize().dot(dir) > 0.8 {
                        consider(toward.length(), Target::LockedDoor { edge: e, room: self.room });
                    }
                }
            }
        }
        best.map(|(_, t)| t)
    }

    /// One frame: `eye` and `foot` of the player, where they look, and whether interact was pressed. Returns what happened and, when a flight lands, where
    /// to put the player.
    pub fn update(&mut self, dt: f32, eye: Vec3, look: Vec3, foot: Vec3, interact: bool) -> (Vec<Event>, Option<Teleport>) {
        self.time += dt;
        let mut events = std::mem::take(&mut self.pending);
        let mut teleport = None;

        // ---- A flight in progress ----
        if let Some(mut f) = self.flight.take() {
            match &mut f {
                Flight::In { t, swapped, to, .. } => {
                    *t += dt;
                    if !*swapped && *t >= FLIGHT_IN * SWAP_AT {
                        *swapped = true;
                        let to = to.clone();
                        teleport = self.enter(&to, &mut events);
                    }
                    if *t >= FLIGHT_IN {
                        self.flight = None;
                    } else {
                        self.flight = Some(f);
                    }
                }
                Flight::Out { t, swapped, back } => {
                    *t += dt;
                    if !*swapped && *t >= FLIGHT_OUT * SWAP_AT {
                        *swapped = true;
                        let back = back.clone();
                        teleport = Some(self.leave(back, &mut events));
                    }
                    if *t >= FLIGHT_OUT {
                        self.flight = None;
                    } else {
                        self.flight = Some(f);
                    }
                }
            }
            self.tick_doors(dt, &mut events);
            return (events, teleport);
        }

        // ---- Walking about ----
        if self.place == Place::Halls {
            let now = RoomId::at(foot);
            if now != self.room {
                let before = self.layout.room(self.room);
                self.room = now;
                let room = self.layout.room(now);
                let first = self.progress.visited.insert(now);
                self.progress.last = now;
                events.push(Event::EnteredRoom {
                    room: now,
                    first,
                    theme: room.theme,
                    wing_changed: room.wing_theme != before.wing_theme || room.theme != before.theme,
                });
                self.load_around(now, 1);
                self.rebuild_colliders();
            } else {
                // Finish loading the neighbourhood a room a frame.
                let g = self.generation;
                self.load_around(now, 1);
                if self.generation != g {
                    self.rebuild_colliders();
                }
            }
        }

        // ---- A sculpture giving up its key ----
        if let Some((room, index, t)) = self.revealing.as_mut() {
            *t += dt;
            if *t >= REVEAL_SECS {
                let room = *room;
                let _ = index;
                self.revealing = None;
                self.take_key(room, &mut events);
            }
        }

        // ---- The key floating in this world: walk into it ----
        if let (Place::Inside(path), Some(p)) = (&self.place, &self.pocket) {
            if let Some(k) = p.key.filter(|_| !self.key_taken) {
                let k = POCKET_ORIGIN + k;
                if Vec2::new(foot.x - k.x, foot.z - k.z).length() < 1.25 {
                    let room = path.room;
                    self.key_taken = true;
                    self.take_key(room, &mut events);
                }
            }
        }

        if interact {
            match self.aim(eye, look) {
                Some(Target::Exhibit { room, index }) => {
                    let k = self.contents(room);
                    let e = k.exhibits[index as usize].clone();
                    if e.kind.enterable() {
                        let canvas = (e.at, e.facing, e.size);
                        let stand = Teleport { at: Vec3::new(e.at.x, 0.0, e.at.z) + e.facing * (1.6 + e.size.x * 0.35), yaw: yaw_of(-e.facing) };
                        self.fly_in(eye, look, canvas, PocketPath { room, exhibit: index, nest: Vec::new() }, stand, e.art.palette[2], &mut events);
                    } else {
                        self.touched.insert((room, index), self.time);
                        events.push(Event::Touched { room, index });
                        if k.key.depth == 0 && k.key.exhibit == index && !self.solved(room) && self.revealing.is_none() {
                            self.revealing = Some((room, index, 0.0));
                            events.push(Event::Revealing { room, index });
                        }
                    }
                }
                Some(Target::Nested { index }) => {
                    if let (Place::Inside(path), Some(p)) = (&self.place, &self.pocket) {
                        if let Some(f) = p.frames.get(index as usize) {
                            let at = POCKET_ORIGIN + f.at;
                            let canvas = (at, f.facing, f.size);
                            let foot_at = Vec3::new(at.x, POCKET_ORIGIN.y, at.z) + f.facing * (1.6 + f.size.x * 0.3);
                            let stand = Teleport { at: foot_at, yaw: yaw_of(-f.facing) };
                            let to = path.child(index);
                            let pal = f.art.palette[2];
                            self.fly_in(eye, look, canvas, to, stand, pal, &mut events);
                        }
                    }
                }
                Some(Target::WayBack) => self.step_out(&mut events),
                Some(Target::LockedDoor { room, .. }) => events.push(Event::Locked { room }),
                Some(Target::Note { .. }) | None => {}
            }
        }

        self.tick_doors(dt, &mut events);
        if self.colliders_dirty {
            self.rebuild_colliders();
        }
        (events, teleport)
    }

    /// Leaves the world the player is in (the way back, or the pause menu's "step out").
    pub fn step_out(&mut self, events: &mut Vec<Event>) {
        if self.flight.is_some() || self.place == Place::Halls {
            return;
        }
        if let Some(back) = self.returns.pop() {
            if let Place::Inside(p) = &self.place {
                events.push(Event::SteppingOut { depth: p.depth() });
            }
            self.flight = Some(Flight::Out { t: 0.0, swapped: false, back });
        }
    }

    /// Back to the museum at once, from any depth (no flight): returns where to stand.
    pub fn return_to_halls(&mut self) -> Option<Teleport> {
        if self.place == Place::Halls {
            return None;
        }
        let first = self.returns.first().cloned()?;
        self.returns.clear();
        self.flight = None;
        let mut events = Vec::new();
        Some(self.leave(first, &mut events))
    }

    #[allow(clippy::too_many_arguments)]
    fn fly_in(&mut self, eye: Vec3, look: Vec3, canvas: (Vec3, Vec3, Vec2), to: PocketPath, stand: Teleport, palette: [f32; 3], events: &mut Vec<Event>) {
        let depth = to.depth();
        // The way back: stand in front of the painting, in the place the player is now.
        self.returns.push(Return { place: self.place.clone(), stand, canvas });
        events.push(Event::SteppingIn { depth });
        self.flight = Some(Flight::In { from: Pose { eye, look, fov_deg: FOV }, canvas, to, t: 0.0, swapped: false, palette });
    }

    fn enter(&mut self, to: &PocketPath, events: &mut Vec<Event>) -> Option<Teleport> {
        let k = self.contents(to.room);
        let p = pocket(&self.layout, &self.spec.collection, &k, to)?;
        let model = Arc::new(pocket_model(&self.spec.collection, &p, POCKET_ORIGIN));
        self.key_taken = self.solved(to.room);
        let spawn = Teleport { at: POCKET_ORIGIN + p.spawn, yaw: yaw_of(p.spawn_facing) };
        events.push(Event::Arrived { depth: to.depth(), title: p.title.clone() });
        self.progress.entered += 1;
        self.progress.deepest = self.progress.deepest.max(to.depth() as u32);
        self.pocket = Some(Arc::new(p));
        self.pocket_model = Some((to.clone(), model));
        self.place = Place::Inside(to.clone());
        self.generation += 1;
        self.rebuild_colliders();
        Some(spawn)
    }

    fn leave(&mut self, back: Return, events: &mut Vec<Event>) -> Teleport {
        let depth = match &self.place {
            Place::Inside(p) => p.depth(),
            Place::Halls => 0,
        };
        match &back.place {
            Place::Halls => {
                self.place = Place::Halls;
                self.pocket = None;
                self.pocket_model = None;
            }
            Place::Inside(path) => {
                let path = path.clone();
                let mut ignored = Vec::new();
                self.enter(&path, &mut ignored);
                // Re-entering a world on the way back is not a new visit.
                self.progress.entered = self.progress.entered.saturating_sub(1);
            }
        }
        self.generation += 1;
        self.rebuild_colliders();
        events.push(Event::Returned { depth });
        back.stand
    }

    fn take_key(&mut self, room: RoomId, events: &mut Vec<Event>) {
        if self.progress.solved.insert(room) {
            events.push(Event::KeyFound { room, keys: self.keys() });
        }
    }

    fn tick_doors(&mut self, dt: f32, events: &mut Vec<Event>) {
        let mut changed = false;
        let edges: Vec<EdgeId> = self.doors.keys().copied().collect();
        for e in edges {
            let want = if self.door_unlocked(e) { 1.0 } else { 0.0 };
            let now = self.doors[&e];
            if now == want {
                continue;
            }
            // Doors only swing while the player is in the halls to see them.
            if self.place != Place::Halls {
                continue;
            }
            if now == 0.0 {
                events.push(Event::DoorOpening { edge: e });
            }
            let next = if want > now { (now + dt * 0.45).min(1.0) } else { (now - dt * 0.8).max(0.0) };
            if (now < DOOR_PASSABLE) != (next < DOOR_PASSABLE) {
                changed = true;
            }
            self.doors.insert(e, next);
        }
        if changed {
            self.rebuild_colliders();
        }
    }

    /// The theme index of a room.
    pub fn theme_of(&self, r: RoomId) -> usize {
        self.layout.room(r).theme
    }

    /// The room a painting world hangs in, or the room the player is in.
    pub fn depth(&self) -> usize {
        match &self.place {
            Place::Halls => 0,
            Place::Inside(p) => p.depth(),
        }
    }

    /// Rooms seen (visited plus those through their doors), for the map.
    pub fn seen(&self) -> BTreeSet<RoomId> {
        let mut out = self.progress.visited.clone();
        for r in &self.progress.visited {
            for (_, n) in self.layout.neighbours(*r) {
                out.insert(n);
            }
        }
        out
    }

    /// The models' keys, for the renderer's bookkeeping.
    pub fn model_keys(&self) -> Vec<ModelKey> {
        self.models().into_iter().map(|(k, _)| k).collect()
    }

    /// The distance between room centres (for the map's scale).
    pub fn cell() -> f32 {
        CELL
    }

    /// For tools and tests: puts the player inside the world at `path` at once (no flight), returning where they stand.
    pub fn enter_now(&mut self, path: &PocketPath) -> Option<Teleport> {
        let k = self.contents(path.room);
        let e = k.exhibits.get(path.exhibit as usize)?.clone();
        let stand = Teleport { at: Vec3::new(e.at.x, 0.0, e.at.z) + e.facing * 2.0, yaw: yaw_of(-e.facing) };
        self.returns.push(Return { place: Place::Halls, stand, canvas: (e.at, e.facing, e.size) });
        self.room = path.room;
        let mut ev = Vec::new();
        self.enter(path, &mut ev)
    }

    /// For tools: draw only `model` (a sheet of every style, say) as if the player stood inside it, lit by its own lamps.
    pub fn stage(&mut self, model: Model) {
        let path = PocketPath { room: self.room, exhibit: 0, nest: vec![99] };
        self.pocket_model = Some((path.clone(), Arc::new(model)));
        self.place = Place::Inside(path);
        self.generation += 1;
    }

    /// A helper for tests and tools: take the key of `room` as if it were found.
    pub fn grant_key(&mut self, room: RoomId) {
        let mut ev = Vec::new();
        self.take_key(room, &mut ev);
        self.pending.extend(ev);
    }
}

/// What a model is (the renderer keys its uploads by it).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ModelKey {
    Room(RoomId),
    Edge(EdgeId),
    Pocket(PocketPath),
}

/// The camera yaw (radians) that looks along `dir` (yaw 0 looks along -z, as the player's camera does).
pub fn yaw_of(dir: Vec3) -> f32 {
    (-dir.x).atan2(-dir.z)
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Distance along a ray to a rectangle centred at `c`, facing `n` (only from its front), `size` across.
fn ray_rect(eye: Vec3, dir: Vec3, c: Vec3, n: Vec3, size: Vec2) -> Option<f32> {
    let denom = dir.dot(n);
    if denom > -1e-4 {
        return None;
    }
    let d = (c - eye).dot(n) / denom;
    if d <= 0.0 {
        return None;
    }
    let p = eye + dir * d - c;
    let right = (-n).cross(Vec3::Y).normalize_or(Vec3::X);
    let up = Vec3::Y;
    (p.dot(right).abs() <= size.x * 0.5 && p.dot(up).abs() <= size.y * 0.5).then_some(d)
}

/// Distance along a ray to an upright cylinder standing on `base`.
fn ray_cylinder(eye: Vec3, dir: Vec3, base: Vec3, r: f32, h: f32) -> Option<f32> {
    let o = Vec2::new(eye.x - base.x, eye.z - base.z);
    let d = Vec2::new(dir.x, dir.z);
    let a = d.dot(d);
    if a < 1e-8 {
        return None;
    }
    let b = 2.0 * o.dot(d);
    let c = o.dot(o) - r * r;
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()) / (2.0 * a);
    let t = if t > 0.0 { t } else { (-b + disc.sqrt()) / (2.0 * a) };
    if t <= 0.0 {
        return None;
    }
    let y = eye.y + dir.y * t;
    (y >= base.y - 0.1 && y <= base.y + h).then_some(t)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn spec() -> Arc<MuseumSpec> {
        let v: serde_json::Value = serde_json::from_str(include_str!("../../content/collection.json")).unwrap();
        let collection = super::super::spec::parse_collection(&v, None).expect("the game's collection parses");
        Arc::new(MuseumSpec {
            collection_file: "collection.json".into(),
            collection,
            art_file: None,
            art: None,
            seed: red_engine2::atmosphere::LaunchSeed::Fixed(1),
            start_theme: None,
        })
    }

    fn look_at(m: &Museum, target: Vec3, from: Vec3) -> (Vec3, Vec3) {
        let _ = m;
        (from, (target - from).normalize())
    }

    #[test]
    fn the_first_room_teaches_the_whole_game_step_in_take_the_key_and_the_doors_open() {
        let mut m = Museum::new(spec(), 3, None);
        let k = m.contents(RoomId::ORIGIN);
        assert_eq!(k.key.depth, 1);
        let e = k.exhibits[k.key.exhibit as usize].clone();
        // Doors are shut until the key is found.
        for s in Side::ALL {
            assert_eq!(m.door_openness(EdgeId::of(RoomId::ORIGIN, s)), 0.0);
        }
        let stand = Vec3::new(e.at.x, 0.0, e.at.z) + e.facing * 2.5;
        let (eye, dir) = look_at(&m, e.at, stand + Vec3::Y * 1.6);
        assert_eq!(m.aim(eye, dir), Some(Target::Exhibit { room: RoomId::ORIGIN, index: e.index }));
        let (ev, _) = m.update(0.016, eye, dir, stand, true);
        assert!(ev.contains(&Event::SteppingIn { depth: 1 }));
        // Fly until we land.
        let mut landed = None;
        for _ in 0..400 {
            let (ev, tp) = m.update(0.016, eye, dir, stand, false);
            if let Some(tp) = tp {
                landed = Some(tp);
            }
            if ev.iter().any(|e| matches!(e, Event::Arrived { .. })) {
                assert!(m.pose().is_none());
            }
        }
        let landed = landed.expect("the flight lands");
        assert!(matches!(m.place, Place::Inside(_)));
        assert!(m.free());
        let key = m.key_at().expect("the key floats in the first painting");
        // Walk to the key.
        let (ev, _) = m.update(0.016, key, Vec3::NEG_Z, Vec3::new(key.x, landed.at.y, key.z), false);
        assert!(ev.iter().any(|e| matches!(e, Event::KeyFound { keys: 1, .. })), "{ev:?}");
        assert!(m.key_at().is_none());
        // Back out through the way back.
        let p = m.pocket.clone().unwrap();
        let way = POCKET_ORIGIN + p.exit + Vec3::Y * 1.3;
        let from = POCKET_ORIGIN + p.spawn + Vec3::Y * 1.6;
        assert_eq!(m.aim(from, (way - from).normalize()), Some(Target::WayBack));
        m.update(0.016, from, (way - from).normalize(), POCKET_ORIGIN + p.spawn, true);
        let mut back = None;
        for _ in 0..300 {
            if let (_, Some(tp)) = m.update(0.016, from, Vec3::NEG_Z, POCKET_ORIGIN + p.spawn, false) {
                back = Some(tp);
            }
        }
        let back = back.expect("back in the museum");
        assert_eq!(m.place, Place::Halls);
        assert!(m.room.x == 0 && m.room.z == 0 && back.at.y == 0.0);
        // The doors swing open over a few seconds.
        for _ in 0..400 {
            m.update(0.016, back.at + Vec3::Y * 1.6, Vec3::NEG_Z, back.at, false);
        }
        for s in Side::ALL {
            assert_eq!(m.door_openness(EdgeId::of(RoomId::ORIGIN, s)), 1.0);
        }
        assert_eq!(m.keys(), 1);
        assert!(m.progress.entered >= 1);
    }

    #[test]
    fn progress_round_trips_and_a_shut_door_blocks() {
        let mut m = Museum::new(spec(), 9, None);
        let has_door_collider = |m: &Museum, e: EdgeId| m.colliders().iter().any(|c| *c == door_collider(e));
        let e = EdgeId::of(RoomId::ORIGIN, Side::North);
        assert!(has_door_collider(&m, e));
        m.grant_key(RoomId::ORIGIN);
        for _ in 0..400 {
            m.update(0.016, Vec3::Y * 1.6, Vec3::NEG_Z, Vec3::ZERO, false);
        }
        assert!(!has_door_collider(&m, e));
        let json = m.progress.to_json();
        let back = Progress::from_json(&json).unwrap();
        assert_eq!(back, m.progress);
        let again = Museum::new(spec(), 9, Some(back));
        assert_eq!(again.keys(), 1);
        assert_eq!(again.door_openness(e), 1.0, "a door already opened starts open");
    }
}

#[cfg(test)]
mod peek {
    use super::*;
    #[test]
    #[ignore]
    fn print_rooms() {
        let mut m = Museum::new(tests::spec(), 12345, None);
        for z in -6..6 {
            let row: String = (-8..8).map(|x| format!("{:>3}", m.layout.room(RoomId::new(x, z)).theme)).collect();
            println!("{row}");
        }
        for (x, z) in [(0, 0), (1, 0), (0, -1), (2, 3)] {
            let k = m.contents(RoomId::new(x, z));
            let t = &m.spec.collection.themes[k.room.theme];
            println!(
                "== Gallery {} ({x},{z}) {} / {} size {:?} h {} plan {:?} key {:?}",
                RoomId::new(x, z).number(),
                t.wing,
                t.name,
                k.room.size,
                k.room.height,
                k.room.plan,
                k.key
            );
            println!("   note: {}", k.note.text);
            for e in &k.exhibits {
                println!(
                    "  [{}] {:?} {:?} {:?} style {} — \"{}\" — {} — {} — {}\n      {}",
                    e.index, e.kind, e.family, e.frame, e.art.style, e.plaque.title, e.plaque.artist, e.plaque.year, e.plaque.medium, e.plaque.story
                );
            }
        }
    }
}
