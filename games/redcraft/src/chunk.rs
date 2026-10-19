//! A chunk: a 16 x 128 x 16 column of blocks with the light in each cell. Block (x, y, z) fills the metre cube from (x, y, z) to (x+1, y+1, z+1);
//! chunk (cx, cz) holds x in 16cx..16cx+16 and z in 16cz..16cz+16, every y from 0 (bedrock) to 127.

use crate::block::Block;

/// Blocks across a chunk.
pub const W: i32 = 16;
/// Blocks up a chunk (and the world).
pub const H: i32 = 128;
/// Cells in a chunk.
pub const CELLS: usize = (W * W * H) as usize;
/// Height of a section (the unit a chunk is meshed and drawn in).
pub const SECTION: i32 = 16;
/// Sections up a chunk.
pub const SECTIONS: usize = (H / SECTION) as usize;

/// Which chunk.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct ChunkPos {
    pub x: i32,
    pub z: i32,
}

impl ChunkPos {
    pub fn new(x: i32, z: i32) -> ChunkPos {
        ChunkPos { x, z }
    }

    /// The chunk holding block column (x, z).
    pub fn of(x: i32, z: i32) -> ChunkPos {
        ChunkPos { x: x.div_euclid(W), z: z.div_euclid(W) }
    }

    /// The world x and z of its first column.
    pub fn origin(self) -> (i32, i32) {
        (self.x * W, self.z * W)
    }

    pub fn offset(self, dx: i32, dz: i32) -> ChunkPos {
        ChunkPos { x: self.x + dx, z: self.z + dz }
    }

    /// Chebyshev distance in chunks.
    pub fn ring(self, other: ChunkPos) -> i32 {
        (self.x - other.x).abs().max((self.z - other.z).abs())
    }
}

/// The index of local cell (x, y, z) (each 0..16 / 0..128).
#[inline]
pub fn idx(x: i32, y: i32, z: i32) -> usize {
    ((y * W + z) * W + x) as usize
}

/// A chunk's cells.
#[derive(Clone)]
pub struct ChunkData {
    pub blocks: Box<[Block]>,
    /// Sky light in the high nibble, block light in the low one.
    pub light: Box<[u8]>,
    /// Whether the player changed it since it was generated (it is saved).
    pub edited: bool,
}

impl Default for ChunkData {
    fn default() -> Self {
        ChunkData { blocks: vec![Block::AIR; CELLS].into_boxed_slice(), light: vec![0u8; CELLS].into_boxed_slice(), edited: false }
    }
}

impl ChunkData {
    /// The block at local (x, y, z); air above the world, bedrock below it.
    #[inline]
    pub fn get(&self, x: i32, y: i32, z: i32) -> Block {
        if y >= H {
            return Block::AIR;
        }
        if y < 0 {
            return Block::BEDROCK;
        }
        self.blocks[idx(x, y, z)]
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, z: i32, b: Block) {
        if (0..H).contains(&y) {
            self.blocks[idx(x, y, z)] = b;
        }
    }

    /// Sky light at local (x, y, z) (15 above the world).
    #[inline]
    pub fn sky(&self, x: i32, y: i32, z: i32) -> u8 {
        if y >= H {
            return 15;
        }
        if y < 0 {
            return 0;
        }
        self.light[idx(x, y, z)] >> 4
    }

    /// Block light at local (x, y, z).
    #[inline]
    pub fn glow(&self, x: i32, y: i32, z: i32) -> u8 {
        if !(0..H).contains(&y) {
            return 0;
        }
        self.light[idx(x, y, z)] & 15
    }

    /// The highest y whose block is not air (or -1).
    pub fn top(&self, x: i32, z: i32) -> i32 {
        (0..H).rev().find(|&y| !self.get(x, y, z).is_air()).unwrap_or(-1)
    }

    /// Whether section `s` holds nothing but air.
    pub fn section_empty(&self, s: usize) -> bool {
        let lo = idx(0, s as i32 * SECTION, 0);
        let hi = idx(0, (s as i32 + 1) * SECTION, 0);
        self.blocks[lo..hi].iter().all(|b| b.is_air())
    }

    /// The blocks, run-length coded as (count, id) byte pairs (a save).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.blocks.len() {
            let b = self.blocks[i];
            let mut n = 1;
            while i + n < self.blocks.len() && self.blocks[i + n] == b && n < 255 {
                n += 1;
            }
            out.push(n as u8);
            out.push(b.0);
            i += n;
        }
        out
    }

    /// A chunk from [`ChunkData::encode`]'s bytes (`None` when they do not fill one exactly).
    pub fn decode(bytes: &[u8]) -> Option<ChunkData> {
        let mut c = ChunkData::default();
        let mut i = 0;
        for pair in bytes.chunks(2) {
            let [n, b] = pair else { return None };
            let n = *n as usize;
            if i + n > CELLS || n == 0 {
                return None;
            }
            c.blocks[i..i + n].fill(Block(*b));
            i += n;
        }
        (i == CELLS).then_some(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_round_down_into_their_chunk() {
        assert_eq!(ChunkPos::of(0, 0), ChunkPos::new(0, 0));
        assert_eq!(ChunkPos::of(15, 16), ChunkPos::new(0, 1));
        assert_eq!(ChunkPos::of(-1, -16), ChunkPos::new(-1, -1));
        assert_eq!(ChunkPos::of(-17, 5), ChunkPos::new(-2, 0));
        assert_eq!(ChunkPos::new(-2, 3).origin(), (-32, 48));
    }

    #[test]
    fn a_chunk_survives_its_save_coding() {
        let mut c = ChunkData::default();
        for y in 0..60 {
            for z in 0..W {
                for x in 0..W {
                    c.set(x, y, z, if (x + z + y) % 7 == 0 { Block::DIRT } else { Block::STONE });
                }
            }
        }
        let bytes = c.encode();
        let back = ChunkData::decode(&bytes).unwrap();
        assert!(back.blocks == c.blocks);
        assert!(bytes.len() < CELLS, "{} bytes", bytes.len());
        assert!(ChunkData::decode(&bytes[..bytes.len() - 2]).is_none());
        assert_eq!(c.top(3, 4), 59);
        assert!(c.section_empty(7) && !c.section_empty(0));
    }
}
