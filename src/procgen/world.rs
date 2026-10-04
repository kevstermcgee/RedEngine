//! The world generator: ground, climate, biomes and plant placement, all pure functions of `(seed, position)`.
//!
//! Nothing is stored. Ask for the height at `(x, z)` and you get the same answer, on any machine, whether you are at the start or a hundred
//! kilometres away; ask for the plants in a [`ChunkId`] and you get the same plants. That is what lets a world be infinite: the client builds
//! only the chunks near the player and forgets the far ones, and a server and its clients, never exchanging a byte of terrain, agree on every tree.
//!
//! **Ground** is a few layers of noise: slow rolling hills whose height itself varies from region to region (some country is flat as a table),
//! plus a gentle small-scale undulation. It is kept below a walkable slope.
//!
//! **Climate** is three slow fields ([`Climate`]): `cool`, `wet` and `wood`. From them come the biomes: open meadows, wildflower fields, groves,
//! broadleaf forest, conifer woods and sunny glades cut into the forest. Biomes are labels for the map and for things like which birds sing;
//! what actually grows is decided by densities and by each species' liking for the local climate ([`super::flora`]).
//!
//! **Placement** uses one jittered grid per layer (trees on 6 m cells, shrubs 4 m, flower patches 12 m, grass tufts 1 m), aligned so a cell
//! belongs to exactly one chunk and the content of a cell depends only on its own coordinates. Flower patches spill over chunk borders, so a
//! chunk also reads its neighbours' patch cells and keeps only the flowers that land inside it. Each plant has a `rank` in `[0, 1)`; keeping
//! only those below a threshold thins a layer evenly (and the thinned set always nests inside the fuller one), which is how far chunks stay cheap.

use super::flora::{self, Climate, Kind, SpeciesId};
use super::noise::{fbm, smooth, Rng};
use crate::strict::check_keys;
use serde_json::{Map, Value};

/// Side of a chunk in metres.
pub const CHUNK: f64 = 48.0;
/// Tree cell size in metres (eight to a chunk side).
const TREE_CELL: f64 = 6.0;
/// Shrub cell size in metres.
const SHRUB_CELL: f64 = 4.0;
/// Flower patch cell size in metres.
const PATCH_CELL: f64 = 12.0;
/// Grass tuft cell size in metres.
const GRASS_CELL: f64 = 1.0;
/// The radius around the start that stays an open, flowery meadow.
const SPAWN_CLEARING: f64 = 30.0;

/// `procgen` keys.
pub const PROCGEN_KEYS: &[&str] = &["seed", "relief", "trees", "flowers", "grass"];

/// The settings of a generated world.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// The seed: one number, one world.
    pub seed: u32,
    /// Multiplier on the height of the hills (0 is flat).
    pub relief: f32,
    /// Multiplier on tree density.
    pub trees: f32,
    /// Multiplier on how many flower patches there are and how full they are.
    pub flowers: f32,
    /// Multiplier on grass density.
    pub grass: f32,
}

impl Default for Config {
    fn default() -> Self {
        Config { seed: 1, relief: 1.0, trees: 1.0, flowers: 1.0, grass: 1.0 }
    }
}

/// Parses a `procgen` block: `{ "seed": 7, "relief": 1.0, "trees": 1.0, "flowers": 1.0, "grass": 1.0 }`.
pub fn parse_config(obj: &Map<String, Value>) -> Result<Config, Vec<String>> {
    let mut errors = Vec::new();
    check_keys(&mut errors, "procgen", obj, PROCGEN_KEYS);
    let mut cfg = Config::default();
    if let Some(v) = obj.get("seed") {
        match v.as_u64().filter(|s| *s <= u32::MAX as u64) {
            Some(s) => cfg.seed = s as u32,
            None => errors.push("procgen.seed: expected a whole number from 0 to 4294967295".into()),
        }
    }
    for (key, slot, max) in
        [("relief", &mut cfg.relief, 3.0), ("trees", &mut cfg.trees, 2.0), ("flowers", &mut cfg.flowers, 3.0), ("grass", &mut cfg.grass, 2.0)]
    {
        if let Some(v) = obj.get(key) {
            match v.as_f64().filter(|n| (0.0..=max).contains(n)) {
                Some(n) => *slot = n as f32,
                None => errors.push(format!("procgen.{key}: expected a number from 0 to {max}")),
            }
        }
    }
    if errors.is_empty() {
        Ok(cfg)
    } else {
        Err(errors)
    }
}

/// A chunk's coordinates: chunk `(0, 0)` covers `x, z` from 0 to [`CHUNK`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkId {
    /// Chunk column (east).
    pub x: i32,
    /// Chunk row (south).
    pub z: i32,
}

impl ChunkId {
    /// The chunk containing a world position.
    pub fn at(x: f64, z: f64) -> ChunkId {
        ChunkId { x: (x / CHUNK).floor() as i32, z: (z / CHUNK).floor() as i32 }
    }

    /// The world position of the chunk's minimum corner.
    pub fn origin(self) -> (f64, f64) {
        (self.x as f64 * CHUNK, self.z as f64 * CHUNK)
    }

    /// The world position of the chunk's centre.
    pub fn centre(self) -> (f64, f64) {
        let (x, z) = self.origin();
        (x + CHUNK / 2.0, z + CHUNK / 2.0)
    }
}

/// The kind of country at a spot (labels for maps, birds and mood; growth is decided by densities).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Biome {
    /// Open grass with scattered flowers.
    Meadow,
    /// An open field thick with flowers.
    Wildflowers,
    /// Open land with scattered trees.
    Grove,
    /// Broadleaf woodland.
    Forest,
    /// Conifer woodland.
    Pinewood,
    /// A sunny clearing in the woods.
    Glade,
}

impl Biome {
    /// A short name.
    pub fn name(self) -> &'static str {
        match self {
            Biome::Meadow => "meadow",
            Biome::Wildflowers => "wildflower field",
            Biome::Grove => "grove",
            Biome::Forest => "forest",
            Biome::Pinewood => "pinewood",
            Biome::Glade => "glade",
        }
    }
}

/// A tree or shrub trunk, for walking into.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trunk {
    /// World x.
    pub x: f64,
    /// World z.
    pub z: f64,
    /// Radius in metres.
    pub radius: f32,
}

/// One placed plant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plant {
    /// Which species.
    pub species: SpeciesId,
    /// World x.
    pub x: f64,
    /// World z.
    pub z: f64,
    /// Ground height under it.
    pub y: f32,
    /// Turn about the vertical axis, radians.
    pub yaw: f32,
    /// Height in metres.
    pub height: f32,
    /// Where along the species' palette its colour falls (0 to 1).
    pub tint: f32,
    /// Seeds the variation of its shape.
    pub seed: u32,
    /// Position in a random order (0 to 1): keep only plants below a threshold to thin a layer evenly.
    pub rank: f32,
}

/// The scene's `procgen` block, if it has one.
pub fn parse_procgen(root: &Map<String, Value>) -> Result<Option<Config>, Vec<String>> {
    let Some(raw) = root.get("procgen") else { return Ok(None) };
    match raw.as_object() {
        Some(o) => parse_config(o).map(Some),
        None => Err(vec!["procgen: must be an object like {\"seed\": 7}".to_string()]),
    }
}

/// A generated world.
#[derive(Debug, Clone)]
pub struct World {
    cfg: Config,
}

/// Salts that separate the noise fields (so hills and forests do not follow each other).
mod salt {
    pub const HILLS: u32 = 1;
    pub const HILLY: u32 = 2;
    pub const DETAIL: u32 = 3;
    pub const WOOD: u32 = 4;
    pub const COOL: u32 = 5;
    pub const WET: u32 = 6;
    pub const GLADE: u32 = 7;
    pub const BLOOM: u32 = 8;
    pub const CLUMP: u32 = 9;
    pub const STAND: u32 = 10;
    pub const GROUND: u32 = 11;
    pub const TREES: u32 = 100;
    pub const SHRUBS: u32 = 200;
    pub const PATCHES: u32 = 300;
    pub const GRASS: u32 = 400;
}

impl World {
    /// A world from its settings.
    pub fn new(cfg: Config) -> World {
        World { cfg }
    }

    /// The settings.
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    fn seed(&self, salt: u32) -> u32 {
        self.cfg.seed.wrapping_mul(0x9e37_79b1).wrapping_add(salt.wrapping_mul(0x85eb_ca6b))
    }

    /// A noise field remapped to 0..1 with a bit of contrast.
    fn field(&self, salt: u32, x: f64, z: f64, wavelength: f64, octaves: u32) -> f32 {
        smooth(-0.5, 0.5, fbm(self.seed(salt), x / wavelength, z / wavelength, octaves))
    }

    /// The ground height at a position, metres.
    pub fn height(&self, x: f64, z: f64) -> f32 {
        let hilly = 0.2 + 0.8 * smooth(-0.35, 0.45, fbm(self.seed(salt::HILLY), x / 700.0, z / 700.0, 2));
        let hills = fbm(self.seed(salt::HILLS), x / 240.0, z / 240.0, 3) * 15.0 * hilly;
        let detail = fbm(self.seed(salt::DETAIL), x / 23.0, z / 23.0, 2) * 0.45;
        (hills + detail) * self.cfg.relief
    }

    /// The upward unit normal of the ground at a position.
    pub fn normal(&self, x: f64, z: f64) -> [f32; 3] {
        let e = 0.75;
        let dx = self.height(x + e, z) - self.height(x - e, z);
        let dz = self.height(x, z + e) - self.height(x, z - e);
        let (nx, ny, nz) = (-dx, 2.0 * e as f32, -dz);
        let len = (nx * nx + ny * ny + nz * nz).sqrt();
        [nx / len, ny / len, nz / len]
    }

    /// The climate fields at a position.
    pub fn climate(&self, x: f64, z: f64) -> Climate {
        let wood = self.field(salt::WOOD, x, z, 380.0, 3);
        let cool = self.field(salt::COOL, x, z, 900.0, 2);
        let low = (-self.height(x, z) / 8.0).clamp(0.0, 1.0);
        let wet = (self.field(salt::WET, x, z, 300.0, 3) * 0.8 + low * 0.25).clamp(0.0, 1.0);
        Climate { cool, wet, wood }
    }

    /// How much of the sky a glade opens in the woods here (0 none, 1 a clearing).
    fn glade(&self, x: f64, z: f64) -> f32 {
        smooth(0.72, 0.86, self.field(salt::GLADE, x, z, 60.0, 2))
    }

    /// How thickly wildflowers want to grow here (0 to 1).
    fn bloom(&self, x: f64, z: f64) -> f32 {
        smooth(0.3, 0.8, self.field(salt::BLOOM, x, z, 150.0, 2))
    }

    /// The share of cells that grow a tree (0 to 1) before the density setting, for the climate and glade.
    fn forest_density(&self, c: &Climate, glade: f32) -> f32 {
        let forest = smooth(0.50, 0.64, c.wood) * 0.8;
        let grove = smooth(0.40, 0.50, c.wood) * 0.2;
        forest.max(grove) * (1.0 - glade * smooth(0.4, 0.7, c.wood))
    }

    fn near_start(x: f64, z: f64) -> f32 {
        let d = ((x * x + z * z).sqrt()) as f32;
        smooth(SPAWN_CLEARING as f32 * 0.55, SPAWN_CLEARING as f32 * 1.4, d)
    }

    /// The tree density at a position (0 to 1): the share of the 6 m cells there that hold a tree.
    pub fn tree_density(&self, x: f64, z: f64) -> f32 {
        let c = self.climate(x, z);
        self.forest_density(&c, self.glade(x, z)) * Self::near_start(x, z) * self.cfg.trees.min(1.2)
    }

    /// What kind of country this is.
    pub fn biome(&self, x: f64, z: f64) -> Biome {
        let c = self.climate(x, z);
        let glade = self.glade(x, z);
        let density = self.forest_density(&c, glade) * Self::near_start(x, z);
        if density > 0.45 {
            if c.cool > 0.58 {
                Biome::Pinewood
            } else {
                Biome::Forest
            }
        } else if c.wood > 0.58 && glade > 0.5 {
            Biome::Glade
        } else if density > 0.08 {
            Biome::Grove
        } else if self.bloom(x, z) > 0.55 || Self::near_start(x, z) < 0.5 {
            Biome::Wildflowers
        } else {
            Biome::Meadow
        }
    }

    /// The colour of the ground at a position (linear RGB): lush where it is damp, golden where dry, dark and leafy under trees.
    pub fn ground_color(&self, x: f64, z: f64) -> [f32; 3] {
        let c = self.climate(x, z);
        let glade = self.glade(x, z);
        let shade = self.forest_density(&c, glade) * Self::near_start(x, z);
        let lush = [0.085, 0.30, 0.045];
        let sunny = [0.16, 0.42, 0.055];
        let dry = [0.38, 0.40, 0.085];
        let litter = [0.115, 0.14, 0.04];
        let mut col = lerp3(sunny, lush, c.wet);
        col = lerp3(col, dry, smooth(0.55, 0.0, c.wet) * 0.55);
        col = lerp3(col, litter, shade * 0.75);
        // Wildflower fields tint the ground, so a far field reads as flowers when the flowers themselves are not drawn.
        let bloom = self.bloom(x, z) * (1.0 - shade) * Self::near_start(x, z).max(0.35);
        let field = fbm(self.seed(salt::BLOOM + 40), x / 9.0, z / 9.0, 2);
        let hue = if field > 0.0 { [0.34, 0.2, 0.09] } else { [0.30, 0.27, 0.05] };
        col = lerp3(col, hue, bloom * 0.45 * smooth(-0.3, 0.3, field.abs() * 2.0 - 0.2));
        // A mottle at metre scale so a field is never flat colour, and slightly bluer cool country.
        let mottle = fbm(self.seed(salt::GROUND), x / 4.5, z / 4.5, 2) * 0.12;
        let k = 1.0 + mottle;
        [col[0] * k, col[1] * (k + 0.02 * c.cool), col[2] * k]
    }

    /// The tree, shrub, flower or grass plants in a chunk.
    pub fn plants(&self, id: ChunkId, kind: Kind) -> Vec<Plant> {
        match kind {
            Kind::Tree => self.scatter(id, kind, TREE_CELL, salt::TREES, &[]),
            Kind::Shrub => self.scatter(id, kind, SHRUB_CELL, salt::SHRUBS, &self.tree_discs(id)),
            Kind::Grass => self.scatter(id, kind, GRASS_CELL, salt::GRASS, &self.tree_discs(id)),
            Kind::Flower => self.flowers(id),
        }
    }

    /// The trunks in a chunk that the player must walk around.
    pub fn trunks(&self, id: ChunkId) -> Vec<Trunk> {
        [Kind::Tree, Kind::Shrub]
            .into_iter()
            .flat_map(|k| self.plants(id, k))
            .filter_map(|p| {
                let r = tree_trunk(&p);
                (r > 0.0).then_some(Trunk { x: p.x, z: p.z, radius: r })
            })
            .collect()
    }

    /// What stops the player in a chunk: the trunks, widened to the leaves' reach where those fill a walking child's body band (see [`blocking_crown`]).
    pub fn blockers(&self, id: ChunkId) -> Vec<Trunk> {
        [Kind::Tree, Kind::Shrub]
            .into_iter()
            .flat_map(|k| self.plants(id, k))
            .filter_map(|p| {
                let r = tree_trunk(&p);
                (r > 0.0).then_some(Trunk { x: p.x, z: p.z, radius: r.max(blocking_crown(&p)) })
            })
            .collect()
    }

    /// The trees in and just around a chunk, as discs the smaller plants must keep out of (a daisy does not grow through an oak).
    fn tree_discs(&self, id: ChunkId) -> Vec<Trunk> {
        let (x0, z0) = id.origin();
        let reach = 3.0;
        let mut out = Vec::new();
        for dz in -1..=1 {
            for dx in -1..=1 {
                let other = ChunkId { x: id.x + dx, z: id.z + dz };
                for p in self.scatter(other, Kind::Tree, TREE_CELL, salt::TREES, &[]) {
                    if p.x > x0 - reach && p.x < x0 + CHUNK + reach && p.z > z0 - reach && p.z < z0 + CHUNK + reach {
                        out.push(Trunk { x: p.x, z: p.z, radius: tree_trunk(&p) });
                    }
                }
            }
        }
        out
    }

    /// The density of a layer's cells that grow something at a position, with the climate it was computed from.
    fn layer_density(&self, kind: Kind, x: f64, z: f64, c: &Climate, glade: f32) -> f32 {
        let forest = self.forest_density(c, glade) * Self::near_start(x, z);
        match kind {
            Kind::Tree => forest * self.cfg.trees.min(1.2),
            // Hedgerow shrubs on the fringes of woods, bracken in them, a lone bush now and then in the open.
            Kind::Shrub => (0.012 + 0.26 * forest + 0.22 * (smooth(0.35, 0.5, c.wood) * (1.0 - forest))).min(0.5) * Self::near_start(x, z).max(0.3),
            Kind::Grass => {
                let clump = smooth(-0.25, 0.35, fbm(self.seed(salt::CLUMP), x / 7.0, z / 7.0, 2));
                (0.95 - 0.45 * forest) * (0.3 + 0.7 * clump) * self.cfg.grass.min(1.5)
            }
            Kind::Flower => 0.0,
        }
    }

    /// Picks a species of a kind for a spot, weighted by how well each fits the climate and by patchy local stands.
    fn pick(&self, kind: Kind, x: f64, z: f64, c: &Climate, roll: f32) -> Option<SpeciesId> {
        let mut weights: [(SpeciesId, f32); 24] = [(SpeciesId(0), 0.0); 24];
        let mut n = 0;
        let mut total = 0.0;
        for (id, s) in flora::of_kind(kind) {
            // Species form stands: each has its own slowly-varying patchiness, so birches gather in warm woods rather than spread evenly.
            let stand =
                if kind == Kind::Tree { 0.35 + 1.3 * smooth(-0.4, 0.4, fbm(self.seed(salt::STAND + id.0 as u32 * 31), x / 55.0, z / 55.0, 2)) } else { 1.0 };
            let w = s.affinity(c) * stand;
            weights[n] = (id, w);
            total += w;
            n += 1;
        }
        if total < 1e-3 {
            return None;
        }
        let mut t = roll * total;
        for (id, w) in &weights[..n] {
            if t < *w {
                return Some(*id);
            }
            t -= w;
        }
        weights[..n].iter().rev().find(|(_, w)| *w > 0.0).map(|(id, _)| *id)
    }

    fn make(&self, species: SpeciesId, x: f64, z: f64, rng: &mut Rng) -> Plant {
        let s = flora::species(species);
        let h = rng.white();
        Plant {
            species,
            x,
            z,
            y: self.height(x, z),
            yaw: rng.range(0.0, std::f32::consts::TAU),
            height: s.height.0 + (s.height.1 - s.height.0) * (h * 0.7 + rng.white() * 0.3),
            tint: rng.white(),
            seed: rng.bits(),
            rank: rng.white(),
        }
    }

    /// A jittered-grid layer: one candidate per cell, kept with the density's probability.
    fn scatter(&self, id: ChunkId, kind: Kind, cell: f64, layer_salt: u32, avoid: &[Trunk]) -> Vec<Plant> {
        let per_side = (CHUNK / cell).round() as i32;
        let seed = self.seed(layer_salt);
        let mut out = Vec::new();
        for cj in 0..per_side {
            for ci in 0..per_side {
                let (gi, gj) = (id.x * per_side + ci, id.z * per_side + cj);
                let mut rng = Rng::at(seed, gi, gj);
                let (keep, jx, jz, roll) = (rng.white(), rng.range(0.12, 0.88), rng.range(0.12, 0.88), rng.white());
                let (x, z) = ((gi as f64 + jx as f64) * cell, (gj as f64 + jz as f64) * cell);
                // Cheap rejection first: most cells in an open field hold no tree.
                let c = self.climate(x, z);
                let glade = self.glade(x, z);
                if keep >= self.layer_density(kind, x, z, &c, glade) {
                    continue;
                }
                if blocked(avoid, x, z, 0.25) {
                    continue;
                }
                let Some(species) = self.pick(kind, x, z, &c, roll) else { continue };
                out.push(self.make(species, x, z, &mut rng));
            }
        }
        out
    }

    /// Flower patches: a handful of species in drifts, each patch a disc that thins toward its edge.
    fn flowers(&self, id: ChunkId) -> Vec<Plant> {
        let discs = self.tree_discs(id);
        let per_side = (CHUNK / PATCH_CELL).round() as i32;
        let seed = self.seed(salt::PATCHES);
        let (x0, z0) = id.origin();
        let mut out = Vec::new();
        for cj in -1..=per_side {
            for ci in -1..=per_side {
                let (gi, gj) = (id.x * per_side + ci, id.z * per_side + cj);
                let mut rng = Rng::at(seed, gi, gj);
                let (keep, jx, jz) = (rng.white(), rng.range(0.1, 0.9), rng.range(0.1, 0.9));
                let (cx, cz) = ((gi as f64 + jx as f64) * PATCH_CELL, (gj as f64 + jz as f64) * PATCH_CELL);
                let c = self.climate(cx, cz);
                let glade = self.glade(cx, cz);
                let forest = self.forest_density(&c, glade);
                let bloom = self.bloom(cx, cz);
                let open = 1.0 - forest;
                let start = 1.0 - Self::near_start(cx, cz);
                let chance = ((0.12 + 1.0 * bloom) * (0.25 + 0.75 * open) + glade * smooth(0.4, 0.7, c.wood) * 0.8 + start * 0.6) * self.cfg.flowers.min(1.6);
                if keep >= chance {
                    continue;
                }
                let radius = rng.range(2.5, 5.0 + 4.0 * bloom) as f64;
                let primary = self.pick(Kind::Flower, cx, cz, &c, rng.white());
                let secondary = self.pick(Kind::Flower, cx, cz, &c, rng.white());
                let (Some(primary), Some(secondary)) = (primary, secondary) else { continue };
                let mix = rng.range(0.15, 0.4);
                let per_m2 = rng.range(1.2, 3.0) * (0.6 + 2.2 * bloom + 1.5 * start) * self.cfg.flowers.min(1.6);
                let count = (std::f64::consts::PI * radius * radius * per_m2 as f64) as u32;
                for _ in 0..count {
                    let (dx, dz) = (rng.range(-1.0, 1.0) as f64 * radius, rng.range(-1.0, 1.0) as f64 * radius);
                    let edge = ((dx * dx + dz * dz) / (radius * radius)) as f32;
                    let thin = rng.white();
                    let species_roll = rng.white();
                    // The sequence of random numbers drawn per flower is fixed, so a flower is the same whichever chunk generates it.
                    let mut frng = Rng::at(rng.bits(), gi, gj);
                    if edge >= 1.0 || thin < edge * 0.8 {
                        continue;
                    }
                    let (x, z) = (cx + dx, cz + dz);
                    if x < x0 || x >= x0 + CHUNK || z < z0 || z >= z0 + CHUNK {
                        continue;
                    }
                    if blocked(&discs, x, z, 0.35) {
                        continue;
                    }
                    let species = if species_roll < mix { secondary } else { primary };
                    out.push(self.make(species, x, z, &mut frng));
                }
            }
        }
        out
    }
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// The trunk radius of a placed tree or shrub: the species' radius at its middle height, scaled with this plant's height.
fn tree_trunk(p: &Plant) -> f32 {
    let s = flora::species(p.species);
    s.trunk * p.height / ((s.height.0 + s.height.1) * 0.5)
}

/// How far a plant's leaves reach at the height of a walking child, when that is more than its trunk: a hawthorn's crown fills the body band from the
/// ground up and a willow's streamers hang down to head height, so a player stopped only by the trunk would walk through the plant.
fn blocking_crown(p: &Plant) -> f32 {
    let s = flora::species(p.species);
    match (s.kind, s.key) {
        (Kind::Shrub, _) => s.spread * 0.75 * p.height,
        (_, "willow") => s.spread * 0.5 * p.height,
        _ => 0.0,
    }
}

/// Whether a point is within `margin` metres of any disc.
fn blocked(discs: &[Trunk], x: f64, z: f64, margin: f32) -> bool {
    discs.iter().any(|d| {
        let r = (d.radius + margin) as f64;
        let (dx, dz) = (d.x - x, d.z - z);
        dx * dx + dz * dz < r * r
    })
}

/// A hash of a chunk's heights and plants: pins the generator so an accidental change to the world is noticed.
pub fn fingerprint(world: &World, radius: i32) -> u32 {
    let mut h = 0x811c_9dc5u32;
    let mut mix = |v: u32| h = (h ^ v).wrapping_mul(0x0100_0193);
    for cz in -radius..=radius {
        for cx in -radius..=radius {
            let id = ChunkId { x: cx, z: cz };
            let (x, z) = id.centre();
            mix((world.height(x, z) * 1000.0).round() as i32 as u32);
            for kind in [Kind::Tree, Kind::Shrub, Kind::Flower, Kind::Grass] {
                let plants = world.plants(id, kind);
                mix(plants.len() as u32);
                for p in plants.iter().take(40) {
                    mix(p.species.0 as u32);
                    mix((p.x * 100.0).round() as i64 as u32);
                    mix((p.z * 100.0).round() as i64 as u32);
                }
            }
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World {
        World::new(Config { seed: 7, ..Config::default() })
    }

    #[test]
    fn the_same_seed_makes_the_same_world_and_another_seed_a_different_one() {
        let a = world();
        let b = world();
        let c = World::new(Config { seed: 8, ..Config::default() });
        let id = ChunkId { x: 3, z: -2 };
        assert_eq!(a.plants(id, Kind::Tree), b.plants(id, Kind::Tree));
        assert_eq!(a.height(123.4, -56.7), b.height(123.4, -56.7));
        assert_ne!(a.height(123.4, -56.7), c.height(123.4, -56.7));
        assert_ne!(fingerprint(&a, 1), fingerprint(&c, 1));
    }

    #[test]
    fn the_world_is_pinned() {
        // If this changes, every world changes: only update it on purpose (and say so in the ADR).
        assert_eq!(fingerprint(&world(), 2), PINNED, "the generator changed: the world is different");
    }
    const PINNED: u32 = 1421298761;

    #[test]
    fn ground_is_walkable_everywhere_and_never_cliffs() {
        let w = world();
        let mut steepest = 0.0f32;
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for i in 0..160 {
            for j in 0..160 {
                let (x, z) = (i as f64 * 12.5 - 1000.0, j as f64 * 12.5 - 1000.0);
                let h = w.height(x, z);
                lo = lo.min(h);
                hi = hi.max(h);
                let n = w.normal(x, z);
                steepest = steepest.max((1.0 - n[1] * n[1]).max(0.0).sqrt() / n[1]);
            }
        }
        assert!(steepest < 0.45, "steepest slope {steepest}");
        assert!(hi - lo > 6.0 && hi - lo < 40.0, "relief {lo}..{hi}");
    }

    #[test]
    fn ground_is_continuous_across_chunk_borders() {
        let w = world();
        let at = 48.0 * 5.0;
        assert!((w.height(at - 1e-4, 10.0) - w.height(at + 1e-4, 10.0)).abs() < 1e-3);
    }

    #[test]
    fn the_start_is_an_open_meadow_with_flowers() {
        let w = world();
        assert_eq!(w.tree_density(0.0, 0.0), 0.0);
        let near: usize = (-1..=0).flat_map(|z| (-1..=0).map(move |x| ChunkId { x, z })).map(|id| w.plants(id, Kind::Flower).len()).sum();
        assert!(near > 300, "{near} flowers by the start");
        let trees: usize =
            (-1..=0).flat_map(|z| (-1..=0).map(move |x| ChunkId { x, z })).flat_map(|id| w.plants(id, Kind::Tree)).filter(|p| p.x.hypot(p.z) < 18.0).count();
        assert_eq!(trees, 0);
    }

    #[test]
    fn plants_stay_in_their_chunk_and_on_the_ground() {
        let w = world();
        let id = ChunkId { x: -4, z: 9 };
        let (x0, z0) = id.origin();
        for kind in [Kind::Tree, Kind::Shrub, Kind::Flower, Kind::Grass] {
            let plants = w.plants(id, kind);
            assert!(!plants.is_empty(), "{kind:?}");
            for p in &plants {
                assert!(p.x >= x0 && p.x < x0 + CHUNK && p.z >= z0 && p.z < z0 + CHUNK, "{kind:?} at {},{} outside {id:?}", p.x, p.z);
                assert_eq!(p.y, w.height(p.x, p.z));
                assert_eq!(flora::species(p.species).kind, kind);
                let s = flora::species(p.species);
                assert!(p.height >= s.height.0 - 1e-3 && p.height <= s.height.1 + 1e-3);
                assert!((0.0..1.0).contains(&p.rank));
            }
        }
    }

    #[test]
    fn flower_patches_continue_across_chunk_borders() {
        // Neighbouring chunks are generated independently: with no seam, the flower density just either side of the border matches the density just inside.
        let w = world();
        let (mut left, mut right, mut inner) = (0usize, 0usize, 0usize);
        for k in 0..30 {
            let (cx, cz) = (k - 15, 3);
            for p in w.plants(ChunkId { x: cx, z: cz }, Kind::Flower) {
                let (x0, _) = ChunkId { x: cx, z: cz }.origin();
                let dx = p.x - x0;
                if dx > CHUNK - 4.0 {
                    left += 1;
                } else if dx < 4.0 {
                    right += 1;
                } else if (22.0..26.0).contains(&dx) {
                    inner += 1;
                }
            }
        }
        let (l, r, i) = (left as f32, right as f32, inner as f32);
        assert!(i > 100.0, "{i} flowers in the sample");
        assert!((l / i - 1.0).abs() < 0.5 && (r / i - 1.0).abs() < 0.5, "border density {l},{r} vs inside {i}");
    }

    #[test]
    fn trunks_do_not_overlap_and_trees_are_spaced() {
        let w = world();
        let mut found = 0;
        for cz in -6..6 {
            for cx in -6..6 {
                let trunks = w.trunks(ChunkId { x: cx, z: cz });
                for (i, a) in trunks.iter().enumerate() {
                    for b in &trunks[i + 1..] {
                        assert!(((a.x - b.x).hypot(a.z - b.z)) as f32 > (a.radius + b.radius) * 0.9, "overlapping trunks");
                        found += 1;
                    }
                }
            }
        }
        assert!(found > 1000, "{found} pairs examined");
    }

    #[test]
    fn the_world_has_every_kind_of_country_and_fair_shares_of_them() {
        let w = world();
        let mut count = std::collections::HashMap::new();
        let n = 120;
        for i in 0..n {
            for j in 0..n {
                let (x, z) = (i as f64 * 25.0 - 1500.0, j as f64 * 25.0 - 1500.0);
                *count.entry(w.biome(x, z).name()).or_insert(0usize) += 1;
            }
        }
        let share = |k: &str| *count.get(k).unwrap_or(&0) as f32 / (n * n) as f32;
        for k in ["meadow", "wildflower field", "grove", "forest", "pinewood", "glade"] {
            assert!(share(k) > 0.015, "{k} is {:.1}%: {count:?}", share(k) * 100.0);
        }
        let wooded = share("forest") + share("pinewood");
        assert!((0.15..0.55).contains(&wooded), "woods cover {:.0}%", wooded * 100.0);
        assert!(share("meadow") + share("wildflower field") > 0.25, "fields: {count:?}");
    }

    #[test]
    fn every_species_grows_somewhere() {
        let w = world();
        let mut seen = std::collections::HashSet::new();
        'outer: for cz in -25..25 {
            for cx in -25..25 {
                let id = ChunkId { x: cx, z: cz };
                for kind in [Kind::Tree, Kind::Shrub, Kind::Flower] {
                    for p in w.plants(id, kind) {
                        seen.insert(p.species);
                    }
                }
                for p in w.plants(id, Kind::Grass).iter().take(30) {
                    seen.insert(p.species);
                }
                if seen.len() == flora::SPECIES.len() {
                    break 'outer;
                }
            }
        }
        let missing: Vec<&str> = flora::SPECIES.iter().enumerate().filter(|(i, _)| !seen.contains(&SpeciesId(*i as u8))).map(|(_, s)| s.key).collect();
        assert!(missing.is_empty(), "never placed: {missing:?}");
    }

    #[test]
    fn thinning_by_rank_nests() {
        let w = world();
        let all = w.plants(ChunkId { x: 1, z: 1 }, Kind::Grass);
        let half: Vec<_> = all.iter().filter(|p| p.rank < 0.5).collect();
        let share = half.len() as f32 / all.len() as f32;
        assert!((share - 0.5).abs() < 0.08, "{share}");
    }

    #[test]
    fn far_away_is_just_as_good() {
        let w = world();
        let id = ChunkId { x: 200_000, z: -150_000 };
        assert!(!w.plants(id, Kind::Grass).is_empty());
        let (x, z) = id.centre();
        assert!(w.height(x, z).is_finite());
        assert!((w.height(x, z) - w.height(x + 0.01, z)).abs() < 0.01);
    }

    #[test]
    fn chunks_build_quickly() {
        let w = world();
        let started = std::time::Instant::now();
        let mut n = 0;
        for cx in 0..6 {
            for kind in [Kind::Tree, Kind::Shrub, Kind::Flower, Kind::Grass] {
                n += w.plants(ChunkId { x: cx, z: 30 }, kind).len();
            }
        }
        let per_chunk = started.elapsed().as_secs_f32() * 1000.0 / 6.0;
        assert!(n > 1000);
        // A generous bound for an unoptimised test build; the budget in release is a few milliseconds (see the ADR).
        assert!(per_chunk < 400.0, "{per_chunk:.1} ms per chunk");
    }

    #[test]
    fn a_procgen_block_is_checked() {
        let ok = serde_json::json!({"seed": 9, "trees": 0.5});
        assert_eq!(parse_config(ok.as_object().unwrap()).unwrap().seed, 9);
        for (bad, needle) in
            [(serde_json::json!({"seed": -1}), "seed"), (serde_json::json!({"trees": 9}), "trees"), (serde_json::json!({"forest": 1}), "forest")]
        {
            let e = parse_config(bad.as_object().unwrap()).unwrap_err().join(" ");
            assert!(e.contains(needle), "{e}");
        }
    }
}
