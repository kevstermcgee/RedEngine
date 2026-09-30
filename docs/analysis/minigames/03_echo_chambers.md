# Minigame 03 Feedback: Echo Chambers (Sequence Memory)

## 1. Premise & Objective
- **Premise**: Harmonic memory sequence / Simon-says pattern puzzle.
- **Core Loop**: Four elemental monoliths (North, South, East, West) surround a sealed central altar. The player must strike/activate the monoliths in a designated harmonic pattern (N -> S -> E -> W). Successfully executing the full sequence unseals the sanctum altar, while an out-of-order strike triggers dissonance.

---

## 2. RedEngine Implementation & Feedback

### What Worked Well:
- **Spatial Pad Zones**: Placing trigger zones around each physical monolith (`pad_n`, `pad_s`, `pad_e`, `pad_w`) allowed smooth step-on activation without requiring raycast button presses.
- **Custom Game Events**: Emitting specific audio/semantic events (`note_n`, `note_s`, `note_e`, `harmony_unsealed`) makes verifying puzzle state transparent in both the HUD and `checks.sim`.
- **Sequential State Machine**: Managing `seq_step` via integer increment rules in data without writing any external script or Rust logic.

### Friction Points & Pain Points Encountered:
1. **Rule Order & Overlapping Edge Traversal**:
   - As noted in RedEngine specs, when stepping between zones, entering a new zone triggers before exiting the previous zone. In memory puzzles where a player might step across multiple pads, multiple rules can evaluate in the same tick if padding overlaps.
2. **Audio Primitives in Data**:
   - RedEngine can emit game events (`emit: "note_n"`), but playing a distinct musical tone or chord currently requires external audio triggers or sound files linked to prefabs, rather than a built-in synth beep/tone emitter.

### Proposed Engine Patches for RedEngine:
- **Built-in Audio Synth Emitter**: Allow an action like `{"play_tone": {"frequency_hz": 440, "duration_ms": 300}}` for rapid prototyping of audio puzzles and cues without bundling wav assets.
- **Ordered Sequence Rule Macro**: A high-level rule helper `{"when": {"sequence": ["pad_n", "pad_s", "pad_e", "pad_w"]}}` that automatically tracks step indices and resets.

---

## 3. BlueEngine Implementation & Feedback

### What Worked Well:
- **Direct Interaction Gating**: Using `condition: {"counter": "seq_step", "equals": X}` on each `on_interact` rule enforces sequence locking at the engine level with zero race conditions.
- **Interactable State Toggling**: Disabling used monoliths (`set_enabled: false`) prevents accidental duplicate presses.

### Friction Points & Pain Points Encountered:
1. **Lack of Dynamic Reset in Declarative Schema**:
   - If a player interacts with the wrong monolith out of sequence, ideally the puzzle should reset `seq_step` to 0 and re-enable previous monoliths. In BlueEngine's GameDocument schema, implementing "if wrong input, reset all" requires creating fallback rules for every wrong combination, quickly ballooning the rule count towards the 64-rule limit.
2. **Action Limit per Rule (Max 4)**:
   - BlueEngine caps actions per rule at 4 (`"limits": {"actions_per_rule": 4}`). When unsealing the altar and re-enabling multiple interactables, having only 4 actions forces splitting logic across daisy-chained intermediate counters.

### Proposed Engine Patches for BlueEngine:
- **Expand `actions_per_rule`**: Increase the limit from 4 to 8 or 16 actions per rule to accommodate multi-object puzzle state transitions.
- **Default / Fallback Rule Trigger**: Allow a catch-all rule `{"on_interact": "monolith_*", "fallback": true}` that executes if no specific conditional rule matched.

---

## 4. Key Takeaways for Future AIs
- In **RedEngine**: Separate puzzle pads with at least 0.5m clearance to avoid multi-zone evaluation during movement interpolation.
- In **BlueEngine**: Mind the 4-action-per-rule limit when authoring complex puzzle resets, chaining counters if more actions are required.
