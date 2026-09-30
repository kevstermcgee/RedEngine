# Minigame 08 Feedback: Turret Trench (Cover-to-Cover Infiltration)

## 1. Premise & Objective
- **Premise**: Timing-based stealth infiltration through an active defense perimeter.
- **Core Loop**: Synchronized defensive turrets alternate cyclically between active firing arcs (danger phase) and reload/cooldown sweeps (safe window). The player must sprint between reinforced blast bunkers during reload windows, avoiding the open firing trench while the turrets are active, and reach the master override terminal to disable the grid.

---

## 2. RedEngine Implementation & Feedback

### What Worked Well:
- **Modulo Expressions**: RedEngine's expression parser seamlessly handles arithmetic modulo operations: `"if": "turret_cycle % 2 == 1"`. This simplifies implementing cyclic alternating states (e.g. fire/reload cycles) without writing two separate toggling rules.
- **Raycast and Line of Sight**: Combining spatial zones with cover walls naturalizes line-of-sight gameplay.

### Friction Points & Pain Points Encountered:
1. **Dynamic Turret Swivel / Aim**:
   - In RedEngine, placing a turret prop (`turret_mesh`) works well, but animating the turret barrel to actively track the player or sweep back and forth requires keyframed animation blocks (`keyframes`) in the JSON scene, which cannot currently be paused or dynamically gated by rule variables in real-time.
2. **Hitscan Projectiles in Rules**:
   - RedEngine supports weapon hitscan and projectile simulation in bot matches, but rules cannot directly trigger a synthetic bullet tracer (`shoot_ray: [from, to]`) as an action.

### Proposed Engine Patches for RedEngine:
- **`fire_projectile` / `cast_ray` Action**: Provide an authoritative action `{"shoot_ray": {"origin": [x,y,z], "target": [x,y,z], "damage": 25}}` for environmental hazard emitters like turrets and cannons.
- **Variable-Driven Animation Playback**: Allow rules to trigger or pause named keyframe tracks: `{"play_anim": "turret_sweep", "loop": true}`.

---

## 3. BlueEngine Implementation & Feedback

### What Worked Well:
- **Deterministic Cycle Counter**: `sweep_timer` triggering `increment cycle_phase 1` reliably synchronizes phase progression across all clients.
- **Discrete Waypoint Progression**: Enforcing linear infiltration via `bunker_progress` prevents players from skipping checkpoints.

### Friction Points & Pain Points Encountered:
1. **No Modulo / Math Operations in Conditions**:
   - BlueEngine conditions only test exact equality against a fixed integer: `{"counter": "name", "equals": int}`. There is no modulo operator (`%`), range test (`>`), or bitwise mask. Simulating a repeating 2-state cycle (0 -> 1 -> 0 -> 1) requires either resetting the counter back to 0 on state 1, or adding separate rules for each tick.
2. **No Dynamic Hazard Zone Gating**:
   - In BlueEngine, `trigger_zones` are always active if declared. You cannot conditionally disable a trigger zone's eligibility through a rule action unless you wrap the logic in a counter condition.

### Proposed Engine Patches for BlueEngine:
- **Counter Modulo Condition**: Add `"modulo": 2, "equals": 0` to the condition schema.
- **Zone Activation Action**: Add `{"action": "set_zone_enabled", "zone": "id", "enabled": bool}`.

---

## 4. Key Takeaways for Future AIs
- In **RedEngine**: Use modulo arithmetic (`var % 2 == 1`) in conditional rules for clean cyclical timing systems.
- In **BlueEngine**: Modulo is unsupported in declarative conditions; reset the phase counter to 0 upon reaching max cycle (`set_counter: 0`) to maintain an alternating loop.
