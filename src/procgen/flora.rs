//! The plants of a temperate world: real species with their Latin names, sizes, colours and the climate each one grows in.
//!
//! The table is data. A species says how tall it grows, how wide its crown or tuft is, what colours it comes in, and where it likes to live
//! as bands over the four climate fields ([`Climate`]): `cool` (warm broadleaf country to cold conifer country), `wet` (dry slopes to damp
//! hollows), `wood` (open fields to deep forest) and, for flowers, whether the ground is a glade or a field. The world picks a species for a
//! spot by weighing every species of the right [`Kind`] by its affinity there, so oaks and birches mix in warm woods, spruce takes over the
//! cold ones, willows stand in the wet, bluebells carpet shady ground and poppies, cornflowers and daisies colour the dry open fields.
//! Meshes for them come from the flora module of the renderer (the species `id` is the key); nothing here touches a GPU.

use super::noise::smooth;

/// What sort of plant: decides which placement layer grows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A tall woody plant with a trunk that blocks the way.
    Tree,
    /// A woody or leafy plant at or below head height.
    Shrub,
    /// A flowering plant in patches.
    Flower,
    /// Tufts of grass.
    Grass,
}

/// The index of a species in [`SPECIES`]; stable, so it is safe to store and to key meshes by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpeciesId(pub u8);

/// The climate at a spot, each field 0 to 1 (0.5 is the middle).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Climate {
    /// 0 warm lowland, 1 cold upland: broadleaf trees give way to conifers.
    pub cool: f32,
    /// 0 dry, 1 waterlogged.
    pub wet: f32,
    /// 0 open field, 1 deep wood (the forest density before clearings are cut).
    pub wood: f32,
}

/// A closed range of a climate field a species tolerates (the edges are soft).
pub type Band = (f32, f32);

/// One species.
#[derive(Debug, Clone, Copy)]
pub struct Species {
    /// Short stable key (`oak`, `ox_eye_daisy` ...): used in scenes, meshes and the map legend.
    pub key: &'static str,
    /// The everyday name.
    pub common: &'static str,
    /// The scientific name: these are real plants.
    pub latin: &'static str,
    /// The placement layer.
    pub kind: Kind,
    /// Height range in metres.
    pub height: (f32, f32),
    /// Crown or spread radius as a fraction of the height.
    pub spread: f32,
    /// Radius of the trunk in metres (what blocks the player; 0 for plants that are walked through).
    pub trunk: f32,
    /// The climate bands it grows in.
    pub cool: Band,
    /// The wetness bands it grows in.
    pub wet: Band,
    /// The woodland bands it grows in.
    pub wood: Band,
    /// Foliage or petal colours (sRGB hex); each plant takes one, blended by its own random number.
    pub colors: &'static [&'static str],
}

const ANY: Band = (0.0, 1.0);

/// Every species in the temperate flora.
pub const SPECIES: &[Species] = &[
    // Trees.
    Species {
        key: "oak",
        common: "English oak",
        latin: "Quercus robur",
        kind: Kind::Tree,
        height: (12.0, 18.0),
        spread: 0.40,
        trunk: 0.45,
        cool: (0.1, 0.75),
        wet: (0.15, 0.85),
        wood: (0.25, 1.0),
        colors: &["#5c8a38", "#6b9a3e", "#78a644"],
    },
    Species {
        key: "birch",
        common: "silver birch",
        latin: "Betula pendula",
        kind: Kind::Tree,
        height: (8.0, 13.0),
        spread: 0.26,
        trunk: 0.16,
        cool: (0.25, 1.0),
        wet: (0.0, 0.9),
        wood: ANY,
        colors: &["#8cb04c", "#9bbd58", "#aac75f"],
    },
    Species {
        key: "spruce",
        common: "Norway spruce",
        latin: "Picea abies",
        kind: Kind::Tree,
        height: (12.0, 22.0),
        spread: 0.22,
        trunk: 0.28,
        cool: (0.55, 1.0),
        wet: ANY,
        wood: (0.35, 1.0),
        colors: &["#2c5838", "#27503a", "#34603c"],
    },
    Species {
        key: "pine",
        common: "Scots pine",
        latin: "Pinus sylvestris",
        kind: Kind::Tree,
        height: (11.0, 17.0),
        spread: 0.28,
        trunk: 0.28,
        cool: (0.45, 1.0),
        wet: (0.0, 0.55),
        wood: (0.2, 1.0),
        colors: &["#3e6a44", "#46744a", "#385e40"],
    },
    Species {
        key: "willow",
        common: "weeping willow",
        latin: "Salix babylonica",
        kind: Kind::Tree,
        height: (8.0, 12.0),
        spread: 0.45,
        trunk: 0.30,
        cool: (0.0, 0.7),
        wet: (0.62, 1.0),
        wood: ANY,
        colors: &["#9bb95a", "#aac868", "#8fb050"],
    },
    Species {
        key: "cherry",
        common: "wild cherry",
        latin: "Prunus avium",
        kind: Kind::Tree,
        height: (6.0, 10.0),
        spread: 0.38,
        trunk: 0.18,
        cool: (0.0, 0.75),
        wet: (0.1, 0.9),
        wood: (0.0, 0.8),
        colors: &["#e888aa", "#f09cb8", "#e47a9e"],
    },
    // Shrubs.
    Species {
        key: "hawthorn",
        common: "hawthorn",
        latin: "Crataegus monogyna",
        kind: Kind::Shrub,
        height: (2.5, 4.5),
        spread: 0.45,
        trunk: 0.08,
        cool: (0.0, 0.85),
        wet: ANY,
        wood: (0.1, 0.9),
        colors: &["#5a8a3a", "#6a9a42", "#f3eee4"],
    },
    Species {
        key: "bracken",
        common: "bracken",
        latin: "Pteridium aquilinum",
        kind: Kind::Shrub,
        height: (0.8, 1.4),
        spread: 0.55,
        trunk: 0.0,
        cool: ANY,
        wet: (0.0, 0.85),
        wood: (0.45, 1.0),
        colors: &["#5f8f3f", "#6d9a45", "#78a44c"],
    },
    // Flowers.
    Species {
        key: "ox_eye_daisy",
        common: "oxeye daisy",
        latin: "Leucanthemum vulgare",
        kind: Kind::Flower,
        height: (0.4, 0.7),
        spread: 0.20,
        trunk: 0.0,
        cool: (0.0, 0.85),
        wet: (0.15, 0.85),
        wood: (0.0, 0.7),
        colors: &["#ffffff", "#fbfaf2"],
    },
    Species {
        key: "poppy",
        common: "common poppy",
        latin: "Papaver rhoeas",
        kind: Kind::Flower,
        height: (0.4, 0.7),
        spread: 0.20,
        trunk: 0.0,
        cool: (0.0, 0.7),
        wet: (0.0, 0.55),
        wood: (0.0, 0.5),
        colors: &["#e23a28", "#d92b1e", "#ec4a30"],
    },
    Species {
        key: "cornflower",
        common: "cornflower",
        latin: "Centaurea cyanus",
        kind: Kind::Flower,
        height: (0.5, 0.8),
        spread: 0.18,
        trunk: 0.0,
        cool: (0.0, 0.7),
        wet: (0.0, 0.6),
        wood: (0.0, 0.5),
        colors: &["#4a70e2", "#5a80e8", "#3e62d0"],
    },
    Species {
        key: "lavender",
        common: "lavender",
        latin: "Lavandula angustifolia",
        kind: Kind::Flower,
        height: (0.5, 0.75),
        spread: 0.35,
        trunk: 0.0,
        cool: (0.0, 0.55),
        wet: (0.0, 0.35),
        wood: (0.0, 0.5),
        colors: &["#9b7fd2", "#8a6cc6", "#a98fda"],
    },
    Species {
        key: "bluebell",
        common: "bluebell",
        latin: "Hyacinthoides non-scripta",
        kind: Kind::Flower,
        height: (0.3, 0.45),
        spread: 0.30,
        trunk: 0.0,
        cool: ANY,
        wet: (0.3, 1.0),
        wood: (0.5, 1.0),
        colors: &["#5a5fd8", "#6a6ee0", "#4c52c8"],
    },
    Species {
        key: "dandelion",
        common: "dandelion",
        latin: "Taraxacum officinale",
        kind: Kind::Flower,
        height: (0.15, 0.3),
        spread: 0.35,
        trunk: 0.0,
        cool: ANY,
        wet: ANY,
        wood: (0.0, 0.7),
        colors: &["#ffd23a", "#ffc928"],
    },
    Species {
        key: "buttercup",
        common: "meadow buttercup",
        latin: "Ranunculus acris",
        kind: Kind::Flower,
        height: (0.4, 0.7),
        spread: 0.20,
        trunk: 0.0,
        cool: ANY,
        wet: (0.45, 1.0),
        wood: (0.0, 0.7),
        colors: &["#ffdc20", "#ffe23c"],
    },
    Species {
        key: "red_clover",
        common: "red clover",
        latin: "Trifolium pratense",
        kind: Kind::Flower,
        height: (0.25, 0.4),
        spread: 0.30,
        trunk: 0.0,
        cool: ANY,
        wet: (0.2, 0.85),
        wood: (0.0, 0.6),
        colors: &["#d8588a", "#c84a7c", "#e06a98"],
    },
    Species {
        key: "foxglove",
        common: "common foxglove",
        latin: "Digitalis purpurea",
        kind: Kind::Flower,
        height: (0.9, 1.4),
        spread: 0.14,
        trunk: 0.0,
        cool: (0.2, 1.0),
        wet: (0.2, 0.9),
        wood: (0.4, 1.0),
        colors: &["#c468c0", "#b657ae", "#d27ccc"],
    },
    Species {
        key: "harebell",
        common: "harebell",
        latin: "Campanula rotundifolia",
        kind: Kind::Flower,
        height: (0.25, 0.4),
        spread: 0.25,
        trunk: 0.0,
        cool: (0.3, 1.0),
        wet: (0.0, 0.5),
        wood: (0.0, 0.6),
        colors: &["#7b90e2", "#8a9eea"],
    },
    // Grasses.
    Species {
        key: "meadow_grass",
        common: "smooth meadow-grass",
        latin: "Poa pratensis",
        kind: Kind::Grass,
        height: (0.25, 0.45),
        spread: 0.55,
        trunk: 0.0,
        cool: ANY,
        wet: ANY,
        wood: ANY,
        colors: &["#80ae46", "#8fba50", "#72a03e"],
    },
    Species {
        key: "timothy",
        common: "timothy grass",
        latin: "Phleum pratense",
        kind: Kind::Grass,
        height: (0.6, 0.95),
        spread: 0.30,
        trunk: 0.0,
        cool: ANY,
        wet: (0.1, 0.9),
        wood: (0.0, 0.6),
        colors: &["#b6b96c", "#a9b55b", "#c4c47a"],
    },
];

/// The species with an id.
pub fn species(id: SpeciesId) -> &'static Species {
    &SPECIES[id.0 as usize]
}

/// The id of the species with a key.
pub fn by_key(key: &str) -> Option<SpeciesId> {
    SPECIES.iter().position(|s| s.key == key).map(|i| SpeciesId(i as u8))
}

/// Every species of a kind.
pub fn of_kind(kind: Kind) -> impl Iterator<Item = (SpeciesId, &'static Species)> {
    SPECIES.iter().enumerate().filter(move |(_, s)| s.kind == kind).map(|(i, s)| (SpeciesId(i as u8), s))
}

/// How well `x` fits a band: 1 inside, falling to 0 over 0.12 outside (the ends of 0 and 1 are open-ended).
fn band(x: f32, (lo, hi): Band) -> f32 {
    let up = if lo <= 0.0 { 1.0 } else { smooth(lo - 0.12, lo + 0.12, x) };
    let down = if hi >= 1.0 { 1.0 } else { 1.0 - smooth(hi - 0.12, hi + 0.12, x) };
    up * down
}

impl Species {
    /// How much this species wants to live in this climate, 0 (it will not grow there) to 1.
    pub fn affinity(&self, c: &Climate) -> f32 {
        band(c.cool, self.cool) * band(c.wet, self.wet) * band(c.wood, self.wood)
    }

    /// The colour palette as linear RGB.
    pub fn palette(&self) -> Vec<[f32; 3]> {
        self.colors.iter().map(|h| crate::color::parse_hex_to_linear(h).map(|v| v.to_array()).unwrap_or([1.0, 0.0, 1.0])).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_well_formed() {
        assert!(SPECIES.len() < 256);
        for (i, s) in SPECIES.iter().enumerate() {
            assert!(s.height.0 > 0.0 && s.height.0 < s.height.1, "{}", s.key);
            assert!(s.latin.contains(' '), "{} wants a binomial name", s.key);
            assert!(!s.colors.is_empty() && s.palette().iter().all(|c| *c != [1.0, 0.0, 1.0]), "{} palette", s.key);
            assert_eq!(by_key(s.key), Some(SpeciesId(i as u8)), "keys are unique: {}", s.key);
            for b in [s.cool, s.wet, s.wood] {
                assert!(b.0 < b.1 && b.0 >= 0.0 && b.1 <= 1.0, "{} band {b:?}", s.key);
            }
            assert!(s.kind != Kind::Tree || s.trunk > 0.0, "{}: a tree has a trunk", s.key);
        }
    }

    #[test]
    fn there_is_a_real_variety() {
        assert!(of_kind(Kind::Tree).count() >= 6);
        assert!(of_kind(Kind::Flower).count() >= 10);
        assert!(of_kind(Kind::Grass).count() >= 2);
    }

    #[test]
    fn climate_picks_the_right_trees() {
        let best = |c: Climate| of_kind(Kind::Tree).max_by(|a, b| a.1.affinity(&c).total_cmp(&b.1.affinity(&c))).map(|(_, s)| s.key).unwrap();
        assert_eq!(best(Climate { cool: 0.95, wet: 0.75, wood: 0.9 }), "spruce");
        assert_eq!(best(Climate { cool: 0.3, wet: 0.95, wood: 0.5 }), "willow");
        let oak = by_key("oak").map(species).unwrap();
        let spruce = by_key("spruce").map(species).unwrap();
        let warm = Climate { cool: 0.3, wet: 0.5, wood: 0.8 };
        assert!(oak.affinity(&warm) > 0.9 && spruce.affinity(&warm) < 0.05);
    }

    #[test]
    fn bluebells_want_shade_and_poppies_want_sun() {
        let bluebell = by_key("bluebell").map(species).unwrap();
        let poppy = by_key("poppy").map(species).unwrap();
        let wood = Climate { cool: 0.5, wet: 0.6, wood: 0.9 };
        let field = Climate { cool: 0.4, wet: 0.3, wood: 0.05 };
        assert!(bluebell.affinity(&wood) > 0.8 && bluebell.affinity(&field) < 0.05);
        assert!(poppy.affinity(&field) > 0.8 && poppy.affinity(&wood) < 0.05);
    }
}
