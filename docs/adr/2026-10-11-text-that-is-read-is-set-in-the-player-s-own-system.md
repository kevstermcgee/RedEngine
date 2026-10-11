# 2026-10-11. Text that is read is set in the player's own system serif
Status: accepted
Summary: Labels, maps and titles a player reads are drawn with ab_glyph in the first serif face installed (Georgia, Palatino's twin, Noto, Liberation or DejaVu Serif; RE2_FONT overrides), falling back to the bitmap capitals: no font file ships.

## Context
A museum is read: titles, artists, years, a paragraph of story under every piece, the name of each room, a map with wing names. The engine's only
type is a 5x7 bitmap of capitals, fine for a HUD's counters and unreadable for a paragraph. A font file in the repository would be an imported
binary asset (ADR 0008) with a licence to carry.

## Decision
`ui::serif::Serif` draws with `ab_glyph` (already in the dependency tree through winit's decorations; now a direct, `render`-only dependency) in the
first serif face installed: Georgia (Windows, macOS), Palatino Linotype, Times; on Linux P052 (Palatino's free twin), Noto, Liberation or DejaVu Serif;
`RE2_FONT=path` overrides. Italic is looked for alongside. With none it draws the bitmap capitals, so text always appears. `Image` is a small RGBA
target with blended rectangles, discs, lines and kerned, word-wrapped text.

## Consequences
- The museum's labels, titles, start card, pause menu and map look like a museum's; nothing ships.
- The same text looks a little different from machine to machine (the typeface found); tests use `Serif::bitmap_only()` where pixels matter.
- The engine's own screens (`ui::screens`) keep the bitmap capitals and their audits.
