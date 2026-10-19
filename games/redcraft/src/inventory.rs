//! The player's inventory (a hotbar of 9 and 27 more slots) and the clicks every screen shares: picking up, putting down, splitting and
//! swapping stacks with the mouse, shift-click to send a stack across, and taking what a crafting grid or a furnace made.

use crate::item::{Item, Stack};

/// Slots in the hotbar (the first slots of the inventory).
pub const HOTBAR: usize = 9;
/// All the player's slots.
pub const SLOTS: usize = 36;

/// What the player carries.
#[derive(Clone, Debug, PartialEq)]
pub struct Inventory {
    pub slots: [Stack; SLOTS],
    /// The hotbar slot in hand.
    pub selected: usize,
}

impl Default for Inventory {
    fn default() -> Self {
        Inventory { slots: [Stack::EMPTY; SLOTS], selected: 0 }
    }
}

/// Puts as much of `s` as fits into `slots`, onto stacks of its kind first and then into empty slots, in order; returns what is left.
pub fn fill(slots: &mut [Stack], mut s: Stack, order: &[usize]) -> Stack {
    let max = s.item.max_stack();
    for &i in order {
        if s.is_empty() {
            break;
        }
        let t = &mut slots[i];
        if !t.is_empty() && t.same_kind(&s) && t.count < max {
            let n = (max - t.count).min(s.count);
            t.count += n;
            s.count -= n;
        }
    }
    for &i in order {
        if s.is_empty() {
            break;
        }
        if slots[i].is_empty() {
            let n = s.count.min(max);
            slots[i] = Stack { count: n, ..s };
            s.count -= n;
        }
    }
    if s.count == 0 {
        Stack::EMPTY
    } else {
        s
    }
}

impl Inventory {
    /// The stack in hand.
    pub fn held(&self) -> Stack {
        self.slots[self.selected]
    }

    pub fn held_item(&self) -> Option<Item> {
        let s = self.held();
        (!s.is_empty()).then_some(s.item)
    }

    /// Picks something up: into the hotbar's stacks of its kind, then the rest's, then empty slots (hotbar first). Returns what did not fit.
    pub fn add(&mut self, s: Stack) -> Stack {
        let order: Vec<usize> = (0..SLOTS).collect();
        fill(&mut self.slots, s, &order)
    }

    /// Uses up `n` of what is in hand.
    pub fn use_held(&mut self, n: u8) {
        let s = &mut self.slots[self.selected];
        s.count = s.count.saturating_sub(n);
        if s.count == 0 {
            *s = Stack::EMPTY;
        }
    }

    /// How many of `item` it holds.
    pub fn count(&self, item: Item) -> u32 {
        self.slots.iter().filter(|s| s.item == item && !s.is_empty()).map(|s| s.count as u32).sum()
    }

    /// Puts the item in hand into the hotbar slot for it: the slot already holding it, or the selected one (creative's pick block).
    pub fn pick(&mut self, item: Item) {
        if let Some(i) = (0..HOTBAR).find(|&i| self.slots[i].item == item && !self.slots[i].is_empty()) {
            self.selected = i;
            return;
        }
        self.slots[self.selected] = Stack::new(item, item.max_stack());
    }
}

/// A mouse click on a slot while `cursor` holds what the mouse carries. `right`: the right button (half / one at a time).
pub fn click(slot: &mut Stack, cursor: &mut Stack, right: bool) {
    if cursor.is_empty() {
        if slot.is_empty() {
            return;
        }
        let take = if right { slot.count.div_ceil(2) } else { slot.count };
        *cursor = Stack { count: take, ..*slot };
        slot.count -= take;
        if slot.count == 0 {
            *slot = Stack::EMPTY;
        }
        return;
    }
    if slot.is_empty() {
        let put = if right { 1 } else { cursor.count };
        *slot = Stack { count: put, ..*cursor };
        cursor.count -= put;
    } else if slot.same_kind(cursor) {
        let room = slot.item.max_stack().saturating_sub(slot.count);
        let put = if right { 1.min(room) } else { cursor.count.min(room) };
        slot.count += put;
        cursor.count -= put;
    } else {
        std::mem::swap(slot, cursor);
    }
    if cursor.count == 0 {
        *cursor = Stack::EMPTY;
    }
}

/// Taking from a slot that is only taken from (a craft's result, a furnace's output): the whole stack, onto the cursor when it fits there.
/// Returns whether it was taken.
pub fn take_result(result: &Stack, cursor: &mut Stack) -> bool {
    if result.is_empty() {
        return false;
    }
    if cursor.is_empty() {
        *cursor = *result;
        return true;
    }
    if cursor.same_kind(result) && cursor.count as u16 + result.count as u16 <= cursor.item.max_stack() as u16 {
        cursor.count += result.count;
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;
    use crate::item::id;

    #[test]
    fn picking_up_fills_stacks_then_the_hotbar_then_the_rest() {
        let mut inv = Inventory::default();
        let dirt = Item::of(Block::DIRT);
        assert!(inv.add(Stack::new(dirt, 40)).is_empty());
        assert!(inv.add(Stack::new(dirt, 40)).is_empty());
        assert_eq!((inv.slots[0].count, inv.slots[1].count), (64, 16));
        for _ in 0..40 {
            inv.add(Stack::new(Item(id::WOODEN_PICKAXE), 1));
        }
        assert_eq!(inv.count(Item(id::WOODEN_PICKAXE)), 34, "tools one to a slot, until the inventory is full");
        let left = inv.add(Stack::new(Item(id::STICK), 5));
        assert_eq!(left.count, 5, "a full inventory leaves it");
        inv.selected = 1;
        inv.use_held(16);
        assert!(inv.held().is_empty());
    }

    #[test]
    fn the_mouse_picks_up_puts_down_splits_merges_and_swaps() {
        let dirt = Item::of(Block::DIRT);
        let stone = Item::of(Block::STONE);
        let mut slot = Stack::new(dirt, 9);
        let mut cursor = Stack::EMPTY;
        click(&mut slot, &mut cursor, true);
        assert_eq!((slot.count, cursor.count), (4, 5), "right: half, rounded up, to the cursor");
        click(&mut slot, &mut cursor, true);
        assert_eq!((slot.count, cursor.count), (5, 4), "right with a stack: one back");
        click(&mut slot, &mut cursor, false);
        assert_eq!((slot.count, cursor.count), (9, 0));
        let mut other = Stack::new(stone, 3);
        click(&mut slot, &mut cursor, false);
        click(&mut other, &mut cursor, false);
        assert_eq!((other.item, cursor.item), (dirt, stone), "different kinds swap");
        let mut empty = Stack::EMPTY;
        click(&mut empty, &mut cursor, true);
        assert_eq!((empty, cursor.count), (Stack::new(stone, 1), 2));
        let made = Stack::new(Item(id::STICK), 4);
        let mut c = Stack::new(Item(id::STICK), 62);
        assert!(!take_result(&made, &mut c), "no room on the cursor");
        let mut c = Stack::EMPTY;
        assert!(take_result(&made, &mut c) && c.count == 4);
    }
}
