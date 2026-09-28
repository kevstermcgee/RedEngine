# 2026-09-28. Remove the revolver: ten firearms in one model, ammunition is a game-wide setting
Status: accepted
Summary: The silver revolver, the one weapon with its own model file, constants, sound and scene key, is gone; ten data-driven firearms remain and `weapons.ammo` replaces `weapons.revolver.ammo`.

## Context
ADR 0013 added the revolver before ADR 0038 introduced the data-driven arsenal, and it stayed a special case long after every other gun was one `FirearmSpec` row plus a procedural model:
its own model file (`revolver.rs`), its own damage/cooldown/range constants, its own synthesized sound, a `weapons.revolver` block in the scene (damage and the ammunition every gun shared),
and a handful of `Weapon::Revolver` branches in the simulation, the bots, the audio and the client. Building Trigger Happy on the engine showed the cost: the revolver did not behave like its
siblings (it was the only gun whose damage came from a different key and whose ammunition setting lived under its name), and the game's authoring script had to special-case it. The request
was to take it out of the engine and out of future games instead of keeping an exception.

## Decision
- `Weapon::Revolver`, `revolver.rs`, `REVOLVER_*`, the revolver sound and the `weapons.revolver` scene key are removed. `Weapon::ALL` is the bat plus ten firearms (`Weapon::FIREARMS`);
  the pistol is now the first weapon after the bat in scroll order, and a new gun is one `FirearmSpec` row plus one procedural model with no other code path.
- Ammunition is `weapons.ammo` (`"infinite"` by default, or `{loaded, capacity, reserve}`), one setting for every firearm. Each player still has a single ammunition state shared by their guns
  (ADR 0038), so the behaviour of an existing limited-ammo scene is unchanged apart from the default magazine size (`DEFAULT_MAGAZINE` 30 and `DEFAULT_RESERVE` 90 when only some numbers are given).
- Scenes that still say `weapons.revolver`, `weapons.starting: "revolver"` or `"revolver"` in a ladder do not silently degrade: `validate` fails with the replacement (`weapons.ammo`, `pistol`).
- The number of a weapon on the wire is its index in `Weapon::ALL`, so removing an entry renumbers the others: **protocol version 9**. Old clients and servers refuse each other at the hello.

## Consequences
- Every firearm is tuned the same way (damage, cadence, range, impulse and recoil in `FirearmSpec`; nothing per weapon lives in the scene except `bat_damage`, `starting` and `ladder`), which is what a
  game's tuning tooling and `bot-match` can rely on.
- Games and scenes that used the revolver (Trigger Happy's ladder had it as rung 1) edit their scene once; `validate` names each place. The Trigger Happy ladder is now ten firearms and the bat.
- A trace checksums each player's combat state, which includes the weapon's number on the wire, so a trace recorded before this change in which someone held a firearm no longer replays (the committed
  `coin_run` fixture never leaves the bat and is unaffected). The weapon-dependent proofs (`interactions`, `net_interactions`, `combat_rules`) now use the pistol.
- ADR 0013 is superseded by this one; the text of ADR 0038 that calls the revolver "compatible" describes the state it was written in.
