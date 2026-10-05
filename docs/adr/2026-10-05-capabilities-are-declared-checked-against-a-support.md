# 2026-10-05. Capabilities are declared, checked against a support matrix, and never downgraded silently
Status: accepted
Summary: A game declares presentation, platforms, networking, input and persistence; unsupported combinations fail at validate with the reason and the way out, and every support level is Supported, Unverified, Prepared or Not supported.

## Context
Adding a second presentation (2D) and a second platform (the browser) creates combinations that look plausible and are not built (3D in a browser; authoritative UDP networking in a browser; a native window for a 2D game). A model asked for "a browser game with multiplayer" would otherwise get a smaller game, silently, or a build that fails late.

## Decision
A game declares `capabilities`: `presentation` (2d/3d), `platforms` (web/windows/linux), `networking` (offline/authoritative), `input` and `persistence`. `red2d::caps` holds the support matrix with four levels: **Supported** (a test runs it), **Unverified** (built, nothing here runs it: touch, gamepad), **Prepared** (the design allows it, the code does not exist: a native 2D window, browser-authoritative networking) and **Not supported** (3D in a browser). Only the first two pass. A failing declaration is an error at `validate` with the path, the target, the level, the reason and the way out, for example "Browser target cannot use the native UDP transport. Supported networking for web games: offline. A browser-compatible authoritative transport is architecturally prepared but not implemented yet." The same file is cross-checked against what it uses (a button needs `mouse`; `persist` needs `progress`; a music setting needs `settings`). `red_engine2 capabilities` prints the matrix and answers one question (`capabilities 3d web`). Nothing is ever downgraded: a target the engine cannot deliver is refused, never replaced by a smaller one.

## Consequences
Easier: a model learns what exists in one command and is stopped before it builds the wrong thing; the matrix is data a catalog can read. Harder: every new platform or mode must move a row from Prepared to Supported with a test that runs it, and the matrix text is part of the contract (tests pin the messages). To undo: delete `caps.rs` and the cross-checks in `game.rs`.
