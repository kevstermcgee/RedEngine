//! Blocks: what each cell of the world holds (one byte), and everything the game knows about each kind: how it is drawn (its shape and a
//! texture per face, numbers the game's `surface` shader paints), how it collides, how light passes through it, how long it takes to mine with
//! which tool, and what it drops.
//!
//! Blocks that face a way (a furnace's front, a chest's latch, a torch on a wall) take one id per facing: `facing` 0 looks north (-Z), 1 east
//! (+X), 2 south (+Z), 3 west (-X).

/// One cell of the world.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, PartialOrd, Ord)]
pub struct Block(pub u8);

/// How a block is drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    /// Nothing.
    Air,
    /// A full cube.
    Cube,
    /// A liquid: a cube whose top sits lower when nothing of its kind is above.
    Liquid,
    /// A small model of boxes (flowers, grass, a dead bush).
    Plant(Plant),
    /// A torch on the floor (`None`) or on a wall, leaning out from the wall behind it (`Some(facing)`: the way it leans).
    Torch(Option<u8>),
    /// A cactus: a cube whose sides are drawn a sixteenth in.
    Cactus,
}

/// The small plant models.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Plant {
    Dandelion,
    Poppy,
    Cornflower,
    TallGrass,
    DeadBush,
}

/// The tool a block is quickest to mine with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Any,
    Pickaxe,
    Axe,
    Shovel,
}

/// What a block sounds like when it is walked on, mined, broken and placed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Sound {
    Stone,
    Wood,
    Grass,
    Gravel,
    Sand,
    Glass,
    Snow,
    Plant,
}

/// What a broken block leaves (when the right tool broke it).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Drop {
    /// The block itself (its facing forgotten).
    Itself,
    /// Something else: an item id and how many.
    Item(u16, u8),
    /// Nothing.
    Nothing,
}

/// Which pass draws a block's faces.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pass {
    Solid,
    /// See-through (water, ice): drawn blended.
    Water,
    /// See-through (glass): drawn blended, fainter.
    Glass,
}

/// Everything known about one kind of block.
#[derive(Clone, Copy, Debug)]
pub struct Info {
    /// Its id in saves and logs.
    pub name: &'static str,
    /// What the player reads.
    pub title: &'static str,
    pub shape: Shape,
    /// The player and the mobs collide with it.
    pub solid: bool,
    /// Drawn as a full opaque cube: hides its neighbours' faces and darkens their corners.
    pub cull: bool,
    /// How much light it takes away passing through: 15 stops it, 0 lets it all through.
    pub light_block: u8,
    /// The light it gives off (0..15).
    pub emit: u8,
    /// Seconds of work by hand with the right tool, before the tool's speed (the original's hardness); negative: unbreakable.
    pub hardness: f32,
    pub tool: Tool,
    /// The tool tier needed for it to drop anything (0 a hand, 1 wood, 2 stone, 3 iron, 4 diamond); needs a pickaxe above 0.
    pub needs: u8,
    pub drop: Drop,
    /// Texture numbers for the top, the bottom, the sides and the front (the side a facing block faces).
    pub tex: [u8; 4],
    pub sound: Sound,
    pub pass: Pass,
}

const fn info(name: &'static str, title: &'static str) -> Info {
    Info {
        name,
        title,
        shape: Shape::Cube,
        solid: true,
        cull: true,
        light_block: 15,
        emit: 0,
        hardness: 1.0,
        tool: Tool::Any,
        needs: 0,
        drop: Drop::Itself,
        tex: [0; 4],
        sound: Sound::Stone,
        pass: Pass::Solid,
    }
}

/// Texture numbers (the shader's `tex_*`; 128 and up are flat colours for models).
pub mod tex {
    pub const STONE: u8 = 1;
    pub const DIRT: u8 = 2;
    pub const GRASS_TOP: u8 = 3;
    pub const GRASS_SIDE: u8 = 4;
    pub const COBBLE: u8 = 5;
    pub const OAK_PLANKS: u8 = 6;
    pub const BEDROCK: u8 = 7;
    pub const WATER: u8 = 8;
    pub const LAVA: u8 = 9;
    pub const SAND: u8 = 10;
    pub const GRAVEL: u8 = 11;
    pub const GOLD_ORE: u8 = 12;
    pub const IRON_ORE: u8 = 13;
    pub const COAL_ORE: u8 = 14;
    pub const DIAMOND_ORE: u8 = 15;
    pub const OAK_LOG: u8 = 16;
    pub const OAK_LOG_TOP: u8 = 17;
    pub const OAK_LEAVES: u8 = 18;
    pub const BIRCH_LOG: u8 = 19;
    pub const BIRCH_LOG_TOP: u8 = 20;
    pub const BIRCH_LEAVES: u8 = 21;
    pub const SPRUCE_LOG: u8 = 22;
    pub const SPRUCE_LOG_TOP: u8 = 23;
    pub const SPRUCE_LEAVES: u8 = 24;
    pub const GLASS: u8 = 25;
    pub const SANDSTONE: u8 = 26;
    pub const SANDSTONE_TOP: u8 = 27;
    pub const BRICKS: u8 = 28;
    pub const STONE_BRICKS: u8 = 29;
    pub const TABLE_TOP: u8 = 30;
    pub const TABLE_SIDE: u8 = 31;
    pub const TABLE_FRONT: u8 = 32;
    pub const SNOW: u8 = 33;
    pub const SNOWY_GRASS_SIDE: u8 = 34;
    pub const ICE: u8 = 35;
    pub const CACTUS_SIDE: u8 = 36;
    pub const CACTUS_TOP: u8 = 37;
    pub const CLAY: u8 = 38;
    pub const OBSIDIAN: u8 = 39;
    pub const BOOKSHELF: u8 = 40;
    pub const BIRCH_PLANKS: u8 = 41;
    pub const SPRUCE_PLANKS: u8 = 42;
    pub const IRON_BLOCK: u8 = 43;
    pub const GOLD_BLOCK: u8 = 44;
    pub const DIAMOND_BLOCK: u8 = 45;
    pub const COAL_BLOCK: u8 = 46;
    pub const MOSSY_COBBLE: u8 = 47;
    pub const TNT_SIDE: u8 = 48;
    pub const TNT_TOP: u8 = 49;
    pub const TNT_BOTTOM: u8 = 50;
    pub const FURNACE_FRONT: u8 = 51;
    pub const FURNACE_FRONT_LIT: u8 = 52;
    pub const FURNACE_SIDE: u8 = 53;
    pub const FURNACE_TOP: u8 = 54;
    pub const CHEST_FRONT: u8 = 55;
    pub const CHEST_SIDE: u8 = 56;
    pub const CHEST_TOP: u8 = 57;
    /// The highest block texture.
    pub const LAST: u8 = CHEST_TOP;

    /// Flat colours (models): 128 + an index into the shader's `flat` table.
    pub const STEM: u8 = 128;
    pub const LEAF: u8 = 129;
    pub const YELLOW: u8 = 130;
    pub const RED: u8 = 131;
    pub const BLUE: u8 = 132;
    pub const BLADE: u8 = 133;
    pub const BLADE_LIGHT: u8 = 134;
    pub const DEAD: u8 = 135;
    pub const TORCH_WOOD: u8 = 136;
    pub const FLAME: u8 = 137;
    pub const FLAME_CORE: u8 = 138;
    pub const PETAL_DARK: u8 = 139;
    pub const YELLOW_DARK: u8 = 140;
    pub const BLUE_DARK: u8 = 141;
    pub const PIG: u8 = 142;
    pub const PIG_SNOUT: u8 = 143;
    pub const EYE: u8 = 144;
    pub const ZOMBIE: u8 = 145;
    pub const SHIRT: u8 = 146;
    pub const PANTS: u8 = 147;
    pub const CREEPER: u8 = 148;
    pub const CREEPER_LIGHT: u8 = 149;
    pub const CREEPER_DARK: u8 = 150;
    pub const EYE_WHITE: u8 = 151;
    pub const HOOF: u8 = 152;
    /// The highest flat colour.
    pub const LAST_FLAT: u8 = HOOF;
}

impl Block {
    pub const AIR: Block = Block(0);
    pub const STONE: Block = Block(1);
    pub const GRASS: Block = Block(2);
    pub const DIRT: Block = Block(3);
    pub const COBBLESTONE: Block = Block(4);
    pub const OAK_PLANKS: Block = Block(5);
    pub const BEDROCK: Block = Block(6);
    pub const WATER: Block = Block(7);
    pub const LAVA: Block = Block(8);
    pub const SAND: Block = Block(9);
    pub const GRAVEL: Block = Block(10);
    pub const GOLD_ORE: Block = Block(11);
    pub const IRON_ORE: Block = Block(12);
    pub const COAL_ORE: Block = Block(13);
    pub const DIAMOND_ORE: Block = Block(14);
    pub const OAK_LOG: Block = Block(15);
    pub const OAK_LEAVES: Block = Block(16);
    pub const BIRCH_LOG: Block = Block(17);
    pub const BIRCH_LEAVES: Block = Block(18);
    pub const SPRUCE_LOG: Block = Block(19);
    pub const SPRUCE_LEAVES: Block = Block(20);
    pub const GLASS: Block = Block(21);
    pub const SANDSTONE: Block = Block(22);
    pub const BRICKS: Block = Block(23);
    pub const STONE_BRICKS: Block = Block(24);
    pub const CRAFTING_TABLE: Block = Block(25);
    pub const SNOW: Block = Block(26);
    pub const SNOWY_GRASS: Block = Block(27);
    pub const ICE: Block = Block(28);
    pub const CACTUS: Block = Block(29);
    pub const CLAY: Block = Block(30);
    pub const OBSIDIAN: Block = Block(31);
    pub const BOOKSHELF: Block = Block(32);
    pub const BIRCH_PLANKS: Block = Block(33);
    pub const SPRUCE_PLANKS: Block = Block(34);
    pub const IRON_BLOCK: Block = Block(35);
    pub const GOLD_BLOCK: Block = Block(36);
    pub const DIAMOND_BLOCK: Block = Block(37);
    pub const COAL_BLOCK: Block = Block(38);
    pub const MOSSY_COBBLESTONE: Block = Block(39);
    pub const TNT: Block = Block(40);
    pub const DANDELION: Block = Block(41);
    pub const POPPY: Block = Block(42);
    pub const CORNFLOWER: Block = Block(43);
    pub const TALL_GRASS: Block = Block(44);
    pub const DEAD_BUSH: Block = Block(45);
    pub const TORCH: Block = Block(46);
    /// A torch on a wall, leaning `facing` (47..=50).
    pub const WALL_TORCH: Block = Block(47);
    /// A furnace facing `facing` (51..=54); lit: 55..=58.
    pub const FURNACE: Block = Block(51);
    pub const FURNACE_LIT: Block = Block(55);
    /// A chest facing `facing` (59..=62).
    pub const CHEST: Block = Block(59);
    /// Water flowing away from a source, level 1 (next to it, or falling) to 7 (its farthest reach): 63..=69.
    pub const FLOWING_WATER: Block = Block(63);
    /// One past the highest id.
    pub const COUNT: usize = 70;

    /// Everything known about this kind of block (an unknown id reads as air).
    pub fn info(self) -> &'static Info {
        INFO.get(self.0 as usize).unwrap_or(&INFO[0])
    }

    pub fn is_air(self) -> bool {
        self.0 == 0
    }

    pub fn solid(self) -> bool {
        self.info().solid
    }

    pub fn cull(self) -> bool {
        self.info().cull
    }

    pub fn is_liquid(self) -> bool {
        self == Block::LAVA || self.is_water()
    }

    /// Water, still or flowing.
    pub fn is_water(self) -> bool {
        self == Block::WATER || (Block::FLOWING_WATER.0..Block::FLOWING_WATER.0 + 7).contains(&self.0)
    }

    /// A water block's level: 0 a source, 1..7 flowing (farther from its source).
    pub fn water_level(self) -> Option<u8> {
        if self == Block::WATER {
            Some(0)
        } else if self.is_water() {
            Some(self.0 - Block::FLOWING_WATER.0 + 1)
        } else {
            None
        }
    }

    /// Flowing water of `level` (1..7).
    pub fn flowing(level: u8) -> Block {
        Block(Block::FLOWING_WATER.0 + level.clamp(1, 7) - 1)
    }

    /// Whether a block can be put into this cell (air, a liquid, grass that gives way).
    pub fn replaceable(self) -> bool {
        self.is_air() || self.is_liquid() || self == Block::TALL_GRASS || self == Block::DEAD_BUSH
    }

    /// The block's base id and its facing, for the kinds that face a way.
    pub fn facing(self) -> Option<(Block, u8)> {
        for base in [Block::WALL_TORCH, Block::FURNACE, Block::FURNACE_LIT, Block::CHEST] {
            if (base.0..base.0 + 4).contains(&self.0) {
                return Some((base, self.0 - base.0));
            }
        }
        None
    }

    /// `base` facing `facing` (0 north, 1 east, 2 south, 3 west).
    pub fn faced(base: Block, facing: u8) -> Block {
        Block(base.0 + (facing & 3))
    }

    /// The kind of block it is when picked up (a lit furnace is a furnace, a wall torch a torch).
    pub fn base(self) -> Block {
        match self.facing() {
            Some((Block::WALL_TORCH, _)) => Block::TORCH,
            Some((Block::FURNACE_LIT, _)) => Block::FURNACE,
            Some((base, _)) => base,
            None => self,
        }
    }

    /// The texture of the face pointing along `dir` (0 +X, 1 -X, 2 +Y, 3 -Y, 4 +Z, 5 -Z).
    pub fn face_tex(self, dir: usize) -> u8 {
        let i = self.info();
        match dir {
            2 => i.tex[0],
            3 => i.tex[1],
            _ => match self.facing() {
                Some((_, f)) if FACING_DIR[f as usize] == dir => i.tex[3],
                _ => i.tex[2],
            },
        }
    }

    /// The block with this save name.
    pub fn by_name(name: &str) -> Option<Block> {
        INFO.iter().position(|i| i.name == name).map(|i| Block(i as u8))
    }
}

/// The face direction (0 +X, 1 -X, 2 +Y, 3 -Y, 4 +Z, 5 -Z) a facing points along: north -Z, east +X, south +Z, west -X.
pub const FACING_DIR: [usize; 4] = [5, 0, 4, 1];

/// The six face directions as unit steps, in the order of [`Block::face_tex`].
pub const DIRS: [[i32; 3]; 6] = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]];

const fn cube(name: &'static str, title: &'static str, tex: u8, hardness: f32, tool: Tool, needs: u8, sound: Sound) -> Info {
    Info { tex: [tex; 4], hardness, tool, needs, sound, ..info(name, title) }
}

const fn tsb(mut i: Info, top: u8, side: u8, bottom: u8) -> Info {
    i.tex = [top, bottom, side, side];
    i
}

const fn drops(mut i: Info, d: Drop) -> Info {
    i.drop = d;
    i
}

const fn see_through(mut i: Info, pass: Pass, light_block: u8) -> Info {
    i.cull = false;
    i.light_block = light_block;
    i.pass = pass;
    i
}

const fn plant(name: &'static str, title: &'static str, p: Plant, drop: Drop) -> Info {
    Info {
        shape: Shape::Plant(p),
        solid: false,
        cull: false,
        light_block: 0,
        hardness: 0.0,
        drop,
        tex: [tex::STEM; 4],
        sound: Sound::Plant,
        ..info(name, title)
    }
}

const fn torch(facing: Option<u8>) -> Info {
    Info {
        shape: Shape::Torch(facing),
        solid: false,
        cull: false,
        light_block: 0,
        emit: 14,
        hardness: 0.0,
        tex: [tex::TORCH_WOOD; 4],
        sound: Sound::Wood,
        ..info(if facing.is_some() { "wall_torch" } else { "torch" }, "Torch")
    }
}

#[allow(clippy::too_many_arguments)]
const fn faced(name: &'static str, title: &'static str, top: u8, side: u8, front: u8, hardness: f32, tool: Tool, needs: u8, sound: Sound) -> Info {
    Info { tex: [top, top, side, front], hardness, tool, needs, sound, ..info(name, title) }
}

const fn lit(mut i: Info, emit: u8) -> Info {
    i.emit = emit;
    i
}

use crate::item::id as it;

/// The table, by id.
pub static INFO: [Info; Block::COUNT] = [
    Info { shape: Shape::Air, solid: false, cull: false, light_block: 0, hardness: 0.0, drop: Drop::Nothing, ..info("air", "Air") },
    drops(cube("stone", "Stone", tex::STONE, 1.5, Tool::Pickaxe, 1, Sound::Stone), Drop::Item(4, 1)),
    drops(tsb(cube("grass_block", "Grass Block", 0, 0.6, Tool::Shovel, 0, Sound::Grass), tex::GRASS_TOP, tex::GRASS_SIDE, tex::DIRT), Drop::Item(3, 1)),
    cube("dirt", "Dirt", tex::DIRT, 0.5, Tool::Shovel, 0, Sound::Gravel),
    cube("cobblestone", "Cobblestone", tex::COBBLE, 2.0, Tool::Pickaxe, 1, Sound::Stone),
    cube("oak_planks", "Oak Planks", tex::OAK_PLANKS, 2.0, Tool::Axe, 0, Sound::Wood),
    cube("bedrock", "Bedrock", tex::BEDROCK, -1.0, Tool::Pickaxe, 9, Sound::Stone),
    Info {
        shape: Shape::Liquid,
        solid: false,
        cull: false,
        light_block: 2,
        hardness: -1.0,
        drop: Drop::Nothing,
        tex: [tex::WATER; 4],
        pass: Pass::Water,
        ..info("water", "Water")
    },
    Info {
        shape: Shape::Liquid,
        solid: false,
        cull: false,
        light_block: 15,
        emit: 15,
        hardness: -1.0,
        drop: Drop::Nothing,
        tex: [tex::LAVA; 4],
        ..info("lava", "Lava")
    },
    cube("sand", "Sand", tex::SAND, 0.5, Tool::Shovel, 0, Sound::Sand),
    cube("gravel", "Gravel", tex::GRAVEL, 0.6, Tool::Shovel, 0, Sound::Gravel),
    cube("gold_ore", "Gold Ore", tex::GOLD_ORE, 3.0, Tool::Pickaxe, 3, Sound::Stone),
    cube("iron_ore", "Iron Ore", tex::IRON_ORE, 3.0, Tool::Pickaxe, 2, Sound::Stone),
    drops(cube("coal_ore", "Coal Ore", tex::COAL_ORE, 3.0, Tool::Pickaxe, 1, Sound::Stone), Drop::Item(it::COAL, 1)),
    drops(cube("diamond_ore", "Diamond Ore", tex::DIAMOND_ORE, 3.0, Tool::Pickaxe, 3, Sound::Stone), Drop::Item(it::DIAMOND, 1)),
    tsb(cube("oak_log", "Oak Log", 0, 2.0, Tool::Axe, 0, Sound::Wood), tex::OAK_LOG_TOP, tex::OAK_LOG, tex::OAK_LOG_TOP),
    drops(see_through(cube("oak_leaves", "Oak Leaves", tex::OAK_LEAVES, 0.2, Tool::Any, 0, Sound::Plant), Pass::Solid, 1), Drop::Nothing),
    tsb(cube("birch_log", "Birch Log", 0, 2.0, Tool::Axe, 0, Sound::Wood), tex::BIRCH_LOG_TOP, tex::BIRCH_LOG, tex::BIRCH_LOG_TOP),
    drops(see_through(cube("birch_leaves", "Birch Leaves", tex::BIRCH_LEAVES, 0.2, Tool::Any, 0, Sound::Plant), Pass::Solid, 1), Drop::Nothing),
    tsb(cube("spruce_log", "Spruce Log", 0, 2.0, Tool::Axe, 0, Sound::Wood), tex::SPRUCE_LOG_TOP, tex::SPRUCE_LOG, tex::SPRUCE_LOG_TOP),
    drops(see_through(cube("spruce_leaves", "Spruce Leaves", tex::SPRUCE_LEAVES, 0.2, Tool::Any, 0, Sound::Plant), Pass::Solid, 1), Drop::Nothing),
    drops(see_through(cube("glass", "Glass", tex::GLASS, 0.3, Tool::Any, 0, Sound::Glass), Pass::Glass, 0), Drop::Nothing),
    tsb(cube("sandstone", "Sandstone", 0, 0.8, Tool::Pickaxe, 1, Sound::Stone), tex::SANDSTONE_TOP, tex::SANDSTONE, tex::SANDSTONE_TOP),
    cube("bricks", "Bricks", tex::BRICKS, 2.0, Tool::Pickaxe, 1, Sound::Stone),
    cube("stone_bricks", "Stone Bricks", tex::STONE_BRICKS, 1.5, Tool::Pickaxe, 1, Sound::Stone),
    tsb(cube("crafting_table", "Crafting Table", 0, 2.5, Tool::Axe, 0, Sound::Wood), tex::TABLE_TOP, tex::TABLE_SIDE, tex::OAK_PLANKS),
    cube("snow", "Snow Block", tex::SNOW, 0.2, Tool::Shovel, 0, Sound::Snow),
    drops(
        tsb(cube("snowy_grass_block", "Snowy Grass Block", 0, 0.6, Tool::Shovel, 0, Sound::Snow), tex::SNOW, tex::SNOWY_GRASS_SIDE, tex::DIRT),
        Drop::Item(3, 1),
    ),
    drops(see_through(cube("ice", "Ice", tex::ICE, 0.5, Tool::Pickaxe, 0, Sound::Glass), Pass::Water, 1), Drop::Nothing),
    Info {
        shape: Shape::Cactus,
        cull: false,
        light_block: 0,
        ..tsb(cube("cactus", "Cactus", 0, 0.4, Tool::Any, 0, Sound::Plant), tex::CACTUS_TOP, tex::CACTUS_SIDE, tex::CACTUS_TOP)
    },
    cube("clay", "Clay", tex::CLAY, 0.6, Tool::Shovel, 0, Sound::Gravel),
    cube("obsidian", "Obsidian", tex::OBSIDIAN, 50.0, Tool::Pickaxe, 4, Sound::Stone),
    tsb(cube("bookshelf", "Bookshelf", 0, 1.5, Tool::Axe, 0, Sound::Wood), tex::OAK_PLANKS, tex::BOOKSHELF, tex::OAK_PLANKS),
    cube("birch_planks", "Birch Planks", tex::BIRCH_PLANKS, 2.0, Tool::Axe, 0, Sound::Wood),
    cube("spruce_planks", "Spruce Planks", tex::SPRUCE_PLANKS, 2.0, Tool::Axe, 0, Sound::Wood),
    cube("iron_block", "Block of Iron", tex::IRON_BLOCK, 5.0, Tool::Pickaxe, 2, Sound::Stone),
    cube("gold_block", "Block of Gold", tex::GOLD_BLOCK, 3.0, Tool::Pickaxe, 3, Sound::Stone),
    cube("diamond_block", "Block of Diamond", tex::DIAMOND_BLOCK, 5.0, Tool::Pickaxe, 3, Sound::Stone),
    cube("coal_block", "Block of Coal", tex::COAL_BLOCK, 5.0, Tool::Pickaxe, 1, Sound::Stone),
    cube("mossy_cobblestone", "Mossy Cobblestone", tex::MOSSY_COBBLE, 2.0, Tool::Pickaxe, 1, Sound::Stone),
    tsb(cube("tnt", "TNT", 0, 0.0, Tool::Any, 0, Sound::Plant), tex::TNT_TOP, tex::TNT_SIDE, tex::TNT_BOTTOM),
    plant("dandelion", "Dandelion", Plant::Dandelion, Drop::Itself),
    plant("poppy", "Poppy", Plant::Poppy, Drop::Itself),
    plant("cornflower", "Cornflower", Plant::Cornflower, Drop::Itself),
    plant("tall_grass", "Grass", Plant::TallGrass, Drop::Nothing),
    plant("dead_bush", "Dead Bush", Plant::DeadBush, Drop::Item(it::STICK, 1)),
    torch(None),
    drops(torch(Some(0)), Drop::Item(46, 1)),
    drops(torch(Some(1)), Drop::Item(46, 1)),
    drops(torch(Some(2)), Drop::Item(46, 1)),
    drops(torch(Some(3)), Drop::Item(46, 1)),
    faced("furnace", "Furnace", tex::FURNACE_TOP, tex::FURNACE_SIDE, tex::FURNACE_FRONT, 3.5, Tool::Pickaxe, 1, Sound::Stone),
    drops(faced("furnace", "Furnace", tex::FURNACE_TOP, tex::FURNACE_SIDE, tex::FURNACE_FRONT, 3.5, Tool::Pickaxe, 1, Sound::Stone), Drop::Item(51, 1)),
    drops(faced("furnace", "Furnace", tex::FURNACE_TOP, tex::FURNACE_SIDE, tex::FURNACE_FRONT, 3.5, Tool::Pickaxe, 1, Sound::Stone), Drop::Item(51, 1)),
    drops(faced("furnace", "Furnace", tex::FURNACE_TOP, tex::FURNACE_SIDE, tex::FURNACE_FRONT, 3.5, Tool::Pickaxe, 1, Sound::Stone), Drop::Item(51, 1)),
    drops(
        lit(faced("lit_furnace", "Furnace", tex::FURNACE_TOP, tex::FURNACE_SIDE, tex::FURNACE_FRONT_LIT, 3.5, Tool::Pickaxe, 1, Sound::Stone), 13),
        Drop::Item(51, 1),
    ),
    drops(
        lit(faced("lit_furnace", "Furnace", tex::FURNACE_TOP, tex::FURNACE_SIDE, tex::FURNACE_FRONT_LIT, 3.5, Tool::Pickaxe, 1, Sound::Stone), 13),
        Drop::Item(51, 1),
    ),
    drops(
        lit(faced("lit_furnace", "Furnace", tex::FURNACE_TOP, tex::FURNACE_SIDE, tex::FURNACE_FRONT_LIT, 3.5, Tool::Pickaxe, 1, Sound::Stone), 13),
        Drop::Item(51, 1),
    ),
    drops(
        lit(faced("lit_furnace", "Furnace", tex::FURNACE_TOP, tex::FURNACE_SIDE, tex::FURNACE_FRONT_LIT, 3.5, Tool::Pickaxe, 1, Sound::Stone), 13),
        Drop::Item(51, 1),
    ),
    faced("chest", "Chest", tex::CHEST_TOP, tex::CHEST_SIDE, tex::CHEST_FRONT, 2.5, Tool::Axe, 0, Sound::Wood),
    drops(faced("chest", "Chest", tex::CHEST_TOP, tex::CHEST_SIDE, tex::CHEST_FRONT, 2.5, Tool::Axe, 0, Sound::Wood), Drop::Item(59, 1)),
    drops(faced("chest", "Chest", tex::CHEST_TOP, tex::CHEST_SIDE, tex::CHEST_FRONT, 2.5, Tool::Axe, 0, Sound::Wood), Drop::Item(59, 1)),
    drops(faced("chest", "Chest", tex::CHEST_TOP, tex::CHEST_SIDE, tex::CHEST_FRONT, 2.5, Tool::Axe, 0, Sound::Wood), Drop::Item(59, 1)),
    Info {
        shape: Shape::Liquid,
        solid: false,
        cull: false,
        light_block: 2,
        hardness: -1.0,
        drop: Drop::Nothing,
        tex: [tex::WATER; 4],
        pass: Pass::Water,
        ..info("flowing_water", "Water")
    },
    Info {
        shape: Shape::Liquid,
        solid: false,
        cull: false,
        light_block: 2,
        hardness: -1.0,
        drop: Drop::Nothing,
        tex: [tex::WATER; 4],
        pass: Pass::Water,
        ..info("flowing_water", "Water")
    },
    Info {
        shape: Shape::Liquid,
        solid: false,
        cull: false,
        light_block: 2,
        hardness: -1.0,
        drop: Drop::Nothing,
        tex: [tex::WATER; 4],
        pass: Pass::Water,
        ..info("flowing_water", "Water")
    },
    Info {
        shape: Shape::Liquid,
        solid: false,
        cull: false,
        light_block: 2,
        hardness: -1.0,
        drop: Drop::Nothing,
        tex: [tex::WATER; 4],
        pass: Pass::Water,
        ..info("flowing_water", "Water")
    },
    Info {
        shape: Shape::Liquid,
        solid: false,
        cull: false,
        light_block: 2,
        hardness: -1.0,
        drop: Drop::Nothing,
        tex: [tex::WATER; 4],
        pass: Pass::Water,
        ..info("flowing_water", "Water")
    },
    Info {
        shape: Shape::Liquid,
        solid: false,
        cull: false,
        light_block: 2,
        hardness: -1.0,
        drop: Drop::Nothing,
        tex: [tex::WATER; 4],
        pass: Pass::Water,
        ..info("flowing_water", "Water")
    },
    Info {
        shape: Shape::Liquid,
        solid: false,
        cull: false,
        light_block: 2,
        hardness: -1.0,
        drop: Drop::Nothing,
        tex: [tex::WATER; 4],
        pass: Pass::Water,
        ..info("flowing_water", "Water")
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_id_names_itself_and_the_named_kinds_sit_where_their_constants_say() {
        for (i, info) in INFO.iter().enumerate() {
            assert!(!info.name.is_empty() && !info.title.is_empty(), "block {i}");
        }
        assert_eq!(Block::by_name("stone"), Some(Block::STONE));
        assert_eq!(Block::TORCH.info().name, "torch");
        assert_eq!(Block(Block::WALL_TORCH.0 + 3).info().shape, Shape::Torch(Some(3)));
        assert_eq!(Block::FURNACE.info().name, "furnace");
        assert_eq!(Block(Block::FURNACE_LIT.0 + 2).info().emit, 13);
        assert_eq!(Block::CHEST.info().name, "chest");
        assert_eq!(Block(Block::CHEST.0 + 3).info().name, "chest");
        assert_eq!(Block::DEAD_BUSH.info().name, "dead_bush");
        assert_eq!(Block::TNT.info().name, "tnt");
        assert_eq!(Block::flowing(7).water_level(), Some(7));
        assert!(Block::flowing(3).is_water() && Block::flowing(3).replaceable());
        assert_eq!(Block(Block::COUNT as u8 - 1), Block::flowing(7));
    }

    #[test]
    fn a_facing_block_shows_its_front_one_way_and_comes_back_as_its_base() {
        let south = Block::faced(Block::FURNACE, 2);
        assert_eq!(south.face_tex(4), tex::FURNACE_FRONT, "south faces +Z");
        assert_eq!(south.face_tex(5), tex::FURNACE_SIDE);
        assert_eq!(south.face_tex(2), tex::FURNACE_TOP);
        assert_eq!(Block::faced(Block::FURNACE_LIT, 1).base(), Block::FURNACE);
        assert_eq!(Block::faced(Block::WALL_TORCH, 0).base(), Block::TORCH);
        assert_eq!(Block::GRASS.face_tex(2), tex::GRASS_TOP);
        assert_eq!(Block::GRASS.face_tex(0), tex::GRASS_SIDE);
        assert_eq!(Block::GRASS.face_tex(3), tex::DIRT);
    }

    #[test]
    fn textures_fit_their_ranges() {
        for info in INFO.iter() {
            for t in info.tex {
                assert!(t <= tex::LAST || (128..=tex::LAST_FLAT).contains(&t), "{} has texture {t}", info.name);
            }
        }
    }
}
