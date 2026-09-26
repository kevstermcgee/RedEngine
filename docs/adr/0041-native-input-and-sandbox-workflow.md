# 0041. Native input and a reusable sandbox workflow
Status: accepted

## Context
Keyboard-only movement prevented controller testing, and assets and reference maps were difficult to inspect together.
The two-character picker also coupled presentation to gameplay body choices.

## Decision
Use optional gilrs under the gfx feature for native controller input (Windows Gaming Input, macOS and Linux backends).
Keep stick shaping, button edges and focus/disconnect rearming in a renderer-independent controller module.
Input flag bit 7 selects signed 127-step analog axes; digital input is unchanged. Shared movement clamps total wish strength.
Protocol v7 carries those axes without increasing packet size; traces already record axes and flags.
Controller actions call the same existing gameplay/menu actions as keyboard input.

Wizard, cowboy, alien and robot use the human collision/movement/weapon rig and stable bone indices.
Their costume geometry lives in costumes; humanoid.style permits scene showcases and Character wire values 2-5 select playable versions.
Who::Human rules include all human-rig costumes. The rat retains its distinct body.

The graphical client's local project browser discovers declared game.json maps and retains the native window/device during travel.
RedEngineSandbox is a standalone RedEngineGames project, with generated self-contained maps and a complete asset-index.json.
Its generator refreshes galleries from catalog --json rather than maintaining a duplicate asset list.
Reference animation scenes receive explicit playable-perimeter adaptations; source demos remain unchanged.

## Consequences
Headless builds do not depend on gilrs; Linux gfx builds need libudev-dev.
One connected controller owns gamepad input until disconnect. Controls must return neutral after focus loss or menu transitions.
The connect form still needs a keyboard for arbitrary address/key/name text; controllers navigate fields and confirm.
Rumble, remapping UI and simultaneous local multiplayer are outside this change.
Map travel is local-only; it does not disconnect or migrate an online session.
Physical controller hardware testing must be distinguished from synthetic input and build tests.
