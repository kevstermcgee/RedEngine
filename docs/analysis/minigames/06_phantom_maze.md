# Minigame 06 Feedback: Phantom Maze (Blind Sonar Navigation)

## 1. Premise & Objective
- **Premise**: Cloaked labyrinth navigation with pulse sonar illumination.
- **Core Loop**: The player is placed in a darkened maze where internal partitions are visually hidden. Stepping onto acoustic sonar beacons pings the environment, revealing cloaked obstacle walls (`phantom_wall_1`, `phantom_wall_2`) and charting safe paths. The player must navigate around the revealed barriers to reach the exit portal.

---

## 2. RedEngine Implementation & Feedback

### What Worked Well:
- **Offscreen off-the-shelf visibility toggling**: `"do": [{"show": "phantom_wall_1"}]` and `"hide"` commands manipulate scene node presentation without interrupting physical collision.
- **Off-mesh Navigation Collision Audit**: In `checks.sim`, the simulator detected that the player collided with `phantom_wall_2` at `(1.45, -4.99)`, providing exact tick and coordinate feedback so the author could adjust routing waypoints around the obstacle boundary.

### Friction Points & Pain Points Encountered:
1. **Collision Still Active While Hidden**:
   - In RedEngine, props with `collide: true` remain solid barriers even when hidden via `{"hide": "object_id"}`. If an author wants walls to be ethereal until discovered, both `hide`/`show` and `{"collision": [id, bool]}` must be coordinated.
2. **Lack of Timed Auto-Fade (Sonar Decay)**:
   - A true sonar pulse should illuminate walls for a brief period (e.g. 3 seconds) before fading back to darkness. RedEngine rules do not currently support a `"delay"` or `"duration"` parameter inside a `show` action (e.g. `{"show": "wall_1", "duration": 3.0}`); instead, a secondary `"after"` or `"every"` rule is required to turn visibility back off.

### Proposed Engine Patches for RedEngine:
- **Temporary Visibility Action**: Add a duration parameter to `show`/`hide`: `{"show": "object_id", "for_seconds": 3.0}`.
- **Volumetric Sonar Wave**: Add an engine visual effect primitive for an expanding spherical ring wave triggered by `emit`.

---

## 3. BlueEngine Implementation & Feedback

### What Worked Well:
- **Deterministic Checksum Validation**: State transitions for sonar pings and exit activation verified across headless simulations with zero discrepancies.
- **Clean Action Model**: Incrementing `ping_count` and disabling the beacon entity cleanly gates the exit portal.

### Friction Points & Pain Points Encountered:
1. **`set_visible` Presentation Decoupling**:
   - In BlueEngine's `GameDocument`, `set_visible` only affects presentation entities; it explicitly never alters collision (`"set_visible: interactable presentation only; never changes eligibility or collision"`).
2. **No Dynamic Material Alpha / Fade**:
   - BlueEngine nodes have static material references; there is no runtime action to interpolate opacity or emission intensity, making pulse fade-out effects impossible without external custom shaders.

### Proposed Engine Patches for BlueEngine:
- **`set_material_property` Action**: Allow declarative rules to adjust emission or alpha: `{"action": "set_emission", "entity": "id", "value": 1.0}`.
- **Sound Trigger Binding**: Allow `on_interact` to directly trigger an engine audio asset playback.

---

## 4. Key Takeaways for Future AIs
- In **RedEngine**: Remember that `show` and `hide` do not alter collision; use `{"collision": [id, true/false]}` if the physical obstacle should disappear alongside the visuals.
- In **BlueEngine**: `set_visible` affects interactable rendering only; true wall phasing requires moving the collider via `movers` or custom simulation.
