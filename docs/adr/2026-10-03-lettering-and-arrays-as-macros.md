# 2026-10-03. Lettering and arrays as macros
Status: accepted
Summary: A text macro draws signs from the engine font as merged boxes and an array macro repeats a template with generated ids, so neither needs hundreds of hand-named boxes.

## Context
Moonlight Delivery's author (`docs/analysis/2026-10-02-moonlight-delivery-feedback.md`, item 4) built sign lettering from hundreds of tiny boxes, and the engine
correctly rejected the children they had not named. It worked, but it was bulky, easy to get wrong, and invisible to anyone reading the scene. The engine already had a
5x7 bitmap font (for labelling analysis images and the 2-D UI) and a macro mechanism (`wall`, `fence`) that expands one compact description into ordinary boxes at parse time,
so nothing downstream (rendering, collision, lint, `reach`) has to know macros exist.

## Decision
- `text` (`macros::expand_text`): the font's pixels for the text are merged into the fewest rectangles by a greedy sweep (first free ink pixel, grow right, grow down), each a box
  named `<id>.l<row>_<col>` (stable under edits to other letters). A 17-letter sign is a few dozen boxes, not hundreds; `merge_pixels` is tested to cover every pixel exactly once.
  It reads along +X and faces +Z, centred on `position`, with `height`, `depth`, `align`, `spacing`, `line_gap`, an optional `backing` board and a `material`. The group takes the
  object's `position`/`rotation`/`scale` and is decoration (`collide: false`) unless it says otherwise, so a sign flush on a wall does not collide with it or trip lint.
- A character the font lacks is an error listing the characters it has (computed from the font, not a copied list), not a silent blank.
- `array` (`macros::expand_array`): copies of a template with generated ids `<id>.<n>`, by `count`+`step` or explicit `positions`, with `rotation_step`. A group template's descendants are
  renamed per copy so ids stay unique. This is the "helper that generates valid ids for repeated geometry": the author never writes an id for a copy.
- Both are macros, like `wall` and `fence`: JSON-to-JSON, unit-testable without a GPU, invisible to everything downstream. Both are in `describe objects` (examples are test-parsed),
  SPEC, and `strict::object_keys`, so a misspelt field gets the usual did-you-mean.
- `recipe gated_garden` has a sign on the courtyard wall (lint stays clean); the picture was looked at, not just asserted.

## Consequences
- A sign is one short object and lint-clean. Lettering is axis-aligned boxes at the font's pixel size: it is blocky by design and not a typography system (no lowercase, no
  kerning, no curves). Large text is large pixels.
- A text object's size is `7 * (height/7)` tall and `width_in_px * height/7` wide; there is no automatic fit-to-board. `backing.margin` pads it.
- `array` templates move by a fixed `position` (a keyframed track is refused), and copy limits are 500. It repeats, it does not scatter; `scatter` remains the tool for that.
- Not done: a `text` that follows a curve, per-letter colour, and a runtime (not scene-time) text; the HUD already has its own text (`ui`).
