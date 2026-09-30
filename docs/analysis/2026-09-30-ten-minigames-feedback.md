# RedEngine & BlueEngine Minigame Development & Engine Patching Guide

> **Audience**: AI Agents, Game Engine Maintainers, and Gameplay Engineers.  
> **Purpose**: A comprehensive architectural comparison, evaluation, and concrete patch roadmap based on the simultaneous end-to-end authoring and headless simulation of **10 completely unique minigame premises** in both **RedEngine** and **BlueEngine**.

---

## 1. Executive Summary & Verification Matrix

All 10 minigame premises were implemented from scratch, independently in **RedEngine 2** (`red_engine2`) and **BlueEngine** (`be2-tools`), ensuring that no premises duplicated any existing games in either engine catalog (e.g. avoiding deathmatches, prop hunt, coin sprints, dominoes, desert dunes, or magnet mining).

Every game was subjected to the native authoring toolchain:
- **RedEngine**: Verified via `red_engine2 lint` (static geometry, reachability, drop/leak checks) and `red_engine2 sim` (headless 60Hz tick simulation with scripted player traversal, event emission, variable assertions, and terminal outcome validation).
- **BlueEngine**: Verified via `be2-tools game-validate` (strict schema, reference, and collider checks) and `be2-tools sim` (authoritative tick-level scenario simulation with checksum determinism).

### Complete Verification Matrix

| # | Minigame Premise | RedEngine Status | BlueEngine Status | Core Tested Mechanics |
|---|---|---|---|---|
| **01** | **Laser Vault** | `PASS` (752 ticks, 0 alarms) | `PASS` (Checksum: `0x84608df09bab46af`) | Laser tripwire AABBs, multi-console sequential hacking, alarm counters, vault door deactivation. |
| **02** | **Floor is Lava** | `PASS` (125 ticks, y=1.5m) | `PASS` (Checksum: `0x5aa0df4e1752dfad`) | Staged rising hazard timers, vertical platform step heights (<=0.35m), checkpoint perches, evac beacon. |
| **03** | **Echo Chambers** | `PASS` (568 ticks, step=4) | `PASS` (Checksum: `0xe8a2aa410ff34f45`) | Simon-says memory sequence (N->S->E->W), custom audio/event emission, state machine locking. |
| **04** | **Relic Relay** | `PASS` (360 ticks, delivered=1) | `PASS` (Checksum: `0x552151cc7b168dce`) | Fragile courier dash, periodic decay countdown, mid-point stabilization recharge pedestal. |
| **05** | **Prism Matrix** | `PASS` (337 ticks, energized=1) | `PASS` (Checksum: `0xe5b89bcb234ce136`) | Optical beam redirection, multi-station angular alignment, order-independent accumulator gating. |
| **06** | **Phantom Maze** | `PASS` (399 ticks, exit=1) | `PASS` (Checksum: `0x2cab1dfdcc7bc2af`) | Cloaked labyrinth navigation, sonar ping sensor pads, dynamic obstacle visibility reveal. |
| **07** | **Bomb Defusal** | `PASS` (382 ticks, defused=1) | `PASS` (Checksum: `0xa956c78bb17fafc7`) | Ticking ordnance crisis, decreasing countdown loop, strict 3-station chronological wire cutting. |
| **08** | **Turret Trench** | `PASS` (343 ticks, 0 hits) | `PASS` (Checksum: `0x5aa0df4e1752dfad`) | Cyclic turret sweep / cooldown phases (modulo math), cover-to-cover infiltration, grid override. |
| **09** | **Weight & Balance**| `PASS` (359 ticks, balanced=1) | `PASS` (Checksum: `0x2cab1dfdcc7bc2af`) | Physics scale equilibrium, ballast plate placement, counterweight hydraulic gate release. |
| **10** | **Target Gallery** | `PASS` (333 ticks, hits=3) | `PASS` (Checksum: `0x3abb1cbff54f98f7`) | Pop-up silhouette targets, reaction marksmanship, dual visibility & eligibility toggling. |

---

## 2. Deep-Dive Comparative Architecture

```
+-----------------------------------+-----------------------------------+
|            RedEngine 2            |            BlueEngine             |
+-----------------------------------+-----------------------------------+
| Scene Format: Unified JSON        | Format: GameDocument + Map JSON   |
| Rules: Rich boolean expressions   | Rules: Single counter condition   |
| Math: + - * / % < <= > >= == !=   | Math: increment / set_counter     |
| Physics: Rapier3D rigid bodies    | Physics: Kinematic AABB colliders |
| Simulation: 60Hz scripted nav     | Simulation: Discrete input ticks  |
| Outcomes: Custom strings          | Outcomes: complete only           |
+-----------------------------------+-----------------------------------+
```

---

## 3. RedEngine: Friction Points & Actionable Patch Roadmap

### Key Friction Points Discovered
1. **Perimeter Leak Rejection on Small Rooms**:
   - `red_engine2 lint` aborts with `ERROR [leak]` if any walkable cell touches the outer boundary. Even for abstract minigames, the author must explicitly place enclosing `wall` macros (`from`/`to`).
2. **Decoupling of `hide` and `collision`**:
   - Calling `{"hide": "door"}` only alters rendering. The physical collision mesh remains solid, causing simulation agents to collide with invisible barriers unless paired with `{"collision": ["door", false]}`.
3. **Spawn Points Inside Trigger Volumes**:
   - Spawning a player inside a `{"when": {"enter": {"zone": "id"}}}` generates **no** `enter` event. Players must either spawn at least 0.5m outside or use `{"when": {"start": true}}`.
4. **Lack of Dynamic Rotation in Actions**:
   - While `impulse` kicks props and `place` resets them, there is no declarative action to rotate an object by a fixed angle (e.g. spinning a prism or opening a swinging gate).

### Recommended RedEngine Patches
- [ ] **Patch R-1: Atomic `deactivate` Action**:
  Introduce `{"deactivate": "object_id"}` that atomically calls `hide` and sets `collision: false`.
- [ ] **Patch R-2: Auto-Enclose Flag for Prototypes**:
  Support `meta: {"auto_perimeter": true}` in scene JSON to automatically generate bounding boundary colliders during lint.
- [ ] **Patch R-3: `allow_spawn_enter` for Zones**:
  Allow zones to declare `"allow_spawn_enter": true` so players spawning inside trigger volumes receive an immediate tick-0 `enter` dispatch.
- [ ] **Patch R-4: `rotate_object` Action**:
  Add `{"rotate": [object_id, [pitch, yaw, roll]]}` for interactive puzzle machinery.

---

## 4. BlueEngine: Friction Points & Actionable Patch Roadmap

### Key Friction Points Discovered
1. **The 4-Way Synchronization Constraint**:
   - An interactable entity declared in `game.json` will fail `game-validate` unless **four** separate records match in `map.json`:
     1. `scene.materials[mat_id]`
     2. `scene.nodes[node_id]` (shape box, pos, rot, scale)
     3. `colliders[node_id]` (`min` and `max` vector3 float arrays)
     4. `entities[node_id]` (`id`, `label`, `bounds`, `action`)
   - Furthermore, bounds must be arrays of floats (`[-3.3, 1.2, 0.7]`), not strings.
2. **Timer Schema Naming**:
   - BlueEngine timers require `duration_ticks`, `auto_start`, and `repeats`. Naming this `interval_ticks` causes an immediate rejection.
3. **Condition Expressiveness Gap**:
   - `condition` only accepts `{"counter": "name", "equals": int}` or `null`. Inequalities (`<`, `>`), modulo (`%`), and multi-variable boolean conjunctions are unsupported in declarative rules.
4. **No Authoritative `fail` Outcome**:
   - `GameDocument` has `complete` to win, but no `fail` action to end matches on failure conditions (like bomb detonation or timer expiration).
5. **Action Limit per Rule**:
   - Rules are strictly limited to 4 actions (`"limits": {"actions_per_rule": 4}`). Complex resets require daisy-chaining intermediate counter triggers.

### Recommended BlueEngine Patches
- [ ] **Patch B-1: `be2-tools add-interactable` CLI Tool**:
  Provide a CLI utility that appends all four matching structures (`material`, `node`, `collider`, `entity`) to `map.json` in one command.
- [ ] **Patch B-2: Rich Conditions**:
  Expand `condition` schema to accept operators (`less_than`, `greater_than`, `modulo`) and compound blocks (`all: [...]`, `any: [...]`).
- [ ] **Patch B-3: Authoritative `fail` Action**:
  Add `{"action": "fail", "reason": "string"}` alongside `{"action": "complete"}`.
- [ ] **Patch B-4: Serde Aliases for Timers**:
  Support `interval_ticks` and `period_ticks` as serde aliases for `duration_ticks`.
- [ ] **Patch B-5: Expand `actions_per_rule`**:
  Increase `actions_per_rule` from 4 to 8 or 16.

---

## 5. Directory Index of Created Minigames

All games are stored locally under `scratch/minigames/`:

```
scratch/minigames/
├── red_engine/
│   ├── 01_laser_vault/laser_vault.json
│   ├── 02_floor_is_lava/floor_is_lava.json
│   ├── 03_echo_chambers/echo_chambers.json
│   ├── 04_relic_relay/relic_relay.json
│   ├── 05_prism_matrix/prism_matrix.json
│   ├── 06_phantom_maze/phantom_maze.json
│   ├── 07_bomb_defusal/bomb_defusal.json
│   ├── 08_turret_trench/turret_trench.json
│   ├── 09_weight_balance/weight_balance.json
│   └── 10_target_gallery/target_gallery.json
├── blue_engine/
│   ├── 01_laser_vault/ {game.json, map.json, scenario.json}
│   ├── 02_floor_is_lava/ {game.json, map.json, scenario.json}
│   ├── 03_echo_chambers/ {game.json, map.json, scenario.json}
│   ├── 04_relic_relay/ {game.json, map.json, scenario.json}
│   ├── 05_prism_matrix/ {game.json, map.json, scenario.json}
│   ├── 06_phantom_maze/ {game.json, map.json, scenario.json}
│   ├── 07_bomb_defusal/ {game.json, map.json, scenario.json}
│   ├── 08_turret_trench/ {game.json, map.json, scenario.json}
│   ├── 09_weight_balance/ {game.json, map.json, scenario.json}
│   └── 10_target_gallery/ {game.json, map.json, scenario.json}
└── feedback/
    ├── 01_laser_vault.md
    ├── 02_floor_is_lava.md
    ├── 03_echo_chambers.md
    ├── 04_relic_relay.md
    ├── 05_prism_matrix.md
    ├── 06_phantom_maze.md
    ├── 07_bomb_defusal.md
    ├── 08_turret_trench.md
    ├── 09_weight_balance.md
    ├── 10_target_gallery.md
    └── ENGINE_PATCHING_GUIDE.md
```
