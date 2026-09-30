# Minigame 05 Feedback: Prism Matrix (Optical Beam Redirection)

## 1. Premise & Objective
- **Premise**: Optical beam deflection / mirror matrix alignment.
- **Core Loop**: A high-energy laser beam enters from the north portal. Along the path are three optical prism stations. The player must rotate and align each prism station so the beam connects consecutively into the target photosensitive receptacle at the central mainframe portal, energizing the terminal and completing the circuit.

---

## 2. RedEngine Implementation & Feedback

### What Worked Well:
- **Fast Multivariable Evaluation**: `"if": "prism_1_aligned == 1 && prism_2_aligned == 1 && prism_3_aligned == 1"` allows expressing simultaneous multi-station completion in a single rule line.
- **Visual Emissives**: RedEngine props support `"emissive": "#660066"` directly on materials, making it easy to create glowing lasers, prisms, and energized targets without external lighting hacks.

### Friction Points & Pain Points Encountered:
1. **Dynamic Beam Visuals**:
   - In RedEngine, drawing a continuous visible 3D laser beam currently requires placing static elongated thin cylinders or boxes (`size: [0.05, 0.05, 12]`) and showing/hiding them. There is no native dynamic `line` or `laser_beam` component with dynamic reflection endpoints.
2. **Prop Rotation in Actions**:
   - RedEngine has rule actions `place` (which resets upright to rest) and `impulse` (physics kick), but lacks a lightweight `rotate` action to physically spin an object by 45 or 90 degrees around an axis. Emulating prism rotation required toggling boolean state variables rather than spinning the visual mesh.

### Proposed Engine Patches for RedEngine:
- **`rotate_object` Action**: Add `{"rotate": [object_id, [pitch, yaw, roll]]}` or `{"turn": [object_id, yaw_deg]}` to dynamically reorient decorative and puzzle props in the simulation.
- **Dynamic Laser/Beam Actor**: Add a native object type `{"type": "beam", "from": [x,y,z], "to": [x,y,z], "color": "#00ffcc", "reflect": true}`.

---

## 3. BlueEngine Implementation & Feedback

### What Worked Well:
- **Order-Independent Gathering**: Using a unified counter `aligned_count` allowed the player to align the prisms in any order (1 -> 2 -> 3, or 3 -> 1 -> 2) seamlessly before activating the mainframe terminal.
- **State Compression**: The BlueEngine state vector compactly encoded 4 interactables and the counter in 4 bytes (`v: 15`).

### Friction Points & Pain Points Encountered:
1. **Lack of Dynamic Mesh Rotation / Morphing**:
   - BlueEngine's `GameDocument` schema only allows `set_visible` and `set_mover` (for translation). It has no rotational mover or shape swap action. Representing an altered prism state visually requires having two overlapping nodes (one unaligned, one aligned) and swapping their visibility via `set_visible`.
2. **Static Bounds Limitation**:
   - Colliders and interactables in BlueEngine are strictly axis-aligned bounding boxes (AABB). Rotating an object diagonally does not rotate its interaction box; it remains axis-aligned.

### Proposed Engine Patches for BlueEngine:
- **Rotational Movers**: Extend `movers` schema to support rotational interpolation (`"mode": "rotate"`, `"axis": "y"`, `"degrees": 90.0`).
- **Entity State Swapping**: Introduce an action `{"action": "set_state", "entity": "id", "state": "active"}` tied to multi-state meshes in `map.json`.

---

## 4. Key Takeaways for Future AIs
- In **RedEngine**: Emulate laser beam paths using thin glowing box props (`material.emissive`), and manage prism states via `vars` flags.
- In **BlueEngine**: For multi-element puzzle completion where order doesn't matter, incrementing an accumulator counter (`aligned_count`) is vastly cleaner than maintaining separate flags per entity.
