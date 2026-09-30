# Minigame 09 Feedback: Weight & Balance (Pressure Equilibrium)

## 1. Premise & Objective
- **Premise**: Physics-based counterweight scale / balance pan equilibrium puzzle.
- **Core Loop**: A hydraulic vault lock gate is anchored to a dual-pan balance scale (Left Pan vs Right Pan). The player must distribute ballast masses evenly across both scale platforms until the system reaches mechanical equilibrium (`weight_left == weight_right`), triggering the counterweight release latch.

---

## 2. RedEngine Implementation & Feedback

### What Worked Well:
- **Loose Prop Queries**: RedEngine provides native functions (`props_in(zone)`, `mass(id)`, `prop_y(id)`) designed specifically for weighing physics props on surfaces.
- **Rapier Physics Integration**: Loose props feature full mass and gravity simulation, meaning physical props stacked on platforms naturally exert physical force and trigger spatial volumes.

### Friction Points & Pain Points Encountered:
1. **Dormant Physics Initiation**:
   - In RedEngine, loose props stay dormant until physically disturbed or shoved by an impulse/player contact (ADR 0012). If an automated crane or script places a prop into a zone without a velocity kick, Rapier keeps it dormant until disturbed, which can delay continuous volume intersection checks unless `prop_enter` triggers are utilized.
2. **Dynamic Platform Tilt Animation**:
   - When one side of a balance scale is heavier than the other, visually tilting the beam requires either physics constraints (revolute joints) or scripted rotation. While Rapier supports rigid bodies, RedEngine's scene JSON format does not expose joint constraints (like hinge joints or springs) declaratively in JSON.

### Proposed Engine Patches for RedEngine:
- **Declarative Physics Constraints**: Expose Rapier joint constraints in scene JSON: `{"type": "revolute_joint", "body_a": "beam", "body_b": "fulcrum", "anchor": [0,0,0]}`.
- **Dynamic Scale Balance Helper**: Add a built-in macro for seesaw/scale platforms that tilts based on the mass differential of occupying bodies.

---

## 3. BlueEngine Implementation & Feedback

### What Worked Well:
- **Deterministic Counter Abstraction**: Emulating weights using integer counters (`plates_placed`, `weight_differential`) provides foolproof, zero-jitter state tracking.
- **Robust Interaction Raycasting**: Clear interactable volumes with inspect actions allow players to add ballast plates cleanly.

### Friction Points & Pain Points Encountered:
1. **Absence of Declarative Physics Bodies**:
   - BlueEngine's declarative `GameDocument` has no rigid body physics engine attached. All entities are either static colliders or kinematic movers. True physics puzzles involving sliding, tipping, or weighing loose props require writing a full Rust custom-simulation (`custom-sim` template), as noted in `PAIN_POINTS.md`.
2. **No Expression Math for Balance Tolerance**:
   - Declarative conditions cannot evaluate absolute difference expressions like `abs(weight_a - weight_b) <= 1`. Authors must manually calculate the difference via chained rules.

### Proposed Engine Patches for BlueEngine:
- **Declarative Lightweight Physics (Particles / Actors)**: As requested in `BlueEngineGames/games/magnet-mine/FEEDBACK_PROMPT.md`, introduce a declarative lightweight deterministic particle/body system to `GameDocument` without requiring a full custom-sim binary.
- **Absolute Value / Delta Conditions**: Add `condition: {"counter_diff": ["scale_a", "scale_b"], "within": 1}`.

---

## 4. Key Takeaways for Future AIs
- In **RedEngine**: Take advantage of `props_in(zone)` and `mass(id)` for physics scales, but note that mechanical joints are not yet declaratively authorable.
- In **BlueEngine**: Declarative games must abstract physical weights into discrete counters and interactable states.
