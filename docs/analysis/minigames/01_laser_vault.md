# Minigame 01 Feedback: Laser Vault (Cyber Heist)

## 1. Premise & Objective
- **Premise**: Cyber-heist security vault infiltration.
- **Core Loop**: Player infiltrates a guarded chamber, avoids crossing laser tripwire hazard planes, hacks security console A, hacks security console B in strict sequence, unsealing the vault door to retrieve the vault safe asset before alarm lockdowns trigger.

---

## 2. RedEngine Implementation & Feedback

### What Worked Well:
- **Spatial Zones**: `zones` with `{ "enter": { "zone": "laser_1" } }` allow precise rectangular tripwires with sub-meter accuracy.
- **Rich Expression Language**: Rules natively support compound boolean conditions (`"if": "vault_open == 1 && alarms < 3"`, `"if": "hacked_a == 1"`), making multi-step heist sequencing very concise.
- **Headless Sim Testing**: `checks.sim` with multi-waypoint walking scripts (`walk: "-5,-8; -5,-2; -5,-7; 5,-7; 5,-2; 5,2.5; 0,2.5; 0,5"`) validated both the stealth path and the clumsy alarm-trip failure scenario.

### Friction Points & Pain Points Encountered:
1. **Perimeter Leak Lint Rejection**:
   - `red_engine2 lint` failed with `ERROR [leak] the player can walk off the map: 514 border cells reachable`. Even for small test chambers, RedEngine strictly requires closed perimeter geometry (e.g. `type: "wall"` with `from`/`to`). Future AIs need to remember that floors cannot simply exist as open planes without surrounding boundary colliders.
2. **`hide` vs `collision` Decoupling**:
   - Calling `{"hide": "vault_door"}` hid the asset graphically, but left the physics collider intact (`thief got stuck on leg 3 stopped at (0.04, 2.95)`). Disabling the door required explicitly pairing `{"collision": ["vault_door", false]}` alongside `{"hide": "vault_door"}`.
3. **Sim Straight-Line Traversal**:
   - Simulated walking between two waypoints performs direct linear interpolation. If laser tripwires lie between the two waypoints, the agent gets flagged unless intermediate routing waypoints are specified around the tripwire perimeter.

### Proposed Engine Patches for RedEngine:
- **`hide_and_disable` action macro**: Add an optional flag `{"hide": "id", "disable_collision": true}` or atomic action `{"deactivate": "id"}` so doors/forcefields can be cleared in one operation.
- **Perimeter auto-bounding flag**: Allow scene meta `unbounded: true` or `auto_fence: true` for rapid prototype arenas to bypass the strict perimeter boundary leak requirement during early iteration.

---

## 3. BlueEngine Implementation & Feedback

### What Worked Well:
- **Clean Event Model**: Direct interaction via `on_interact` with automatic raycast proximity (up to 2.5m) makes button and console activation intuitive.
- **Strict Deterministic State**: State checksums (`final_checksum: 0x84608df09bab46af`) ensure zero drift across headless runs.
- **Atomic Counters & Actions**: `increment`, `set_enabled`, and `complete` form a reliable core set of primitives for discrete quest logic.

### Friction Points & Pain Points Encountered:
1. **Four-Way Synchronization Requirement**:
   - Adding an interactable entity (`console_a`) caused validation errors:
     - `invalid type: string "-3.3 1.2 0.7", expected an array of length 3` (schema requires vector3 floats).
     - `{"error":"Interactables require matching static box node IDs"}`: BlueEngine strictly requires every interactable in `game.json` to have 4 synchronized structures in `map.json`:
       1. `scene.materials[mat_id]`
       2. `scene.nodes[node_id]` (with shape `"box"`, pos, rot, scale)
       3. `colliders[node_id]` (with `min` and `max` vector3s)
       4. `entities[node_id]` (with `id`, `label`, `bounds`, `action`)
     - If any one of these is missing or bounds mismatch, `game-validate` fails.
2. **Condition Expressiveness Gap**:
   - `condition` only accepts `{"counter": "name", "equals": int}` or `null`. Compound logic (`counter_a == 1 AND counter_b == 0`, or inequalities like `alarms < 3`) cannot be expressed in a single condition. Instead, the author must chain intermediate state counters.
3. **Trigger Zones Lack Visual Nodes**:
   - Trigger zones exist purely as bounding boxes in `game.json`, with no automatic visual feedback mesh attached unless authored manually in `map.json`.

### Proposed Engine Patches for BlueEngine:
- **`be2-tools add-interactable` CLI command**: A CLI utility that automatically appends the 4 synchronized structures (`material`, `node`, `collider`, `entity`) to `map.json` in one command.
- **Range / Compound Conditions**: Expand `condition` schema to accept comparison operators (`equals`, `less_than`, `greater_than`) and boolean combinations (`all: [...]`, `any: [...]`).

---

## 4. Key Takeaways for Future AIs
- In **RedEngine**: Always wrap maps with closed perimeter `wall`s, and pair `hide` with `collision: [id, false]` when opening pathways.
- In **BlueEngine**: Always synchronize `entities`, `scene.nodes`, `colliders`, and `materials` when adding interactables, and serialize bounds as float arrays `[x, y, z]`.
