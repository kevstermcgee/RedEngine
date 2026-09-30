# Minigame 10 Feedback: Target Gallery (Reaction Marksmanship)

## 1. Premise & Objective
- **Premise**: Fast-reaction popup marksmanship trial.
- **Core Loop**: Mechanical silhouette targets pop up at varying depths and lateral offsets. The player must acquire targets and strike/interact with each target before resetting. Accumulating 3 target hits satisfies the marksman qualification quota, unlocking the victory prize booth.

---

## 2. RedEngine Implementation & Feedback

### What Worked Well:
- **`hide` State Integration**: Toggling target visibility upon hit (`{"hide": "target_1"}`) makes targets cleanly drop down or vanish on impact.
- **Event-Driven Score Tracking**: Tracking hits via `{"add": ["targets_hit", 1]}` and triggering victory once the score threshold is satisfied works out of the box in headless simulation.

### Friction Points & Pain Points Encountered:
1. **Raycast Shooting vs Walking Proximity**:
   - RedEngine has a complete hitscan system in its player weapon simulation (e.g. bat swings, pistols), but authoring a purely declarative gallery with hitscan shooting from a fixed shooting rail requires either weapon items in the scene or abstracting the shots via walking/aiming pads.
2. **Dynamic Target Respawning**:
   - RedEngine rules do not currently have a declarative `loop` or `respawn` rule that automatically pops a target back up after a random delay without chaining multiple timer rules.

### Proposed Engine Patches for RedEngine:
- **Declarative Hitscan Target Component**: Add an object type `{"type": "target", "on_shot": "event_name"}` that natively triggers rules when hit by a bullet or projectile.
- **Random Delay Timer Action**: Add `{"delay_random": [min_sec, max_sec]}` to rules.

---

## 3. BlueEngine Implementation & Feedback

### What Worked Well:
- **`set_visible` & `set_enabled` Dual Toggling**: Calling both `set_visible: false` and `set_enabled: false` cleanly deactivates the target entity visually and prevents double-clicking.
- **Interaction Ray Distance**: BlueEngine's 2.5m interact raycast allows shooting/tapping targets from a realistic distance without clipping into their colliders.

### Friction Points & Pain Points Encountered:
1. **Lack of Native Projectile Interaction**:
   - In BlueEngine's declarative `GameDocument`, interaction is exclusively bound to the `E` key (within 2.5m). There is no declarative concept of shooting a weapon, throwing a dart, or casting a crosshair raycast beyond 2.5m without building a custom simulation.
2. **Missing Moving Target Rails**:
   - Standard shooting galleries feature moving targets sliding along tracks. In BlueEngine, while `movers` exist, they are primarily designed for sliding doors/elevators with open/close states rather than continuous cyclic patrol paths.

### Proposed Engine Patches for BlueEngine:
- **Ranged Interaction Action**: Allow interactable entities to specify `max_distance: float` (e.g. 10.0m) to simulate long-range marksmanship.
- **Continuous Path Movers**: Extend `movers` to support continuous ping-pong or looping patrol paths (`"loop": true`).

---

## 4. Key Takeaways for Future AIs
- In **RedEngine**: `hide` drops targets visually while keeping spatial coordinates intact.
- In **BlueEngine**: Always set both `set_enabled: false` and `set_visible: false` when knocking down interactive targets to prevent phantom interactions.
