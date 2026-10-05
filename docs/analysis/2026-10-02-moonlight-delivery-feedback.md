# Feedback: building Moonlight Delivery on RedEngine (revision 03499af9)

Source: an AI author's report after building a complete small game with no engine Rust changes: six physical parcels, collection tracking, a gate
that opens, optional collectibles, a timer and two endings, all as scene data and rules. Recorded verbatim in substance; triage notes are marked **Triage**.

## What worked (keep)
- **Rules as data.** "Parcel is inside the depot, not held, not already scored" expressed the mechanic directly; no inventory, physics or scoring code.
- **Verification tools.** Eight scripted scenarios covered movement, pickups, deliveries, progression, both endings, "carrying through the depot
  does not score" and "a parcel cannot score twice".
- **Automated graphical client testing.** Rendered frames, carrying state, rule variables and failures inspectable with no desktop.
- **Distribution.** The published Windows runtime matched the exact engine revision; the game is a small editable project with a pinned dependency.
- **Recommendation: keep this architecture; no rewrite.** The next gain is presenting and verifying a small game end to end.

## Opportunities, by the author's priority
1. **Test gameplay phases.** Static lint called the garden unreachable while its gate was closed, though a rule opens it later; the author redesigned
   the level (permanent side alley) to satisfy the tool. *Want:* named states (initial, gate opened, completed) with explicit reachability expectations.
2. **One action vocabulary for sim and graphical client.** Sim scripts aim at world coordinates; the graphical script needs hand-computed move
   durations and camera angles, so first pickup attempts missed. *Want:* object-based actions ("approach parcel", "look at parcel", "interact") with
   clear completion conditions in both paths.
3. **Game presentation.** Stock HUD looks like an engine overlay, instructions truncate, labels leak variable names (`Moon_stamps`). *Want:* a
   declarative interface for objective text, friendly labels, progress counters, start screen, completion screen, restart button.
4. **Authoring conveniences.** Sign lettering was hundreds of tiny boxes. *Want:* a sign/text component and helpers that generate valid IDs for
   repeated geometry.
5. **Fresh-machine start.** Needed Rust, native deps and an engine build before the self-describing CLI was usable. *Want:* Linux authoring
   binaries published beside the Windows runtime, plus a short bootstrap/doctor flow.

## Not engine defects (author's own)
Authoring began before the CLI was ready, with a guessed schema and missing IDs on lettering children (correctly rejected); a graphical test was
aimed wrongly and one assertion was too weak. Repeated compiler archive failures were cured by moving build output to a temp directory; root cause
not established (CLAUDE.md already advises `CARGO_TARGET_DIR`; worth checking whether the cause is disk allowance or a stale `target/`).

## Triage
- #1 and #2 are verification gaps and fit existing ladders (`lint`/`verify`/`sim` and the scripted client); #1 is the cleanest first slice.
- #3 overlaps the UI kit (ADR 0026) and the persisted audio/settings work; check what `src/ui/**` already exposes before designing.
- #4: check `describe` for an existing text/sign primitive before adding one (not verified when this was written).
- #5 is a release-pipeline change (publish Linux `red_engine2` CLI in releases) plus a `doctor` bootstrap hint.

## Status
- **#1 done:** ADR 2026-10-03 "Level states (phases) for lint, reach and walk"; `recipe gated_garden`.
- **#2 done:** ADR 2026-10-03 "Object-based player actions for sim and the scripted client": `approach` / `look_at` / `interact` by object id in `checks.sim` and the client script (`sim::approach`).
- Full CI green with both (2007 passed).
- Side finding: the `interact` event a peaceful scene injects cannot be used as a rule trigger (the validator says nothing emits it).
- **#4 done:** ADR 2026-10-03 "Lettering and arrays as macros": a `text` macro (signs from the engine font as merged boxes, generated ids) and an `array` macro (copies with generated ids).
- **#3 done:** ADR 2026-10-03 "A scene-declared game UI" (labels, counters, objective, start/end cards, restart).
- **#5 done:** ADR 2026-10-04 "Prebuilt Linux binaries and a bootstrap script" (PR #24).
