//! Which screen is up, and what a click on one of its slots does to the world being played: the original's rules (left takes or puts the
//! stack, right takes half or puts one, shift sends a stack across; a crafting result is taken whole and uses one of each ingredient, shift
//! crafts as many as fit; a furnace's output is only taken; in creative a click on the list takes a stack, and a stack dropped on the list is
//! thrown away).

use crate::craft::{consume, craft};
use crate::inventory::{click, fill, take_result, HOTBAR, SLOTS};
use crate::item::{Item, Stack};
use crate::play::Play;
use crate::ui::{Button, SlotRef};
use glam::IVec3;

/// The title screen's pages.
#[derive(Clone, Debug, PartialEq)]
pub enum Title {
    Main,
    NewWorld { name: String, seed: String, creative: bool, focus: Button },
    Worlds { page: usize },
}

/// What is on screen.
#[derive(Clone, Debug, PartialEq)]
pub enum Screen {
    /// Playing: the HUD over the world.
    Hud,
    Pause,
    Inventory,
    Table(IVec3),
    Furnace(IVec3),
    Chest(IVec3),
    Creative {
        page: usize,
    },
    Death,
    Title(Title),
}

impl Screen {
    /// A screen with slots (the carried stacks go back when it closes).
    pub fn has_slots(&self) -> bool {
        matches!(self, Screen::Inventory | Screen::Table(_) | Screen::Furnace(_) | Screen::Chest(_) | Screen::Creative { .. })
    }

    /// The crafting grid's width on this screen (0: none).
    pub fn grid_width(&self) -> usize {
        match self {
            Screen::Inventory => 2,
            Screen::Table(_) => 3,
            _ => 0,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Screen::Hud => "hud",
            Screen::Pause => "pause",
            Screen::Inventory => "inventory",
            Screen::Table(_) => "crafting_table",
            Screen::Furnace(_) => "furnace",
            Screen::Chest(_) => "chest",
            Screen::Creative { .. } => "creative",
            Screen::Death => "death",
            Screen::Title(_) => "title",
        }
    }
}

/// What the grid of this screen makes now.
pub fn crafted(p: &Play, screen: &Screen) -> Stack {
    let w = screen.grid_width();
    if w == 0 {
        return Stack::EMPTY;
    }
    craft(&p.grid[..w * w], w).unwrap_or(Stack::EMPTY)
}

/// The stack behind a slot of this screen.
pub fn stack_of(p: &Play, screen: &Screen, r: SlotRef, creative: &[Item]) -> Stack {
    match r {
        SlotRef::Inv(i) => p.inv.slots[i],
        SlotRef::Craft(i) => p.grid[i],
        SlotRef::CraftOut => crafted(p, screen),
        SlotRef::FurnaceIn | SlotRef::FurnaceFuel | SlotRef::FurnaceOut => match screen {
            Screen::Furnace(at) => p.furnaces.get(at).map_or(Stack::EMPTY, |f| match r {
                SlotRef::FurnaceIn => f.input,
                SlotRef::FurnaceFuel => f.fuel,
                _ => f.output,
            }),
            _ => Stack::EMPTY,
        },
        SlotRef::Chest(i) => match screen {
            Screen::Chest(at) => p.chests.get(at).and_then(|c| c.get(i)).copied().unwrap_or(Stack::EMPTY),
            _ => Stack::EMPTY,
        },
        SlotRef::Creative(n) => creative.get(n).map_or(Stack::EMPTY, |it| Stack::new(*it, 1)),
    }
}

/// A click on slot `r`.
pub fn click_slot(p: &mut Play, screen: &Screen, r: SlotRef, right: bool, shift: bool, creative: &[Item]) {
    match r {
        SlotRef::Inv(i) => {
            if shift {
                let s = std::mem::replace(&mut p.inv.slots[i], Stack::EMPTY);
                let left = send_from_inventory(p, screen, i, s);
                p.inv.slots[i] = left;
            } else {
                let mut cursor = p.cursor;
                click(&mut p.inv.slots[i], &mut cursor, right);
                p.cursor = cursor;
            }
        }
        SlotRef::Craft(i) => {
            if shift {
                let s = std::mem::replace(&mut p.grid[i], Stack::EMPTY);
                p.grid[i] = p.inv.add(s);
            } else {
                let mut cursor = p.cursor;
                click(&mut p.grid[i], &mut cursor, right);
                p.cursor = cursor;
            }
        }
        SlotRef::CraftOut => {
            let w = screen.grid_width();
            if w == 0 {
                return;
            }
            if shift {
                // As many as the ingredients and the inventory allow.
                for _ in 0..64 {
                    let Some(made) = craft(&p.grid[..w * w], w) else { break };
                    let mut probe = p.inv.clone();
                    if !probe.add(made).is_empty() {
                        break;
                    }
                    p.inv = probe;
                    consume(&mut p.grid[..w * w]);
                }
            } else if let Some(made) = craft(&p.grid[..w * w], w) {
                if take_result(&made, &mut p.cursor) {
                    consume(&mut p.grid[..w * w]);
                }
            }
        }
        SlotRef::FurnaceIn | SlotRef::FurnaceFuel | SlotRef::FurnaceOut => {
            let Screen::Furnace(at) = screen else { return };
            let mut cursor = p.cursor;
            let Some(f) = p.furnaces.get_mut(at) else { return };
            match r {
                SlotRef::FurnaceOut => {
                    if shift {
                        let s = std::mem::replace(&mut f.output, Stack::EMPTY);
                        f.output = p.inv.add(s);
                    } else if take_result(&f.output, &mut cursor) {
                        f.output = Stack::EMPTY;
                    }
                }
                SlotRef::FurnaceIn => {
                    if shift {
                        let s = std::mem::replace(&mut f.input, Stack::EMPTY);
                        f.input = p.inv.add(s);
                    } else {
                        click(&mut f.input, &mut cursor, right);
                    }
                }
                _ => {
                    if shift {
                        let s = std::mem::replace(&mut f.fuel, Stack::EMPTY);
                        f.fuel = p.inv.add(s);
                    } else {
                        click(&mut f.fuel, &mut cursor, right);
                    }
                }
            }
            p.cursor = cursor;
        }
        SlotRef::Chest(i) => {
            let Screen::Chest(at) = screen else { return };
            let mut cursor = p.cursor;
            let Some(c) = p.chests.get_mut(at) else { return };
            if shift {
                let s = std::mem::replace(&mut c[i], Stack::EMPTY);
                c[i] = p.inv.add(s);
            } else {
                click(&mut c[i], &mut cursor, right);
            }
            p.cursor = cursor;
        }
        SlotRef::Creative(n) => {
            if !p.cursor.is_empty() {
                // Dropped on the list: gone.
                p.cursor = Stack::EMPTY;
                return;
            }
            let Some(item) = creative.get(n) else { return };
            let s = Stack::new(*item, if right { 1 } else { item.max_stack() });
            if shift {
                p.inv.add(s);
            } else {
                p.cursor = s;
            }
        }
    }
}

/// Shift-click from the player's slot `i`: into the open container when it takes it, otherwise between the hotbar and the rest.
fn send_from_inventory(p: &mut Play, screen: &Screen, i: usize, s: Stack) -> Stack {
    if s.is_empty() {
        return s;
    }
    match screen {
        Screen::Chest(at) => {
            if let Some(c) = p.chests.get_mut(at) {
                let order: Vec<usize> = (0..c.len()).collect();
                return fill(c, s, &order);
            }
        }
        Screen::Furnace(at) => {
            if let Some(f) = p.furnaces.get_mut(at) {
                let target = if crate::craft::smelt(s.item).is_some() {
                    Some(&mut f.input)
                } else if crate::craft::burn_secs(s.item).is_some() {
                    Some(&mut f.fuel)
                } else {
                    None
                };
                if let Some(t) = target {
                    let mut one = [*t];
                    let left = fill(&mut one, s, &[0]);
                    *t = one[0];
                    return left;
                }
            }
        }
        _ => {}
    }
    let order: Vec<usize> = if i < HOTBAR { (HOTBAR..SLOTS).collect() } else { (0..HOTBAR).collect() };
    fill(&mut p.inv.slots, s, &order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;
    use crate::item::id;
    use crate::save::GameMode;

    fn play() -> Play {
        Play::create(std::env::temp_dir().join("redcraft-screens"), "T", 1, GameMode::Survival, 1, 0)
    }

    #[test]
    fn crafting_planks_then_a_table_through_the_inventory_screen() {
        let mut p = play();
        p.inv.slots[0] = Stack::new(Item::of(Block::OAK_LOG), 2);
        let s = Screen::Inventory;
        // Log into the grid, one at a time with the right button.
        click_slot(&mut p, &s, SlotRef::Inv(0), false, false, &[]);
        click_slot(&mut p, &s, SlotRef::Craft(0), true, false, &[]);
        assert_eq!((p.grid[0].count, p.cursor.count), (1, 1));
        click_slot(&mut p, &s, SlotRef::Inv(0), false, false, &[]);
        assert_eq!(crafted(&p, &s), Stack::new(Item::of(Block::OAK_PLANKS), 4));
        click_slot(&mut p, &s, SlotRef::CraftOut, false, false, &[]);
        assert_eq!(p.cursor, Stack::new(Item::of(Block::OAK_PLANKS), 4));
        assert!(p.grid[0].is_empty(), "the log was used");
        // Planks into all four cells, then take the table.
        for k in 0..4 {
            click_slot(&mut p, &s, SlotRef::Craft(k), true, false, &[]);
        }
        assert!(p.cursor.is_empty());
        assert_eq!(crafted(&p, &s).item, Item::of(Block::CRAFTING_TABLE));
        click_slot(&mut p, &s, SlotRef::CraftOut, false, true, &[]);
        assert_eq!(p.inv.count(Item::of(Block::CRAFTING_TABLE)), 1, "shift put it in the inventory");
        assert!(p.grid.iter().all(|g| g.is_empty()));
    }

    #[test]
    fn shift_moves_into_a_furnace_by_kind_and_a_chest_takes_stacks() {
        let mut p = play();
        let at = IVec3::new(1, 70, 1);
        p.furnaces.insert(at, Default::default());
        p.inv.slots[0] = Stack::new(Item::of(Block::IRON_ORE), 5);
        p.inv.slots[1] = Stack::new(Item(id::COAL), 3);
        let s = Screen::Furnace(at);
        click_slot(&mut p, &s, SlotRef::Inv(0), false, true, &[]);
        click_slot(&mut p, &s, SlotRef::Inv(1), false, true, &[]);
        let f = p.furnaces[&at];
        assert_eq!((f.input.count, f.fuel.count), (5, 3));
        let c = IVec3::new(2, 70, 1);
        p.chests.insert(c, vec![Stack::EMPTY; 27]);
        p.inv.slots[2] = Stack::new(Item::of(Block::DIRT), 64);
        click_slot(&mut p, &Screen::Chest(c), SlotRef::Inv(2), false, true, &[]);
        assert_eq!(p.chests[&c][0].count, 64);
        assert!(p.inv.slots[2].is_empty());
        // Hotbar to the rest of the inventory, with nothing open.
        p.inv.slots[3] = Stack::new(Item(id::STICK), 2);
        click_slot(&mut p, &Screen::Inventory, SlotRef::Inv(3), false, true, &[]);
        assert!(p.inv.slots[3].is_empty() && p.inv.count(Item(id::STICK)) == 2);
    }

    #[test]
    fn creative_takes_whole_stacks_and_throws_away_what_is_dropped_on_it() {
        let mut p = play();
        let list = vec![Item::of(Block::STONE), Item(id::DIAMOND_SWORD)];
        let s = Screen::Creative { page: 0 };
        click_slot(&mut p, &s, SlotRef::Creative(0), false, false, &list);
        assert_eq!(p.cursor, Stack::new(Item::of(Block::STONE), 64));
        click_slot(&mut p, &s, SlotRef::Creative(1), false, false, &list);
        assert!(p.cursor.is_empty());
        click_slot(&mut p, &s, SlotRef::Creative(1), false, true, &list);
        assert_eq!(p.inv.count(Item(id::DIAMOND_SWORD)), 1);
    }
}
