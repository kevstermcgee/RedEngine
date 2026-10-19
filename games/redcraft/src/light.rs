//! Light, as the original works it out: sky light (15 straight down through open air, losing what leaves and water take) and block light (from
//! torches, lava, a lit furnace), each spreading to the six neighbours one level less per step (more through leaves and water), stopped by solid
//! blocks. Light reaches at most 15 cells from where it starts, which bounds every computation here:
//!
//! * a chunk's light is exact when worked out over it and its eight neighbours ([`Region`] 48 x 48 with nothing outside);
//! * after one block changes, only the cells within 15 columns of it can change: [`Region`] over that box (all heights), with the light already
//!   in the cells around the box held fixed, gives exactly the new light (the world calls it for every edit).

use crate::block::Block;
use crate::chunk::H;
use std::collections::VecDeque;

/// Light worked out over a box of columns: `sx` x `sz` columns from (x0, z0), all heights, inside a ring of cells whose light is given.
pub struct Region {
    pub x0: i32,
    pub z0: i32,
    pub sx: i32,
    pub sz: i32,
    blocks: Vec<Block>,
    sky: Vec<u8>,
    glow: Vec<u8>,
}

/// What a step into `b` costs (`None`: light does not enter).
#[inline]
fn step_cost(b: Block) -> Option<u8> {
    let lb = b.info().light_block;
    if lb >= 15 {
        None
    } else {
        Some(lb.max(1))
    }
}

impl Region {
    /// Light for the box: `block` gives every cell in it and in the ring around it, `ring` the light (sky, block) of the ring's cells, which stays
    /// as it is (zero for a box with nothing known around it).
    pub fn compute(x0: i32, z0: i32, sx: i32, sz: i32, block: impl Fn(i32, i32, i32) -> Block, ring: impl Fn(i32, i32, i32) -> (u8, u8)) -> Region {
        // Stored with the ring: local column (i, k) is world (x0 - 1 + i, z0 - 1 + k).
        let (wx, wz) = (sx + 2, sz + 2);
        let cells = (wx * wz * H) as usize;
        let mut r = Region { x0, z0, sx, sz, blocks: vec![Block::AIR; cells], sky: vec![0; cells], glow: vec![0; cells] };
        let at = |i: i32, y: i32, k: i32| ((y * wz + k) * wx + i) as usize;
        let ring_col = |i: i32, k: i32| i == 0 || k == 0 || i == wx - 1 || k == wz - 1;
        for k in 0..wz {
            for i in 0..wx {
                let (x, z) = (x0 - 1 + i, z0 - 1 + k);
                let edge = ring_col(i, k);
                for y in 0..H {
                    let n = at(i, y, k);
                    r.blocks[n] = block(x, y, z);
                    if edge {
                        let (s, g) = ring(x, y, z);
                        r.sky[n] = s;
                        r.glow[n] = g;
                    }
                }
            }
        }
        // Straight down from the sky, inside the box; `floor` is the lowest y of each column's full sky light.
        let mut floor = vec![H; (wx * wz) as usize];
        for k in 0..wz {
            for i in 0..wx {
                if ring_col(i, k) {
                    let mut f = H;
                    while f > 0 && r.sky[at(i, f - 1, k)] == 15 {
                        f -= 1;
                    }
                    floor[(k * wx + i) as usize] = f;
                    continue;
                }
                let mut level = 15u8;
                let mut f = H;
                for y in (0..H).rev() {
                    let n = at(i, y, k);
                    let lb = r.blocks[n].info().light_block;
                    level = if lb >= 15 { 0 } else { level.saturating_sub(lb) };
                    r.sky[n] = level;
                    if level == 15 {
                        f = y;
                    }
                }
                floor[(k * wx + i) as usize] = f;
            }
        }
        // Spread: from every cell that may light a darker neighbour (below the highest floor around it), and from the ring.
        let mut queue: VecDeque<u32> = VecDeque::new();
        for k in 0..wz {
            for i in 0..wx {
                let mut top = floor[(k * wx + i) as usize];
                for (di, dk) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (ni, nk) = (i + di, k + dk);
                    if (0..wx).contains(&ni) && (0..wz).contains(&nk) {
                        top = top.max(floor[(nk * wx + ni) as usize]);
                    }
                }
                for y in 0..(top + 1).min(H) {
                    let n = at(i, y, k);
                    if r.sky[n] >= 2 {
                        queue.push_back(n as u32);
                    }
                }
            }
        }
        r.spread(&mut queue, true, wx, wz);
        for k in 0..wz {
            for i in 0..wx {
                let edge = ring_col(i, k);
                for y in 0..H {
                    let n = at(i, y, k);
                    if edge {
                        if r.glow[n] >= 2 {
                            queue.push_back(n as u32);
                        }
                    } else {
                        let e = r.blocks[n].info().emit;
                        if e > 0 {
                            r.glow[n] = e;
                            queue.push_back(n as u32);
                        }
                    }
                }
            }
        }
        r.spread(&mut queue, false, wx, wz);
        r
    }

    fn spread(&mut self, queue: &mut VecDeque<u32>, sky: bool, wx: i32, wz: i32) {
        let plane = (wx * wz) as usize;
        while let Some(n) = queue.pop_front() {
            let n = n as usize;
            let level = if sky { self.sky[n] } else { self.glow[n] };
            if level < 2 {
                continue;
            }
            let i = (n % wx as usize) as i32;
            let k = ((n / wx as usize) % wz as usize) as i32;
            let y = (n / plane) as i32;
            let mut visit = |m: usize| {
                let Some(cost) = step_cost(self.blocks[m]) else { return };
                let new = level.saturating_sub(cost);
                let cur = if sky { &mut self.sky[m] } else { &mut self.glow[m] };
                if new > *cur {
                    *cur = new;
                    queue.push_back(m as u32);
                }
            };
            // Never into the ring: its light is given.
            for (di, dy, dk) in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
                let (ni, ny, nk) = (i + di, y + dy, k + dk);
                if ni < 1 || nk < 1 || ni > wx - 2 || nk > wz - 2 || !(0..H).contains(&ny) {
                    continue;
                }
                visit(((ny * wz + nk) * wx + ni) as usize);
            }
        }
    }

    /// The light at world (x, y, z) inside the box: sky in the high nibble, block in the low.
    pub fn light(&self, x: i32, y: i32, z: i32) -> u8 {
        let (i, k) = (x - self.x0 + 1, z - self.z0 + 1);
        let n = ((y * (self.sz + 2) + k) * (self.sx + 2) + i) as usize;
        (self.sky[n] << 4) | self.glow[n]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(blocks: impl Fn(i32, i32, i32) -> Block, x0: i32, z0: i32, n: i32) -> Region {
        Region::compute(x0, z0, n, n, blocks, |_, _, _| (0, 0))
    }

    #[test]
    fn open_sky_is_full_and_a_roof_casts_a_soft_shadow() {
        // Ground at y 10, a 9 x 9 roof at y 14 over (0..9, 0..9).
        let world = |x: i32, y: i32, z: i32| {
            if y <= 10 || (y == 14 && (0..9).contains(&x) && (0..9).contains(&z)) {
                Block::STONE
            } else {
                Block::AIR
            }
        };
        let r = lit(world, -8, -8, 26);
        assert_eq!(r.light(-6, 11, -6) >> 4, 15, "open ground");
        assert_eq!(r.light(4, 20, 4) >> 4, 15, "above the roof");
        assert_eq!(r.light(4, 14, 4) >> 4, 0, "inside the roof");
        let under_edge = r.light(0, 12, 4) >> 4;
        let under_middle = r.light(4, 12, 4) >> 4;
        assert_eq!(under_edge, 14, "one step in from the open side");
        assert_eq!(under_middle, 10, "five steps in");
        assert_eq!(r.light(4, 10, 4), 0, "inside the ground");
    }

    #[test]
    fn a_torch_lights_fourteen_and_fades_one_a_step_but_not_through_stone() {
        let world = |x: i32, y: i32, z: i32| {
            if !(1..=40).contains(&y) || x == 6 {
                Block::STONE
            } else if (x, y, z) == (0, 5, 0) {
                Block::TORCH
            } else {
                Block::AIR
            }
        };
        let r = lit(world, -10, -10, 30);
        assert_eq!(r.light(0, 5, 0) & 15, 14);
        assert_eq!(r.light(3, 5, 0) & 15, 11);
        assert_eq!(r.light(0, 5, 5) & 15, 9);
        assert_eq!(r.light(5, 5, 0) & 15, 9);
        assert_eq!(r.light(7, 5, 0) & 15, 0, "the wall at x 6 stops it ({} on the near side)", r.light(5, 5, 0) & 15);
        assert_eq!(r.light(0, 30, 0) >> 4, 0, "a sealed cave has no sky light");
    }

    #[test]
    fn leaves_and_water_take_some_light_and_the_ring_feeds_the_box() {
        let world = |_x: i32, y: i32, _z: i32| match y {
            0..=9 => Block::STONE,
            10..=13 => Block::WATER,
            20 => Block::OAK_LEAVES,
            _ => Block::AIR,
        };
        let r = lit(world, 0, 0, 4);
        assert_eq!(r.light(1, 21, 1) >> 4, 15);
        assert_eq!(r.light(1, 20, 1) >> 4, 14, "a leaf takes one");
        assert_eq!(r.light(1, 13, 1) >> 4, 12, "the top water cell takes two");
        assert_eq!(r.light(1, 10, 1) >> 4, 6);
        // A dark box next to a bright ring: light comes in from the ring.
        let sealed = |x: i32, y: i32, _z: i32| if y == 0 || y > 3 || x < 0 { Block::STONE } else { Block::AIR };
        let r = Region::compute(0, 0, 5, 5, sealed, |x, _y, _z| if x < 0 { (0, 0) } else { (0, 13) });
        assert_eq!(r.light(2, 2, 0) & 15, 12, "one step in from the ring");
        assert_eq!(r.light(2, 2, 2) & 15, 10);
    }
}
