# 2026-10-03. Level states (phases) for lint, reach and walk
Status: accepted
Summary: A scene names the states its rules create, so lint, reach, walk and verify check a gate closed and open instead of calling what is behind it unreachable.

## Context
An author building a small game (Moonlight Delivery, `docs/analysis/2026-10-02-moonlight-delivery-feedback.md`) had a garden behind a gate that a
rule opens once the player has delivered enough parcels. `lint` reported the garden `unreachable`, because the analysis tools build their world once, from
the scene as authored. `MapWorld` already opened what an unconditional `start` rule switches off (ADR 2026-09-29 verification-honours-the-map) and nothing
else, since "the tools cannot know when" a conditional rule fires. The author redesigned the level (a permanent side alley) to satisfy the tool, which is
the tool dictating the design. It also meant no check could say the garden was sealed at first AND walkable afterwards.

Two simpler routes were rejected. Treating every rule-openable object as open everywhere would make lint blind to a gate that never opens and could not
express "still sealed". Simulating the game to find the states would make a static tool depend on a play-through and on the scripted player's skill.

## Decision
- A scene may declare `"phases": {"name": ["rule_id", ...]}`. A phase is the set of rules the tools assume have fired. Ids are validated when the scene
  loads (`parse_phases`, with a did-you-mean); the implicit first state is `initial`.
- `RuleSet::open_objects(phase)` is the one place that decides which top-level objects are non-solid: the unconditional `start` rules first, then the
  phase's rules in order, honouring `collision` (both ways), `deactivate` and `activate`. It replaces the `start`-only filter in `MapWorld`, which also
  ignored `deactivate` (a `start` rule using it left the gate solid to the tools).
- `MapWorld::from_text_phase` / `load_phase` build the world for a phase; `checks.reach[]` and `checks.walk[]` take `phase`, `reach` takes
  `"reachable": false`, and `reach|lint|walk|plan` take `--phase`.
- `lint_phases` runs lint in every declared phase: a `zone`, `floor` or `unreachable` finding that some phase resolves is dropped; a finding that exists only
  in a phase is added with a `[phase name]` prefix. A scene without `phases` lints exactly as before.
- The game and `checks.sim` are untouched: phases are an assertion about the level, not a mechanism. The scenario that really opens the gate stays the
  proof that the phase is honest. `recipe gated_garden` shows all of it.

## Consequences
- A designer states the level's states once and gets reach, walk, lint and plan for each; a gate that must stay shut is a `"reachable": false` check.
- A phase can lie (list a rule that never fires in play). Nothing links them mechanically, so keep a `checks.sim` scenario per phase; a later step could
  have `sim` assert the phase's rules actually fired.
- Only collision effects are modelled (what the tools reason about). Phases that move props or teleport are not modelled.
- Lint costs one more reach flood-fill and lint pass per declared phase (seconds on a big map; none for scenes without phases).
- Undo: remove `phases`; every tool falls back to the initial-state behaviour.
