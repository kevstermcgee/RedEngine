//! A world being played: the streamed terrain, the body, the inventory, the furnaces and chests, health and air, mining and building, the
//! time of day, and the save. It ticks at a fixed 60 Hz from [`Input`] and reports what should be heard and shown as [`Event`]s, so the game
//! mode only routes input in and sounds out (and tests drive it with no window).

use crate::block::{Block, Shape, Sound, DIRS, FACING_DIR};
use crate::craft::Furnace;
use crate::entities::{Entity, Kind};
use crate::inventory::Inventory;
use crate::item::{id, mining, Item, Stack};
use crate::physics::{Body, Controls};
use crate::save::{GameMode, Level};
use crate::stream::Terrain;
use crate::world::{Hit, World};
use glam::{IVec3, Vec3};
use std::collections::HashMap;
use std::path::PathBuf;

mod life;
mod water;

/// One tick, seconds.
pub const TICK: f32 = 1.0 / 60.0;
/// Most health (ten hearts).
pub const MAX_HEALTH: f32 = 20.0;
/// Seconds of breath under water.
pub const MAX_AIR: f32 = 15.0;

/// What the player does this tick.
#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    pub forward: f32,
    pub strafe: f32,
    pub jump: bool,
    pub sneak: bool,
    pub sprint: bool,
    pub yaw: f32,
    pub pitch: f32,
    /// The mine button is held.
    pub mine: bool,
    /// The use button went down this tick, or is held.
    pub use_pressed: bool,
    pub use_held: bool,
}

/// Something to hear or show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Dig(Sound),
    Broke(Sound),
    Place(Sound),
    Step(Sound),
    Hurt,
    Pop,
    Eat,
    Splash,
    Click,
    Swing,
    Death,
    Oink,
    Groan,
    Fuse,
    Explode,
    /// A mob was struck.
    MobHurt(Kind),
    /// A screen to open (a block was used).
    Open(Opened),
    /// A new day dawned.
    Dawn(u32),
}

/// A screen a block opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opened {
    Table(IVec3),
    Furnace(IVec3),
    Chest(IVec3),
}

/// A block being mined.
#[derive(Clone, Copy, Debug)]
pub struct Mining {
    pub pos: IVec3,
    pub done: f32,
    pub needs: f32,
    pub blow: f32,
}

/// A world being played.
pub struct Play {
    pub dir: PathBuf,
    pub name: String,
    pub seed: u32,
    pub mode: GameMode,
    pub terrain: Terrain,
    pub body: Body,
    /// Where the body was a tick ago (the camera moves between the two).
    pub prev: Vec3,
    pub inv: Inventory,
    /// What the mouse carries on a screen.
    pub cursor: Stack,
    /// The crafting grid of the screen that is open (2 x 2 uses the first four).
    pub grid: [Stack; 9],
    pub furnaces: HashMap<IVec3, Furnace>,
    pub chests: HashMap<IVec3, Vec<Stack>>,
    pub health: f32,
    pub air: f32,
    pub since_hurt: f32,
    regen: f32,
    hazard: f32,
    pub hurt_flash: f32,
    pub spawn: Vec3,
    /// The ground around the player is loaded and the body stands on it.
    pub ready: bool,
    pub acc: f32,
    pub mining: Option<Mining>,
    pub target: Option<Hit>,
    use_wait: f32,
    break_wait: f32,
    step_at: f32,
    pub swing: f32,
    pub played: f64,
    pub days: u32,
    pub time: f32,
    pub save_in: f32,
    pub dead: Option<String>,
    pub events: Vec<Event>,
    jump_was: bool,
    jump_tap: f32,
    pub mined: u32,
    pub placed: u32,
    rng: u32,
    /// Whether this world's first ground has been found (a new world's spawn is put on top of what is really there).
    fresh: bool,
    /// What moves: dropped items, mobs, lit TNT.
    pub entities: Vec<Entity>,
    next_id: u32,
    spawn_in: f32,
    attack_wait: f32,
    /// How much daylight there is now (0 night .. 1 noon), from the game's clock.
    pub daylight: f32,
    pub kills: u32,
    /// Fragments of broken blocks, flying for a moment (only seen, never touched).
    pub particles: Vec<Particle>,
    /// Where the next drop pops out (the middle of the block just broken).
    drop_at: Vec3,
    /// Cells where water may move next, and when it next does.
    water_queue: std::collections::VecDeque<IVec3>,
    water_in: f32,
}

impl Play {
    /// A new world.
    pub fn create(dir: PathBuf, name: &str, seed: u32, mode: GameMode, view: i32, workers: usize) -> Play {
        let world = World::new(seed);
        let (x, y, z) = world.gen.spawn_point();
        let spawn = Vec3::new(x as f32 + 0.5, y as f32, z as f32 + 0.5);
        let mut p = Play::empty(dir, name, seed, mode, Terrain::new(world, view, workers), spawn);
        p.fresh = true;
        if mode == GameMode::Creative {
            p.body.flying = false;
        }
        p
    }

    /// A saved world.
    pub fn open(dir: PathBuf, level: Level, chunks: HashMap<crate::chunk::ChunkPos, Vec<u8>>, view: i32, workers: usize) -> Play {
        let mut world = World::new(level.seed);
        world.stored = chunks;
        let mut p = Play::empty(dir, &level.name, level.seed, level.mode, Terrain::new(world, view, workers), level.spawn);
        p.body.pos = level.pos;
        p.prev = level.pos;
        p.body.flying = level.flying && level.mode == GameMode::Creative;
        p.health = level.health;
        for (i, s) in level.slots.iter().enumerate().take(crate::inventory::SLOTS) {
            p.inv.slots[i] = *s;
        }
        p.inv.selected = level.selected;
        p.furnaces = level.furnaces;
        p.chests = level.chests;
        p.time = level.time;
        p.days = level.days;
        p.played = level.played;
        p
    }

    fn empty(dir: PathBuf, name: &str, seed: u32, mode: GameMode, terrain: Terrain, spawn: Vec3) -> Play {
        Play {
            dir,
            name: name.to_string(),
            seed,
            mode,
            terrain,
            body: Body::new(spawn),
            prev: spawn,
            inv: Inventory::default(),
            cursor: Stack::EMPTY,
            grid: [Stack::EMPTY; 9],
            furnaces: HashMap::new(),
            chests: HashMap::new(),
            health: MAX_HEALTH,
            air: MAX_AIR,
            since_hurt: 99.0,
            regen: 0.0,
            hazard: 0.0,
            hurt_flash: 0.0,
            spawn,
            ready: false,
            acc: 0.0,
            mining: None,
            target: None,
            use_wait: 0.0,
            break_wait: 0.0,
            step_at: 0.0,
            swing: 0.0,
            played: 0.0,
            days: 0,
            time: 0.27,
            save_in: 30.0,
            dead: None,
            events: Vec::new(),
            jump_was: false,
            jump_tap: 99.0,
            mined: 0,
            placed: 0,
            rng: seed | 1,
            fresh: false,
            entities: Vec::new(),
            next_id: 1,
            spawn_in: 2.0,
            attack_wait: 0.0,
            daylight: 1.0,
            kills: 0,
            drop_at: Vec3::ZERO,
            water_queue: std::collections::VecDeque::new(),
            water_in: 0.0,
            particles: Vec::new(),
        }
    }

    /// Adds an entity; returns its id.
    pub fn spawn(&mut self, kind: Kind, pos: Vec3) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.entities.push(Entity::new(id, kind, pos));
        id
    }

    /// Drops a stack at `at`, popping out with a little speed.
    pub fn drop_item(&mut self, s: Stack, at: Vec3, vel: Vec3, pickup_after: f32) {
        if s.is_empty() {
            return;
        }
        let id = self.spawn(Kind::Item(s), at);
        if let Some(e) = self.entities.iter_mut().find(|e| e.id == id) {
            e.body.vel = vel;
            e.pickup_after = pickup_after;
        }
    }

    /// Throws what is in hand (one, or the whole stack) the way the player looks.
    pub fn throw(&mut self, all: bool, yaw: f32, pitch: f32) {
        let held = self.inv.held();
        if held.is_empty() {
            return;
        }
        let n = if all { held.count } else { 1 };
        self.inv.use_held(n);
        let dir = crate::tool::look_dir(yaw, pitch);
        let at = self.body.pos + Vec3::Y * (self.body.eye_height() - 0.3) + dir * 0.4;
        self.drop_item(Stack { count: n, ..held }, at, dir * 6.0 + Vec3::Y * 1.5, 1.5);
    }

    pub fn creative(&self) -> bool {
        self.mode == GameMode::Creative
    }

    fn roll(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// The level to save.
    pub fn level(&self, pitch: f32) -> Level {
        Level {
            name: self.name.clone(),
            seed: self.seed,
            mode: self.mode,
            pos: self.body.pos,
            yaw: 0.0,
            pitch,
            health: self.health,
            flying: self.body.flying,
            spawn: self.spawn,
            slots: self.inv.slots.to_vec(),
            selected: self.inv.selected,
            time: self.time,
            days: self.days,
            furnaces: self.furnaces.clone(),
            chests: self.chests.clone(),
            played: self.played,
            last_played: crate::save::now_secs(),
        }
    }

    /// Writes the world (the carried stacks go back into the inventory first).
    pub fn save(&mut self, yaw: f32, pitch: f32) -> Result<(), String> {
        self.return_carried();
        let mut level = self.level(pitch);
        level.yaw = yaw;
        let chunks = self.terrain.world.edited_chunks();
        self.save_in = 30.0;
        crate::save::save(&self.dir, &level, &chunks)
    }

    /// The cursor's and the crafting grid's stacks back into the inventory (a screen closed).
    pub fn return_carried(&mut self) {
        for i in 0..9 {
            let s = std::mem::replace(&mut self.grid[i], Stack::EMPTY);
            if !s.is_empty() {
                self.inv.add(s);
            }
        }
        let c = std::mem::replace(&mut self.cursor, Stack::EMPTY);
        if !c.is_empty() {
            self.inv.add(c);
        }
    }

    /// The eye, between the last two ticks (`alpha` 0..1).
    pub fn eye(&self, alpha: f32) -> Vec3 {
        self.prev.lerp(self.body.pos, alpha) + Vec3::Y * self.body.eye_height()
    }

    /// The reach of a hand.
    pub fn reach(&self) -> f32 {
        if self.creative() {
            5.0
        } else {
            4.5
        }
    }

    /// Runs real time `dt` in fixed ticks.
    pub fn advance(&mut self, input: &Input, dt: f32) {
        self.acc = (self.acc + dt).min(0.25);
        while self.acc >= TICK {
            self.acc -= TICK;
            self.tick(input);
        }
    }

    pub fn tick(&mut self, input: &Input) {
        self.prev = self.body.pos;
        self.played += TICK as f64;
        self.terrain.update(self.body.pos);
        if !self.ready {
            if self.terrain.ready_around(self.body.pos, 1) {
                self.ready = true;
                if self.fresh {
                    let (x, z) = (self.spawn.x.floor() as i32, self.spawn.z.floor() as i32);
                    if let Some(g) = self.terrain.world.ground(x, z) {
                        self.spawn.y = g as f32 + 1.0;
                    }
                    self.body.pos = self.spawn;
                    self.prev = self.spawn;
                    self.fresh = false;
                }
                // A saved position inside a block (an edit lost with a crash) is lifted out.
                while !self.body.fits(&self.terrain.world) && self.body.pos.y < crate::chunk::H as f32 {
                    self.body.pos.y += 1.0;
                }
                self.body.fall_from = self.body.pos.y;
            }
            return;
        }
        if self.dead.is_some() {
            return;
        }
        self.since_hurt += TICK;
        self.hurt_flash = (self.hurt_flash - TICK * 3.0).max(0.0);
        self.swing = (self.swing - TICK * 4.0).max(0.0);
        // Two taps of jump in creative: fly, or stop flying.
        if input.jump && !self.jump_was {
            if self.creative() && self.jump_tap < 0.3 {
                self.body.flying = !self.body.flying;
                self.jump_tap = 99.0;
            } else {
                self.jump_tap = 0.0;
            }
        }
        self.jump_was = input.jump;
        self.jump_tap += TICK;
        let controls = Controls { forward: input.forward, strafe: input.strafe, jump: input.jump, sneak: input.sneak, sprint: input.sprint, yaw: input.yaw };
        let landing = self.body.step(&self.terrain.world, &controls, TICK);
        if landing.splash {
            self.events.push(Event::Splash);
        }
        if landing.fell > 3.4 && !self.creative() {
            self.hurt((landing.fell - 3.0).floor(), "You hit the ground too hard");
        }
        if self.body.on_ground && self.body.walked - self.step_at > 1.75 {
            self.step_at = self.body.walked;
            let under = self.under_feet();
            if !under.is_air() {
                self.events.push(Event::Step(under.info().sound));
            }
        }
        if self.body.jumped {
            let under = self.under_feet();
            if !under.is_air() {
                self.events.push(Event::Step(under.info().sound));
            }
        }
        self.surroundings();
        self.target = {
            let eye = self.body.pos + Vec3::Y * self.body.eye_height();
            self.terrain.world.raycast(eye, crate::tool::look_dir(input.yaw, input.pitch), self.reach())
        };
        if !self.attack(input) {
            self.mine(input);
        } else {
            self.mining = None;
        }
        self.use_hand(input);
        self.furnaces();
        self.entities_tick();
        self.flow_water();
        for q in self.particles.iter_mut() {
            q.age += TICK;
            q.vel.y -= 16.0 * TICK;
            q.pos += q.vel * TICK;
            // They rest on whatever they land on.
            let c = q.pos.floor().as_ivec3();
            if self.terrain.world.block(c.x, c.y, c.z).solid() {
                q.pos -= q.vel * TICK;
                q.vel = Vec3::ZERO;
            }
        }
        self.particles.retain(|q| q.age < q.life);
        self.save_in -= TICK;
    }

    fn under_feet(&self) -> Block {
        let p = self.body.pos - Vec3::Y * 0.2;
        self.terrain.world.block(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
    }

    /// Breath, lava, cactus, the void, and getting better.
    fn surroundings(&mut self) {
        if self.creative() {
            self.air = MAX_AIR;
            return;
        }
        if self.body.head_in_water {
            self.air -= TICK;
            if self.air <= 0.0 {
                self.hazard += TICK;
                if self.hazard >= 1.0 {
                    self.hazard = 0.0;
                    self.hurt(2.0, "You drowned");
                }
            }
        } else {
            self.air = (self.air + TICK * 5.0).min(MAX_AIR);
        }
        let cactus = self.touching(Block::CACTUS);
        if self.body.in_lava || cactus {
            self.hazard += TICK;
            if self.hazard >= 0.5 {
                self.hazard = 0.0;
                if self.body.in_lava {
                    self.hurt(4.0, "You tried to swim in lava");
                } else {
                    self.hurt(1.0, "You were pricked to death");
                }
            }
        }
        if self.body.pos.y < -20.0 {
            self.hurt(100.0, "You fell out of the world");
        }
        if self.health < MAX_HEALTH && self.since_hurt > 4.0 {
            self.regen += TICK;
            if self.regen >= 2.5 {
                self.regen = 0.0;
                self.health = (self.health + 1.0).min(MAX_HEALTH);
            }
        }
    }

    fn touching(&self, b: Block) -> bool {
        let (lo, hi) = self.body.bounds();
        let (lo, hi) = (lo - Vec3::splat(0.05), hi + Vec3::splat(0.05));
        for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
            for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
                    if self.terrain.world.block(x, y, z) == b {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Takes `amount` half hearts; `cause` names the death it may be.
    pub fn hurt(&mut self, amount: f32, cause: &str) {
        if self.creative() || self.dead.is_some() || amount <= 0.0 {
            return;
        }
        self.health -= amount;
        self.since_hurt = 0.0;
        self.hurt_flash = 1.0;
        self.events.push(Event::Hurt);
        if self.health <= 0.0 {
            self.health = 0.0;
            self.dead = Some(cause.to_string());
            self.mining = None;
            self.events.push(Event::Death);
            // Everything carried falls where the player fell.
            let at = self.body.pos + Vec3::Y;
            for i in 0..crate::inventory::SLOTS {
                let s = std::mem::replace(&mut self.inv.slots[i], Stack::EMPTY);
                let (a, b) = (self.roll() - 0.5, self.roll() - 0.5);
                self.drop_item(s, at, Vec3::new(a * 5.0, 3.0, b * 5.0), 2.0);
            }
            self.return_carried();
            for i in 0..crate::inventory::SLOTS {
                let s = std::mem::replace(&mut self.inv.slots[i], Stack::EMPTY);
                self.drop_item(s, at, Vec3::new(0.0, 3.0, 0.0), 2.0);
            }
        }
    }

    /// Back at the spawn, whole again (the inventory is kept).
    pub fn respawn(&mut self) {
        self.dead = None;
        self.health = MAX_HEALTH;
        self.air = MAX_AIR;
        self.body = Body { auto_jump: self.body.auto_jump, ..Body::new(self.spawn) };
        self.prev = self.spawn;
        self.ready = false;
    }

    fn mine(&mut self, input: &Input) {
        self.break_wait = (self.break_wait - TICK).max(0.0);
        let Some(hit) = self.target.filter(|_| input.mine) else {
            self.mining = None;
            return;
        };
        if self.break_wait > 0.0 {
            return;
        }
        let held = self.inv.held_item();
        let Some((needs, _)) = mining(hit.block, held) else {
            self.mining = None;
            return;
        };
        let needs = if self.creative() { 0.0 } else { needs };
        let m = match self.mining {
            Some(m) if m.pos == hit.pos => m,
            _ => Mining { pos: hit.pos, done: 0.0, needs, blow: 0.0 },
        };
        let mut m = m;
        m.done += TICK;
        m.blow -= TICK;
        if self.swing <= 0.0 {
            self.swing = 1.0;
        }
        if m.blow <= 0.0 && needs > 0.05 {
            m.blow = 0.25;
            self.events.push(Event::Dig(hit.block.info().sound));
        }
        if m.done >= needs {
            self.break_block(hit.pos);
            self.mining = None;
            // A held button mines the next block after a short beat, as the original does.
            self.break_wait = if self.creative() { 0.25 } else { 0.1 };
        } else {
            self.mining = Some(m);
        }
    }

    /// Breaks the block at `p` as the player does: its drops into the inventory, the tool worn, what stood on it or hung from it falling too.
    pub fn break_block(&mut self, p: IVec3) {
        let b = self.terrain.world.block_at(p);
        if b.is_air() {
            return;
        }
        let held = self.inv.held_item();
        let drops = mining(b, held).is_some_and(|(_, harvest)| harvest);
        self.drop_at = p.as_vec3() + Vec3::new(0.5, 0.3, 0.5);
        self.set(p, Block::AIR);
        self.events.push(Event::Broke(b.info().sound));
        self.burst(p, b);
        self.mined += 1;
        if !self.creative() {
            if drops {
                self.give_drop(b);
            }
            // A tool wears with every block it breaks (a sword twice), and breaks when worn out.
            if let Some(t) = held.and_then(Item::tool) {
                let s = &mut self.inv.slots[self.inv.selected];
                s.wear += if t.kind == crate::item::ToolKind::Sword { 2 } else { 1 };
                if s.wear >= t.durability() {
                    *s = Stack::EMPTY;
                    self.events.push(Event::Broke(Sound::Wood));
                }
            }
        }
        // A container spills what it held into the inventory.
        let centre = p.as_vec3() + Vec3::new(0.5, 0.5, 0.5);
        if let Some(f) = self.furnaces.remove(&p) {
            for s in [f.input, f.fuel, f.output] {
                self.drop_item(s, centre, Vec3::Y * 2.0, 0.3);
            }
        }
        if let Some(c) = self.chests.remove(&p) {
            for s in c {
                let (a, b) = (self.roll() - 0.5, self.roll() - 0.5);
                self.drop_item(s, centre, Vec3::new(a * 3.0, 3.0, b * 3.0), 0.3);
            }
        }
        self.unsupported(p);
    }

    fn give_drop(&mut self, b: Block) {
        let s = match b.info().drop {
            crate::block::Drop::Itself => Stack::new(Item::of(b.base()), 1),
            crate::block::Drop::Item(item, n) => Stack::new(Item(item), n),
            crate::block::Drop::Nothing => {
                // Leaves drop an apple now and then; gravel flint.
                if b == Block::OAK_LEAVES && self.roll() < 0.05 {
                    Stack::new(Item(id::APPLE), 1)
                } else {
                    Stack::EMPTY
                }
            }
        };
        let s = if b == Block::GRAVEL && self.roll() < 0.1 { Stack::new(Item(id::FLINT), 1) } else { s };
        if !s.is_empty() {
            let at = self.drop_at;
            let (a, c) = (self.roll() - 0.5, self.roll() - 0.5);
            self.drop_item(s, at, Vec3::new(a * 2.0, 3.0, c * 2.0), 0.3);
        }
    }

    /// Whatever needed the block at `p` (a flower on it, a torch on its side, sand on top) follows it.
    fn unsupported(&mut self, p: IVec3) {
        let mut q = p + IVec3::Y;
        loop {
            let above = self.terrain.world.block_at(q);
            let needs_floor = matches!(above.info().shape, Shape::Plant(_) | Shape::Torch(None) | Shape::Cactus);
            if !needs_floor {
                break;
            }
            self.drop_at = q.as_vec3() + Vec3::new(0.5, 0.3, 0.5);
            self.set(q, Block::AIR);
            if !self.creative() {
                self.give_drop(above);
            }
            q += IVec3::Y;
        }
        for f in 0..4u8 {
            // A torch leaning `f` hangs from the block behind it.
            let n = p + dir_of(f);
            let b = self.terrain.world.block_at(n);
            if b.facing() == Some((Block::WALL_TORCH, f)) {
                self.drop_at = n.as_vec3() + Vec3::new(0.5, 0.3, 0.5);
                self.set(n, Block::AIR);
                if !self.creative() {
                    self.give_drop(b);
                }
            }
        }
        self.settle_falling(p);
    }

    /// Sand and gravel over an empty cell fall to the ground.
    fn settle_falling(&mut self, p: IVec3) {
        let mut q = p + IVec3::Y;
        while matches!(self.terrain.world.block_at(q), Block::SAND | Block::GRAVEL) {
            let b = self.terrain.world.block_at(q);
            let mut to = q;
            while to.y > 0 && self.terrain.world.block_at(to - IVec3::Y).replaceable() {
                to -= IVec3::Y;
            }
            if to == q {
                break;
            }
            self.set(q, Block::AIR);
            self.set(to, b);
            q += IVec3::Y;
        }
    }

    fn set(&mut self, p: IVec3, b: Block) {
        self.terrain.set_block(p.x, p.y, p.z, b);
        // Water around a changed cell may move.
        self.stir_water(p);
    }

    fn use_hand(&mut self, input: &Input) {
        self.use_wait = (self.use_wait - TICK).max(0.0);
        if !(input.use_pressed || (input.use_held && self.use_wait <= 0.0)) {
            return;
        }
        if !input.use_pressed && self.use_wait > 0.0 {
            return;
        }
        self.use_wait = 0.25;
        let held = self.inv.held();
        // Food is eaten when there is health to win back.
        if let Some(heal) = (!held.is_empty()).then(|| held.item.food()).flatten() {
            if self.health < MAX_HEALTH || self.creative() {
                if !self.creative() {
                    self.inv.use_held(1);
                }
                self.health = (self.health + heal).min(MAX_HEALTH);
                self.events.push(Event::Eat);
                return;
            }
        }
        let Some(hit) = self.target else { return };
        // A block that opens (unless sneaking: then the held block is put against it).
        if !input.sneak {
            let opened = match hit.block.facing().map(|(b, _)| b).unwrap_or(hit.block) {
                Block::CRAFTING_TABLE => Some(Opened::Table(hit.pos)),
                Block::FURNACE | Block::FURNACE_LIT => {
                    self.furnaces.entry(hit.pos).or_default();
                    Some(Opened::Furnace(hit.pos))
                }
                Block::CHEST => {
                    self.chests.entry(hit.pos).or_insert_with(|| vec![Stack::EMPTY; 27]);
                    Some(Opened::Chest(hit.pos))
                }
                _ => None,
            };
            if let Some(o) = opened {
                self.events.push(Event::Open(o));
                self.events.push(Event::Click);
                return;
            }
        }
        if held.item.0 == id::FLINT_AND_STEEL && hit.block == Block::TNT {
            self.swing = 1.0;
            self.ignite(hit.pos);
            return;
        }
        let Some(block) = (!held.is_empty()).then(|| held.item.block()).flatten() else { return };
        if self.place(hit, block, input.yaw) {
            self.swing = 1.0;
            self.placed += 1;
            if !self.creative() {
                self.inv.use_held(1);
            }
            self.events.push(Event::Place(block.info().sound));
        }
    }

    /// Lights a TNT block: it goes off where it is (the explosion clears what is near and hurts what stands close).
    pub fn ignite(&mut self, p: IVec3) {
        self.set(p, Block::AIR);
        self.spawn(Kind::Tnt, p.as_vec3() + Vec3::new(0.5, 0.0, 0.5));
        self.events.push(Event::Fuse);
    }

    /// An explosion at `at`: blocks within `r` are cleared (what hard rock and water do not resist), the player hurt by how close.
    pub fn explode(&mut self, at: Vec3, r: f32) {
        let ri = r.ceil() as i32;
        let mut chain = Vec::new();
        for dy in -ri..=ri {
            for dz in -ri..=ri {
                for dx in -ri..=ri {
                    let d = Vec3::new(dx as f32, dy as f32, dz as f32);
                    let jitter = self.roll() * 0.8;
                    if d.length() > r - jitter {
                        continue;
                    }
                    let p = (at + d).floor().as_ivec3();
                    let b = self.terrain.world.block_at(p);
                    if b.is_air() || b.is_liquid() || matches!(b, Block::BEDROCK | Block::OBSIDIAN) {
                        continue;
                    }
                    if b == Block::TNT {
                        chain.push(p);
                    }
                    self.drop_at = p.as_vec3() + Vec3::splat(0.5);
                    self.set(p, Block::AIR);
                    // A few of the blocks come back as drops.
                    if !self.creative() && b != Block::TNT && self.roll() < 0.3 && b.info().hardness < 10.0 {
                        self.give_drop(b);
                    }
                }
            }
        }
        let dist = (self.body.pos + Vec3::Y - at).length();
        if dist < r * 2.0 {
            let dmg = ((1.0 - dist / (r * 2.0)) * 24.0).round();
            self.hurt(dmg, "You were blown up");
            let push = (self.body.pos - at).normalize_or_zero() * (1.0 - dist / (r * 2.0)) * 14.0;
            self.body.vel += push + Vec3::Y * 4.0;
        }
        self.events.push(Event::Explode);
        // TNT caught in the blast is lit, with a shorter fuse.
        for p in chain {
            let id = self.spawn(Kind::Tnt, p.as_vec3() + Vec3::new(0.5, 0.0, 0.5));
            let f = 0.5 + self.roll();
            if let Some(e) = self.entities.iter_mut().find(|e| e.id == id) {
                e.fuse = f;
            }
        }
        // Mobs and items in the blast are thrown, and hurt.
        for e in self.entities.iter_mut() {
            let to = e.body.pos + Vec3::Y * 0.5 - at;
            let d = to.length();
            if d < r * 2.0 {
                let k = 1.0 - d / (r * 2.0);
                e.body.vel += to.normalize_or_zero() * k * 12.0 + Vec3::Y * 3.0;
                if e.kind.mob() && e.alive() {
                    e.health -= k * 30.0;
                    e.hurt = 1.0;
                    if e.health <= 0.0 {
                        e.dying = 0.0;
                    }
                }
            }
        }
    }

    /// Puts `block` against the face `hit` points at (into the cell itself when that holds grass that gives way). Returns whether it was put.
    pub fn place(&mut self, hit: Hit, block: Block, yaw: f32) -> bool {
        let cell = if hit.block.replaceable() { hit.pos } else { hit.pos + hit.normal };
        if !(0..crate::chunk::H).contains(&cell.y) || !self.terrain.world.ready(cell.x, cell.z) {
            return false;
        }
        let here = self.terrain.world.block_at(cell);
        if !here.replaceable() {
            return false;
        }
        let below = self.terrain.world.block_at(cell - IVec3::Y);
        let shaped = match block.info().shape {
            Shape::Torch(_) => {
                if hit.normal == IVec3::Y || hit.block.replaceable() {
                    if !below.solid() || !below.cull() {
                        return false;
                    }
                    Block::TORCH
                } else if hit.normal.y == 0 {
                    // Leans out from the face it was put on.
                    let f = (0..4u8).find(|f| dir_of(*f) == hit.normal).unwrap_or(0);
                    if !hit.block.cull() {
                        return false;
                    }
                    Block::faced(Block::WALL_TORCH, f)
                } else {
                    return false;
                }
            }
            Shape::Plant(p) => {
                let ok = match p {
                    crate::block::Plant::DeadBush => matches!(below, Block::SAND | Block::DIRT | Block::GRASS),
                    _ => matches!(below, Block::GRASS | Block::DIRT | Block::SNOWY_GRASS),
                };
                if !ok {
                    return false;
                }
                block
            }
            Shape::Cactus => {
                if !matches!(below, Block::SAND | Block::CACTUS) {
                    return false;
                }
                block
            }
            _ => match block.facing() {
                // Its front toward the player.
                Some((base, _)) => Block::faced(base, facing_toward_player(yaw)),
                None => block,
            },
        };
        if shaped.solid() && self.body.overlaps_cell(cell) {
            return false;
        }
        self.set(cell, shaped);
        if matches!(shaped, Block::SAND | Block::GRAVEL) {
            self.settle_falling(cell - IVec3::Y);
        }
        true
    }

    /// Every furnace burns on; one that lights or goes out changes its block (and its light).
    fn furnaces(&mut self) {
        let mut changes = Vec::new();
        for (p, f) in self.furnaces.iter_mut() {
            let burning = f.tick(TICK);
            let b = self.terrain.world.block_at(*p);
            if let Some((base, facing)) = b.facing() {
                let want = if burning { Block::FURNACE_LIT } else { Block::FURNACE };
                if (base == Block::FURNACE || base == Block::FURNACE_LIT) && base != want {
                    changes.push((*p, Block::faced(want, facing)));
                }
            }
        }
        for (p, b) in changes {
            self.set(p, b);
        }
    }

    /// Takes the events since the last call.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// The time of day (0..1) moved to `tod`; counts a dawn when it passes 0.25.
    pub fn set_time(&mut self, tod: f32) {
        if self.time < 0.25 && tod >= 0.25 && tod - self.time < 0.5 {
            self.days += 1;
            self.events.push(Event::Dawn(self.days));
        }
        self.time = tod;
    }
}

/// A fragment of a broken block.
#[derive(Clone, Copy, Debug)]
pub struct Particle {
    pub pos: Vec3,
    pub vel: Vec3,
    /// The texture it shows.
    pub tex: u8,
    pub age: f32,
    pub life: f32,
    pub size: f32,
}

impl Play {
    /// A broken block bursts into a dozen small fragments of its own texture.
    fn burst(&mut self, p: IVec3, b: Block) {
        let tex = b.face_tex(0);
        if tex == 0 || self.particles.len() > 400 {
            return;
        }
        for _ in 0..12 {
            let (x, y, z) = (self.roll(), self.roll(), self.roll());
            let at = p.as_vec3() + Vec3::new(0.2 + x * 0.6, 0.2 + y * 0.6, 0.2 + z * 0.6);
            let vel = Vec3::new(x - 0.5, 0.6 + y * 0.8, z - 0.5) * 4.0;
            let life = 0.5 + self.roll() * 0.6;
            let size = 0.07 + self.roll() * 0.06;
            self.particles.push(Particle { pos: at, vel, tex, age: 0.0, life, size });
        }
    }
}

/// The step a facing points along (0 north -Z, 1 east +X, 2 south +Z, 3 west -X).
pub fn dir_of(f: u8) -> IVec3 {
    let d = DIRS[FACING_DIR[(f & 3) as usize]];
    IVec3::new(d[0], d[1], d[2])
}

/// The facing that looks back at a player looking along `yaw`.
pub fn facing_toward_player(yaw: f32) -> u8 {
    // Looking north (yaw 0) the block faces south (2).
    let quarter = ((yaw.to_degrees().rem_euclid(360.0) + 45.0) / 90.0) as u8 % 4;
    (quarter + 2) % 4
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_at_spawn(mode: GameMode) -> Play {
        let dir = std::env::temp_dir().join(format!("redcraft-play-{}-{}", std::process::id(), mode.name()));
        let mut p = Play::create(dir, "Test", 7, mode, 2, 0);
        for _ in 0..20 {
            p.tick(&Input::default());
            if p.ready {
                break;
            }
        }
        assert!(p.ready, "the spawn loads");
        for _ in 0..60 {
            p.tick(&Input::default());
        }
        p
    }

    #[test]
    fn a_new_player_stands_on_the_ground_and_punches_a_block_free_by_hand() {
        let mut p = world_at_spawn(GameMode::Survival);
        assert!(p.body.on_ground, "{:?}", p.body.pos);
        // Look straight down and dig.
        let input = Input { pitch: -1.5, mine: true, ..Default::default() };
        let under = p.terrain.world.block_at((p.body.pos - Vec3::Y * 0.5).floor().as_ivec3());
        for _ in 0..120 {
            p.tick(&input);
        }
        assert!(p.mined >= 1, "dug into {under:?}");
        assert!(p.inv.count(Item::of(Block::DIRT)) >= 1 || p.inv.slots.iter().any(|s| !s.is_empty()), "the block is in the inventory");
        assert!(p.take_events().iter().any(|e| matches!(e, Event::Broke(_))));
    }

    #[test]
    fn placing_needs_a_free_cell_away_from_the_body_and_faces_the_player() {
        let mut p = world_at_spawn(GameMode::Creative);
        p.inv.slots[0] = Stack::new(Item::of(Block::FURNACE), 1);
        let eye = p.body.pos + Vec3::Y * p.body.eye_height();
        let ground = (p.body.pos - Vec3::Y * 0.5).floor().as_ivec3();
        // Into the cell the body stands in: refused.
        let at_feet = Hit { pos: ground, normal: IVec3::Y, block: p.terrain.world.block_at(ground), distance: 1.0 };
        assert!(!p.place(at_feet, Block::FURNACE, 0.0));
        // Two cells north: put, facing south (back at a player looking north).
        let spot = ground + IVec3::new(0, 0, -2);
        let top = (0..crate::chunk::H).rev().find(|&y| p.terrain.world.block(spot.x, y, spot.z).solid()).unwrap();
        let spot = IVec3::new(spot.x, top, spot.z);
        let hit = Hit { pos: spot, normal: IVec3::Y, block: p.terrain.world.block_at(spot), distance: (spot.as_vec3() - eye).length() };
        assert!(p.place(hit, Block::FURNACE, 0.0));
        assert_eq!(p.terrain.world.block_at(spot + IVec3::Y), Block::faced(Block::FURNACE, 2));
        // A torch on its side leans out of it.
        let side = Hit { pos: spot + IVec3::Y, normal: IVec3::Z, block: Block::faced(Block::FURNACE, 2), distance: 2.0 };
        assert!(p.place(side, Block::TORCH, 0.0));
        let torch = p.terrain.world.block_at(spot + IVec3::Y + IVec3::Z);
        assert_eq!(torch.facing(), Some((Block::WALL_TORCH, 2)));
        // Breaking the furnace takes the torch with it.
        p.break_block(spot + IVec3::Y);
        assert!(p.terrain.world.block_at(spot + IVec3::Y + IVec3::Z).is_air());
    }

    #[test]
    fn sand_falls_and_a_fall_hurts_in_survival() {
        let mut p = world_at_spawn(GameMode::Survival);
        let g = (p.body.pos - Vec3::Y * 0.5).floor().as_ivec3() + IVec3::new(3, 0, 0);
        let top = (0..crate::chunk::H).rev().find(|&y| p.terrain.world.block(g.x, y, g.z).solid()).unwrap();
        let air_cell = IVec3::new(g.x, top + 4, g.z);
        let hit = Hit { pos: air_cell - IVec3::Y, normal: IVec3::Y, block: Block::STONE, distance: 2.0 };
        // Put sand in mid-air (as if against a block that was there): it falls to the ground.
        p.set(air_cell - IVec3::Y, Block::STONE);
        assert!(p.place(hit, Block::SAND, 0.0));
        p.break_block(air_cell - IVec3::Y);
        assert_eq!(p.terrain.world.block(g.x, top + 1, g.z), Block::SAND, "it fell onto the ground");
        let before = p.health;
        p.body.pos.y += 12.0;
        p.body.fall_from = p.body.pos.y;
        for _ in 0..180 {
            p.tick(&Input::default());
        }
        assert!(p.health < before - 7.0, "a 12-block fall hurts: {} -> {}", before, p.health);
    }

    #[test]
    fn a_saved_world_opens_where_it_was_left() {
        let mut p = world_at_spawn(GameMode::Survival);
        let spot = (p.body.pos + Vec3::new(2.0, 0.0, 0.0)).floor().as_ivec3();
        p.set(spot, Block::GLASS);
        p.inv.slots[3] = Stack::new(Item(id::DIAMOND), 7);
        p.save(0.5, -0.1).unwrap();
        let (level, chunks) = crate::save::load(&p.dir).unwrap();
        let mut q = Play::open(p.dir.clone(), level, chunks, 2, 0);
        for _ in 0..30 {
            q.tick(&Input::default());
        }
        assert!(q.ready);
        assert_eq!(q.terrain.world.block_at(spot), Block::GLASS);
        assert_eq!(q.inv.count(Item(id::DIAMOND)), 7);
        assert!((q.body.pos - p.body.pos).length() < 0.5);
        let _ = std::fs::remove_dir_all(&p.dir);
    }

    #[test]
    fn water_pours_into_a_hole_dug_beside_it_and_goes_back_when_cut_off() {
        let mut p = world_at_spawn(GameMode::Creative);
        // A pool of source water three across, a trench beside it.
        let g = (p.body.pos - Vec3::Y * 0.5).floor().as_ivec3() + IVec3::new(4, 0, 0);
        let top = (0..crate::chunk::H).rev().find(|&y| p.terrain.world.block(g.x, y, g.z).solid()).unwrap();
        let base = IVec3::new(g.x, top + 1, g.z);
        let mut cells = Vec::new();
        for x in -1..=12 {
            for z in -1..=1 {
                for y in -1..=1 {
                    cells.push((base + IVec3::new(x, y, z), if y == -1 || z != 0 || x == -1 || x == 12 { Block::STONE } else { Block::AIR }));
                }
            }
        }
        p.terrain.set_many(&cells);
        p.terrain.set_many(&[(base, Block::WATER)]);
        p.stir_water(base);
        for _ in 0..60 * 4 {
            p.tick(&Input::default());
        }
        let w = |p: &Play, x: i32| p.terrain.world.block_at(base + IVec3::new(x, 0, 0)).water_level();
        assert_eq!(w(&p, 0), Some(0));
        assert_eq!(w(&p, 3), Some(3), "three cells out it is level 3");
        assert_eq!(w(&p, 7), Some(7));
        assert_eq!(w(&p, 8), None, "no farther than seven");
        // Take the source away: the stream drains.
        p.set(base, Block::STONE);
        for _ in 0..60 * 4 {
            p.tick(&Input::default());
        }
        assert!((1..=8).all(|x| w(&p, x).is_none()), "drained: {:?}", (1..=8).map(|x| w(&p, x)).collect::<Vec<_>>());
    }

    #[test]
    fn facings_look_back_at_the_player() {
        assert_eq!(facing_toward_player(0.0), 2);
        assert_eq!(facing_toward_player(std::f32::consts::FRAC_PI_2), 3, "looking east, it faces west");
        assert_eq!(dir_of(1), IVec3::X);
    }
}
