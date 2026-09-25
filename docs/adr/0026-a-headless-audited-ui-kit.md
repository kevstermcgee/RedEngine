# 0026. A headless, audited UI kit: `ui-shot` and `ui-check`
Status: accepted

## Context
The game UI is a CPU-painted 2-D overlay using the engine's 5x7 bitmap font, laid out with magic numbers
(`ph = 126 * s`, `y0 + 42 * s`). Working on it was blind: an AI had to write a throwaway test that dumped a PPM, convert it
with Python and read the PNG, and that exposed real bugs (a taller panel clamped up into the title at 720p; messages wider than
their panel). Auditing the engine's own launch menu by reading the code found more (text wider than the window at portrait and
small sizes). None of it was catchable by a test because painting and layout were the same untestable code.

## Decision
`src/ui/` is renderer-free (it builds in the headless server configuration and in game-project tests):
- A screen is a `Layout`: widgets (`Panel`, `Button`, `Label`) with rectangles, text, colours and containers. **One list drives
  painting, hit-testing and the audit**, so what is checked is what is drawn and where a click lands.
- Helpers measure for you: `label_fit` (shrinks to the largest scale that fits, then ellipsizes), `label_wrapped`, `wrap`,
  `fit_scale`, `ellipsize`.
- `Layout::check` reports `offscreen`, `outside-container`, `text-overflow` (buttons) and `overlap` (any two text/button widgets).
- `red_engine2 ui-shot <screen> out.png --size WxH --hover resume --message "..."` renders a screen to a PNG with no window or GPU;
  `red_engine2 ui-check` audits every registered screen at nine window sizes (small, 720p/1080p/1440p, 4:3, portrait) and is also a
  unit test. The engine's launch and pause menus were ported onto it (`ui/screens.rs`; `menu.rs` re-exports the old API) and the
  overflow bugs the audit found were fixed.

## Consequences
A new screen is a function `(w, h, options) -> Layout` registered in `screens::build`; it is then covered at every size for free.
The look is unchanged (same font, colours, proportions). Text is bitmap-only (ASCII upper case); no font files, no layout engine:
if a game needs rich text or flexible flow, that is a different tool. The 3-D backdrop of the launch menu is still drawn by the
live renderer, so `ui-shot` shows the overlay over a gradient, not over the models.
