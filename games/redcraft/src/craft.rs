//! Crafting and smelting: the recipes, as the original has them (shapes on a 2 x 2 or 3 x 3 grid, mirrored shapes counting too, any kind of
//! planks or logs where the original takes any), and what a furnace makes of what and how long its fuel burns.

use crate::block::Block;
use crate::item::{id, Item, Stack};

/// What may fill a cell of a recipe.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Is(Item),
    AnyPlanks,
    AnyLog,
}

impl Key {
    fn fits(self, i: Item) -> bool {
        match self {
            Key::Is(want) => want == i,
            Key::AnyPlanks => matches!(i.block(), Some(Block::OAK_PLANKS | Block::BIRCH_PLANKS | Block::SPRUCE_PLANKS)),
            Key::AnyLog => matches!(i.block(), Some(Block::OAK_LOG | Block::BIRCH_LOG | Block::SPRUCE_LOG)),
        }
    }
}

/// A recipe: rows of the pattern (`.` an empty cell; any other character a key), the keys, and what it makes.
pub struct Recipe {
    pub rows: &'static [&'static str],
    pub keys: &'static [(char, Key)],
    pub out: Item,
    pub count: u8,
}

const fn b(block: Block) -> Item {
    Item(block.0 as u16)
}

const P: (char, Key) = ('P', Key::AnyPlanks);
const S: (char, Key) = ('S', Key::Is(Item(id::STICK)));

macro_rules! tools {
    ($m:expr, $pick:expr, $axe:expr, $shovel:expr, $sword:expr) => {
        [
            Recipe { rows: &["MMM", ".S.", ".S."], keys: &[('M', $m), S], out: Item($pick), count: 1 },
            Recipe { rows: &["MM", "MS", ".S"], keys: &[('M', $m), S], out: Item($axe), count: 1 },
            Recipe { rows: &["M", "S", "S"], keys: &[('M', $m), S], out: Item($shovel), count: 1 },
            Recipe { rows: &["M", "M", "S"], keys: &[('M', $m), S], out: Item($sword), count: 1 },
        ]
    };
}

const WOOD_TOOLS: [Recipe; 4] = tools!(Key::AnyPlanks, id::WOODEN_PICKAXE, id::WOODEN_AXE, id::WOODEN_SHOVEL, id::WOODEN_SWORD);
const STONE_TOOLS: [Recipe; 4] = tools!(Key::Is(b(Block::COBBLESTONE)), id::STONE_PICKAXE, id::STONE_AXE, id::STONE_SHOVEL, id::STONE_SWORD);
const IRON_TOOLS: [Recipe; 4] = tools!(Key::Is(Item(id::IRON_INGOT)), id::IRON_PICKAXE, id::IRON_AXE, id::IRON_SHOVEL, id::IRON_SWORD);
const DIAMOND_TOOLS: [Recipe; 4] = tools!(Key::Is(Item(id::DIAMOND)), id::DIAMOND_PICKAXE, id::DIAMOND_AXE, id::DIAMOND_SHOVEL, id::DIAMOND_SWORD);

const OTHER: &[Recipe] = &[
    Recipe { rows: &["L"], keys: &[('L', Key::Is(b(Block::OAK_LOG)))], out: b(Block::OAK_PLANKS), count: 4 },
    Recipe { rows: &["L"], keys: &[('L', Key::Is(b(Block::BIRCH_LOG)))], out: b(Block::BIRCH_PLANKS), count: 4 },
    Recipe { rows: &["L"], keys: &[('L', Key::Is(b(Block::SPRUCE_LOG)))], out: b(Block::SPRUCE_PLANKS), count: 4 },
    Recipe { rows: &["P", "P"], keys: &[P], out: Item(id::STICK), count: 4 },
    Recipe { rows: &["PP", "PP"], keys: &[P], out: b(Block::CRAFTING_TABLE), count: 1 },
    Recipe { rows: &["C", "S"], keys: &[('C', Key::Is(Item(id::COAL))), S], out: b(Block::TORCH), count: 4 },
    Recipe { rows: &["CCC", "C.C", "CCC"], keys: &[('C', Key::Is(b(Block::COBBLESTONE)))], out: b(Block::FURNACE), count: 1 },
    Recipe { rows: &["PPP", "P.P", "PPP"], keys: &[P], out: b(Block::CHEST), count: 1 },
    Recipe { rows: &["SS", "SS"], keys: &[('S', Key::Is(b(Block::SAND)))], out: b(Block::SANDSTONE), count: 1 },
    Recipe { rows: &["SS", "SS"], keys: &[('S', Key::Is(b(Block::STONE)))], out: b(Block::STONE_BRICKS), count: 4 },
    Recipe { rows: &["SS", "SS"], keys: &[('S', Key::Is(b(Block::SNOW)))], out: b(Block::SNOW), count: 4 },
    Recipe { rows: &["SCS", "CSC", "SCS"], keys: &[('S', Key::Is(b(Block::SAND))), ('C', Key::Is(Item(id::COAL)))], out: b(Block::TNT), count: 1 },
    Recipe { rows: &["PPP", "LLL", "PPP"], keys: &[P, ('L', Key::AnyLog)], out: b(Block::BOOKSHELF), count: 1 },
    Recipe { rows: &["IF"], keys: &[('I', Key::Is(Item(id::IRON_INGOT))), ('F', Key::Is(Item(id::FLINT)))], out: Item(id::FLINT_AND_STEEL), count: 1 },
    Recipe { rows: &["III", "III", "III"], keys: &[('I', Key::Is(Item(id::IRON_INGOT)))], out: b(Block::IRON_BLOCK), count: 1 },
    Recipe { rows: &["III", "III", "III"], keys: &[('I', Key::Is(Item(id::GOLD_INGOT)))], out: b(Block::GOLD_BLOCK), count: 1 },
    Recipe { rows: &["III", "III", "III"], keys: &[('I', Key::Is(Item(id::DIAMOND)))], out: b(Block::DIAMOND_BLOCK), count: 1 },
    Recipe { rows: &["III", "III", "III"], keys: &[('I', Key::Is(Item(id::COAL)))], out: b(Block::COAL_BLOCK), count: 1 },
    Recipe { rows: &["B"], keys: &[('B', Key::Is(b(Block::IRON_BLOCK)))], out: Item(id::IRON_INGOT), count: 9 },
    Recipe { rows: &["B"], keys: &[('B', Key::Is(b(Block::GOLD_BLOCK)))], out: Item(id::GOLD_INGOT), count: 9 },
    Recipe { rows: &["B"], keys: &[('B', Key::Is(b(Block::DIAMOND_BLOCK)))], out: Item(id::DIAMOND), count: 9 },
    Recipe { rows: &["B"], keys: &[('B', Key::Is(b(Block::COAL_BLOCK)))], out: Item(id::COAL), count: 9 },
    Recipe { rows: &["CV"], keys: &[('C', Key::Is(b(Block::COBBLESTONE))), ('V', Key::Is(b(Block::OAK_LEAVES)))], out: b(Block::MOSSY_COBBLESTONE), count: 1 },
];

/// Every recipe.
pub fn recipes() -> impl Iterator<Item = &'static Recipe> {
    OTHER.iter().chain(WOOD_TOOLS.iter()).chain(STONE_TOOLS.iter()).chain(IRON_TOOLS.iter()).chain(DIAMOND_TOOLS.iter())
}

/// The occupied part of a `w` x `w` grid: its top-left cell and size.
fn trimmed(grid: &[Stack], w: usize) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, w, 0, 0);
    for y in 0..w {
        for x in 0..w {
            if !grid[y * w + x].is_empty() {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    (x0 <= x1).then(|| (x0, y0, x1 - x0 + 1, y1 - y0 + 1))
}

fn matches(r: &Recipe, grid: &[Stack], w: usize, x0: usize, y0: usize, mirror: bool) -> bool {
    let rh = r.rows.len();
    let rw = r.rows[0].len();
    for y in 0..rh {
        let row = r.rows[y].as_bytes();
        for x in 0..rw {
            let ch = row[if mirror { rw - 1 - x } else { x }] as char;
            let cell = &grid[(y0 + y) * w + x0 + x];
            if ch == '.' {
                if !cell.is_empty() {
                    return false;
                }
                continue;
            }
            let Some((_, key)) = r.keys.iter().find(|(c, _)| *c == ch) else { return false };
            if cell.is_empty() || !key.fits(cell.item) {
                return false;
            }
        }
    }
    true
}

/// What a crafting grid (`w` 2 or 3, row by row) makes now.
pub fn craft(grid: &[Stack], w: usize) -> Option<Stack> {
    let (x0, y0, gw, gh) = trimmed(grid, w)?;
    for r in recipes() {
        if r.rows.len() != gh || r.rows[0].len() != gw {
            continue;
        }
        if matches(r, grid, w, x0, y0, false) || matches(r, grid, w, x0, y0, true) {
            return Some(Stack::new(r.out, r.count));
        }
    }
    None
}

/// Takes one of everything in the grid (a craft was taken).
pub fn consume(grid: &mut [Stack]) {
    for s in grid.iter_mut() {
        if !s.is_empty() {
            s.count -= 1;
            if s.count == 0 {
                *s = Stack::EMPTY;
            }
        }
    }
}

/// What a furnace makes of `input`.
pub fn smelt(input: Item) -> Option<Item> {
    let out = match input.block() {
        Some(Block::IRON_ORE) => Item(id::IRON_INGOT),
        Some(Block::GOLD_ORE) => Item(id::GOLD_INGOT),
        Some(Block::SAND) => b(Block::GLASS),
        Some(Block::COBBLESTONE) => b(Block::STONE),
        Some(Block::CLAY) => b(Block::BRICKS),
        Some(Block::OAK_LOG | Block::BIRCH_LOG | Block::SPRUCE_LOG) => Item(id::COAL),
        Some(Block::CACTUS) => b(Block::CACTUS),
        _ => match input.0 {
            id::PORKCHOP => Item(id::COOKED_PORKCHOP),
            _ => return None,
        },
    };
    (out != input).then_some(out)
}

/// Seconds a fuel burns (a furnace smelts one item in [`SMELT_SECS`]).
pub fn burn_secs(fuel: Item) -> Option<f32> {
    if let Some(t) = fuel.tool() {
        return (t.tier == 1).then_some(10.0);
    }
    match fuel.block() {
        Some(Block::COAL_BLOCK) => Some(800.0),
        Some(
            Block::OAK_LOG
            | Block::BIRCH_LOG
            | Block::SPRUCE_LOG
            | Block::OAK_PLANKS
            | Block::BIRCH_PLANKS
            | Block::SPRUCE_PLANKS
            | Block::CRAFTING_TABLE
            | Block::CHEST
            | Block::BOOKSHELF,
        ) => Some(15.0),
        Some(Block::DEAD_BUSH) => Some(5.0),
        _ => match fuel.0 {
            id::COAL => Some(80.0),
            id::STICK => Some(5.0),
            _ => None,
        },
    }
}

/// Seconds to smelt one item.
pub const SMELT_SECS: f32 = 10.0;

/// A furnace's slots and fire.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Furnace {
    pub input: Stack,
    pub fuel: Stack,
    pub output: Stack,
    /// Seconds of fire left, and how long the current fuel burns in all.
    pub burn: f32,
    pub burn_total: f32,
    /// Seconds the current item has cooked.
    pub cook: f32,
}

impl Furnace {
    /// Whether it can work now: something to smelt, and room for what it makes.
    fn can_smelt(&self) -> bool {
        let Some(out) = (!self.input.is_empty()).then(|| smelt(self.input.item)).flatten() else { return false };
        self.output.is_empty() || (self.output.item == out && self.output.count < out.max_stack())
    }

    /// Runs `dt` seconds; returns whether it is burning.
    pub fn tick(&mut self, dt: f32) -> bool {
        let mut left = dt;
        while left > 0.0 {
            if self.burn <= 0.0 {
                if !self.can_smelt() {
                    self.cook = 0.0;
                    return false;
                }
                let Some(secs) = (!self.fuel.is_empty()).then(|| burn_secs(self.fuel.item)).flatten() else {
                    self.cook = 0.0;
                    return false;
                };
                self.burn = secs;
                self.burn_total = secs;
                self.fuel.count -= 1;
                if self.fuel.count == 0 {
                    self.fuel = Stack::EMPTY;
                }
            }
            let step = left.min(self.burn).min(SMELT_SECS - self.cook).max(1e-4);
            self.burn -= step;
            left -= step;
            if self.can_smelt() {
                self.cook += step;
                if self.cook >= SMELT_SECS - 1e-4 {
                    self.cook = 0.0;
                    let out = smelt(self.input.item).unwrap_or(Item::NONE);
                    self.input.count -= 1;
                    if self.input.count == 0 {
                        self.input = Stack::EMPTY;
                    }
                    if self.output.is_empty() {
                        self.output = Stack::new(out, 1);
                    } else {
                        self.output.count += 1;
                    }
                }
            } else {
                self.cook = 0.0;
            }
        }
        self.burn > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(w: usize, cells: &[(usize, Item)]) -> Vec<Stack> {
        let mut g = vec![Stack::EMPTY; w * w];
        for (i, it) in cells {
            g[*i] = Stack::new(*it, 1);
        }
        g
    }

    #[test]
    fn logs_planks_sticks_and_a_table_in_the_hand_grid() {
        let g = grid(2, &[(3, b(Block::BIRCH_LOG))]);
        assert_eq!(craft(&g, 2), Some(Stack::new(b(Block::BIRCH_PLANKS), 4)));
        let g = grid(2, &[(1, b(Block::OAK_PLANKS)), (3, b(Block::SPRUCE_PLANKS))]);
        assert_eq!(craft(&g, 2), Some(Stack::new(Item(id::STICK), 4)), "any planks, anywhere in the grid");
        let g = grid(2, &[(0, b(Block::OAK_PLANKS)), (1, b(Block::OAK_PLANKS)), (2, b(Block::OAK_PLANKS)), (3, b(Block::OAK_PLANKS))]);
        assert_eq!(craft(&g, 2), Some(Stack::new(b(Block::CRAFTING_TABLE), 1)));
        let mut g = grid(2, &[(0, b(Block::OAK_PLANKS)), (1, b(Block::OAK_PLANKS))]);
        assert_eq!(craft(&g, 2), None, "two planks side by side are nothing");
        g[0].count = 2;
        consume(&mut g);
        assert_eq!((g[0].count, g[1].is_empty()), (1, true));
    }

    #[test]
    fn tools_need_the_table_and_mirror() {
        let s = Item(id::STICK);
        let c = b(Block::COBBLESTONE);
        let pick = grid(3, &[(0, c), (1, c), (2, c), (4, s), (7, s)]);
        assert_eq!(craft(&pick, 3).map(|s| s.item), Some(Item(id::STONE_PICKAXE)));
        let axe = grid(3, &[(1, c), (2, c), (4, s), (5, c), (7, s)]);
        assert_eq!(craft(&axe, 3).map(|s| s.item), Some(Item(id::STONE_AXE)), "the axe the other way round");
        let d = Item(id::DIAMOND);
        let sword = grid(3, &[(2, d), (5, d), (8, s)]);
        assert_eq!(craft(&sword, 3).map(|s| s.item), Some(Item(id::DIAMOND_SWORD)));
        let furnace = grid(3, &[(0, c), (1, c), (2, c), (3, c), (5, c), (6, c), (7, c), (8, c)]);
        assert_eq!(craft(&furnace, 3).map(|s| s.item), Some(b(Block::FURNACE)));
        let torch = grid(3, &[(4, Item(id::COAL)), (7, s)]);
        assert_eq!(craft(&torch, 3), Some(Stack::new(b(Block::TORCH), 4)));
    }

    #[test]
    fn a_furnace_smelts_with_fuel_and_stops_without() {
        let mut f = Furnace { input: Stack::new(b(Block::IRON_ORE), 3), fuel: Stack::new(Item(id::STICK), 1), ..Default::default() };
        assert!(f.tick(1.0));
        f.tick(4.5);
        assert_eq!(f.output.count, 0, "a stick burns 5 s: not enough for one ingot");
        assert!(!f.tick(1.0));
        f.fuel = Stack::new(Item(id::COAL), 1);
        f.tick(30.0);
        assert_eq!(f.output, Stack::new(Item(id::IRON_INGOT), 3));
        assert!(f.input.is_empty());
        assert!(f.burn > 0.0, "the coal still burns");
        assert_eq!(smelt(b(Block::SAND)), Some(b(Block::GLASS)));
        assert_eq!(smelt(Item(id::STICK)), None);
    }

    #[test]
    fn every_recipe_makes_a_real_item_with_well_formed_rows() {
        for r in recipes() {
            assert!(r.out.0 != 0 && r.out.title() != "?", "{:?}", r.out);
            let w = r.rows[0].len();
            assert!(r.rows.iter().all(|row| row.len() == w) && w <= 3 && r.rows.len() <= 3);
            for row in r.rows {
                for ch in row.chars().filter(|c| *c != '.') {
                    assert!(r.keys.iter().any(|(k, _)| *k == ch), "{ch} has no key");
                }
            }
        }
    }
}
