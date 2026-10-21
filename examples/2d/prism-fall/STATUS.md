# Prism Fall development

- Built with latest upstream RedEngine at session start: `91beedf852ec` (2026-10-21).
- Implemented deterministic falling-block rules, SRS rotation, seven-bag randomizer, hold, ghost, lock delay, scoring and three modes.
- Applied the requested simple UI: edge-to-edge centred board, no outer frame or gutters, black/charcoal background, compact score/line header, small hold and two tiny next-piece previews.
- Main verification: 37 checks, including a deterministic forty-piece / thirteen-line playthrough, frames, sound waveforms and save round-trip.
- Targeted regression scenarios: 34, including 912 bag draws across twelve seeds.
- Native Linux input and persistence smoke passed for 480 ticks, using a source-matched engine prebuilt.
- Release page and engine feedback travel with the game through the standard RedEngine publication workflow.
- Controller verification: nine tests compile the actual current engine mapping/module, including every D-pad direction, diagonals and release. The game has a full controller-action menu/gameplay scenario and switches prompts when a pad connects.
