//! The generated world as something to stand on: the ground height under any point and the tree trunks near the player.
//!
//! The simulation (the single-player game, the authoritative server and a client's prediction all share it) asks [`ProcgenGround`] for the floor
//! height and for trunks to walk around, so all of them agree on where the ground and every tree is, with no terrain ever stored or sent.
//! Trunks are looked up chunk by chunk and the last few chunks are remembered, because a player standing still asks every tick.

use super::world::{ChunkId, Config, Trunk, World};
use crate::collide::Collider2D;
use glam::Vec2;
use std::sync::{Arc, Mutex};

/// How many chunks of trunks are remembered.
const CACHE: usize = 12;
/// Trunks above the ground that count as blocking (the player's body band is 2 m).
const TRUNK_HEIGHT: f32 = 3.0;

/// A generated world as ground and obstacles.
pub struct ProcgenGround {
    world: World,
    trunks: Mutex<Vec<(ChunkId, Arc<Vec<Trunk>>)>>,
}

impl ProcgenGround {
    /// A ground for a world's settings.
    pub fn new(cfg: Config) -> ProcgenGround {
        ProcgenGround { world: World::new(cfg), trunks: Mutex::new(Vec::new()) }
    }

    /// The generator.
    pub fn world(&self) -> &World {
        &self.world
    }

    /// The ground height under a point.
    pub fn height(&self, xz: Vec2) -> f32 {
        self.world.height(xz.x as f64, xz.y as f64)
    }

    fn chunk_trunks(&self, id: ChunkId) -> Arc<Vec<Trunk>> {
        let mut cache = self.trunks.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = cache.iter().position(|(c, _)| *c == id) {
            let hit = cache.remove(i);
            let out = hit.1.clone();
            cache.push(hit);
            return out;
        }
        let made = Arc::new(self.world.trunks(id));
        if cache.len() >= CACHE {
            cache.remove(0);
        }
        cache.push((id, made.clone()));
        made
    }

    /// The trunks within `reach` metres of a point, as boxes the player's collision treats like any wall (`base` first, unchanged).
    pub fn colliders_near(&self, pos: Vec2, reach: f32, base: &[Collider2D]) -> Vec<Collider2D> {
        let mut out = Vec::with_capacity(base.len() + 8);
        out.extend_from_slice(base);
        let (lo, hi) = (ChunkId::at((pos.x - reach) as f64, (pos.y - reach) as f64), ChunkId::at((pos.x + reach) as f64, (pos.y + reach) as f64));
        for z in lo.z..=hi.z {
            for x in lo.x..=hi.x {
                for t in self.chunk_trunks(ChunkId { x, z }).iter() {
                    let (tx, tz) = (t.x as f32, t.z as f32);
                    if (tx - pos.x).abs() > reach + t.radius || (tz - pos.y).abs() > reach + t.radius {
                        continue;
                    }
                    let y = self.world.height(t.x, t.z);
                    out.push(Collider2D {
                        min: Vec2::new(tx - t.radius, tz - t.radius),
                        max: Vec2::new(tx + t.radius, tz + t.radius),
                        min_y: y - 1.0,
                        max_y: y + TRUNK_HEIGHT,
                    });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::{step_horizontal, PLAYER_RADIUS};

    fn ground() -> ProcgenGround {
        ProcgenGround::new(Config { seed: 7, ..Config::default() })
    }

    /// A trunk somewhere in a forest, with the ground height there.
    fn a_trunk(g: &ProcgenGround) -> Trunk {
        for cz in -20..20 {
            for cx in -20..20 {
                if let Some(t) = g.world.trunks(ChunkId { x: cx, z: cz }).into_iter().find(|t| t.radius > 0.2) {
                    return t;
                }
            }
        }
        panic!("no trees anywhere");
    }

    #[test]
    fn height_matches_the_generator() {
        let g = ground();
        assert_eq!(g.height(Vec2::new(12.5, -80.25)), g.world().height(12.5, -80.25));
    }

    #[test]
    fn a_player_cannot_walk_through_a_trunk_but_can_walk_round_it() {
        let g = ground();
        let t = a_trunk(&g);
        let start = Vec2::new(t.x as f32 - 3.0, t.z as f32);
        let foot_y = g.world().height(t.x - 3.0, t.z);
        let mut pos = start;
        for _ in 0..200 {
            let near = g.colliders_near(pos, 4.0, &[]);
            pos = step_horizontal(&near, pos, foot_y, Vec2::new(0.05, 0.0));
        }
        assert!(pos.x < t.x as f32 - t.radius + 0.01, "walked into the tree: {pos:?} vs trunk {t:?}");
        // Sliding along it gets past.
        let mut pos = Vec2::new(t.x as f32 - 1.5, t.z as f32 - 0.2);
        for _ in 0..400 {
            let near = g.colliders_near(pos, 4.0, &[]);
            pos = step_horizontal(&near, pos, foot_y, Vec2::new(0.03, 0.02));
        }
        assert!(pos.x > t.x as f32 + 0.5, "never got past: {pos:?}");
        assert!(PLAYER_RADIUS > 0.0);
    }

    #[test]
    fn only_nearby_trunks_come_back_and_the_base_list_is_kept() {
        let g = ground();
        let t = a_trunk(&g);
        let base = [Collider2D { min: Vec2::ZERO, max: Vec2::ONE, min_y: 0.0, max_y: 1.0 }];
        let near = g.colliders_near(Vec2::new(t.x as f32, t.z as f32), 3.0, &base);
        assert_eq!(near[0].max, Vec2::ONE);
        assert!(near.len() > 1);
        assert!(near.len() < 40, "{} colliders within 3 m", near.len());
        let far = g.colliders_near(Vec2::new(1.0e5, 1.0e5), 0.5, &[]);
        assert!(far.len() <= 2);
    }

    #[test]
    fn repeated_queries_are_cheap() {
        let g = ground();
        let t = a_trunk(&g);
        let p = Vec2::new(t.x as f32, t.z as f32);
        let _ = g.colliders_near(p, 4.0, &[]);
        let started = std::time::Instant::now();
        for _ in 0..2000 {
            std::hint::black_box(g.colliders_near(p, 4.0, &[]));
        }
        let per = started.elapsed().as_secs_f32() * 1000.0 / 2000.0;
        assert!(per < 0.2, "{per:.3} ms per tick");
    }
}
