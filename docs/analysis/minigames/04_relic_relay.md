# Minigame 04 Feedback: Relic Relay (Fragile Courier Dash)

## 1. Premise & Objective
- **Premise**: Fragile courier dash with decay timers and recharge stations.
- **Core Loop**: An unstable energy relic is secured at the start depot, initiating a containment decay timer. The player must run through an obstacle gauntlet, stepping onto a mid-point recharge station to stabilize the core before a meltdown occurs, and successfully delivering the relic into the final extraction vault.

---

## 2. RedEngine Implementation & Feedback

### What Worked Well:
- **Periodic Timer Rules**: `"when": { "every": 1.0 }` combined with condition `"if": "relic_picked == 1 && delivered == 0"` provides an ultra-clean, data-driven periodic tick loop without boilerplate.
- **Chained Event Triggers**: Rules triggering off `"when": { "event": "timer_tick" }` allow clean separation between timer advancement and critical threshold evaluation (`decay_timer >= 6`).

### Friction Points & Pain Points Encountered:
1. **Initial Zone Overlap Suppressing `enter`**:
   - Spawning the player directly inside `start_depot` resulted in `relic_secured` never firing (`events seen: none`). In RedEngine, a player who spawns inside a volume receives no `enter` event. To fix this, authors must either spawn players outside the trigger volume or use `"when": { "start": true }`. Future AIs need to be wary of zone bounds overlapping spawn points.
2. **Carry Simulation vs Abstract Flag**:
   - RedEngine has full physical prop carrying (`interact: true` picks up props within 2.3m, with throw mechanics and `held_by` assertions). For simple minigames, abstracting the item as an enter zone + status flag (`relic_picked`) is much easier to script in `checks.sim` than timing physical look-at and pickup raycasts.

### Proposed Engine Patches for RedEngine:
- **`spawn_inside: true` option for zones**: Allow zones to declare `allow_spawn_enter: true` or automatically fire `enter` on tick 0 if a body spawns within their bounds.
- **Visual Inventory Slot**: Add a standard screen-space inventory slot in the client HUD for carried game variables (e.g. `relic_held`).

---

## 3. BlueEngine Implementation & Feedback

### What Worked Well:
- **Explicit Timer Lifecycle**: `start_timer` and `stop_timer` actions give precise programmatic control over when countdowns commence.
- **Action Sequence Safety**: The 4-action limit easily accommodated picking up the item, starting the timer, disabling the pedestal, and enabling the next objective.

### Friction Points & Pain Points Encountered:
1. **No Direct Failure Outcome Action**:
   - In BlueEngine's GameDocument schema, actions only include `complete` (which marks the game won). There is no native `fail` or `defeat` action in the declarative schema! If a timer expires and the player fails, the engine can only stop or mutate counters, but cannot explicitly emit an authoritative defeat outcome without custom-sim Rust code.
2. **Decoupled Physical Entity State**:
   - The relic prop on the pedestal does not physically attach to the player's avatar when picked up; authoring visual carrying requires manually hiding the pedestal entity and showing an avatar-attached mesh.

### Proposed Engine Patches for BlueEngine:
- **Add `fail` Action**: Add `{"action": "fail", "reason": "string"}` to the GameDocument action schema alongside `{"action": "complete"}`.
- **Attach Node Action**: Add an action `{"action": "attach_to_player", "entity": "id"}` for visual courier/pickup minigames.

---

## 4. Key Takeaways for Future AIs
- In **RedEngine**: Never place spawn points inside `when: {enter}` trigger zones; keep spawns at least 0.5m outside.
- In **BlueEngine**: Declarative GameDocuments currently only support `complete` as an end state; model failure as an un-completable counter state or fallback timer loop.
