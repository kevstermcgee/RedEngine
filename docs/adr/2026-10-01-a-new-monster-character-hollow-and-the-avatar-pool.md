# 2026-10-01. A new monster character: Hollow, and the avatar-pool sizing bug it caught
Status: accepted
Summary: A genuinely distinct, unsettling costume (not a recolor) reusing the human rig, and the fix for a latent out-of-bounds panic in the avatar pool that any 9th Character would have hit

## Context
"The Hollow"'s monster used `Character::Alien` — the ordinary human rig in a green recolor, the same
construction every costume (Wizard, Cowboy, Robot) shares. It read as a person in a costume, not a monster, and
the user asked for an actual new model that is genuinely unsettling.

A new skeleton (different bone proportions, not just a recolor) is out of scope: it would touch animation,
hit-shapes (`hit::raycast_shapes` uses the real character meshes, SPEC "Characters, hit-testing"), and every
costume test, for one character. The existing costume mechanism is the right-sized tool instead —
`src/costumes.rs::decorate()` already appends real procedural geometry per `Character` onto fixed rig bones
(Alien's antenna, Wizard's hat, the soldier kits), and `ObjectKind::Humanoid`'s `height`/`build` fields already
scale a whole body (`src/characters.rs::human_object`) — both already proven, reusable levers, just never
combined into something frightening before.

**The bug this surfaced**: `Character::ALL` was already 8 entries (`Ridgeback`/`Nightfall`, Killchain's team
skins, had already grown it from 6) when `net::session::avatars_made` was written, but its return type stayed a
hardcoded `[usize; 6]`, indexed by `body_index()` which walks the real `Character::ALL`. It never panicked
because `Ridgeback`/`Nightfall`'s avatar-pool count is only ever non-zero inside a team (`shooter`) match
(`avatar_plan`'s soldier gate zeroes it otherwise), so `build_avatar_pools` always skipped them before the
6-wide array could be indexed at 6 or 7. **A 9th character used by an ordinary (non-team) bot roster — exactly
what a new monster needs — would have indexed `made[8]` on a 6-slot array and panicked**, taking `game check`
down with it (`src/tools/game.rs::avatar_line` calls `avatars_made` directly). `AvatarPlan`'s `pool`/`needed`
fields had the same hardcoded-8 problem one layer up, for the same underlying reason.

## Decision
- **`src/net/session.rs`**: `BODY_KINDS: usize = Character::ALL.len()` is now the one definition driving every
  array these functions use (`AvatarPlan::pool`/`needed`, `avatars_made`'s `made`) — sized arrays can never
  silently drift behind `Character::ALL` again; adding a 10th character and forgetting to touch this file is now
  a compile error (array-length mismatch), not a runtime panic reachable only by the exact body/scene
  combination that happens to need the missing slot. Regression test:
  `avatars_made_never_panics_for_any_character_all_entry_in_an_ordinary_bot_roster` builds a one-bot scene for
  every real fighting body (`Character::ALL` minus `Rat`, which cannot be a bot) and asserts it never panics and
  always reports a non-zero pool — this is what would have caught the bug before it shipped.
- **`Character::Hollow`** (`src/player.rs`): shares the existing Human/Wizard/.../Nightfall `BodySpec` — same
  collision, eye height, speed. Only the visual differs, deliberately, so this cannot change how any map's
  movement or hit-testing feels.
- **`src/characters.rs::character_object`**: `height: HUMAN_HEIGHT * 1.28` (~2.3 m), `build: 0.82` — unnaturally
  tall and gaunt, the cheapest and most reliable "this is wrong" silhouette cue (Mr. X, Nemesis, Slenderman all
  lean on exactly this). Shirt color matches skin (near-black, `#0d0c10`-ish) — one wrong-colored shape, not a
  person wearing black.
- **`src/costumes.rs::decorate()`**: a larger, elongated, off-axis head shape fully encloses the rig's built-in
  face (eyes/brows/nose/mouth are still drawn underneath, by `human_parts`, for every costume — this one just
  visually swallows them rather than carving an exception into shared code); two small pale points deep in the
  head shadow stand in for eyes (no emissive glow — `CharPart` has no emissive field, and adding one is a
  render-pipeline change, not a character-data one; see Consequences); thin cone claws well past the ordinary
  hand position. No accessories, nothing symmetric or colorful — restraint instead of a costume-shop prop.
- Verified by rendering, not just tests: `the-hollow`'s roster switched to `"hollow"`, built, and inspected via
  `playtest` at medium range (a tall black figure, unmistakably not human-shaped, barely visible in the dark —
  exactly the intent) and close range (looms over the camera, a claw clearly in frame).

## Consequences
The monster is now visually load-bearing on its own, not costume-dependent lighting tricks. The avatar-pool
sizing class of bug cannot recur silently for a future character. Not done: emissive glowing eyes (the
high-contrast-dot approach was judged sufficient by the actual render; revisit only if it doesn't read well in
real play, since that would mean touching `CharPart`/the render pipeline, a bigger change); any sound design
(no growl/breathing cue — `sfx.rs` synthesis work, out of scope here). To undo: remove `Character::Hollow` (and
its four match-arm additions: `body()`, `name()`, `parse()`, `character_to_wire()`) and the `costumes.rs`/
`characters.rs` arms; the `BODY_KINDS` fix in `session.rs` should stay regardless — it is correct independent of
this character.
