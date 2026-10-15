//! What moving on in a conversation means, once, for every presenter (ADR 2026-10-15-narrative-is-compiled-content-run-by-one-story-system):
//! the 2D `dialogue` widget (`red2d`) and the story runtime ([`super::runtime`]) both answer "the player is at line N and moved on / picked
//! choice K" here, and only the drawing and the keys differ.
//!
//! A conversation is a flat list of lines. A line goes on to the next one in order, to a named line ([`Goto::Line`]) or to the end
//! ([`Goto::End`]); a line with choices waits for one, and a choice may send the conversation elsewhere (its goto wins over its line's).
//! Moving on fires the line's effects first, then the choice's ([`Moved`]). Past the last line the conversation is closed. Nothing here
//! reads keys, time or state: a presenter turns input into an [`Answer`], and the caller applies the [`Moved`] it gets back.

/// Where a line (or a choice) goes when the player moves on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Goto {
    /// The line at this index.
    Line(usize),
    /// The conversation closes.
    End,
}

/// What the player did at the line they are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Moved on past a line without choices.
    MoveOn,
    /// Picked choice `k` (0-based) of the line's choices.
    Choose(usize),
}

/// The shape of a conversation as moving on needs it.
pub trait Script {
    /// How many lines it has.
    fn lines(&self) -> usize;
    /// How many choices line `line` offers (0: the player just moves on).
    fn choices(&self, line: usize) -> usize;
    /// Where line `line` goes (`None`: the next line).
    fn goto(&self, line: usize) -> Option<Goto>;
    /// Where choice `choice` of line `line` goes (`None`: where its line goes).
    fn choice_goto(&self, line: usize, choice: usize) -> Option<Goto>;
}

/// What an answer did: fire the line's effects, then the chosen choice's (if one), and land on `next` (`closed`: past the end).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moved {
    /// The line the player answered at (its effects fire first).
    pub line: usize,
    /// The choice they picked, if the line had choices (its effects fire second).
    pub choice: Option<usize>,
    /// The line it lands on (`>= lines()` when it closed).
    pub next: usize,
    /// It went past the last line (or to [`Goto::End`]).
    pub closed: bool,
}

/// Why an answer was not taken (nothing changes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// The conversation is not at a line (closed, or never opened).
    Closed,
    /// The line has choices: the player must pick one.
    ChooseOne(usize),
    /// The line has no choice with that number.
    NoSuchChoice {
        /// The number asked for.
        asked: usize,
        /// How many there are.
        offered: usize,
    },
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Closed => write!(f, "the conversation is closed"),
            Refused::ChooseOne(n) => write!(f, "this line has {n} choices: pick one"),
            Refused::NoSuchChoice { asked, offered } => write!(f, "there is no choice {} (the line offers {offered})", asked + 1),
        }
    }
}

/// The player at line `at` gave `answer`.
pub fn answer(script: &dyn Script, at: usize, answer: Answer) -> Result<Moved, Refused> {
    let n = script.lines();
    if at >= n {
        return Err(Refused::Closed);
    }
    let offered = script.choices(at);
    let (choice, goto) = match answer {
        Answer::MoveOn if offered > 0 => return Err(Refused::ChooseOne(offered)),
        Answer::MoveOn => (None, script.goto(at)),
        Answer::Choose(k) if k >= offered => return Err(Refused::NoSuchChoice { asked: k, offered }),
        Answer::Choose(k) => (Some(k), script.choice_goto(at, k).or(script.goto(at))),
    };
    let next = match goto {
        Some(Goto::Line(i)) => i,
        Some(Goto::End) => n,
        None => at + 1,
    };
    Ok(Moved { line: at, choice, next, closed: next >= n })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines as `(choices with their gotos, the line's goto)`.
    struct Lines(Vec<(Vec<Option<Goto>>, Option<Goto>)>);

    impl Script for Lines {
        fn lines(&self) -> usize {
            self.0.len()
        }
        fn choices(&self, line: usize) -> usize {
            self.0[line].0.len()
        }
        fn goto(&self, line: usize) -> Option<Goto> {
            self.0[line].1
        }
        fn choice_goto(&self, line: usize, choice: usize) -> Option<Goto> {
            self.0[line].0[choice]
        }
    }

    #[test]
    fn moving_on_follows_the_choice_then_the_line_then_the_order() {
        let s = Lines(vec![(vec![], None), (vec![Some(Goto::Line(3)), None, Some(Goto::End)], Some(Goto::Line(0))), (vec![], Some(Goto::End)), (vec![], None)]);
        assert_eq!(answer(&s, 0, Answer::MoveOn), Ok(Moved { line: 0, choice: None, next: 1, closed: false }));
        assert_eq!(answer(&s, 1, Answer::Choose(0)).map(|m| m.next), Ok(3), "the choice's goto");
        assert_eq!(answer(&s, 1, Answer::Choose(1)).map(|m| m.next), Ok(0), "no goto of its own: the line's");
        assert_eq!(answer(&s, 1, Answer::Choose(2)).map(|m| m.closed), Ok(true));
        assert_eq!(answer(&s, 2, Answer::MoveOn).map(|m| (m.next, m.closed)), Ok((4, true)));
        assert_eq!(answer(&s, 3, Answer::MoveOn).map(|m| m.closed), Ok(true), "past the last line");
    }

    #[test]
    fn an_answer_that_does_not_fit_the_line_is_refused() {
        let s = Lines(vec![(vec![None, None], None)]);
        assert_eq!(answer(&s, 0, Answer::MoveOn), Err(Refused::ChooseOne(2)));
        assert_eq!(answer(&s, 0, Answer::Choose(2)), Err(Refused::NoSuchChoice { asked: 2, offered: 2 }));
        assert_eq!(answer(&s, 1, Answer::MoveOn), Err(Refused::Closed));
        assert_eq!(Refused::NoSuchChoice { asked: 2, offered: 2 }.to_string(), "there is no choice 3 (the line offers 2)");
    }
}
