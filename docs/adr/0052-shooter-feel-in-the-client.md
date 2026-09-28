# 0052. Shooter feel in the client: sounds, screen effects, a combat HUD
Status: accepted

## Context
ADR 0051 put the facts of a fight on the wire. The graphical client did nothing with them: another player's gun made no sound, a hit gave no sign, being shot showed nothing but a
smaller number, and the offline shot sound was the revolver's for every weapon. A shooter is fun when each action answers at once, in sound and in the picture, and when the screen
always says how you are doing. None of that needs a new engine concept; it needs a place to put the decisions where they can be tested without a window or a speaker.

## Decision
Three layers, from pure to physical:

- **`feel`** (`src/feel.rs`, headless): a state machine that turns what happened (`net::happenings::Happenings`), our own state (`Own`: hit points, dead, protected, weapon) and the
  match (phase, countdown) into **cues to hear** (`Cue`: a shot placed at its shooter, a hit tick, a kill, a level-up a beat after the kill, hurt, death, respawn, heartbeat below 35 hp,
  countdown beeps, go, win and lose stings, footsteps by distance walked, jump, landing by fall speed, a jump pad's launch) and **effects to see** every frame (`FxParams`: a red vignette
  that swells with damage and pulses with low health, a damage arc pointing at the attacker relative to where we look, a hit or kill marker, flashes for a level-up or a respawn, a cool
  tint under spawn protection). It is time-driven and deterministic; its timing is unit-tested. Nothing in it affects the simulation.
- **`sfx`** and **`fx`** (graphical build): `sfx` is the sound bank, synthesized in code like every other asset (ADR 0008): one voice per firearm (a crack, a sweeping boom, a body and a
  tail, in proportions that keep ten shots a second from smearing), the feedback cues, movement sounds and match stings, each a finite, bounded, fully decayed clip checked by tests, and
  `spatial` / `pan_gains` place a sound by distance, side and whether it is behind us. `fx` draws `FxParams` in one fullscreen pass (`shaders/fx.wgsl`, analytic, no textures), so an
  effect animates at the display rate instead of waiting for the CPU-painted overlay to be re-uploaded.
- **`avatar`** (headless): how the *other* players look. `avatar::animate` poses a pooled avatar from a remote player's interpolated pose: the walk cycle, the gun arm raised level and
  following their aim with a kick on each shot (the shot counter moving), the bat's swing when the swing flag rises, and a body that falls over when they die and stands when they
  respawn; it returns where the weapon in their hand is (`RemoteHand`), and `LiveRenderer::set_remote_hands` draws it, with its muzzle flash, like the third-person copy of our own.
  Another player's shot is heard when their avatar fires (the interpolation delay after the snapshot), so the sound lands with the flash.
- **The `re2` client** wires them: it feeds `Feel` from the snapshots (`NetSession::take_happened`), plays cues through `Audio::play_at`, and shows a combat HUD (`ui::online::CombatView`
  inside `OnlineView`: a health bar, the weapon in hand, the weapon ladder as one pip per rung with the current one lifted, who leads, a level-up line, and the "eliminated / respawning in n"
  banner), audited at every window size by `ui-check` like every other screen.

Online, the server owns the weapons, but the click has to feel instant: the client predicts, from the same input and the same rules (a button acts when it goes down, automatic
weapons repeat while held, a firearm waits out its cooldown), only the *cosmetics* of its own attack (recoil, muzzle flash, the bat's swing, the sound). What was actually hit is the
server's word and comes back as the hit marker. A dead player's client sends a still input and lies where it fell, so it is not pulled back by every snapshot; on respawn it faces the way
the server placed it, and a weapon the server changes (a rung up the ladder) plays the lower-and-raise animation.

## Consequences
- The effect of a change to how a fight feels is a change to one small headless file with tests, not to the window code.
- Debug switches make each effect checkable without playing: `RE2_FEEL=hit|kill|hurt|dead|low|protected|flash` holds an effect on screen, `RE2_LOG_CUES=1` prints every cue, and
  `RE2_AUTOWALK` / `RE2_AUTOFIRE` drive a client without hands, so a real window can be screenshotted mid-fight.
- The clips cannot be listened to in CI, so tests check what can be checked: finite, bounded, audible, ends in silence, longer where a gun should ring out, positional pan and falloff.
  Balance by ear is a human's job.
- Limits: sounds are mono clips panned and attenuated, with no occlusion or reverb; the vignette, arc and marker are drawn over the whole frame, not per pixel of the world.
