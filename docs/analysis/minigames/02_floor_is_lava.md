# Minigame 02 Feedback: Floor is Lava (Vertical Ascent)

## 1. Premise & Objective
- **Premise**: Ascending vertical platforming under a rising environmental hazard.
- **Core Loop**: Toxic lava / radiation activates across the bottom floor after a fixed countdown. The player must parkour across stepping crates and rising platform tiers, reaching checkpoint perches before being burned, and activating the rooftop evacuation sanctuary beacon.

---

## 2. RedEngine Implementation & Feedback

### What Worked Well:
- **3D Physics & Ledges**: RedEngine automatically handles stepped ledges <= 0.35m as walkable stairs, allowing modular verticality using basic box props.
- **Temporal Event Rules**: The `"when": { "after": 2.0 }` rule provides native time-delayed state activation (`lava_active = 1`) without requiring external timer entities.
- **Integrated Teleportation**: `"teleport": "spawn_start"` smoothly resets the player upon hazard contact.

### Friction Points & Pain Points Encountered:
1. **Drop Warnings on Elevated Platforms**:
   - `red_engine2 lint` flags all ledges higher than 0.5m with `WARN [drop] unprotected X m drop near ... walkable floor ends with nothing to stop the player`. For intentional vertical platformers, this generates noise unless explicit lint ignores (`checks.lint.ignore: ["drop"]`) are specified.
2. **Zone vs Box Elevation Nuance**:
   - Standard 2D `rect` zones project infinitely or throughout default vertical extents unless bound by `y` and `height`, or defined via `{ "box": [x0, y0, z0, x1, y1, z1] }`. Authors must be cautious to ensure that a floor hazard zone does not overlap players standing on elevated platforms overhead.

### Proposed Engine Patches for RedEngine:
- **Platformer preset lint rule**: Add a metadata tag `"genre": "platformer"` that automatically suppresses `[drop]` warnings.
- **Visual Hazard Volumes**: Provide a native zone material shader (e.g. `type: "hazard_plane"`, `emission: "#ff3300"`) so hazard zones render glowing volumetric effects in the live client.

---

## 3. BlueEngine Implementation & Feedback

### What Worked Well:
- **Native Timer System**: `timers: [{"id": "lava_timer", "duration_ticks": 30, "auto_start": true, "repeats": true}]` provides fixed-tick deterministic periodic pulses.
- **Deterministic Action Queues**: Timers and interactable events execute in strict document order.

### Friction Points & Pain Points Encountered:
1. **Timer Schema Documentation Pitfall**:
   - The field for repeating timers in BlueEngine `game.json` is `duration_ticks`, `auto_start`, and `repeats`—NOT `interval_ticks` or `period`. Misnaming this produces `{"error":"unknown field interval_ticks, expected one of id, duration_ticks, auto_start, repeats"}`.
2. **Static Geometry Rigidity**:
   - BlueEngine's declarative GameDocument does not allow real-time continuous vertical translation of colliders unless scripted via `movers`. Simulating a smooth rising water/lava surface requires discrete trigger zone staging or Rust custom-sim.
3. **Timer Event Handling Limits**:
   - A timer trigger (`on_timer`) can only fire actions, but cannot directly evaluate spatial positions unless coupled with `trigger_zones`.

### Proposed Engine Patches for BlueEngine:
- **Alias `interval_ticks` -> `duration_ticks`**: Add serde alias for `duration_ticks` / `interval_ticks` to prevent common authoring errors.
- **Mover Collision Push**: Ensure `movers` can push player entities upwards rather than clipping through them during vertical elevator/hazard translation.

---

## 4. Key Takeaways for Future AIs
- In **RedEngine**: Use step sizes <= 0.35m for climbable platforms without jump inputs, and set `checks.lint.ignore: ["drop"]` on platforming scenes.
- In **BlueEngine**: Timers require `duration_ticks`, `auto_start: true`, and `repeats: true`.
