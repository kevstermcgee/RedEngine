# Feedback: Prism Fall (2026-10-21)

- By: kevstermcgee (AI session)
- About: engine
- Context: Created Prism Fall against main 91beedf852ec, published through the native 2D route. The user requested a simpler, centred board with two small previews. All findings below were encountered in this session; cost estimates are approximate.

## Findings

```json findings
[
  {
    "id": "F1",
    "key": "no-conditional-action-or-array-vars",
    "area": "format",
    "title": "A 10-by-22 falling-block well needs encoded rows and hundreds of expanded rules",
    "severity": 2,
    "cost_min": 30,
    "evidence": "examples/2d/prism-fall/build.js encodes ten colour cells into each of r0..r21 as base-eight digits. probe selects a row and a power of eight with expanded equality expressions; row compaction is generated one assignment per row. The resulting game has 365 rules and over 1500 widgets, most of them seven conditional images per board cell.",
    "workaround": "A dependency-free JavaScript generator, exact 30-bit row values and a temporary-fixture regression runner. The shipped game still uses the real RedEngine simulation and renderer.",
    "proposal": "Add bounded indexed numeric arrays, indexed reads/writes, and a small palette-driven grid widget. Preserve deterministic simulation, validation and trace hashes. A ten-by-twenty grid should not need 1400 image widgets."
  },
  {
    "id": "F2",
    "key": "no-2d-keycode-press-trigger",
    "area": "format",
    "title": "Independent rotation and hold keys require offscreen buttons",
    "severity": 2,
    "cost_min": 5,
    "evidence": "describe 2d exposes only seven press actions. Prism Fall binds Q to reverse rotation, E to clockwise rotation and C to hold through 1x1 buttons at (-40,-40). The same workaround is reported in the Engine notes on RedEngine PR #201.",
    "workaround": "Offscreen HUD buttons with KeyboardEvent.code keys, while the normal action mapping continues to support gamepads.",
    "proposal": "Add declared named input actions with keyboard codes and optional pad bindings, usable by press and held expressions. Alternatively accept a keyboard code directly in press without requiring a HUD widget."
  },
  {
    "id": "F3",
    "key": "releases-not-validated-against-engine",
    "area": "verify",
    "title": "Native input evidence still needs an external X11 input harness",
    "severity": 2,
    "cost_min": 10,
    "evidence": "verify correctly distinguishes simulation, render and audio-waveform claims from native input. To test the latest source-matched prebuilt in an actual window, this session launched Xvfb, used libXtst key events, captured the screen with ffmpeg and inspected a native save after 480 ticks. --max-ticks alone would only prove startup without exercising Enter, held arrows, Q, C, Space, pause or fullscreen.",
    "workaround": "An external temporary X11 harness exercised native start, repeat movement, both rotations, hold, drop, pause/resume, fullscreen and persistence. Main verification also checks a forty-piece thirteen-line playthrough.",
    "proposal": "Add play2d --drive with recorded key codes/buttons, native screenshots and a final structured input/save report. Use the packaged Windows player in a release smoke that checks both input and saved records, keeping simulation and platform claims separate."
  },
  {
    "id": "F4",
    "key": "2d-sim-json-text-only",
    "area": "cli",
    "title": "The 2D sim JSON envelope contains text rather than scenario state",
    "severity": 1,
    "cost_min": 4,
    "evidence": "red_engine2 sim examples/2d/prism-fall.game2d.json --only 'start move rotate hold and hard drop' --json returns data.text containing 'vars: phase=1 ...', plus textual events and coordinates. It does not return scenario arrays or a vars object. A temporary showcase planner had to parse name=value tokens out of that text.",
    "workaround": "Use engine expectations for the committed regression checks; parse the text only in the temporary script that selected actual placements for the gallery/showcase sequence.",
    "proposal": "Return scenarios with typed vars, events, sounds, entity positions, ticks, failures and hash in data. Retain data.text for human display if needed."
  },
  {
    "id": "F5",
    "key": "lint-2d-routes-to-3d",
    "area": "cli",
    "title": "lint parses a .game2d.json as a 3D scene and prints misleading prefab errors",
    "severity": 1,
    "cost_min": 1,
    "evidence": "scripts/dev red lint examples/2d/prism-fall.game2d.json reported dozens of unknown tag/shape/size/layer prefab fields, while validate and verify pass for the same file. AGENTS has a general never-trust-an-edit-without-lint rule, followed by a separate 2D validate/verify workflow.",
    "workaround": "Use validate and verify for 2D content; no engine change was needed to ship the game.",
    "proposal": "Dispatch lint by the game2d header or file suffix, or return one short diagnostic directing the author to validate/verify for 2D instead of attempting the 3D schema."
  }
]
```

## What helped

`scripts/dev start` found an exact-source prebuilt of today's main without a compile. `describe 2d` was sufficient for the native file format. Strict validation named the reserved `level` variable and invalid effect names immediately. Deterministic scripted checks, exact shared screenshots, measured sound waveforms and save round-trip checks made a complete game practical without changing any engine code. Thirty-seven engine checks and thirty-four targeted regression scenarios pass; the latter include 912 bag draws across twelve seeds.

## Product feedback from this session

The initial UI used large cards and decorative framing. The user repeatedly asked for less border, a centred board, only one or two small previews, and black/charcoal rather than navy. The final 300-by-644 layout makes the 300-by-600 well the entire width, with no outer frame or side gutters. A 44-pixel header holds score, hold and two eight-pixel-cell previews. Instructions appear in the menu and pause screen. Default game starters and suggested layouts should make the playable area dominant before adding decorative panels.
