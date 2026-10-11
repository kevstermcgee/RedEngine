//! The museum's words: templates filled from word lists with a seeded draw.
//!
//! A template is plain text with slots: `{noun}` draws from the theme's list of that name, then the collection's; `{Noun}` does the same and capitalises
//! the first letter; `{a|b|c}` picks one of the inline choices; a drawn word may itself hold slots (up to a few levels deep). The engine fills a few
//! slots itself (`{title}`, `{artist}`, `{year}`, `{kind}`, ... see [`super::spec::ENGINE_SLOTS`]) through the `fixed` table.

use super::spec::{Collection, Theme};
use std::collections::BTreeMap;

/// A tiny deterministic generator (splitmix64): the same seed always writes the same words.
#[derive(Debug, Clone)]
pub struct Draw(u64);

impl Draw {
    /// A draw keyed by any numbers (seed, room, piece, what for).
    pub fn new(keys: &[i64]) -> Draw {
        let mut h: u64 = 0x9E37_79B9_7F4A_7C15;
        for k in keys {
            h ^= (*k as u64).wrapping_add(0x9E37_79B9_7F4A_7C15).wrapping_add(h << 6).wrapping_add(h >> 2);
            h = mix64(h);
        }
        Draw(h)
    }

    /// The next 64 random bits.
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix64(self.0)
    }

    /// A number in 0..1.
    pub fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// A number in `lo..hi`.
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    /// A whole number in `0..n` (0 when `n` is 0).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }

    /// One element of a non-empty slice.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }

    /// An index drawn with the given weights.
    pub fn weighted(&mut self, weights: &[f32]) -> usize {
        let total: f32 = weights.iter().sum();
        let mut x = self.unit() * total;
        for (i, w) in weights.iter().enumerate() {
            if x < *w {
                return i;
            }
            x -= w;
        }
        weights.len().saturating_sub(1)
    }
}

fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Fills `template` from the theme's words, the collection's words and `fixed` (the engine's own slots).
pub fn fill(template: &str, theme: &Theme, c: &Collection, fixed: &BTreeMap<&str, String>, draw: &mut Draw) -> String {
    tidy(&fill_depth(template, theme, c, fixed, draw, 0))
}

fn fill_depth(template: &str, theme: &Theme, c: &Collection, fixed: &BTreeMap<&str, String>, draw: &mut Draw, depth: u32) -> String {
    let mut out = String::with_capacity(template.len() + 32);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let inner = &rest[open + 1..open + close];
        rest = &rest[open + close + 1..];
        let capital = inner.chars().next().is_some_and(char::is_uppercase);
        let word = if inner.contains('|') {
            let choices: Vec<&str> = inner.split('|').collect();
            draw.pick(&choices).to_string()
        } else {
            let key = inner.to_ascii_lowercase();
            if let Some(v) = fixed.get(key.as_str()) {
                v.clone()
            } else if let Some(list) = theme.words.get(&key).or_else(|| c.words.get(&key)) {
                draw.pick(list).clone()
            } else {
                inner.to_string()
            }
        };
        let word = if depth < 4 && word.contains('{') { fill_depth(&word, theme, c, fixed, draw, depth + 1) } else { word };
        if capital {
            out.push_str(&capitalise(&word));
        } else {
            out.push_str(&word);
        }
    }
    out.push_str(rest);
    out
}

/// The first letter upper-case.
pub fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Collapses doubled spaces and starts the text, and every sentence after a full stop or an exclamation, with a capital (a template may begin a sentence
/// with a slot whose words are lower-case). A question mark is left alone: in "'Why?' and then" the sentence goes on.
pub fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    let mut last_mark = '.';
    let mut spaced = true;
    for ch in s.chars() {
        if ch == ' ' {
            if !last_space && !out.is_empty() {
                out.push(' ');
            }
            last_space = true;
            spaced = true;
            continue;
        }
        last_space = false;
        if ch.is_alphabetic() && spaced && matches!(last_mark, '.' | '!') {
            out.extend(ch.to_uppercase());
        } else {
            out.push(ch);
        }
        spaced = false;
        last_mark = ch;
    }
    out.trim().replace(" ,", ",").replace(" .", ".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_repeat_for_the_same_keys_and_differ_otherwise() {
        let a: Vec<u64> = (0..4).map(|_| Draw::new(&[1, 2, 3]).next()).collect();
        assert!(a.windows(2).all(|w| w[0] == w[1]));
        assert_ne!(Draw::new(&[1, 2, 3]).next(), Draw::new(&[1, 2, 4]).next());
        let mut d = Draw::new(&[9]);
        for _ in 0..1000 {
            let u = d.unit();
            assert!((0.0..1.0).contains(&u));
        }
    }

    #[test]
    fn tidy_capitalises_sentences_and_squeezes_spaces() {
        assert_eq!(tidy("it was  late. the lamp , once lit."), "It was late. The lamp, once lit.");
        assert_eq!(tidy("'Why?' and then it was c. 1890 for J. Moreau."), "'Why?' and then it was c. 1890 for J. Moreau.");
    }
}
