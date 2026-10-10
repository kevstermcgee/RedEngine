//! The loadout shooter's world as the client draws it: weapons lying on the floor, rockets and grenades in flight, smoke clouds, fires and
//! explosions (ADR 2026-09-30-killchain-loadout-shooter).
//!
//! The server sends an [`ArenaSnap`] with every snapshot (which map pickups are there, what was dropped, what flies, what burns, what just went
//! off). [`ShooterWorld`] turns it into scene objects: everything that can ever appear is added to the scene once, hidden ([`ShooterWorld::add_to_scene`], before
//! the renderer is built), and [`ShooterWorld::update`] claims, moves and releases them each frame. Pure: no window, no GPU, tested on scenes.

use crate::firearms::{projectile_model, world_model};
use crate::net::protocol::{ArenaSnap, FxSnap, MAX_FX_SNAP, MAX_ZONE_SNAP};
use crate::scene_pool::{ScenePool, HIDDEN_SCALE};
use crate::schema::{Material, Object, ObjectKind, PrimKind, Scene};
use crate::sim::ordnance::FxKind;
use crate::track::Track;
use crate::weapons::Weapon;
use glam::{EulerRot, Quat, Vec3};
use std::collections::{HashMap, VecDeque};

/// Copies of each kind of dropped weapon that can lie on the floor at once.
pub const DROPPED_PER_KIND: usize = 2;
/// The weapons that fly.
pub const FLYERS: [Weapon; 9] =
    [Weapon::Lancer, Weapon::Thumper, Weapon::Frag, Weapon::Flash, Weapon::Smoke, Weapon::Incendiary, Weapon::Lobber, Weapon::Flare, Weapon::Impact];
/// Copies of each flying weapon alive at once.
pub const FLYERS_PER_KIND: usize = 4;
/// Explosions animating at once.
pub const BLASTS: usize = 6;
/// Spheres that make one smoke cloud.
pub const PUFFS_PER_CLOUD: usize = 7;
/// Cones that make one fire.
pub const FLAMES_PER_FIRE: usize = 6;
/// How long a fireball burns, seconds.
pub const FIREBALL_SECS: f32 = 0.5;
/// How long the smoke of an explosion hangs, seconds.
pub const DUST_SECS: f32 = 2.4;
/// How long a flash pop glows, seconds.
pub const FLASH_SECS: f32 = 0.22;

fn solid(id: String, kind: PrimKind, color: Vec3, emissive: Vec3, opacity: f32) -> Object {
    Object {
        id,
        position: Track::constant(Vec3::ZERO),
        rotation: Track::constant(Vec3::ZERO),
        scale: Track::constant(Vec3::splat(HIDDEN_SCALE)),
        material: Some(Material { color: Track::constant(color), metallic: 0.0, roughness: 0.9, emissive, opacity }),
        collide: false,
        prefab: None,
        movable: Some(false),
        kind: ObjectKind::Prim(kind),
    }
}

fn set(scene: &mut Scene, index: usize, pos: Vec3, rot_deg: Vec3, scale: Vec3) {
    if let Some(o) = scene.objects.get_mut(index) {
        o.position = Track::constant(pos);
        o.rotation = Track::constant(rot_deg);
        o.scale = Track::constant(scale);
    }
}

/// Euler degrees (the scene's order) for a group whose +Z points along `dir`.
pub fn heading_euler(dir: Vec3) -> Vec3 {
    let d = dir.normalize_or_zero();
    if d == Vec3::ZERO {
        return Vec3::ZERO;
    }
    let q = Quat::from_rotation_arc(Vec3::Z, d);
    let (x, y, z) = q.to_euler(EulerRot::XYZ);
    Vec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees())
}

#[derive(Debug, Clone, Copy)]
struct Blast {
    fire: usize,
    dust: usize,
    at: Vec3,
    size: f32,
    age: f32,
}

#[derive(Debug, Clone, Copy)]
struct FlashPop {
    slot: usize,
    at: Vec3,
    size: f32,
    age: f32,
}

#[derive(Debug, Clone, Copy)]
struct Flyer {
    kind: usize,
    slot: usize,
    pos: Vec3,
    vel: Vec3,
    since: f32,
}

/// The scene objects of the loadout world and what is on them.
pub struct ShooterWorld {
    pickup_first: usize,
    pickup_weapons: Vec<Option<Weapon>>,
    pickup_visible: Vec<bool>,
    dropped: Vec<ScenePool>,
    dropped_live: HashMap<u16, (usize, usize)>,
    flyer_pools: Vec<ScenePool>,
    flyer_live: HashMap<u16, Flyer>,
    fireballs: ScenePool,
    dust: ScenePool,
    blasts: Vec<Blast>,
    flash_pool: ScenePool,
    flashes: Vec<FlashPop>,
    smoke: ScenePool,
    smoke_live: HashMap<u16, Vec<usize>>,
    fire: ScenePool,
    fire_live: HashMap<u16, Vec<usize>>,
    seen_fx: VecDeque<u16>,
    time: f32,
    /// Flags, the bomb and the bomb sites (maps that have them).
    objective: Option<crate::objective_world::ObjectiveWorld>,
}

impl ShooterWorld {
    /// Adds every object the loadout world can need to `scene` (hidden), if the scene has a `shooter` block. Call it before the renderer is built.
    pub fn add_to_scene(scene: &mut Scene) -> Option<ShooterWorld> {
        let cfg = scene.shooter.clone()?;
        let pickup_first = scene.objects.len();
        for (i, p) in cfg.pickups.iter().enumerate() {
            let mut o = world_model(p.weapon, &format!("pickup_{i}"));
            o.position = Track::constant(p.at);
            scene.objects.push(o);
        }
        let dropped =
            Weapon::ROSTER.iter().map(|w| ScenePool::add(scene, DROPPED_PER_KIND, |k| world_model(Some(*w), &format!("drop_{}_{k}", w.wire())))).collect();
        let flyer_pools = FLYERS.iter().map(|w| ScenePool::add(scene, FLYERS_PER_KIND, |k| projectile_model(*w, &format!("fly_{}_{k}", w.wire())))).collect();
        let fireballs = ScenePool::add(scene, BLASTS, |k| {
            solid(format!("fx_fireball_{k}"), PrimKind::Sphere { radius: 1.0 }, Vec3::new(1.0, 0.55, 0.15), Vec3::new(5.0, 2.4, 0.6), 1.0)
        });
        let dust =
            ScenePool::add(scene, BLASTS, |k| solid(format!("fx_dust_{k}"), PrimKind::Sphere { radius: 1.0 }, Vec3::new(0.06, 0.06, 0.06), Vec3::ZERO, 0.55));
        let flash_pool =
            ScenePool::add(scene, 4, |k| solid(format!("fx_flash_{k}"), PrimKind::Sphere { radius: 1.0 }, Vec3::ONE, Vec3::new(6.0, 6.0, 6.0), 1.0));
        let smoke = ScenePool::add(scene, MAX_ZONE_SNAP * PUFFS_PER_CLOUD, |k| {
            solid(format!("fx_smoke_{k}"), PrimKind::Sphere { radius: 1.0 }, Vec3::new(0.42, 0.43, 0.44), Vec3::ZERO, 0.94)
        });
        let fire = ScenePool::add(scene, MAX_ZONE_SNAP * FLAMES_PER_FIRE, |k| {
            solid(format!("fx_flame_{k}"), PrimKind::Cone { radius: 0.5, height: 1.0 }, Vec3::new(1.0, 0.4, 0.05), Vec3::new(3.0, 1.2, 0.2), 0.85)
        });
        let objective = crate::objective_world::ObjectiveWorld::add_to_scene(scene);
        Some(ShooterWorld {
            pickup_first,
            pickup_weapons: cfg.pickups.iter().map(|p| p.weapon).collect(),
            pickup_visible: vec![true; cfg.pickups.len()],
            dropped,
            dropped_live: HashMap::new(),
            flyer_pools,
            flyer_live: HashMap::new(),
            fireballs,
            dust,
            blasts: Vec::new(),
            flash_pool,
            flashes: Vec::new(),
            smoke,
            smoke_live: HashMap::new(),
            fire,
            fire_live: HashMap::new(),
            seen_fx: VecDeque::new(),
            time: 0.0,
            objective,
        })
    }

    /// Ids of the objects that should not be drawn right now (unclaimed pool slots and pickup spots whose item is gone).
    pub fn hidden_ids<'a>(&'a self, scene: &'a Scene) -> Vec<&'a str> {
        let mut out: Vec<&str> = Vec::new();
        for (i, visible) in self.pickup_visible.iter().enumerate() {
            if !visible {
                if let Some(o) = scene.objects.get(self.pickup_first + i) {
                    out.push(o.id.as_str());
                }
            }
        }
        for pool in self.dropped.iter().chain(self.flyer_pools.iter()) {
            out.extend(pool.hidden_ids(scene));
        }
        for pool in [&self.fireballs, &self.dust, &self.flash_pool, &self.smoke, &self.fire] {
            out.extend(pool.hidden_ids(scene));
        }
        if let Some(o) = &self.objective {
            out.extend(o.hidden_ids(scene));
        }
        out
    }

    /// How many pickup spots the map has.
    pub fn pickup_count(&self) -> usize {
        self.pickup_weapons.len()
    }

    /// How many dropped weapons are drawn right now.
    pub fn dropped_drawn(&self) -> usize {
        self.dropped_live.len()
    }

    /// How many projectiles are drawn right now.
    pub fn flyers_drawn(&self) -> usize {
        self.flyer_live.len()
    }

    /// Moves everything to match `arena` and advances the animations by `dt`; returns the explosions and pops that are new since the last call (to play).
    pub fn update(&mut self, scene: &mut Scene, arena: Option<&ArenaSnap>, dt: f32) -> Vec<FxSnap> {
        self.time += dt;
        let t = self.time;
        let mut fresh = Vec::new();
        // Map pickups: present or not, turning slowly and bobbing a little.
        for i in 0..self.pickup_weapons.len() {
            let here = arena.is_none_or(|a| a.pickups[i / 8 % a.pickups.len()] & (1 << (i % 8)) != 0);
            self.pickup_visible[i] = here;
            if let Some(o) = scene.objects.get_mut(self.pickup_first + i) {
                if here {
                    let base = match o.position.sample(0.0) {
                        p if p.is_finite() => p,
                        _ => Vec3::ZERO,
                    };
                    o.rotation = Track::constant(Vec3::new(0.0, (t * 40.0 + i as f32 * 53.0) % 360.0, 0.0));
                    o.scale = Track::constant(Vec3::ONE);
                    o.position = Track::constant(base);
                } else {
                    o.scale = Track::constant(Vec3::splat(HIDDEN_SCALE));
                }
            }
        }
        if let Some(o) = self.objective.as_mut() {
            o.update(scene, arena.map(|a| &a.obj), dt);
        }
        let Some(arena) = arena else {
            self.clear(scene);
            return fresh;
        };
        // Dropped weapons.
        let ids: Vec<u16> = arena.dropped.iter().map(|d| d.id).collect();
        let gone: Vec<u16> = self.dropped_live.keys().copied().filter(|k| !ids.contains(k)).collect();
        for id in gone {
            if let Some((kind, slot)) = self.dropped_live.remove(&id) {
                self.dropped[kind].release(scene, slot);
            }
        }
        for d in &arena.dropped {
            let kind = d.weapon as usize;
            if !self.dropped_live.contains_key(&d.id) {
                if let Some(pool) = self.dropped.get_mut(kind) {
                    if let Some(slot) = pool.claim() {
                        self.dropped_live.insert(d.id, (kind, slot));
                    }
                }
            }
            if let Some(&(kind, slot)) = self.dropped_live.get(&d.id) {
                let index = self.dropped[kind].object_index(slot);
                set(scene, index, Vec3::from(d.pos) - Vec3::Y * 0.22, Vec3::new(0.0, (d.id as f32 * 77.0 + t * 30.0) % 360.0, 0.0), Vec3::ONE);
            }
        }
        // Projectiles.
        let pids: Vec<u16> = arena.projectiles.iter().map(|p| p.id).collect();
        let gone: Vec<u16> = self.flyer_live.keys().copied().filter(|k| !pids.contains(k)).collect();
        for id in gone {
            if let Some(f) = self.flyer_live.remove(&id) {
                self.flyer_pools[f.kind].release(scene, f.slot);
            }
        }
        for p in &arena.projectiles {
            let weapon = Weapon::from_wire(p.weapon);
            let Some(kind) = FLYERS.iter().position(|w| *w == weapon) else { continue };
            let (pos, vel) = (Vec3::from(p.pos), Vec3::from(p.vel));
            let entry = match self.flyer_live.get_mut(&p.id) {
                Some(f) => {
                    if (f.pos - pos).length_squared() > 1e-6 {
                        f.pos = pos;
                        f.vel = vel;
                        f.since = 0.0;
                    } else {
                        f.since += dt;
                    }
                    Some(*f)
                }
                None => match self.flyer_pools[kind].claim() {
                    Some(slot) => {
                        let f = Flyer { kind, slot, pos, vel, since: 0.0 };
                        self.flyer_live.insert(p.id, f);
                        Some(f)
                    }
                    None => None,
                },
            };
            if let Some(f) = entry {
                let shown = f.pos + f.vel * f.since.min(0.12);
                let index = self.flyer_pools[kind].object_index(f.slot);
                // A grenade tumbles; a rocket points where it is going.
                let euler = if matches!(weapon, Weapon::Lancer | Weapon::Thumper) && f.vel.length() > 0.5 {
                    heading_euler(f.vel)
                } else {
                    Vec3::new((t * 420.0 + p.id as f32 * 40.0) % 360.0, (t * 170.0) % 360.0, 0.0)
                };
                set(scene, index, shown, euler, Vec3::ONE);
            }
        }
        // New explosions and pops.
        for f in &arena.fx {
            if self.seen_fx.contains(&f.id) {
                continue;
            }
            self.seen_fx.push_back(f.id);
            while self.seen_fx.len() > MAX_FX_SNAP * 6 {
                self.seen_fx.pop_front();
            }
            let at = Vec3::from(f.pos);
            let size = f.size_dm as f32 / 10.0;
            match FxKind::from_wire(f.kind) {
                Some(FxKind::Blast) => {
                    if let (Some(fire), Some(dust)) = (self.fireballs.claim(), self.dust.claim()) {
                        self.blasts.push(Blast { fire, dust, at, size, age: 0.0 });
                    }
                }
                Some(FxKind::FlashPop) => {
                    if let Some(slot) = self.flash_pool.claim() {
                        self.flashes.push(FlashPop { slot, at, size, age: 0.0 });
                    }
                }
                _ => {}
            }
            fresh.push(*f);
        }
        let mut i = 0;
        while i < self.blasts.len() {
            let b = &mut self.blasts[i];
            b.age += dt;
            let (fi, di) = (self.fireballs.object_index(b.fire), self.dust.object_index(b.dust));
            if b.age >= DUST_SECS {
                let (fire, dust) = (b.fire, b.dust);
                self.fireballs.release(scene, fire);
                self.dust.release(scene, dust);
                self.blasts.swap_remove(i);
                continue;
            }
            let k = (b.age / FIREBALL_SECS).clamp(0.0, 1.0);
            let ease = 1.0 - (1.0 - k) * (1.0 - k);
            let fire_scale = if b.age < FIREBALL_SECS { (0.3 + 0.55 * ease) * b.size * (1.0 - 0.6 * k * k) } else { HIDDEN_SCALE };
            let lift = b.age * 0.6;
            let dust_k = (b.age / DUST_SECS).clamp(0.0, 1.0);
            let dust_scale = (0.35 + 0.45 * (1.0 - (1.0 - dust_k).powi(2))) * b.size * if dust_k > 0.8 { (1.0 - dust_k) * 5.0 } else { 1.0 };
            let (at, fire_done) = (b.at, b.age >= FIREBALL_SECS);
            if fire_done {
                if let Some(o) = scene.objects.get_mut(fi) {
                    o.scale = Track::constant(Vec3::splat(HIDDEN_SCALE));
                }
            } else {
                set(scene, fi, at + Vec3::Y * 0.4, Vec3::ZERO, Vec3::splat(fire_scale));
            }
            set(scene, di, at + Vec3::Y * (0.8 + lift), Vec3::ZERO, Vec3::splat(dust_scale.max(HIDDEN_SCALE)));
            i += 1;
        }
        let mut i = 0;
        while i < self.flashes.len() {
            let f = &mut self.flashes[i];
            f.age += dt;
            let index = self.flash_pool.object_index(f.slot);
            if f.age >= FLASH_SECS {
                let slot = f.slot;
                self.flash_pool.release(scene, slot);
                self.flashes.swap_remove(i);
                continue;
            }
            let k = f.age / FLASH_SECS;
            set(scene, index, f.at + Vec3::Y * 0.2, Vec3::ZERO, Vec3::splat((0.3 + 0.9 * k) * (1.0 - k).max(0.05) * f.size.min(6.0) * 0.4));
            i += 1;
        }
        // Smoke clouds and fires.
        let zids: Vec<u16> = arena.zones.iter().map(|z| z.id).collect();
        let gone: Vec<u16> = self.smoke_live.keys().copied().filter(|k| !zids.contains(k)).collect();
        for id in gone {
            for slot in self.smoke_live.remove(&id).unwrap_or_default() {
                self.smoke.release(scene, slot);
            }
        }
        let gone: Vec<u16> = self.fire_live.keys().copied().filter(|k| !zids.contains(k)).collect();
        for id in gone {
            for slot in self.fire_live.remove(&id).unwrap_or_default() {
                self.fire.release(scene, slot);
            }
        }
        for z in &arena.zones {
            let (center, radius) = (Vec3::from(z.pos), z.radius_dm as f32 / 10.0);
            if z.kind == 0 {
                if !self.smoke_live.contains_key(&z.id) {
                    let slots: Vec<usize> = (0..PUFFS_PER_CLOUD).filter_map(|_| self.smoke.claim()).collect();
                    self.smoke_live.insert(z.id, slots);
                }
                let slots = self.smoke_live.get(&z.id).cloned().unwrap_or_default();
                // The cloud grows for the first second and thins away in the last.
                let grow = ((z.left_ticks as f32 / 60.0 - 0.0).min(1e9), 0.0).1;
                let _ = grow;
                let life_left = z.left_ticks as f32 / 60.0;
                let fade = (life_left / 2.5).clamp(0.0, 1.0);
                for (k, slot) in slots.iter().enumerate() {
                    let a = k as f32 * 2.399 + z.id as f32;
                    let ring = if k == 0 { 0.0 } else { radius * 0.45 };
                    let p = center + Vec3::new(a.cos() * ring, radius * 0.45 + (k % 3) as f32 * radius * 0.12, a.sin() * ring);
                    let wobble = 1.0 + 0.04 * ((t * 0.8 + k as f32).sin());
                    let index = self.smoke.object_index(*slot);
                    set(scene, index, p, Vec3::ZERO, Vec3::splat(radius * 0.62 * wobble * fade.max(0.001)));
                }
            } else {
                if !self.fire_live.contains_key(&z.id) {
                    let slots: Vec<usize> = (0..FLAMES_PER_FIRE).filter_map(|_| self.fire.claim()).collect();
                    self.fire_live.insert(z.id, slots);
                }
                let slots = self.fire_live.get(&z.id).cloned().unwrap_or_default();
                let fade = (z.left_ticks as f32 / 60.0 / 1.0).clamp(0.0, 1.0);
                for (k, slot) in slots.iter().enumerate() {
                    let a = k as f32 * 1.047 + z.id as f32;
                    let ring = if k == 0 { 0.0 } else { radius * 0.6 };
                    let flicker = 0.75 + 0.25 * (t * (9.0 + k as f32 * 2.1) + k as f32).sin();
                    let p = center + Vec3::new(a.cos() * ring, 0.05, a.sin() * ring);
                    let index = self.fire.object_index(*slot);
                    set(scene, index, p + Vec3::Y * 0.5 * flicker * fade, Vec3::ZERO, Vec3::new(radius * 0.55, 1.1 * flicker * fade.max(0.001), radius * 0.55));
                }
            }
        }
        fresh
    }

    /// Releases everything (the match ended or the connection was lost).
    pub fn clear(&mut self, scene: &mut Scene) {
        for (_, (kind, slot)) in self.dropped_live.drain() {
            self.dropped[kind].release(scene, slot);
        }
        for (_, f) in self.flyer_live.drain() {
            self.flyer_pools[f.kind].release(scene, f.slot);
        }
        for b in self.blasts.drain(..) {
            self.fireballs.release(scene, b.fire);
            self.dust.release(scene, b.dust);
        }
        for f in self.flashes.drain(..) {
            self.flash_pool.release(scene, f.slot);
        }
        for (_, slots) in self.smoke_live.drain() {
            for s in slots {
                self.smoke.release(scene, s);
            }
        }
        for (_, slots) in self.fire_live.drain() {
            for s in slots {
                self.fire.release(scene, s);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::protocol::{DroppedSnap, FxSnap, ProjSnap, ZoneSnap, PICKUP_MASK_BYTES};

    fn scene() -> Scene {
        crate::schema::parse_scene(
            r##"{"camera":{"position":[0,1.7,0],"target":[0,1.7,-10]},
            "shooter":{"pickups":[{"weapon":"rifle","at":[3,0.3,0]},{"weapon":"frag","at":[5,0.3,0]},{"ammo":true,"at":[7,0.3,0]}]},
            "objects":[{"id":"floor","type":"plane","size":[40,40]}]}"##,
        )
        .unwrap_or_else(|e| panic!("{e:?}"))
    }

    fn arena() -> ArenaSnap {
        let mut pickups = [0u8; PICKUP_MASK_BYTES];
        pickups[0] = 0b101;
        ArenaSnap { pickups, ..Default::default() }
    }

    #[test]
    fn a_scene_without_the_block_has_no_shooter_world() {
        let mut plain = crate::schema::parse_scene(r##"{"camera":{"position":[0,1.7,0],"target":[0,1.7,-10]},"objects":[]}"##).unwrap();
        assert!(ShooterWorld::add_to_scene(&mut plain).is_none());
    }

    #[test]
    fn pickup_spots_show_only_while_their_item_is_there() {
        let mut s = scene();
        let mut w = ShooterWorld::add_to_scene(&mut s).unwrap();
        assert_eq!(w.pickup_count(), 3);
        w.update(&mut s, Some(&arena()), 0.016);
        let hidden = w.hidden_ids(&s);
        assert!(
            !hidden.contains(&"pickup_0") && hidden.contains(&"pickup_1") && !hidden.contains(&"pickup_2"),
            "the frag at spot 1 was taken: {:?}",
            &hidden[..3.min(hidden.len())]
        );
        let pos = s.objects.iter().find(|o| o.id == "pickup_0").unwrap().position.sample(0.0);
        assert_eq!(pos, Vec3::new(3.0, 0.3, 0.0), "it stays where the map put it");
    }

    #[test]
    fn dropped_weapons_projectiles_and_blasts_come_and_go() {
        let mut s = scene();
        let mut w = ShooterWorld::add_to_scene(&mut s).unwrap();
        let mut a = arena();
        a.dropped.push(DroppedSnap { id: 7, weapon: Weapon::Rifle.wire(), pos: [10.0, 0.3, 1.0] });
        a.projectiles.push(ProjSnap { id: 9, weapon: Weapon::Lancer.wire(), pos: [0.0, 1.5, -3.0], vel: [0.0, 0.0, -40.0] });
        a.fx.push(FxSnap { id: 11, kind: FxKind::Blast.to_wire(), pos: [0.0, 1.0, -30.0], size_dm: 55 });
        let fresh = w.update(&mut s, Some(&a), 0.016);
        assert_eq!(fresh.len(), 1, "the explosion is reported once");
        assert_eq!((w.dropped_drawn(), w.flyers_drawn()), (1, 1));
        assert!(w.update(&mut s, Some(&a), 0.016).is_empty(), "and not again");
        // The rocket moves on its velocity between snapshots.
        let before = s.objects.iter().find(|o| o.id.starts_with("fly_") && o.scale.sample(0.0).x > 0.5).unwrap().position.sample(0.0);
        for _ in 0..5 {
            w.update(&mut s, Some(&a), 0.016);
        }
        let after = s.objects.iter().find(|o| o.id.starts_with("fly_") && o.scale.sample(0.0).x > 0.5).unwrap().position.sample(0.0);
        assert!(after.z < before.z - 0.5, "{before:?} -> {after:?}");
        // Gone from the snapshot: released.
        a.dropped.clear();
        a.projectiles.clear();
        w.update(&mut s, Some(&a), 0.016);
        assert_eq!((w.dropped_drawn(), w.flyers_drawn()), (0, 0));
        // The explosion plays out and frees its objects.
        for _ in 0..200 {
            w.update(&mut s, Some(&a), 0.016);
        }
        assert!(w.blasts.is_empty(), "the blast animation ended");
    }

    #[test]
    fn smoke_and_fire_claim_a_cloud_of_objects_and_give_them_back() {
        let mut s = scene();
        let mut w = ShooterWorld::add_to_scene(&mut s).unwrap();
        let mut a = arena();
        a.zones.push(ZoneSnap { id: 1, kind: 0, pos: [0.0, 0.0, -8.0], radius_dm: 46, left_ticks: 600 });
        a.zones.push(ZoneSnap { id: 2, kind: 1, pos: [4.0, 0.0, -8.0], radius_dm: 36, left_ticks: 300 });
        w.update(&mut s, Some(&a), 0.016);
        assert_eq!((w.smoke_live.get(&1).map(Vec::len), w.fire_live.get(&2).map(Vec::len)), (Some(PUFFS_PER_CLOUD), Some(FLAMES_PER_FIRE)));
        a.zones.clear();
        w.update(&mut s, Some(&a), 0.016);
        assert!(w.smoke_live.is_empty() && w.fire_live.is_empty());
        let hidden = w.hidden_ids(&s);
        assert!(hidden.iter().any(|id| id.starts_with("fx_smoke_")) && hidden.iter().any(|id| id.starts_with("fx_flame_")), "all free again");
    }

    #[test]
    fn a_heading_points_the_groups_forward_axis_along_the_velocity() {
        for dir in [Vec3::new(0.0, 0.0, -1.0), Vec3::new(1.0, 0.3, 0.2), Vec3::new(-0.4, -0.7, 0.5)] {
            let e = heading_euler(dir);
            let q = Quat::from_euler(EulerRot::XYZ, e.x.to_radians(), e.y.to_radians(), e.z.to_radians());
            assert!(((q * Vec3::Z) - dir.normalize()).length() < 1e-3, "{dir:?}");
        }
    }
}
