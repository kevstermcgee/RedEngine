//! The world generator: a pure function from the seed and a chunk's position to its blocks. Neighbouring chunks agree at their borders because
//! everything is a function of world coordinates: the ground height and biome of a column, the caves (3D noise sampled on a world-aligned grid
//! and interpolated), and the trees (one candidate per world-aligned 5 x 5 cell, built into every chunk its leaves reach).
//!
//! The land: oceans and beaches where the continent noise is low, plains, forests, birch woods, taiga, snowy plains and deserts by temperature and
//! rain, ridged mountains where the erosion noise rises, rivers winding where a river noise crosses zero. Under it stone with dirt and gravel
//! pockets, ores by depth, tunnels and caverns, lava at the bottom of the deepest caves, and bedrock.

use crate::block::Block;
use crate::chunk::{ChunkData, ChunkPos, H, W};
use crate::noise::{hash3, rand2, Noise};

/// The sea's surface: water fills every open cell up to it.
pub const SEA: i32 = 62;

/// Where a column is.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Biome {
    Ocean,
    FrozenOcean,
    Beach,
    River,
    Plains,
    Forest,
    BirchForest,
    Taiga,
    SnowyPlains,
    Desert,
    Mountains,
}

impl Biome {
    pub fn name(self) -> &'static str {
        match self {
            Biome::Ocean => "Ocean",
            Biome::FrozenOcean => "Frozen Ocean",
            Biome::Beach => "Beach",
            Biome::River => "River",
            Biome::Plains => "Plains",
            Biome::Forest => "Forest",
            Biome::BirchForest => "Birch Forest",
            Biome::Taiga => "Taiga",
            Biome::SnowyPlains => "Snowy Plains",
            Biome::Desert => "Desert",
            Biome::Mountains => "Mountains",
        }
    }

    /// Its colour on the map.
    pub fn map_color(self) -> [u8; 3] {
        match self {
            Biome::Ocean => [38, 74, 160],
            Biome::FrozenOcean => [130, 160, 210],
            Biome::Beach => [222, 206, 150],
            Biome::River => [60, 110, 200],
            Biome::Plains => [120, 178, 80],
            Biome::Forest => [52, 120, 48],
            Biome::BirchForest => [96, 152, 70],
            Biome::Taiga => [56, 96, 80],
            Biome::SnowyPlains => [236, 240, 245],
            Biome::Desert => [226, 196, 120],
            Biome::Mountains => [128, 128, 128],
        }
    }

    fn snowy(self) -> bool {
        matches!(self, Biome::SnowyPlains | Biome::FrozenOcean)
    }
}

/// One column of ground, before caves.
#[derive(Clone, Copy, Debug)]
pub struct Column {
    /// The y of the top solid block.
    pub height: i32,
    pub biome: Biome,
    /// The temperature: about -1 (the coldest tenth) .. 1 (the hottest tenth); snow lies below -1.05.
    pub temp: f32,
    /// How thick the soil is over the stone.
    pub soil: i32,
}

/// What a tree is made of.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TreeKind {
    Oak,
    Birch,
    Spruce,
}

/// The generator for one world.
pub struct Generator {
    pub seed: u32,
    continent: Noise,
    erosion: Noise,
    hills: Noise,
    ridges: Noise,
    detail: Noise,
    temp: Noise,
    rain: Noise,
    river: Noise,
    cave_a: Noise,
    cave_b: Noise,
    cavern: Noise,
    patches: Noise,
}

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// The cave grid's spacing across (x and z) and up (y), blocks.
const CAVE_STEP: i32 = 4;
const CAVE_GX: usize = (W / CAVE_STEP + 1) as usize;
const CAVE_GY: usize = (H / CAVE_STEP + 1) as usize;

impl Generator {
    pub fn new(seed: u32) -> Generator {
        let n = |k: u32| Noise::new(seed.wrapping_mul(0x9e37_79b9).wrapping_add(k.wrapping_mul(0x632b_e5ab)));
        Generator {
            seed,
            continent: n(1),
            erosion: n(2),
            hills: n(3),
            ridges: n(4),
            detail: n(5),
            temp: n(6),
            rain: n(7),
            river: n(8),
            cave_a: n(9),
            cave_b: n(10),
            cavern: n(11),
            patches: n(12),
        }
    }

    /// The ground at column (x, z): its height and biome.
    pub fn column(&self, x: i32, z: i32) -> Column {
        let (fx, fz) = (x as f64, z as f64);
        // Each field divided by its measured 90th percentile (about 0.2), so 1 means "high for this field" (`calibrate` prints them).
        let c = self.continent.fbm2(fx, fz, 1100.0, 4) * 7.0;
        let t = self.temp.fbm2(fx + 9000.0, fz - 4000.0, 1300.0, 3) * 6.4;
        let r = self.rain.fbm2(fx - 7000.0, fz + 3000.0, 1000.0, 3) * 6.4;
        // The ground's base: sea floor, coast, rising gently inland.
        let ocean_floor = lerp(SEA as f64 - 26.0, SEA as f64 - 6.0, smoothstep(-1.5, -0.65, c));
        let inland = SEA as f64 + 3.0 + smoothstep(-0.3, 1.3, c) * 10.0;
        let base = lerp(ocean_floor, inland, smoothstep(-0.66, -0.4, c));
        let land = smoothstep(-0.48, -0.3, c);
        // Rolling hills, rounder in the plains, livelier in the woods.
        let hill_amp = 4.0 + 7.0 * smoothstep(-0.3, 1.0, r);
        let hills = self.hills.fbm2(fx, fz, 160.0, 4) * hill_amp * land;
        let detail = self.detail.fbm2(fx, fz, 36.0, 2) * 1.6;
        // Mountains: ridged noise where the erosion field is high, inland only.
        let e = self.erosion.fbm2(fx + 1500.0, fz + 800.0, 800.0, 3) * 7.0;
        let mf = smoothstep(0.7, 1.55, e) * smoothstep(-0.3, 0.1, c);
        let ridge = 1.0 - self.ridges.fbm2(fx, fz, 240.0, 5).abs() * 3.0;
        let mountain = mf * (14.0 + 50.0 * ridge.clamp(0.0, 1.0).powf(1.4));
        let mut h = base + hills + detail + mountain;
        // Rivers: a valley where the river field crosses zero, cut down to just under the sea.
        let rv = self.river.fbm2(fx - 300.0, fz + 1200.0, 900.0, 3).abs();
        let width = 0.0065 * (1.0 - mf);
        let mut river = false;
        if land > 0.5 && width > 0.001 && rv < width * 2.6 {
            let k = smoothstep(width * 0.6, width * 2.6, rv);
            let bed = SEA as f64 - 3.0;
            if h > bed {
                h = lerp(bed, h, k);
            }
            river = rv < width && h < SEA as f64 + 0.5;
        }
        let height = (h.round() as i32).clamp(4, H - 4);
        let biome = if height < SEA - 1 && !river && c < -0.4 {
            if t < -1.15 {
                Biome::FrozenOcean
            } else {
                Biome::Ocean
            }
        } else if river {
            Biome::River
        } else if height <= SEA + 1 && c < -0.3 {
            Biome::Beach
        } else if mf > 0.45 && height > SEA + 24 {
            Biome::Mountains
        } else if t > 0.75 && r < 0.1 {
            Biome::Desert
        } else if t < -1.05 {
            Biome::SnowyPlains
        } else if t < -0.55 {
            Biome::Taiga
        } else if r > 0.6 {
            Biome::Forest
        } else if r > 0.2 {
            if t > 0.35 {
                Biome::BirchForest
            } else {
                Biome::Forest
            }
        } else {
            Biome::Plains
        };
        let soil = 3 + (rand2(self.seed ^ 0x51, x, z) * 2.0) as i32;
        Column { height, biome, temp: t as f32, soil }
    }

    /// The cave field at a grid point (positive: rock; negative: open).
    fn cave_sample(&self, x: i32, y: i32, z: i32) -> f32 {
        let (fx, fy, fz) = (x as f64, y as f64, z as f64);
        // Tunnels: where two noise fields both cross zero, a winding line.
        let a = self.cave_a.at3(fx / 52.0, fy / 30.0, fz / 52.0);
        let b = self.cave_b.at3(fx / 52.0, fy / 30.0, fz / 52.0);
        let tunnel = (a * a + b * b) - 0.0085;
        // Caverns: deep pockets where a third field is high.
        let cv = self.cavern.at3(fx / 80.0, fy / 36.0, fz / 80.0) + self.cavern.at3(fx / 23.0, fy / 17.0, fz / 23.0) * 0.18;
        let deep = smoothstep(52.0, 16.0, fy);
        let cavern = (0.44 - cv) / 4.0 + (1.0 - deep) * 0.2;
        tunnel.min(cavern) as f32
    }

    /// The cave field anywhere: the grid samples around it, interpolated (what [`Generator::generate`] carves with).
    pub fn cave_at(&self, x: i32, y: i32, z: i32) -> f32 {
        let (gx, gy, gz) = (x.div_euclid(CAVE_STEP), y.div_euclid(CAVE_STEP), z.div_euclid(CAVE_STEP));
        let s = CAVE_STEP as f32;
        let (tx, ty, tz) = (x.rem_euclid(CAVE_STEP) as f32 / s, y.rem_euclid(CAVE_STEP) as f32 / s, z.rem_euclid(CAVE_STEP) as f32 / s);
        let v = |i: i32, j: i32, k: i32| self.cave_sample((gx + i) * CAVE_STEP, (gy + j) * CAVE_STEP, (gz + k) * CAVE_STEP);
        trilerp([v(0, 0, 0), v(1, 0, 0), v(0, 1, 0), v(1, 1, 0), v(0, 0, 1), v(1, 0, 1), v(0, 1, 1), v(1, 1, 1)], tx, ty, tz)
    }

    /// The block at depth `d` below the top of a column (0 is the top block).
    fn layer(&self, col: &Column, x: i32, y: i32, z: i32, d: i32, slope: i32) -> Block {
        let wet = col.height < SEA;
        match col.biome {
            Biome::Desert => match d {
                0..=3 => Block::SAND,
                4..=6 => Block::SANDSTONE,
                _ => Block::STONE,
            },
            Biome::Beach => {
                if d < 3 {
                    Block::SAND
                } else if d < 5 {
                    Block::SANDSTONE
                } else {
                    Block::STONE
                }
            }
            Biome::Ocean | Biome::FrozenOcean | Biome::River => {
                if d < 3 {
                    let p = self.patches.at2(x as f64 / 14.0, z as f64 / 14.0);
                    if p > 0.32 {
                        Block::GRAVEL
                    } else if p < -0.38 && col.biome != Biome::River {
                        Block::CLAY
                    } else if col.biome == Biome::River && p < -0.1 {
                        Block::GRAVEL
                    } else {
                        Block::SAND
                    }
                } else {
                    Block::STONE
                }
            }
            Biome::Mountains => {
                let snowline = 98 + (rand2(self.seed ^ 0x77, x, z) * 6.0) as i32;
                if d == 0 && y >= snowline {
                    Block::SNOW
                } else if slope >= 3 || (y > snowline - 8 && d < 2) {
                    if d == 0 && slope < 3 && y < snowline {
                        Block::GRASS
                    } else {
                        Block::STONE
                    }
                } else if d == 0 {
                    if col.temp < -0.6 {
                        Block::SNOWY_GRASS
                    } else {
                        Block::GRASS
                    }
                } else if d < col.soil - 1 {
                    Block::DIRT
                } else {
                    Block::STONE
                }
            }
            _ => {
                if d == 0 {
                    if wet {
                        Block::DIRT
                    } else if col.biome.snowy() {
                        Block::SNOWY_GRASS
                    } else {
                        Block::GRASS
                    }
                } else if d <= col.soil {
                    Block::DIRT
                } else {
                    Block::STONE
                }
            }
        }
    }

    /// The chunk at `pos`, as the world first made it.
    pub fn generate(&self, pos: ChunkPos) -> ChunkData {
        const B: i32 = 3;
        const N: i32 = W + 2 * B;
        let (ox, oz) = pos.origin();
        let cols: Vec<Column> = (0..N * N).map(|i| self.column(ox - B + i % N, oz - B + i / N)).collect();
        let col = |lx: i32, lz: i32| &cols[((lz + B) * N + (lx + B)) as usize];
        let mut c = ChunkData::default();

        // The cave field on its grid, once for the chunk.
        let mut grid = vec![0f32; CAVE_GX * CAVE_GY * CAVE_GX];
        for k in 0..CAVE_GX {
            for j in 0..CAVE_GY {
                for i in 0..CAVE_GX {
                    grid[(k * CAVE_GY + j) * CAVE_GX + i] = self.cave_sample(ox + i as i32 * CAVE_STEP, j as i32 * CAVE_STEP, oz + k as i32 * CAVE_STEP);
                }
            }
        }
        let cave = |lx: i32, y: i32, lz: i32| {
            let (i, j, k) = ((lx / CAVE_STEP) as usize, (y / CAVE_STEP) as usize, (lz / CAVE_STEP) as usize);
            let s = CAVE_STEP as f32;
            let g = |a: usize, b: usize, d: usize| grid[((k + d) * CAVE_GY + (j + b)) * CAVE_GX + (i + a)];
            trilerp(
                [g(0, 0, 0), g(1, 0, 0), g(0, 1, 0), g(1, 1, 0), g(0, 0, 1), g(1, 0, 1), g(0, 1, 1), g(1, 1, 1)],
                (lx % CAVE_STEP) as f32 / s,
                (y % CAVE_STEP) as f32 / s,
                (lz % CAVE_STEP) as f32 / s,
            )
        };

        for lz in 0..W {
            for lx in 0..W {
                let co = *col(lx, lz);
                let (x, z) = (ox + lx, oz + lz);
                let h = co.height;
                let slope = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().map(|(dx, dz)| (col(lx + dx, lz + dz).height - h).abs()).max().unwrap_or(0);
                for y in 0..=h.max(SEA) {
                    let b = if y == 0 || (y <= 4 && (hash3(self.seed ^ 0xbed, x, y, z) % 5) as i32 >= y) {
                        Block::BEDROCK
                    } else if y <= h {
                        self.layer(&co, x, y, z, h - y, slope)
                    } else if y == SEA && (co.biome == Biome::FrozenOcean || (co.temp < -1.05 && co.biome == Biome::River)) {
                        Block::ICE
                    } else {
                        Block::WATER
                    };
                    c.set(lx, y, lz, b);
                }
                // Caves: never just under water (the sea would hang over them), never through the floor of the world.
                let top = if h < SEA + 2 { h - 6 } else { h };
                for y in 5..=top.min(H - 2) {
                    if cave(lx, y, lz) < 0.0 {
                        c.set(lx, y, lz, if y <= 10 { Block::LAVA } else { Block::AIR });
                    }
                }
                // A cave that opened the top of the column leaves no grass floating over it; the dirt under an opened top grows grass.
                if c.get(lx, h, lz).is_air() && h > SEA {
                    let mut y = h - 1;
                    while y > 5 && c.get(lx, y, lz).is_air() {
                        y -= 1;
                    }
                    if c.get(lx, y, lz) == Block::DIRT && !co.biome.snowy() {
                        c.set(lx, y, lz, Block::GRASS);
                    }
                }
            }
        }
        self.ores(pos, &mut c);
        self.trees(pos, &mut c, &cols, B, N);
        self.plants(pos, &mut c, &cols, B, N);
        c
    }

    fn ores(&self, pos: ChunkPos, c: &mut ChunkData) {
        let mut rng = Rng::new(hash3(self.seed ^ 0x0e5, pos.x, 77, pos.z));
        // (ore, veins, lowest y, highest y, size)
        let kinds: [(Block, u32, i32, i32, u32); 6] = [
            (Block::DIRT, 7, 8, 100, 24),
            (Block::GRAVEL, 5, 8, 100, 22),
            (Block::COAL_ORE, 20, 6, 120, 12),
            (Block::IRON_ORE, 16, 6, 64, 8),
            (Block::GOLD_ORE, 3, 6, 32, 7),
            (Block::DIAMOND_ORE, 2, 6, 16, 6),
        ];
        for (ore, veins, lo, hi, size) in kinds {
            for _ in 0..veins {
                if ore == Block::DIAMOND_ORE && rng.roll().is_multiple_of(3) {
                    continue;
                }
                let (mut x, mut y, mut z) = ((rng.roll() % 16) as i32, lo + (rng.roll() % (hi - lo) as u32) as i32, (rng.roll() % 16) as i32);
                let n = size / 2 + rng.roll() % (size / 2 + 1);
                for _ in 0..n {
                    if c.get(x, y, z) == Block::STONE {
                        c.set(x, y, z, ore);
                    }
                    match rng.roll() % 6 {
                        0 => x = (x + 1).min(15),
                        1 => x = (x - 1).max(0),
                        2 => y = (y + 1).min(hi),
                        3 => y = (y - 1).max(lo),
                        4 => z = (z + 1).min(15),
                        _ => z = (z - 1).max(0),
                    }
                }
            }
        }
    }

    /// The tree whose 5 x 5 cell is (gx, gz), if the cell has one: its trunk's column, kind and trunk height.
    pub fn tree_in_cell(&self, gx: i32, gz: i32) -> Option<(i32, i32, TreeKind, i32)> {
        let h = hash3(self.seed ^ 0x7ee, gx, 0, gz);
        let (x, z) = (gx * 5 + (h % 5) as i32, gz * 5 + ((h >> 3) % 5) as i32);
        let roll = ((h >> 8) & 1023) as f32 / 1024.0;
        let col = self.column(x, z);
        let (chance, kind) = match col.biome {
            Biome::Forest => (0.8, if (h >> 20).is_multiple_of(6) { TreeKind::Birch } else { TreeKind::Oak }),
            Biome::BirchForest => (0.7, if (h >> 20).is_multiple_of(8) { TreeKind::Oak } else { TreeKind::Birch }),
            Biome::Taiga => (0.62, TreeKind::Spruce),
            Biome::SnowyPlains => (0.07, TreeKind::Spruce),
            Biome::Mountains => (0.09, TreeKind::Spruce),
            Biome::Plains => (0.035, TreeKind::Oak),
            _ => return None,
        };
        if roll >= chance || col.height < SEA + 1 {
            return None;
        }
        // Mountain trees grow only where the ground is grass: below the snow, off the cliffs.
        if col.biome == Biome::Mountains {
            let steep = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|(dx, dz)| (self.column(x + dx, z + dz).height - col.height).abs() >= 3);
            if steep || col.height > 88 {
                return None;
            }
        }
        // Not on a cave mouth, and not on a cliff.
        if self.cave_at(x, col.height, z) < 0.0 {
            return None;
        }
        let trunk = match kind {
            TreeKind::Oak => 4 + ((h >> 24) % 3) as i32,
            TreeKind::Birch => 5 + ((h >> 24) % 3) as i32,
            TreeKind::Spruce => 6 + ((h >> 24) % 4) as i32,
        };
        Some((x, z, kind, trunk))
    }

    fn trees(&self, pos: ChunkPos, c: &mut ChunkData, cols: &[Column], b: i32, n: i32) {
        let (ox, oz) = pos.origin();
        let reach = 3;
        let (g0x, g1x) = ((ox - reach).div_euclid(5), (ox + W + reach).div_euclid(5));
        let (g0z, g1z) = ((oz - reach).div_euclid(5), (oz + W + reach).div_euclid(5));
        for gz in g0z..=g1z {
            for gx in g0x..=g1x {
                let Some((x, z, kind, trunk)) = self.tree_in_cell(gx, gz) else { continue };
                let (lx, lz) = (x - ox, z - oz);
                if lx < -reach || lx >= W + reach || lz < -reach || lz >= W + reach {
                    continue;
                }
                let base = if (-b..W + b).contains(&lx) && (-b..W + b).contains(&lz) {
                    cols[((lz + b) * n + (lx + b)) as usize].height
                } else {
                    self.column(x, z).height
                };
                // Every chunk the tree reaches decides alike (from the noise, not from its own blocks), so a tree is never half built.
                if (0..W).contains(&lx) && (0..W).contains(&lz) {
                    c.set(lx, base, lz, Block::DIRT);
                }
                self.build_tree(c, lx, base + 1, lz, kind, trunk, hash3(self.seed ^ 0x1eaf, x, 0, z));
            }
        }
    }

    /// Writes a tree with its trunk's foot at local (x, y, z); cells outside the chunk are skipped (the neighbour builds its part).
    #[allow(clippy::too_many_arguments)]
    fn build_tree(&self, c: &mut ChunkData, x: i32, y: i32, z: i32, kind: TreeKind, trunk: i32, h: u32) {
        let (log, leaves) = match kind {
            TreeKind::Oak => (Block::OAK_LOG, Block::OAK_LEAVES),
            TreeKind::Birch => (Block::BIRCH_LOG, Block::BIRCH_LEAVES),
            TreeKind::Spruce => (Block::SPRUCE_LOG, Block::SPRUCE_LEAVES),
        };
        let inside = |x: i32, z: i32| (0..W).contains(&x) && (0..W).contains(&z);
        let leaf = |c: &mut ChunkData, x: i32, y: i32, z: i32| {
            if inside(x, z) && (0..H).contains(&y) && c.get(x, y, z).replaceable() && !c.get(x, y, z).is_liquid() {
                c.set(x, y, z, leaves);
            }
        };
        let top = y + trunk - 1;
        match kind {
            TreeKind::Oak | TreeKind::Birch => {
                for ly in top - 2..=top + 1 {
                    let r: i32 = if ly >= top { 1 } else { 2 };
                    for dz in -r..=r {
                        for dx in -r..=r {
                            let corner = dx.abs() == r && dz.abs() == r;
                            // Corners: missing at random below, always missing on the crown.
                            if corner && (ly == top + 1 || (r == 2 && (h >> ((dx + 2 + (dz + 2) * 5 + ly) & 31)) & 1 == 0) || (r == 1 && ly == top)) {
                                continue;
                            }
                            leaf(c, x + dx, ly, z + dz);
                        }
                    }
                }
            }
            TreeKind::Spruce => {
                leaf(c, x, top + 1, z);
                leaf(c, x, top + 2, z);
                let mut r: i32 = 0;
                let mut ly = top;
                let bottom = y + 2;
                let mut step = 0;
                while ly >= bottom {
                    for dz in -r..=r {
                        for dx in -r..=r {
                            if r > 0 && dx.abs() == r && dz.abs() == r {
                                continue;
                            }
                            leaf(c, x + dx, ly, z + dz);
                        }
                    }
                    step += 1;
                    r = match step % 3 {
                        0 => 1,
                        1 => (1 + step / 3).min(3),
                        _ => (step / 3).clamp(1, 2),
                    };
                    ly -= 1;
                }
            }
        }
        if inside(x, z) {
            for ly in y..=top {
                if (0..H).contains(&ly) {
                    c.set(x, ly, z, log);
                }
            }
        }
    }

    fn plants(&self, pos: ChunkPos, c: &mut ChunkData, cols: &[Column], b: i32, n: i32) {
        let (ox, oz) = pos.origin();
        for lz in 0..W {
            for lx in 0..W {
                let co = cols[((lz + b) * n + (lx + b)) as usize];
                let (x, z) = (ox + lx, oz + lz);
                let y = c.top(lx, lz);
                if y < 1 || y + 1 >= H {
                    continue;
                }
                let ground = c.get(lx, y, lz);
                let roll = rand2(self.seed ^ 0xf10, x, z);
                match (co.biome, ground) {
                    (Biome::Desert, Block::SAND) => {
                        if roll < 0.006 && self.clear_around(c, lx, y + 1, lz) {
                            let tall = 1 + (hash3(self.seed, x, 1, z) % 3) as i32;
                            for k in 1..=tall {
                                c.set(lx, y + k, lz, Block::CACTUS);
                            }
                        } else if roll < 0.016 {
                            c.set(lx, y + 1, lz, Block::DEAD_BUSH);
                        }
                    }
                    (_, Block::GRASS) => {
                        let (grass, flowers) = match co.biome {
                            Biome::Plains => (0.30, 0.04),
                            Biome::Forest => (0.16, 0.02),
                            Biome::BirchForest => (0.22, 0.05),
                            Biome::Taiga => (0.10, 0.004),
                            Biome::Mountains => (0.08, 0.004),
                            _ => (0.05, 0.004),
                        };
                        if roll < flowers {
                            // Flowers grow in patches of one kind.
                            let p = self.patches.at2(x as f64 / 40.0 + 100.0, z as f64 / 40.0);
                            let f = if p > 0.18 {
                                Block::POPPY
                            } else if p < -0.22 {
                                Block::CORNFLOWER
                            } else {
                                Block::DANDELION
                            };
                            c.set(lx, y + 1, lz, f);
                        } else if roll < flowers + grass {
                            c.set(lx, y + 1, lz, Block::TALL_GRASS);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn clear_around(&self, c: &ChunkData, x: i32, y: i32, z: i32) -> bool {
        [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().all(|(dx, dz)| {
            let (nx, nz) = (x + dx, z + dz);
            !(0..W).contains(&nx) || !(0..W).contains(&nz) || c.get(nx, y, nz).is_air()
        }) && x > 0
            && x < W - 1
            && z > 0
            && z < W - 1
    }

    /// Where a new player starts: the nearest dry land to the middle of the world (feet on the top block).
    pub fn spawn_point(&self) -> (i32, i32, i32) {
        for r in 0..200 {
            for k in 0..(8 * r).max(1) {
                let a = k as f64 / (8 * r).max(1) as f64 * std::f64::consts::TAU;
                let (x, z) = ((a.cos() * r as f64 * 8.0) as i32, (a.sin() * r as f64 * 8.0) as i32);
                let col = self.column(x, z);
                if col.height > SEA && matches!(col.biome, Biome::Plains | Biome::Forest | Biome::BirchForest | Biome::Taiga | Biome::SnowyPlains) {
                    let open = (col.height + 1..col.height + 3).all(|y| self.cave_at(x, y, z) >= 0.0 || y > col.height);
                    if open && self.cave_at(x, col.height, z) >= 0.0 && self.tree_near(x, z).is_none() {
                        return (x, col.height + 1, z);
                    }
                }
            }
        }
        (0, H - 10, 0)
    }

    fn tree_near(&self, x: i32, z: i32) -> Option<(i32, i32)> {
        for gz in (z - 3).div_euclid(5)..=(z + 3).div_euclid(5) {
            for gx in (x - 3).div_euclid(5)..=(x + 3).div_euclid(5) {
                if let Some((tx, tz, _, _)) = self.tree_in_cell(gx, gz) {
                    if (tx - x).abs() <= 2 && (tz - z).abs() <= 2 {
                        return Some((tx, tz));
                    }
                }
            }
        }
        None
    }
}

fn trilerp(v: [f32; 8], tx: f32, ty: f32, tz: f32) -> f32 {
    let l = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = l(v[0], v[1], tx);
    let x10 = l(v[2], v[3], tx);
    let x01 = l(v[4], v[5], tx);
    let x11 = l(v[6], v[7], tx);
    l(l(x00, x10, ty), l(x01, x11, ty), tz)
}

/// A small fast generator (xorshift) for a chunk's ore veins.
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Rng {
        Rng(seed | 1)
    }

    pub fn roll(&mut self) -> u32 {
        let mut s = self.0;
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        self.0 = s;
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn a_chunk_is_the_same_every_time_and_depends_on_the_seed() {
        let g = Generator::new(42);
        let a = g.generate(ChunkPos::new(3, -2));
        let b = g.generate(ChunkPos::new(3, -2));
        assert!(a.blocks == b.blocks);
        let other = Generator::new(43).generate(ChunkPos::new(3, -2));
        assert!(a.blocks != other.blocks);
    }

    #[test]
    fn bedrock_floors_the_world_and_nothing_floats_on_water() {
        let g = Generator::new(7);
        for (cx, cz) in [(0, 0), (5, 9), (-12, 4)] {
            let c = g.generate(ChunkPos::new(cx, cz));
            for z in 0..W {
                for x in 0..W {
                    assert_eq!(c.get(x, 0, z), Block::BEDROCK);
                    let top = c.top(x, z);
                    assert!(top >= SEA - 30, "a column {top} high");
                }
            }
        }
    }

    #[test]
    fn trees_cross_chunk_borders_whole() {
        // Find a tree whose leaves spill over a border, and check both chunks hold their halves.
        let g = Generator::new(11);
        let mut found = 0;
        'search: for gz in -40..40 {
            for gx in -40..40 {
                let Some((x, z, kind, trunk)) = g.tree_in_cell(gx, gz) else { continue };
                if kind == TreeKind::Spruce || x.rem_euclid(W) != 15 {
                    continue;
                }
                let here = g.generate(ChunkPos::of(x, z));
                let east = g.generate(ChunkPos::of(x + 1, z));
                let base = g.column(x, z).height;
                let (lx, lz) = (x.rem_euclid(W), z.rem_euclid(W));
                if here.get(lx, base + 1, lz) != Block::OAK_LOG && here.get(lx, base + 1, lz) != Block::BIRCH_LOG {
                    continue; // the ground changed under it
                }
                let y = base + trunk - 2;
                assert!(matches!(east.get(0, y, lz), Block::OAK_LEAVES | Block::BIRCH_LEAVES), "the leaves east of the trunk at {x},{z} are in the next chunk");
                found += 1;
                if found >= 3 {
                    break 'search;
                }
            }
        }
        assert!(found > 0, "no tree on a chunk border was found");
    }

    #[test]
    fn the_world_has_every_biome_ores_at_their_depths_and_caves() {
        let g = Generator::new(2026);
        let mut biomes: HashMap<Biome, u32> = HashMap::new();
        for z in (-6000..6000).step_by(97) {
            for x in (-6000..6000).step_by(89) {
                *biomes.entry(g.column(x, z).biome).or_default() += 1;
            }
        }
        for b in [
            Biome::Ocean,
            Biome::Beach,
            Biome::River,
            Biome::Plains,
            Biome::Forest,
            Biome::BirchForest,
            Biome::Taiga,
            Biome::SnowyPlains,
            Biome::Desert,
            Biome::Mountains,
        ] {
            assert!(biomes.get(&b).copied().unwrap_or(0) > 0, "no {b:?} in {biomes:?}");
        }
        let mut counts: HashMap<Block, u32> = HashMap::new();
        let mut deepest_diamond = 0;
        let mut air_below_40 = 0;
        for cz in 0..4 {
            for cx in 0..4 {
                let c = g.generate(ChunkPos::new(cx, cz));
                for y in 0..H {
                    for z in 0..W {
                        for x in 0..W {
                            let b = c.get(x, y, z);
                            *counts.entry(b).or_default() += 1;
                            if b == Block::DIAMOND_ORE {
                                deepest_diamond = deepest_diamond.max(y);
                            }
                            if y < 40 && b.is_air() {
                                air_below_40 += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(counts.get(&Block::COAL_ORE).copied().unwrap_or(0) > 200, "{counts:?}");
        assert!(counts.get(&Block::IRON_ORE).copied().unwrap_or(0) > 100, "{counts:?}");
        assert!(deepest_diamond <= 16);
        assert!(air_below_40 > 500, "caves: {air_below_40} open cells under y 40 in 16 chunks");
    }

    #[test]
    fn the_spawn_is_on_dry_open_land() {
        let g = Generator::new(5);
        let (x, y, z) = g.spawn_point();
        let c = g.generate(ChunkPos::of(x, z));
        let (lx, lz) = (x.rem_euclid(W), z.rem_euclid(W));
        assert!(c.get(lx, y - 1, lz).solid(), "standing on {:?}", c.get(lx, y - 1, lz));
        assert!(!c.get(lx, y, lz).solid() && !c.get(lx, y + 1, lz).solid());
        assert!(y > SEA);
    }
}

#[cfg(test)]
mod calibrate {
    use super::*;

    /// `cargo test -p redcraft calibrate -- --ignored --nocapture`: the spread of each field the biomes are cut from.
    #[test]
    #[ignore]
    fn percentiles() {
        let g = Generator::new(1);
        let mut f: [Vec<f64>; 5] = Default::default();
        for i in 0..40000 {
            let (x, z) = ((i % 200) as f64 * 61.0 - 6000.0, (i / 200) as f64 * 67.0 - 6700.0);
            f[0].push(g.continent.fbm2(x, z, 1100.0, 4) * 7.0);
            f[1].push(g.temp.fbm2(x + 9000.0, z - 4000.0, 1300.0, 3) * 6.4);
            f[2].push(g.rain.fbm2(x - 7000.0, z + 3000.0, 1000.0, 3) * 6.4);
            f[3].push(g.erosion.fbm2(x + 1500.0, z + 800.0, 800.0, 3) * 7.0);
            f[4].push(g.river.fbm2(x - 300.0, z + 1200.0, 900.0, 3).abs());
        }
        for (name, v) in ["continent", "temp", "rain", "erosion", "river"].iter().zip(f.iter_mut()) {
            v.sort_by(|a, b| a.total_cmp(b));
            let p = |q: f64| v[((v.len() - 1) as f64 * q) as usize];
            println!(
                "{name:10} p5 {:+.3} p10 {:+.3} p25 {:+.3} p50 {:+.3} p75 {:+.3} p90 {:+.3} p95 {:+.3}",
                p(0.05),
                p(0.1),
                p(0.25),
                p(0.5),
                p(0.75),
                p(0.9),
                p(0.95)
            );
        }
    }
}
