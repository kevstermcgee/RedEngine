//! The 2D `dialogue` widget as the shared conversation core sees it (`red_core::sim::story::talk`): moving on and choosing mean what they
//! mean in a 3D game's story; only the widget's keys and its box are its own.

use crate::game::{Dialogue, Goto};

/// Moving on in the widget is the shared conversation core's (`talk::answer`); only its keys and its box are the widget's.
impl red_core::sim::story::talk::Script for Dialogue {
    fn lines(&self) -> usize {
        self.lines.len()
    }
    fn choices(&self, line: usize) -> usize {
        self.lines.get(line).map_or(0, |l| l.choices.len())
    }
    fn goto(&self, line: usize) -> Option<Goto> {
        self.lines.get(line).and_then(|l| l.goto)
    }
    fn choice_goto(&self, line: usize, choice: usize) -> Option<Goto> {
        self.lines.get(line).and_then(|l| l.choices.get(choice)).and_then(|c| c.goto)
    }
}
