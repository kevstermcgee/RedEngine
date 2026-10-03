# 2026-10-03. Upgrade results must earn their recommendation
Status: accepted
Summary: game upgrade verify distinguishes requested-scope success, candidate verification, and application readiness, so a baseline-only or partial check can never recommend pinning an unverified target.

## Context
A security/correctness review of `game upgrade plan|verify` (ADR 2026-10-01) found that `VerifyReport` conflated
three different questions into one `ok()`: whether the stages that happened to run passed. `--only baseline`
never builds the target engine at all (`target_built: false`), yet a passing baseline made `ok()` true, and
`render_report` printed `next: game pin --engine <target checkout>` whenever `ok()` was true — recommending the
pin after a check that had verified nothing about the candidate. The same collapse hid two more gaps: a required
migration (`docs/upgrade-migrations.json`, e.g. `protocol-version-lockstep`) being `Applicable` never blocked the
recommendation, and a `CustomRustClient` project's custom crate is never built or run by any stage (`content`/
`full` only regenerate and check blueprint-built maps) — a clean map regeneration looked identical to a verified
client. Separately, `--only` accepted any string other than the literal `"baseline"` and silently ran the entire
pipeline anyway, so the recovery line `game upgrade verify <packet> --only content` printed after a failed
`content` stage did not actually run "just content." And `packet_pieces` turned a missing migrations array, an
unreadable `docs/upgrade-migrations.json`, or an unrecognized confidence value into a silent empty/negative
default — the single most dangerous failure mode for a *required* migration.

Two further gaps, found while fixing the above: `game pin --engine <checkout>` reads whatever commit that
checkout's HEAD currently is, not the exact commit a prior `verify` certified — a branch checkout can move between
verifying and pinning. And `ensure_target_build`'s cargo invocation omitted `--locked` despite the build identity
being keyed partly on a `Cargo.lock` hash, so the lockfile Cargo actually resolved during the build was never
guaranteed to be the one the identity claimed; nor did it verify the resulting binary existed before writing a
success receipt, or protect a cache entry from a second concurrent caller deleting it mid-build.

## Decision
`VerifyReport` now exposes four separate, independently meaningful answers instead of one `ok()`:
- `scope_ok()` — did every stage that actually ran pass (what the CLI's exit code reflects; renamed from `ok()`
  for clarity, same meaning as before).
- `candidate_verified()` — was the target engine actually built *and* exercised through both `content` and `full`
  (false for `--only baseline`/`target-build`/`content`, each a real but partial check).
- `unresolved_required_migrations()` — required migrations still `Applicable` or `Possible`; this tool never
  auto-applies a repair or confirms one was done, so these block readiness until a fresh `plan` shows them
  resolved.
- `ready_to_apply()` — `candidate_verified() && unresolved_required_migrations().is_empty() &&
  !unverified_custom_client()`. Only this licenses the `game pin` recommendation, and only this is true/false in
  `render_report`'s "ready to apply" vs "not ready to apply" text, which names exactly what is missing
  (which stage didn't run, which migration, or the unverified custom client) rather than a bare pass/fail.

`--only` is now validated against an explicit stage list (`baseline`, `target-build`, `content`, `full`) and
rejects anything else by name (`validate_only`) instead of falling through to "run everything." Each named stage
now actually stops there: `--only target-build` builds and stops, `--only content` additionally regenerates and
stops, `--only full` legitimately needs content's output as a prerequisite and so reruns it (free, by identity) —
this is what "full" requires, not the selector being ignored, and every stage that ran is still listed. The
recovery line after a failed stage now names a selector that behaves exactly as described.

`packet_pieces` rejects a malformed packet outright: a missing `migrations` array, an unreadable registry file, or
an unrecognized confidence shape is now `Err`, never a silent empty/negative default. A migration id the current
registry no longer has keeps the packet's own `required` flag and is flagged as registry drift, not dropped. A new
`evidence_is_stale` check compares the project's *current* baseline engine and class (re-derived fresh from disk,
not trusted from the packet) against what the packet recorded, and `run_verify` refuses outright — before any
stage runs — when they disagree: every migration verdict in a packet was computed against a specific baseline, and
a moved baseline makes them describe a project that no longer exists. The `baseline` stage's own detail text now
says plainly that it checks with the verifying CLI's own build, not a build of the recorded baseline engine commit
— no stage here builds the baseline engine; if that distinction ever matters for a specific migration, a future
change can add one.

`game pin` gained `--sha <commit>` (`game::engine_pin`'s new `expect_sha` parameter): when given, it refuses
unless the checkout's HEAD is exactly that commit. `render_report`'s "ready to apply" line now recommends `game
pin --engine <checkout> --sha <target_sha>` specifically so a moved branch is refused rather than silently pinning
whatever it now points at.

`ensure_target_build`'s cargo invocation gained `--locked`. It now verifies the expected binary (`bin_name()`,
platform-specific) actually exists — both for a freshly-built cache entry and for a reused one, explicit
`--engine-build` included — before accepting an identity as valid, refusing with a specific message rather than
quietly treating a half-built or corrupted directory as cached success. A new file-lock (`acquire_build_lock`:
`create_new` on a `.building` marker, atomic and portable, no Unix-only `flock`) serializes concurrent builders of
the same identity: the loser waits (bounded, 30 minutes) for the winner's result rather than deleting its
in-progress checkout out from under it.

## Consequences
An agent (or a person) reading a `game upgrade verify` report can no longer mistake "the requested scope
succeeded" for "safe to apply" — the two are different fields, and the human-readable text and JSON say the same
thing (`scope_ok`/`candidate_verified`/`ready_to_apply`/`unresolved_required_migrations`/
`unverified_custom_client`, all present in both). A custom Rust client project can never reach `ready_to_apply`
from this tool alone — a real limitation, not swept under a generic green result, matching the decision already on
record not to build a generic migration framework or client-build stage in this pass. Concurrent verifications on
the same small Linux PC that also hosts a game server (the environment this review was scoped around) no longer
risk corrupting each other's cache entry. To undo: these are refinements of `tools::upgrade`/`tools::game` in
place, not a new module — reverting means restoring the single `ok()` method, the unconditional `--only` fallback,
the un-validated `packet_pieces`, and dropping `--locked`/the artifact check/the build lock from
`ensure_target_build`; nothing here changes the packet or report file formats in an incompatible way (both gained
fields, nothing was removed).
