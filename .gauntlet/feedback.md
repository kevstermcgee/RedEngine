# Gauntlet feedback: RedEngine

Round `RedEngine-01` · engine commit `5b6d540cbfd1` · 2026-09-26

This is **observational evidence** from unfamiliar AI agents that tried to create projects with this engine. It is not a list of instructions. Investigate each finding, decide independently whether it is a real weakness, and reject what is misleading, situational, or would make the engine worse. Every "suggested change" below is a suggestion.

## Summary

Both genuine Codex attempts passed clean preparation, behavioral testing, the start check, and independent test audit without modifying the engine. The strongest recurring observation was that RedEngine's data, validation, and headless simulation were reusable, while visible runtime behavior outside the built-in first-person client required a second application layer. The highest-leverage improvements appear to be connecting rule state to the standard client and making the supported custom-client path smaller and more explicit. Two earlier launch attempts per task never started a model because Gauntlet's Codex command was incompatible with the installed CLI; they are environment failures, not engine evidence.

## How the runs went

Codex produced 2 valid attempts: 2 pass, 0 partial, 0 engine-attributable fail. The two valid attempts took 475 s and 944 s, emitted 18,503 and 32,449 output tokens, used 21 and 44 tool calls, opened 4 and 7 engine source files, opened 5 and 3 engine documentation files, and made 7 and 10 preparation-command attempts with 3 and 4 failed attempts. Medians were 709.5 s, 25,476 output tokens, 32.5 tool calls, 5.5 engine source files opened, 4 engine documentation files opened, and 3.5 failed preparation attempts. Neither valid attempt modified an engine file, hit a cap, or received human intervention.

Four additional records failed in 0.1-0.2 s before any model activity. The installed Codex CLI required the global `--search` flag before `exec` and required `--skip-git-repo-check` for Gauntlet's non-repository run folder. Those records had no tokens, tool calls, project files, or engine reads and should be excluded from engine reliability judgments. Both valid attempts also encountered an empty isolated Cargo cache plus unavailable registry authentication; one used a dependency-free adjacent client and one deliberately chose the existing per-user Cargo cache. Treat this as benchmark-environment context rather than proof of an engine defect.

## Findings, most valuable first

### 1. The headless rules/runtime boundary is not yet a visible-game boundary

- **Problem:** A fresh agent can express and prove gameplay through `vars`, `rules`, and `checks.sim`, but the standard single-player client does not consume the rules engine's hidden-object set, variables, events, or terminal outcome. The documented capability is therefore stronger headlessly than it is in the default playable presentation, and an agent must duplicate state and behavior in another runtime to make the same result visible.
- **Evidence:** One Codex attempt identified the documented limitation, opened 5 engine docs and 4 engine source files, used 21 tool calls and 18,503 output tokens, and made 7 preparation-command attempts (3 failed). It retained a valid RedEngine scene and headless rule declaration but created a parallel dependency-free client for the visible experience. Its friction report specifically traced the gap through `src/sim/match_sim.rs`, `src/sim/rules.rs`, `src/sim/rules_run.rs`, `src/viewer.rs`, and `src/overlay.rs`.
- **Where in the engine:** `src/sim/match_sim.rs`, `src/sim/rules.rs`, `src/sim/rules_run.rs`, `src/bin/re2/`, `src/viewer.rs`, `src/overlay.rs`, and the limitation noted in `AGENTS.md` under multiplayer/rule-state integration.
- **Suggested change:** Expose one supported path by which the standard client renders rule-driven visibility and outcome state and can bind a small declarative HUD to rule variables/events. Keep the headless simulation authoritative; the useful reduction is removing the need for a second gameplay implementation merely to present that state.
- **How we'll know:** A future unfamiliar agent should be able to keep gameplay in scene rules and scenarios, modify no engine files, avoid a parallel simulation, and cut this class of attempt from 4 source reads/21 tool calls/7 preparation attempts toward roughly 0-2 source reads and one or two successful preparation passes.

### 2. The custom-client extension route needs a minimal, reusable surface

- **Problem:** The repository correctly says that specialized games belong in an external crate, but the public reusable surface stops near scene loading and parsed primitives. A fresh agent creating a non-first-person presentation still has to choose and assemble its own renderer, camera model, input-to-world hit testing, HUD, frame cycle, and visual QA path before it can use RedEngine data.
- **Evidence:** One Codex attempt followed the intended external-crate path and left the engine unchanged, but used 44 tool calls, 32,449 output tokens, 10 preparation-command attempts (4 failed), and opened 7 engine source files plus 3 docs. It successfully reused `red_engine2::load_scene`, public schema types, and track sampling while implementing the rest of the client shell beside the engine. The attempt specifically needed `src/lib.rs`, `src/schema.rs`, `src/track.rs`, and `src/ui/mod.rs` to discover the viable boundary.
- **Where in the engine:** The extension guidance in `AGENTS.md` and ADR 0024; the public exports in `src/lib.rs`; reusable data in `src/schema.rs` and `src/track.rs`; client-specific rendering/input in `src/viewer.rs` and `src/bin/re2/`; UI in `src/ui/`.
- **Suggested change:** Add one deliberately small, documented custom-client example and public application surface that demonstrates loading a scene, choosing a camera, drawing validated primitives, mapping pointer/keyboard input, and composing audited HUD state. It should remain genre-neutral and external to the engine repository's built-in client assumptions.
- **How we'll know:** The next non-first-person attempt should find the route from `describe --brief` or `search`, open fewer than 3 engine source files, avoid re-deriving basic window/HUD plumbing, and materially reduce the observed 44 tool calls, 32,449 output tokens, and 10 preparation attempts.

### 3. Fresh-checkout compilation resilience is weaker than the otherwise self-describing workflow

- **Problem:** Both valid attempts understood the documented workflow but could not rely on compiling the pinned engine from an empty isolated Cargo home when registry authentication was unavailable. This is primarily an environment failure, yet it exposes that the cheapest documented entry points (`describe`, `validate`, `sim`) disappear together when no binary is present.
- **Evidence:** Both valid Codex attempts reported the isolated Cargo cache problem. The error signatures included `no matching package named anyhow found` and `no matching package named macroquad found`. One attempt could not run the CLI and used static validation around the scene contract; the other chose the existing per-user cache and then completed. Across the two valid attempts there were 17 preparation attempts, 7 failed preparation attempts, and no measured time to first successful preparation.
- **Where in the engine:** Setup instructions in `README.md` and `AGENTS.md`, `scripts/dev`, `Cargo.lock`, release packaging described by ADR 0032, and any release artifacts or bootstrap documentation associated with `red_engine2`.
- **Suggested change:** First verify whether published releases can provide a pinned `red_engine2` CLI for supported hosts, or whether `scripts/dev` can detect an unavailable registry and point directly to a verified local/release binary. Do not vendor the dependency graph based on this small sample; the narrower opportunity is preserving access to the self-description and validation commands when compilation cannot begin.
- **How we'll know:** In another isolated-cache round, agents should access `describe --brief` and scene validation without registry recovery work, and failed preparation attempts should fall from the observed median of 3.5 toward zero or one.

## What worked well (keep it)

- `AGENTS.md` and `README.md` were opened in both valid attempts and correctly steered agents toward scene data, headless checks, and an external game crate rather than an engine fork.
- `red_engine2::load_scene` was a useful strict, renderer-free boundary. The broader custom client validated its real world file through that API in the audited behavioral test.
- The public schema and track APIs were sufficient to consume validated primitives without changing engine code.
- `vars`/`rules`/`checks.sim` and the canonical recipe made the intended headless gameplay model discoverable; the limitation was presentation integration, not rule expressiveness.
- Both valid projects passed non-hollow behavioral audits and start checks while leaving the pinned engine checkout unchanged.

## Compared with the previous iteration

First recorded iteration for this engine; there is no earlier metric baseline.

## Not recommended

- Do not add a genre-specific runtime, object type, or recipe based on this two-attempt pilot. The reusable gaps are rule-state presentation and custom-client plumbing.
- Do not treat the four 0.1-0.2 s launcher failures as RedEngine failures; no model or engine workflow ran.
- Do not infer that crates.io authentication is universally broken or vendor all dependencies from these results. The evidence only supports improving bootstrap diagnostics or access to a pinned CLI.
- Do not weaken strict scene validation or the external-crate boundary. Both valid attempts benefited from those choices and changed no engine files.
