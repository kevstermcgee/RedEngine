//! The player among the mobs: a blow at the mob under the crosshair ([`Play::attack`]), every entity's tick ([`Play::entities_tick`]: items
//! fall and fly to the player, mobs think and walk, fuses burn) and where new mobs appear ([`Play::spawn_mobs`]).

use super::{Event, Input, Play, TICK};
use crate::entities::{think, Kind};
use crate::item::{Item, Stack};
use crate::physics::Controls;
use glam::Vec3;

impl Play {
    /// A blow at the mob under the crosshair, when it is nearer than the block there. Returns whether the button went to a mob.
    pub(super) fn attack(&mut self, input: &Input) -> bool {
        self.attack_wait = (self.attack_wait - TICK).max(0.0);
        if !input.mine {
            self.attack_wait = 0.0;
            return false;
        }
        let eye = self.body.pos + Vec3::Y * self.body.eye_height();
        let dir = crate::tool::look_dir(input.yaw, input.pitch);
        let block_d = self.target.map_or(f32::MAX, |t| t.distance);
        let reach = self.reach().min(block_d);
        let mut best: Option<(usize, f32)> = None;
        for (i, e) in self.entities.iter().enumerate() {
            if !e.kind.mob() || !e.alive() {
                continue;
            }
            if let Some(d) = e.ray(eye, dir, reach) {
                if best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((i, d));
                }
            }
        }
        let Some((i, _)) = best else { return false };
        if self.attack_wait > 0.0 {
            return true;
        }
        self.attack_wait = 0.5;
        self.swing = 1.0;
        let held = self.inv.held_item();
        let damage = held.and_then(Item::tool).map_or(1.0, |t| t.damage());
        let sprint = self.body.sprinting;
        let e = &mut self.entities[i];
        e.health -= damage;
        e.hurt = 1.0;
        let push = glam::Vec2::new(dir.x, dir.z).normalize_or_zero() * if sprint { 9.0 } else { 6.0 };
        e.body.vel += Vec3::new(push.x, 4.5, push.y);
        let kind = e.kind;
        if e.health <= 0.0 {
            e.dying = 0.0;
            self.kills += 1;
        }
        // A creeper struck forgets its fuse for a moment.
        if kind == crate::entities::Kind::Creeper {
            e.fuse = -1.0;
        }
        self.events.push(Event::MobHurt(kind));
        if let Some(t) = held.and_then(Item::tool) {
            if !self.creative() {
                let s = &mut self.inv.slots[self.inv.selected];
                s.wear += if t.kind == crate::item::ToolKind::Sword { 1 } else { 2 };
                if s.wear >= t.durability() {
                    *s = Stack::EMPTY;
                }
            }
        }
        true
    }

    /// Every entity's tick: items fall and fly to the player, mobs think and walk, fuses burn; and new mobs appear around the player.
    pub(super) fn entities_tick(&mut self) {
        let player = (self.dead.is_none() && !self.creative()).then_some(self.body.pos);
        let me = self.body.pos + Vec3::Y * 0.9;
        let mut strikes = 0u32;
        let mut booms: Vec<Vec3> = Vec::new();
        let mut drops: Vec<(Stack, Vec3)> = Vec::new();
        let mut events: Vec<Event> = Vec::new();
        let mut picked = false;
        let mut dice: Vec<f32> = (0..self.entities.len() * 2).map(|_| self.roll()).collect();
        let world = &self.terrain.world;
        let inv = &mut self.inv;
        let daylight = self.daylight;
        self.entities.retain_mut(|e| {
            let roll = dice.pop().unwrap_or(0.5);
            let roll2 = dice.pop().unwrap_or(0.5);
            e.age += TICK;
            e.hurt = (e.hurt - TICK * 3.0).max(0.0);
            let from_me = me - (e.body.pos + Vec3::Y * 0.2);
            let dist = from_me.length();
            // A chunk that is not loaded (the player went far away): an item waits, a mob is let go.
            if !world.ready(e.body.pos.x.floor() as i32, e.body.pos.z.floor() as i32) {
                return !e.kind.mob();
            }
            match e.kind {
                Kind::Item(stack) => {
                    if e.age > e.pickup_after && dist < 1.9 {
                        if dist < 0.75 {
                            let left = inv.add(stack);
                            if left.is_empty() {
                                picked = true;
                                return false;
                            }
                            e.kind = Kind::Item(left);
                        } else {
                            e.body.vel = from_me.normalize_or_zero() * 7.0;
                        }
                    }
                    e.body.step(world, &Controls::default(), TICK);
                    if e.body.on_ground {
                        e.body.vel.x *= 0.82;
                        e.body.vel.z *= 0.82;
                    }
                    e.age < 300.0
                }
                Kind::Tnt => {
                    e.body.step(world, &Controls::default(), TICK);
                    e.body.vel.x *= 0.9;
                    e.body.vel.z *= 0.9;
                    e.fuse -= TICK;
                    if e.fuse <= 0.0 {
                        booms.push(e.body.pos + Vec3::Y * 0.5);
                        return false;
                    }
                    true
                }
                _ => {
                    if e.dying >= 0.0 {
                        e.dying += TICK;
                        if e.dying > 0.8 {
                            for s in e.drops(roll) {
                                drops.push((s, e.body.pos + Vec3::Y * 0.5));
                            }
                            return false;
                        }
                        e.body.step(world, &Controls::default(), TICK);
                        return true;
                    }
                    let mind = think(e, world, player, roll, TICK);
                    e.body.step(world, &mind.controls, TICK);
                    e.walk += glam::Vec2::new(e.body.vel.x, e.body.vel.z).length() * TICK;
                    if mind.strike {
                        strikes += 1;
                    }
                    if mind.hiss {
                        events.push(Event::Fuse);
                    }
                    if e.kind == Kind::Creeper && e.fuse >= 1.5 {
                        booms.push(e.body.pos + Vec3::Y * 0.6);
                        return false;
                    }
                    // A zombie under the open sky by day burns.
                    let head = e.body.pos + Vec3::Y * 1.6;
                    let open_sky = world.light(head.x.floor() as i32, head.y.floor() as i32, head.z.floor() as i32).0 == 15;
                    e.burning = e.kind == Kind::Zombie && daylight > 0.55 && open_sky && !e.body.in_water;
                    if e.burning {
                        e.health -= TICK * 1.5;
                    }
                    if e.body.in_lava {
                        e.health -= TICK * 8.0;
                    }
                    if e.health <= 0.0 {
                        e.dying = 0.0;
                    }
                    e.noise_in -= TICK;
                    if e.noise_in <= 0.0 {
                        e.noise_in = 6.0 + roll2 * 10.0;
                        if dist < 16.0 {
                            events.push(if e.kind == Kind::Pig { Event::Oink } else { Event::Groan });
                        }
                    }
                    // Far away, they are forgotten (the night's crowd thins as the player moves on).
                    let far = if e.kind.hostile() { 72.0 } else { 120.0 };
                    dist < far && e.body.pos.y > -10.0
                }
            }
        });
        if picked {
            self.events.push(Event::Pop);
        }
        self.events.extend(events);
        for _ in 0..strikes {
            self.hurt(3.0, "You were slain by a zombie");
            self.body.vel += Vec3::Y * 4.0;
        }
        for (s, at) in drops {
            let (a, b) = (self.roll() - 0.5, self.roll() - 0.5);
            self.drop_item(s, at, Vec3::new(a * 2.0, 3.0, b * 2.0), 0.4);
        }
        for at in booms {
            self.explode(at, 3.2);
        }
        self.spawn_mobs();
    }

    /// Now and then, a mob where one may be: pigs on lit grass, zombies and creepers in the dark (at night, or in caves).
    fn spawn_mobs(&mut self) {
        self.spawn_in -= TICK;
        if self.spawn_in > 0.0 {
            return;
        }
        self.spawn_in = 1.0;
        let pigs = self.entities.iter().filter(|e| e.kind == Kind::Pig).count();
        let hostiles = self.entities.iter().filter(|e| e.kind.hostile()).count();
        let me = self.body.pos;
        let angle = self.roll() * std::f32::consts::TAU;
        let r = 24.0 + self.roll() * 30.0;
        let (x, z) = ((me.x + angle.cos() * r).floor() as i32, (me.z + angle.sin() * r).floor() as i32);
        if !self.terrain.world.ready(x, z) {
            return;
        }
        if pigs < 6 && self.roll() < 0.35 {
            if let Some(at) = crate::entities::spawn_spot(&self.terrain.world, Kind::Pig, x, z, None, self.daylight) {
                let n = 1 + (self.roll() * 3.0) as usize;
                for k in 0..n {
                    self.spawn(Kind::Pig, at + Vec3::new(k as f32 * 0.7, 0.0, 0.0));
                }
                return;
            }
        }
        if hostiles >= 10 {
            return;
        }
        let kind = if self.roll() < 0.68 { Kind::Zombie } else { Kind::Creeper };
        // On the surface at night, or down in the dark.
        let cave_y = (5 + (self.roll() * (me.y + 12.0 - 5.0).max(1.0)) as i32).min(crate::chunk::H - 3);
        let at = crate::entities::spawn_spot(&self.terrain.world, kind, x, z, None, self.daylight)
            .or_else(|| (0..12).find_map(|k| crate::entities::spawn_spot(&self.terrain.world, kind, x, z, Some(cave_y - k), self.daylight)));
        if let Some(at) = at {
            if (at - me).length() > 22.0 {
                self.spawn(kind, at);
            }
        }
    }
}
