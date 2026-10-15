//! An optional dialogue presenter for custom clients: subtitles and choices for a game's story (`sim::story`), drawn with the audited UI
//! kit into the game's own HUD layout, with keyboard, pointer and gamepad. Presentation only: it reads the authority's [`View`] and returns
//! an [`Intent`] naming the revision it showed; it never decides anything (ADR 2026-10-15-narrative-is-compiled-content-run-by-one-story-system).
//! A game may use it, restyle it ([`Style`]), or draw its own from the same `View`.
//!
//! - **Reveal, then advance.** A line types out at [`Presenter::chars_per_sec`]; the first confirm shows all of it, the next moves on (or
//!   picks the focused choice). Choices appear only once the line is shown in full.
//! - **Predictable focus.** Up and down move the focus with wrap-around; a new line puts it back on the first choice; `1`-`9` pick
//!   directly; a click picks the row under the pointer.
//! - **No leaking into play.** [`Presenter::captures_input`] is true while a conversation is open: the client then sends no movement or
//!   action (the authority holds the player still too).
//! - Barks are subtitles the player does not answer (`skip` dismisses one, which counts as heard).

use crate::sim::game::Intent;
use crate::sim::story::runtime::View;
use crate::ui::{ellipsize, fit_scale, text_height, wrap, Layout};

/// What the player pressed this frame, from any device (each a press, not a hold).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Controls {
    /// Focus up.
    pub up: bool,
    /// Focus down.
    pub down: bool,
    /// Show the whole line, or move on, or pick the focused choice.
    pub confirm: bool,
    /// A number key (1-9): pick that choice.
    pub pick: Option<usize>,
    /// Leave the conversation.
    pub leave: bool,
    /// Dismiss the bark being presented.
    pub skip: bool,
    /// A click, in window pixels.
    pub click: Option<(f32, f32)>,
}

#[cfg(feature = "gfx")]
impl Controls {
    /// From the keyboard and pointer (E, Space or Enter confirm; W/S or the arrows focus; 1-9 pick; Backspace leaves; Tab skips a bark)
    /// and, when there is one, a gamepad (A confirms, the D-pad focuses, B leaves, Y skips).
    pub fn from_input(input: &crate::app::InputState, pad: Option<&crate::controller::Sample>) -> Controls {
        use crate::app::{KeyCode as K, MouseButton};
        use crate::controller::button as b;
        let any = |keys: &[K]| keys.iter().any(|k| input.pressed(*k));
        let hit = |bt: u16| pad.is_some_and(|p| p.hit(bt));
        let digits = [K::Digit1, K::Digit2, K::Digit3, K::Digit4, K::Digit5, K::Digit6, K::Digit7, K::Digit8, K::Digit9];
        Controls {
            up: any(&[K::KeyW, K::ArrowUp]) || hit(b::UP),
            down: any(&[K::KeyS, K::ArrowDown]) || hit(b::DOWN),
            confirm: any(&[K::KeyE, K::Space, K::Enter]) || hit(b::JUMP),
            pick: digits.iter().position(|k| input.pressed(*k)).map(|i| i + 1),
            leave: any(&[K::Backspace]) || hit(b::CROUCH),
            skip: any(&[K::Tab]) || hit(b::RELOAD),
            click: input.clicked(MouseButton::Left),
        }
    }
}

/// Colours and size of the presenter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// The subtitle panel.
    pub panel: [u8; 4],
    /// Its frame.
    pub frame: [u8; 4],
    /// Spoken text.
    pub text: [u8; 4],
    /// Speaker names.
    pub name: [u8; 4],
    /// The focused choice.
    pub focus: [u8; 4],
    /// Other choices.
    pub choice: [u8; 4],
    /// Hints.
    pub hint: [u8; 4],
    /// Text scale for a 360-pixel-tall window (it grows with the window).
    pub scale: i32,
}

impl Default for Style {
    fn default() -> Style {
        Style {
            panel: [12, 16, 28, 225],
            frame: [120, 140, 180, 255],
            text: [236, 236, 228, 255],
            name: [255, 196, 110, 255],
            focus: [255, 228, 150, 255],
            choice: [170, 180, 200, 255],
            hint: [140, 150, 170, 255],
            scale: 1,
        }
    }
}

/// The presenter's own state: how much of the line is shown, which choice has the focus.
#[derive(Debug, Clone)]
pub struct Presenter {
    line: Option<String>,
    shown: f32,
    focus: usize,
    /// How fast a line types out.
    pub chars_per_sec: f32,
    /// How it looks.
    pub style: Style,
}

impl Default for Presenter {
    fn default() -> Presenter {
        Presenter { line: None, shown: 0.0, focus: 0, chars_per_sec: 45.0, style: Style::default() }
    }
}

fn intent(name: &str, subject: Option<String>, revision: u64) -> Intent {
    Intent { name: format!("story:{name}"), subject, value: revision as i64 }
}

impl Presenter {
    /// A presenter with the default look.
    pub fn new() -> Presenter {
        Presenter::default()
    }

    /// Whether the player's input belongs to the conversation now (a client sends no gameplay input while it does).
    pub fn captures_input(view: &View) -> bool {
        view.talk.as_ref().is_some_and(|t| !t.suspended)
    }

    /// Whether the open line is shown in full.
    pub fn revealed(&self, view: &View) -> bool {
        view.talk.as_ref().is_none_or(|t| self.shown >= t.line.text.chars().count() as f32)
    }

    /// The choice with the focus (index into the offered choices).
    pub fn focus(&self) -> usize {
        self.focus
    }

    /// How many characters of the open line are shown (a repaint key: it changes while the line types out).
    pub fn shown(&self) -> usize {
        self.shown.ceil() as usize
    }

    /// One frame: the time passed, what the authority shows, what was pressed (and the layout last drawn, for clicks). Returns the
    /// intention to send, if the player made one.
    pub fn update(&mut self, view: &View, c: &Controls, dt: f32, drawn: Option<&Layout>) -> Option<Intent> {
        let Some(t) = &view.talk else {
            self.line = None;
            return (c.skip && view.bark.is_some()).then(|| intent("skip", None, view.revision));
        };
        if self.line.as_deref() != Some(t.line.id.as_str()) {
            (self.line, self.shown, self.focus) = (Some(t.line.id.clone()), 0.0, 0);
        }
        self.focus = self.focus.min(t.choices.len().saturating_sub(1));
        if t.suspended {
            return None;
        }
        let len = t.line.text.chars().count() as f32;
        self.shown = (self.shown + dt * self.chars_per_sec).min(len);
        let full = self.shown >= len;
        let n = t.choices.len();
        if c.leave {
            return Some(intent("leave", None, view.revision));
        }
        if full && n > 0 {
            if c.up {
                self.focus = (self.focus + n - 1) % n;
            }
            if c.down {
                self.focus = (self.focus + 1) % n;
            }
            if let Some(k) = c.pick.filter(|k| (1..=n).contains(k)) {
                return Some(intent("choose", Some(t.choices[k - 1].0.clone()), view.revision));
            }
            if let Some((x, y)) = c.click {
                if let Some(k) = drawn.and_then(|l| l.button_at(x, y)).and_then(|id| id.strip_prefix("story-choice-")).and_then(|k| k.parse::<usize>().ok()) {
                    if k < n {
                        return Some(intent("choose", Some(t.choices[k].0.clone()), view.revision));
                    }
                }
            }
        }
        if c.confirm || (c.click.is_some() && n == 0) {
            if !full {
                self.shown = len;
                return None;
            }
            return Some(if n == 0 { intent("advance", None, view.revision) } else { intent("choose", Some(t.choices[self.focus].0.clone()), view.revision) });
        }
        None
    }

    /// Draws the subtitles into `l` (a `w` x `h` window): the open line with its speaker and choices at the bottom, else the bark.
    pub fn draw(&self, l: &mut Layout, view: &View, w: u32, h: u32) {
        let st = &self.style;
        let s = st.scale * (h as i32 / 360).max(1);
        let (w, h) = (w as i32, h as i32);
        let pad = 6 * s;
        let box_w = (w * 4 / 5).min(220 * s).max(w / 2);
        let x0 = (w - box_w) / 2;
        let max_w = box_w - 2 * pad;
        let row = text_height(s) + 3 * s;
        if let Some(t) = &view.talk {
            let text: String = t.line.text.chars().take(self.shown.ceil() as usize).collect();
            let full = self.revealed(view);
            let body = wrap(&t.line.text, max_w, s).len().max(1) as i32;
            let choices = if full { t.choices.len() as i32 } else { 0 };
            let box_h = pad * 2 + row * (1 + body + choices) + row;
            let y0 = h - box_h - 8 * s;
            let panel = l.panel("story-panel", (x0, y0, x0 + box_w, y0 + box_h), None, Some(st.panel), Some((st.frame, 1.max(s / 2))));
            let mut y = y0 + pad;
            let name = t.line.name.as_deref().unwrap_or("");
            l.label("story-speaker", Some(panel), w / 2, y, &name.to_uppercase(), s, st.name);
            y += row;
            let mut lines = wrap(&text, max_w, s);
            lines.resize(body as usize, String::new());
            for (i, line) in lines.iter().enumerate() {
                if !line.is_empty() {
                    l.label(&format!("story-text-{}", i + 1), Some(panel), w / 2, y, line, s, st.text);
                }
                y += row;
            }
            if full {
                for (k, (_, label)) in t.choices.iter().enumerate() {
                    let focused = k == self.focus;
                    let mark = if focused { ">" } else { " " };
                    let fill = if focused { [40, 48, 70, 255] } else { st.panel };
                    let color = if focused { st.focus } else { st.choice };
                    let rect = (x0 + pad, y, x0 + pad + max_w, y + row - s);
                    let text = format!("{mark} {}. {label}", k + 1);
                    let cs = fit_scale(&text, max_w, s);
                    l.button(&format!("story-choice-{k}"), rect, Some(panel), &ellipsize(&text, max_w, cs), cs, fill, (st.frame, 0), color);
                    y += row;
                }
            }
            let hint = match (t.suspended, full, t.choices.is_empty()) {
                (true, _, _) => "...",
                (_, false, _) => "E / A: SHOW ALL",
                (_, true, true) => "E / A: CONTINUE",
                _ => "W/S OR D-PAD: CHOOSE   E / A: PICK   1-9",
            };
            l.label_fit("story-hint", Some(panel), w / 2, y, hint, s, max_w, st.hint);
            if let Some(r) = &t.refusal {
                l.label_fit("story-refusal", None, w / 2, y0 - row, &r.replace('`', ""), s, box_w, [255, 150, 130, 255]);
            }
        } else if let Some(b) = &view.bark {
            let line = match &b.name {
                Some(n) => format!("{}: {}", n.to_uppercase(), b.text),
                None => b.text.clone(),
            };
            let rows = wrap(&line, max_w, s);
            let box_h = pad * 2 + row * rows.len() as i32;
            let y0 = h - box_h - 8 * s;
            let panel = l.panel("story-bark", (x0, y0, x0 + box_w, y0 + box_h), None, Some([12, 16, 28, 170]), None);
            for (i, r) in rows.iter().enumerate() {
                l.label(&format!("story-bark-{}", i + 1), Some(panel), w / 2, y0 + pad + row * i as i32, r, s, st.text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::story::runtime::{LineView, TalkView};

    fn view(choices: &[&str], revision: u64) -> View {
        View {
            revision,
            talk: Some(TalkView {
                conversation: "c".into(),
                line: LineView {
                    id: "c/a/l1".into(),
                    speaker: Some("ann".into()),
                    name: Some("Ann".into()),
                    text: "Two bricks. Not three.".into(),
                    until: None,
                },
                choices: choices.iter().map(|c| (c.to_string(), format!("say {c}"))).collect(),
                unavailable: vec![],
                suspended: false,
                refusal: None,
            }),
            bark: None,
        }
    }

    #[test]
    fn the_first_press_reveals_the_line_and_the_next_moves_on() {
        let mut p = Presenter::new();
        let v = view(&[], 4);
        let press = Controls { confirm: true, ..Controls::default() };
        assert_eq!(p.update(&v, &Controls::default(), 0.1, None), None);
        assert!(!p.revealed(&v));
        assert_eq!(p.update(&v, &press, 0.0, None), None, "reveal, do not advance");
        assert!(p.revealed(&v));
        let i = p.update(&v, &press, 0.0, None).unwrap();
        assert_eq!((i.name.as_str(), i.value), ("story:advance", 4), "it names the revision it showed");
    }

    #[test]
    fn focus_wraps_resets_on_a_new_line_and_numbers_pick_directly() {
        let mut p = Presenter::new();
        let v = view(&["lend", "refuse", "later"], 7);
        p.update(&v, &Controls { confirm: true, ..Controls::default() }, 0.0, None);
        let up = Controls { up: true, ..Controls::default() };
        p.update(&v, &up, 0.0, None);
        assert_eq!(p.focus(), 2, "up from the first wraps to the last");
        let i = p.update(&v, &Controls { confirm: true, ..Controls::default() }, 0.0, None).unwrap();
        assert_eq!(i.subject.as_deref(), Some("later"));
        let i = p.update(&v, &Controls { pick: Some(2), ..Controls::default() }, 0.0, None).unwrap();
        assert_eq!(i.subject.as_deref(), Some("refuse"));
        assert_eq!(p.update(&v, &Controls { pick: Some(9), ..Controls::default() }, 0.0, None), None, "no ninth choice");
        let mut other = view(&["a", "b"], 8);
        other.talk.as_mut().unwrap().line.id = "c/b/l1".into();
        p.update(&other, &Controls::default(), 0.0, None);
        assert_eq!(p.focus(), 0, "a new line focuses its first choice");
    }

    #[test]
    fn the_layout_fits_and_a_click_picks_the_row_under_it() {
        let mut p = Presenter::new();
        let v = view(&["lend", "refuse"], 3);
        p.update(&v, &Controls { confirm: true, ..Controls::default() }, 0.0, None);
        for (w, h) in [(640, 360), (1280, 720), (1920, 1080), (800, 1280)] {
            let mut l = Layout::new(w, h);
            p.draw(&mut l, &v, w, h);
            let bad = l.check();
            assert!(bad.is_empty(), "{w}x{h}: {bad:?}");
        }
        let mut l = Layout::new(1280, 720);
        p.draw(&mut l, &v, 1280, 720);
        let (x0, y0, x1, y1) = l.rect_of("story-choice-1").unwrap();
        let i = p.update(&v, &Controls { click: Some(((x0 + x1) as f32 / 2.0, (y0 + y1) as f32 / 2.0)), ..Controls::default() }, 0.0, Some(&l)).unwrap();
        assert_eq!(i.subject.as_deref(), Some("refuse"));
        assert!(Presenter::captures_input(&v));
    }
}
