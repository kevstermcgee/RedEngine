# 2026-10-04. The boy, the third-person start and a clock that runs in the live client
Status: accepted
Summary: A child character on the human rig, a scene setting to open behind the character, and scene time that follows game ticks when a clock block exists, so the day turns in the real client and in headless scripts.

## Context
Marcel is about a young boy wandering a world whose day turns to night. The engine had adult bodies only (and a rat), always opened in first person, and animated scenes by wall-clock time modulo a `duration`, which is 0 for a game: a `clock` scene never left its first instant in the real client, and a headless script could not see a sunset.

## Decision
- `Character::Boy` ("boy", "child"): the human rig with child proportions (`HumanoidRig::child`: head 1.4x the adult share, shorter legs and reach), 1.35 m tall, a red-orange jumper, messy hair, a scarf and a small backpack; a child's body numbers (eye 1.18 m, walks 2.9 m/s and runs 5.4, no bat, a closer chase camera). Chosen with `player.humans_play_as: "boy"`. It travels as wire byte 9 (the field is a full byte, so the protocol is unchanged); bots never wear it.
- `player.view: "third"` opens the game behind the character (Q still toggles); the chase camera treats generated trees as walls and never dips below the hill behind the boy.
- `App::scene_time()`: in a scene with a `clock` the animation time is the game's own seconds (fixed steps run), not wall time. A paused game keeps its sunset; a headless script (`re2 --headless --script`, `wait: 120`) lives through a day in a moment; screenshots stream the whole world in first (`LiveRenderer::settle_stream`).

## Consequences
A child on the adult rig needed `rig.height` (the nominal height) instead of deriving it from the hip, and every humanoid goes through `HumanoidRig::for_look`. The first-person view of a boy is possible but his eye height is only right for the chase camera's anchor; Marcel plays in third person. Open: no child animations beyond the shared walk cycle, no clothes variants.
