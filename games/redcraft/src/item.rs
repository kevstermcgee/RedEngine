//! Items: everything a slot can hold. Ids below 256 are blocks (the block's own id: a placed item becomes that block); 256 and up are things that
//! are not blocks: materials, tools, food.

use crate::block::{Block, Tool};

/// Item ids that are not blocks.
pub mod id {
    pub const STICK: u16 = 256;
    pub const COAL: u16 = 257;
    pub const IRON_INGOT: u16 = 258;
    pub const GOLD_INGOT: u16 = 259;
    pub const DIAMOND: u16 = 260;
    pub const APPLE: u16 = 261;
    pub const FLINT: u16 = 262;
    pub const FLINT_AND_STEEL: u16 = 263;
    pub const PORKCHOP: u16 = 264;
    pub const COOKED_PORKCHOP: u16 = 265;
    pub const ROTTEN_FLESH: u16 = 266;
    pub const BREAD: u16 = 267;
    pub const WOODEN_PICKAXE: u16 = 270;
    pub const STONE_PICKAXE: u16 = 271;
    pub const IRON_PICKAXE: u16 = 272;
    pub const DIAMOND_PICKAXE: u16 = 273;
    pub const WOODEN_AXE: u16 = 274;
    pub const STONE_AXE: u16 = 275;
    pub const IRON_AXE: u16 = 276;
    pub const DIAMOND_AXE: u16 = 277;
    pub const WOODEN_SHOVEL: u16 = 278;
    pub const STONE_SHOVEL: u16 = 279;
    pub const IRON_SHOVEL: u16 = 280;
    pub const DIAMOND_SHOVEL: u16 = 281;
    pub const WOODEN_SWORD: u16 = 282;
    pub const STONE_SWORD: u16 = 283;
    pub const IRON_SWORD: u16 = 284;
    pub const DIAMOND_SWORD: u16 = 285;
}

/// What kind of tool an item is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolKind {
    Pickaxe,
    Axe,
    Shovel,
    Sword,
}

/// A tool: its kind and tier (1 wood, 2 stone, 3 iron, 4 diamond).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ToolInfo {
    pub kind: ToolKind,
    pub tier: u8,
}

impl ToolInfo {
    /// How much faster than a hand it mines what it is for.
    pub fn speed(self) -> f32 {
        [1.0, 2.0, 4.0, 6.0, 8.0][self.tier.min(4) as usize]
    }

    /// Uses before it breaks.
    pub fn durability(self) -> u16 {
        [0, 59, 131, 250, 1561][self.tier.min(4) as usize]
    }

    /// Damage to a mob, in half hearts.
    pub fn damage(self) -> f32 {
        let base = match self.kind {
            ToolKind::Sword => 4.0,
            ToolKind::Axe => 3.0,
            ToolKind::Pickaxe => 2.0,
            ToolKind::Shovel => 1.5,
        };
        base + self.tier as f32
    }
}

/// An item kind.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, PartialOrd, Ord)]
pub struct Item(pub u16);

impl Item {
    pub const NONE: Item = Item(0);

    pub fn of(block: Block) -> Item {
        Item(block.0 as u16)
    }

    /// The block it places, if it is a block.
    pub fn block(self) -> Option<Block> {
        (self.0 > 0 && self.0 < Block::COUNT as u16).then_some(Block(self.0 as u8))
    }

    /// The tool it is.
    pub fn tool(self) -> Option<ToolInfo> {
        let (kind, first) = match self.0 {
            id::WOODEN_PICKAXE..=id::DIAMOND_PICKAXE => (ToolKind::Pickaxe, id::WOODEN_PICKAXE),
            id::WOODEN_AXE..=id::DIAMOND_AXE => (ToolKind::Axe, id::WOODEN_AXE),
            id::WOODEN_SHOVEL..=id::DIAMOND_SHOVEL => (ToolKind::Shovel, id::WOODEN_SHOVEL),
            id::WOODEN_SWORD..=id::DIAMOND_SWORD => (ToolKind::Sword, id::WOODEN_SWORD),
            _ => return None,
        };
        Some(ToolInfo { kind, tier: (self.0 - first) as u8 + 1 })
    }

    /// How many fit in one slot.
    pub fn max_stack(self) -> u8 {
        if self.tool().is_some() || self.0 == id::FLINT_AND_STEEL {
            1
        } else {
            64
        }
    }

    /// How much health eating it gives back (half hearts), if it is food.
    pub fn food(self) -> Option<f32> {
        match self.0 {
            id::APPLE => Some(4.0),
            id::PORKCHOP => Some(3.0),
            id::COOKED_PORKCHOP => Some(8.0),
            id::ROTTEN_FLESH => Some(2.0),
            id::BREAD => Some(5.0),
            _ => None,
        }
    }

    /// What the player reads.
    pub fn title(self) -> &'static str {
        if let Some(b) = self.block() {
            return b.info().title;
        }
        NAMES.iter().find(|(i, _, _)| *i == self.0).map_or("?", |(_, _, t)| t)
    }

    /// Its id in saves.
    pub fn name(self) -> &'static str {
        if let Some(b) = self.block() {
            return b.info().name;
        }
        NAMES.iter().find(|(i, _, _)| *i == self.0).map_or("unknown", |(_, n, _)| n)
    }

    pub fn by_name(name: &str) -> Option<Item> {
        Block::by_name(name).map(Item::of).or_else(|| NAMES.iter().find(|(_, n, _)| *n == name).map(|(i, _, _)| Item(*i)))
    }

    /// Every item a player can hold, blocks first (for the creative inventory and tests).
    pub fn all() -> Vec<Item> {
        let mut v: Vec<Item> = (1..Block::COUNT as u8)
            .map(Block)
            .filter(|b| b.base() == *b && !b.is_liquid() && *b != Block::BEDROCK && b.facing().is_none_or(|(_, f)| f == 0))
            .map(Item::of)
            .collect();
        v.push(Item::of(Block::BEDROCK));
        v.extend(NAMES.iter().map(|(i, _, _)| Item(*i)));
        v
    }
}

/// Non-block items: id, save name, title.
pub const NAMES: &[(u16, &str, &str)] = &[
    (id::STICK, "stick", "Stick"),
    (id::COAL, "coal", "Coal"),
    (id::IRON_INGOT, "iron_ingot", "Iron Ingot"),
    (id::GOLD_INGOT, "gold_ingot", "Gold Ingot"),
    (id::DIAMOND, "diamond", "Diamond"),
    (id::APPLE, "apple", "Apple"),
    (id::FLINT, "flint", "Flint"),
    (id::FLINT_AND_STEEL, "flint_and_steel", "Flint and Steel"),
    (id::PORKCHOP, "porkchop", "Raw Porkchop"),
    (id::COOKED_PORKCHOP, "cooked_porkchop", "Cooked Porkchop"),
    (id::ROTTEN_FLESH, "rotten_flesh", "Rotten Flesh"),
    (id::BREAD, "bread", "Bread"),
    (id::WOODEN_PICKAXE, "wooden_pickaxe", "Wooden Pickaxe"),
    (id::STONE_PICKAXE, "stone_pickaxe", "Stone Pickaxe"),
    (id::IRON_PICKAXE, "iron_pickaxe", "Iron Pickaxe"),
    (id::DIAMOND_PICKAXE, "diamond_pickaxe", "Diamond Pickaxe"),
    (id::WOODEN_AXE, "wooden_axe", "Wooden Axe"),
    (id::STONE_AXE, "stone_axe", "Stone Axe"),
    (id::IRON_AXE, "iron_axe", "Iron Axe"),
    (id::DIAMOND_AXE, "diamond_axe", "Diamond Axe"),
    (id::WOODEN_SHOVEL, "wooden_shovel", "Wooden Shovel"),
    (id::STONE_SHOVEL, "stone_shovel", "Stone Shovel"),
    (id::IRON_SHOVEL, "iron_shovel", "Iron Shovel"),
    (id::DIAMOND_SHOVEL, "diamond_shovel", "Diamond Shovel"),
    (id::WOODEN_SWORD, "wooden_sword", "Wooden Sword"),
    (id::STONE_SWORD, "stone_sword", "Stone Sword"),
    (id::IRON_SWORD, "iron_sword", "Iron Sword"),
    (id::DIAMOND_SWORD, "diamond_sword", "Diamond Sword"),
];

/// A slot's contents: an item, how many, and (a tool) how worn it is.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Stack {
    pub item: Item,
    pub count: u8,
    /// Uses so far (tools).
    pub wear: u16,
}

impl Stack {
    pub const EMPTY: Stack = Stack { item: Item::NONE, count: 0, wear: 0 };

    pub fn new(item: Item, count: u8) -> Stack {
        Stack { item, count, wear: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0 || self.item == Item::NONE
    }

    /// Whether `other` can be added to this stack.
    pub fn same_kind(&self, other: &Stack) -> bool {
        self.item == other.item && self.wear == other.wear
    }
}

/// Seconds to mine `block` holding `held` (by hand when `None`), and whether it then drops anything; `None` when it cannot be mined at all.
pub fn mining(block: Block, held: Option<Item>) -> Option<(f32, bool)> {
    let info = block.info();
    if info.hardness < 0.0 {
        return None;
    }
    let tool = held.and_then(Item::tool);
    let fits = |t: ToolInfo| matches!((info.tool, t.kind), (Tool::Pickaxe, ToolKind::Pickaxe) | (Tool::Axe, ToolKind::Axe) | (Tool::Shovel, ToolKind::Shovel));
    let speed = match tool {
        Some(t) if fits(t) => t.speed(),
        Some(t) if t.kind == ToolKind::Sword && matches!(block, Block::OAK_LEAVES | Block::BIRCH_LEAVES | Block::SPRUCE_LEAVES) => 1.5,
        _ => 1.0,
    };
    let harvest = info.needs == 0 || tool.is_some_and(|t| t.kind == ToolKind::Pickaxe && t.tier >= info.needs);
    let secs = info.hardness * if harvest { 1.5 } else { 5.0 } / speed;
    Some((secs, harvest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mining_times_follow_the_original() {
        let (dirt, drops) = mining(Block::DIRT, None).unwrap();
        assert!((dirt - 0.75).abs() < 1e-4 && drops);
        let (stone, drops) = mining(Block::STONE, None).unwrap();
        assert!((stone - 7.5).abs() < 1e-4 && !drops, "stone by hand is slow and drops nothing");
        let (stone, drops) = mining(Block::STONE, Some(Item(id::WOODEN_PICKAXE))).unwrap();
        assert!((stone - 1.125).abs() < 1e-4 && drops);
        let (_, drops) = mining(Block::IRON_ORE, Some(Item(id::WOODEN_PICKAXE))).unwrap();
        assert!(!drops, "iron needs a stone pickaxe");
        let (_, drops) = mining(Block::IRON_ORE, Some(Item(id::STONE_PICKAXE))).unwrap();
        assert!(drops);
        let (_, drops) = mining(Block::DIAMOND_ORE, Some(Item(id::STONE_PICKAXE))).unwrap();
        assert!(!drops, "diamond needs an iron pickaxe");
        assert!(mining(Block::BEDROCK, Some(Item(id::DIAMOND_PICKAXE))).is_none());
        let (log, _) = mining(Block::OAK_LOG, Some(Item(id::STONE_AXE))).unwrap();
        assert!((log - 0.75).abs() < 1e-4);
        assert_eq!(mining(Block::TORCH, None), Some((0.0, true)));
    }

    #[test]
    fn items_name_themselves_both_ways_and_tools_stack_alone() {
        for i in Item::all() {
            assert_eq!(Item::by_name(i.name()), Some(i), "{}", i.name());
            assert_ne!(i.title(), "?");
        }
        assert_eq!(Item(id::IRON_PICKAXE).tool(), Some(ToolInfo { kind: ToolKind::Pickaxe, tier: 3 }));
        assert_eq!(Item(id::DIAMOND_SWORD).max_stack(), 1);
        assert_eq!(Item::of(Block::DIRT).max_stack(), 64);
        assert_eq!(Item(id::STICK).block(), None);
        assert_eq!(Item::of(Block::TORCH).block(), Some(Block::TORCH));
        assert!(!Item::all().contains(&Item::of(Block::faced(Block::FURNACE, 2))), "one furnace in the list, not four");
    }
}
