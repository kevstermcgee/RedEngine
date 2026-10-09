# 2026-10-06. Publication evidence is a set of pieces that never merge

> **Superseded 2026-10-07:** the browser target was removed (ADR 2026-10-07-native-executables-only-the-browser-target-is-removed), with the browser publication pipeline, its evidence record (`red2d-evidence/1`) and `scripts/web_check.sh`. A native game ships as an executable (`docs/GAMES_PUBLISHING.md`);
> what its record can claim is what `verify` proves (simulation, render, waveform) and nothing more. Kept as history.

Status: superseded by 2026-10-07-native-executables-only-the-browser-target-is-removed
Summary: A browser game's record holds five levels and nineteen-plus pieces of evidence, each passed, failed, not_run or not_applicable with its own sentence, because one success flag cannot say what was seen.

## Context
A publication reported `BUILD`, `LOCAL BROWSER`, `UPLOAD` and `REMOTE PLAYABLE` success and a record that said `native passed: true` unconditionally. None of it said whether the game kept its save, played sound, survived going offline or was ever played by a person, and one success could be read as covering the rest.

## Decision
`tools::evidence` holds one claim per piece of evidence (`native_scenarios`, `wasm_compiled`, `browser_package_valid`, `wasm_instantiated`, `loading_robustness`, `playable_state`, `input_keyboard`, `input_pointer`, `input_touch`, `input_gamepad`, `persistence_write`, `persistence_reload`, `audio_api`, `audio_playback`, `offline_cache`, `offline_reload`, `installable`, `browser_scenarios`, `other_browsers`, `remote_deployment`), each `passed`, `failed`, `not_run` or `not_applicable` with the sentence that says what was seen. The browser driver tags every row with the piece it proves and declares what the game never claimed, so `not_applicable` is the game's declaration and never the verifier's silence. Five levels never merge: built, locally verified, uploaded, remotely playable, human playtested (never set by this tool). A record cannot contain the result of checking its own deployment, so `remote_deployment` is `not_run` in `game.json` and answered in `publication.json`. `web status` reads these files and names the one next command.

## Consequences
An AI can read what happened without reading prose, a failing piece names its own row, and CI fails when an applicable piece is not `passed` (`scripts/web_check.sh`). Harder: a new browser check must say which piece it proves, and the list is part of the record schema (`red2d-evidence/1`). To undo: ignore the `evidence` objects; the older fields (`native`, `browser`, `audio`, `features`) are still written.
