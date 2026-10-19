//! Water on the move: a changed cell stirs the water around it ([`Play::stir_water`]), and four times a second the stirred water flows
//! ([`Play::flow_water`]): down into any opening, out sideways over solid ground up to seven cells from its source (each cell a level weaker,
//! drawn lower), and back when what fed it is gone. Sources never move or multiply; flowing water is ordinary blocks, saved with its chunk.

use super::{Play, TICK};
use crate::block::Block;
use glam::IVec3;

impl Play {
    pub(crate) fn stir_water(&mut self, p: IVec3) {
        if self.water_queue.len() > 4096 {
            return;
        }
        for d in [IVec3::ZERO, IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z] {
            self.water_queue.push_back(p + d);
        }
    }

    /// Water moves four times a second: down into any opening, out sideways over solid ground up to seven cells from its source, and back
    /// when what fed it is gone.
    pub(super) fn flow_water(&mut self) {
        self.water_in -= TICK;
        if self.water_in > 0.0 || self.water_queue.is_empty() {
            return;
        }
        self.water_in = 0.25;
        let world = &self.terrain.world;
        let n = self.water_queue.len().min(512);
        let mut seen = std::collections::HashSet::new();
        let mut sets: Vec<(IVec3, Block)> = Vec::new();
        let mut next: Vec<IVec3> = Vec::new();
        let sides = [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z];
        for _ in 0..n {
            let Some(c) = self.water_queue.pop_front() else { break };
            if !seen.insert(c) || !world.ready(c.x, c.z) {
                continue;
            }
            let Some(level) = world.block_at(c).water_level() else { continue };
            if level > 0 {
                let fed_above = world.block_at(c + IVec3::Y).is_water();
                let fed_side = sides.iter().any(|d| world.block_at(c + *d).water_level().is_some_and(|l| l < level));
                if !fed_above && !fed_side {
                    sets.push((c, Block::AIR));
                    next.extend(sides.iter().map(|d| c + *d));
                    next.push(c - IVec3::Y);
                    continue;
                }
                // Water that is fed from above falls at full strength.
                if fed_above && level != 1 {
                    sets.push((c, Block::flowing(1)));
                    next.push(c);
                    continue;
                }
            }
            let below = c - IVec3::Y;
            let under = world.block_at(below);
            if below.y >= 0 && under.replaceable() && !under.is_liquid() {
                sets.push((below, Block::flowing(1)));
                next.push(below);
            } else if (!under.replaceable() || under.water_level() == Some(0) || under == Block::LAVA) && level < 7 {
                for d in sides {
                    let n = c + d;
                    let nb = world.block_at(n);
                    let farther = nb.water_level().is_some_and(|l| l > level + 1);
                    if (nb.replaceable() && !nb.is_liquid()) || farther {
                        sets.push((n, Block::flowing(level + 1)));
                        next.push(n);
                    }
                }
            }
        }
        if !sets.is_empty() {
            // One change per cell: the first wins (a cell two ways from water takes the nearer).
            let mut once = std::collections::HashSet::new();
            sets.retain(|(p, _)| once.insert(*p));
            self.terrain.set_many(&sets);
        }
        for p in next {
            if self.water_queue.len() < 4096 {
                self.water_queue.push_back(p);
            }
        }
    }
}
