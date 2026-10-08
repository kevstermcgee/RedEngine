# Stabilization brief for the review (filed 2026-10-07, not implemented)

Filed by the user for the review later tonight. It is the review's own brief, copied unchanged below the note. **Nothing in it has been implemented**: the Killchain session that
filed it was told not to. The baseline it names, `f58e15db0e7383519f472ef0ba5f84871e18bf77`, is the commit that session pushed (Killchain game modes, protocol v15).

## Note from the Killchain session (what it can and cannot vouch for)

Several things that session shipped or claimed overlap this brief, so the review should treat them as **unverified** until the brief's acceptance tests exist:

- **Short-code JOIN against a keyed HOST (brief 1).** The session made the project relay the default and wrote, in the game's README and in its summary, that HOST gives a six-character code
  that works with no setup. It only ever joined a server that had no game join key (direct codes, `red_server` without `--key`). It never ran the graphical HOST defaults
  (random join key, relay code preferred) against a short-code JOIN, which is exactly the case the brief says ends in `NeedsKey`. Treat the README's "no setup" claim as wrong until fixed.
- **Automatic map retry (brief 1).** Tested directly (client with map A, QUIC server on map B, one join logged) but never through the relay, where the claim is one-time and each attempt binds a new socket.
- **Relay reachability.** The relay answers on loopback on the box but not on its public name from the same network, and UPnP is off, so "a friend in another state can join" was never demonstrated.
- **Search-and-destroy reconnect and duel resume (brief 1).** Neither was tested; the session's tests covered a fresh join (a third person refused with `Full`), not a resume.
- **Replay of objective modes (brief 3).** Not attempted. `ArenaState::state_hash` was extended for flags and rounds but, as the brief says, it is not wired into checkpoints.
- **What the session did verify:** `scripts/dev affected --full` green at `f58e15d` (2962 passed, 0 failed), GitHub CI green including Windows, and live-client screenshots of every mode.

The game's own `docs/ENGINE_FEEDBACK.md` (round 2) lists friction from the modes build; this brief is separate and takes priority for the review.

---

You are working on RedEngine:
https://github.com/kevstermcgee/RedEngine

Implement a focused stabilization update that makes the existing multiplayer, AI authoring, replay, and native 2D delivery workflows reliable end to end. Make the changes and complete verification; do not stop at recommendations.

Goal and scope

The engine has useful capabilities, but several supporting systems disagree about authentication, reconnect behavior, source identity, executable selection, replay state, and publishing.

Complete those connections while preserving the native-only direction, shared headless simulation, and inexpensive development workflow.

Keep this milestone bounded. New game modes, weapons, 2D networking, per-player variable replication, and broad engine/game extraction belong to later work unless a narrow prerequisite is necessary for a fix below.

Starting point and working rules

The findings below were observed at review baseline:

f58e15db0e7383519f472ef0ba5f84871e18bf77

The repository is actively developed. Inspect the current revision, identify which findings still apply, and preserve intervening fixes and unrelated work. The source-change fingerprint bug was reproduced in Python; the Rust findings were established through source tracing and need behavioral reproduction where practical.

* Follow the current AGENTS.md and prescribed scripts/dev start / context workflow. Use the paths below as navigation anchors, not a requirement to read every file.
* Record the starting revision and work in an isolated checkout or worktree when needed.
* Use iterate and focused affected checks during development. Run the repository-required full verification at the final revision.
* Keep heavy build jobs coordinated on small machines. Preserve useful caches without accepting binaries or verification results from the wrong source/configuration.
* Add regression tests that demonstrate the reported failure and the corrected behavior.
* Preserve authentication and verification guarantees. Do not make tests pass by weakening assertions, suppressing errors, or overstating capability labels.
* Use local staging destinations for publishing tests. Follow the session's existing authorization for commits, pushes, releases, and deployment.

1. Repair the complete multiplayer join and reconnect lifecycle

Default short-code joining

At the baseline:

* Graphical HOST creates a random game join key.
* LocalHost configures the server to require it.
* HOST prefers displaying the relay's six-character code.
* Short-code JOIN resolves the relay but supplies no game join key.
* The client rejects the keyed server with NeedsKey.

Starting points:
src/bin/re2/kc/app.rs, src/net/host.rs, src/net/client.rs, src/net/relay_server.rs.

Establish a coherent admission flow for the actual HOST defaults and short-code JOIN. Keep transport identity, game admission, and relay claims explicit. Do not solve the mismatch by silently removing required authentication or making one-time claims reusable.

Acceptance: a local relay fixture exercises the real default host configuration, code resolution, game authentication handshake, and successful client entry on the same map.

Automatic map retry through the relay

At the baseline, JOIN resolves once and clones the same relay claim across map candidates. Each attempt creates a new QUIC socket, while the relay consumes the claim once and binds forwarding to the claimed source address.

Make retries obtain valid admission for their transport, or preserve the transport appropriately while negotiating the map.

Acceptance: the joiner initially selects map A, the server runs map B, and B is also installed. The second candidate joins successfully through the relay. Check that abandoned attempts release resources appropriately.

Starting points:
src/bin/re2/kc/app.rs, src/net/quic.rs, src/net/relay_server.rs.

Search-and-destroy reconnects

The parked reconnect state currently retains spatial state but loses combat/death state. Rejoining can create fresh combat state, a starter kit, and spawn protection, reviving an eliminated player during a one-life round.

Define and enforce participation across disconnects, resumes, late joins, and SND subround changes.

Acceptance:

* An eliminated player reconnecting while the same SND round continues does not gain another life.
* A legitimate reconnect follows a documented restoration or spectator policy.
* Reconnecting across an SND subround does not restore an obsolete position or participation state.
* Ordinary respawn modes continue working.

Starting points:
src/net/sessions.rs, src/net/server.rs, src/sim/match_sim.rs, src/sim/objective_run.rs.

Duel capacity after replacement and resume

The fresh-slot path checks capacity, but a resumed parked slot can bypass it.

Acceptance: A and B occupy a duel; A disconnects; C fills the available place; A resumes. Active participation and team capacity remain within the configured limits, with a clear waiting/rejection policy that respects the existing participants.

2. Make AI recovery, executable selection, and guidance accurate

Fix source-change fingerprints

At the baseline, _git() strips leading whitespace from porcelain output. git_changed_files() then slices fixed status columns, turning an unstaged src/lib.rs entry into rc/lib.rs. The nonexistent path receives a null hash, so two different dirty edits can produce the same source identity.

Use a robust Git status parser, preferably with unambiguous record separators.

Acceptance:

* Make one unstaged edit, start a task, make another edit to the same file without committing, and resume.
* Resume detects the second change.
* Cover staged/unstaged changes, deletion, renames, and filenames with spaces as appropriate.
* Preserve the separation between recovery information and the verification system's own evidence.

Starting points:
scripts/red_resolve.py, scripts/launchpad.py.

Unify reported and executed binary selection

At the baseline:

* The resolver reads a binary's build revision but determines freshness using timestamps.
* A newer binary from another worktree can appear ready.
* scripts/dev doctor honors resolver overrides, while scripts/dev red selects its own local binary.

Make discovery and execution follow one consistent selection policy. Validate meaningful source/configuration provenance; timestamps alone are insufficient. Preserve cheap reuse when relevant content is genuinely unchanged.

Acceptance:

* The executable reported for an action is the executable that action runs.
* Explicit overrides behave consistently.
* A known incompatible build cannot appear ready because its timestamp is newer.
* Different worktrees or engine pins cannot silently consume one another's executable.

Starting points:
scripts/red_resolve.py, scripts/dev, and their existing test fixtures.

Keep discovery current and usable

Documentation is embedded in the CLI, but wrapper freshness checks exclude Markdown. Documentation-only corrections can therefore leave normal search serving obsolete instructions.

Prefer reading current checkout documentation with an embedded fallback for packaged installations. Avoid requiring a Rust rebuild merely to expose corrected documentation.

Also resolve:

* Instructions advertising an MCP start tool that the native MCP server does not expose.
* Launchpad routes that crash when a Git-pinned engine has not yet been cloned.
* Current status/help text that contradicts completed native-only work.

Acceptance: corrected documentation appears through normal discovery; every advertised first action exists; a missing engine produces a structured prerequisite and usable next step rather than an exception.

Starting points:
src/tools/search.rs, build.rs, src/cli/mcp.rs, scripts/launchpad.py, AGENTS.md, CLAUDE.md, STATUS.md.

3. Complete objective-mode replay coverage

At the baseline:

* MatchSim::checksum_parts() does not include arena/objective state.
* ArenaState::state_hash() exists but is not connected to checkpoints and itself omits relevant objective details.
* Server mode/team-size overrides affect the simulation, but recordings retain only the original map path/hash.
* Replay reconstructs the original scene without those effective overrides.

Record enough effective configuration to reconstruct the match, and include authoritative objective state that can affect future behavior in verification. Handle trace-version compatibility explicitly.

Acceptance:

* Record and replay objective-mode matches, including host settings that differ from the map.
* Confirm the effective mode and team size are restored.
* Deliberate differences in flag state/position, bomb state/timing, or score are detected.
* Existing supported traces retain their documented behavior.
* Diagnostics identify useful differences rather than merely reporting a generic mismatch.

Starting points:
src/sim/checksum.rs, src/sim/shooter.rs, src/sim/trace.rs, src/sim/replay.rs, src/bin/red_server.rs, src/tools/simrun.rs.

4. Finish the native 2D delivery path

At the baseline:

* The publishing manifest accepts kind: "2d", but resource-completeness checks send every JSON playable through the 3D parser.
* The 2D starter has no server map, while ordinary project publishing requires one.
* Linux engine packaging omits the standalone re2d binary.
* Native-window execution lacks a repeatable checked-in smoke gate.
* Muted/no-audio sessions do not drain sound events because draining occurs only inside the audio-output branch.

Repair these paths using the existing 2D Host and native player implementation.

Acceptance:

1. Create an untouched 2D starter through the documented command.
2. Validate and verify it.
3. Publish it into a local staging checkout using the supported project route.
4. Package the correct executable and required resources.
5. Launch the packaged result independently of the source checkout.
6. Exercise input, clean exit, and save/reload where supported by the starter.
7. Verify sound events are drained when muted or no output device exists.
8. Keep existing 3D publishing working.

Use appropriate native-window automation or a virtual display where available. Distinguish actual window execution from headless simulation/rendering checks, and report any Windows-specific validation still pending.

Starting points:
src/tools/publish_check.rs, src/tools/gamepublish.rs, src/tools/newgame.rs, scripts/package_release.sh, src/play2d.rs, and release workflows.

5. Close verification gaps and demonstrate a completed authoring task

* Ensure hosted CI actually invokes the optional video feature check where applicable; the local full script currently includes a stage absent from hosted workflow calls.
* Make AI authoring tests assert that intended final lint, verification, and simulation steps succeed. A traffic-budget pass must not conceal failed task steps.
* Keep partial iteration evidence distinct from completion evidence.
* Update guidance and capability claims to match demonstrated behavior.

After the fixes, run one bounded authoring task through the normal AI-facing entry point. Use an existing starter or small game. Demonstrate the applicable behavior checks, inspect its presentation, and execute the native result.

Record elapsed time and tool-output volume separately from task success. Identify cold-build versus warm-run conditions. Do not translate reduced tool traffic into unsupported claims about model performance or total development savings.

Final handoff

Report:

1. Starting and final revisions.
2. Which findings still applied, which were already fixed, and what changed.
3. Regression tests demonstrating the repaired behaviors.
4. Exact verification commands and outcomes.
5. Native and platform evidence actually obtained, including pending hosted checks.
6. Remaining limitations and the smallest useful follow-up.

Finish the stabilization work before moving into another feature milestone. The intended result is an engine whose existing multiplayer, recovery, discovery, replay, and delivery workflows can be trusted by the next AI agent.
