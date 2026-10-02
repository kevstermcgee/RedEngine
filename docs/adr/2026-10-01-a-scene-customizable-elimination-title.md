# 2026-10-01. A scene-customizable elimination title
Status: accepted
Summary: The death screen's ELIMINATED title is now optional per scene (death_text); a genre-neutral engine should not hard-code one genre's words

## Context
The standard client's death screen (`src/ui/online.rs`) has always shown a literal `"ELIMINATED"` title while a
player waits to respawn. That reads exactly right for an arena shooter (Trigger Happy, Killchain) and exactly
wrong for anything else: building a horror game this session, getting caught by the monster showed the same
"ELIMINATED / RESPAWNING IN N" an arena match would, undercutting the moment the whole scene is built around.
The engine already supports several genres natively (shooter, race, coin-collecting, horror) through scene data;
one hard-coded combat-shooter string was the one piece of UI that could not follow.

## Decision
- **`"death_text"`** (root scene key, optional string, default `None`), parsed like any other optional string
  key (`src/schema.rs::parse_scene`; added to `src/strict.rs`'s root allow-list and `red_engine2 describe scene`).
- **`None` means "ELIMINATED"**, unchanged — every scene that does not set this key keeps today's exact wording
  and layout; this is purely additive.
- **Threaded through, not reimplemented**: `CombatView` (`src/ui/online.rs`) gains `death_text: Option<String>`;
  the one rendering line that used the literal now reads `c.death_text.as_deref().unwrap_or("ELIMINATED")`.
  `src/bin/re2/feedback.rs::combat_view` passes `self.scene.death_text.clone()` through — the only two call
  sites that needed to change.
- **Only the title, not "RESPAWNING IN N".** The countdown line is mechanical information, not tone — scoped out
  to keep this change to exactly the one thing that was an actual, named problem, not a general reskin of the
  death screen.

## Consequences
A scene can now say what getting caught/killed/out actually means in its own words. Nothing else about the death
screen (layout, the respawn countdown, spawn protection) changed. To undo: remove `death_text` from `Scene`
(`schema.rs`, `strict.rs`, `describe.rs`) and `CombatView` (`ui/online.rs`, `feedback.rs`) — every scene reverts
to "ELIMINATED" automatically since that was always the fallback, never a separate code path.
