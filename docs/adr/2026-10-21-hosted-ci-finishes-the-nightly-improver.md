# 2026-10-21. Hosted CI finishes the nightly improver
Status: accepted
Summary: Hosted checks on the inspected head authorize nightly merges; local failures are reported rather than making permanent drafts, and a recurring settler plus bounded repair sessions carry changes through CI delays, transient failures and conflicts

## Context
The nightly improver promised unattended merges, but a failed local check made a draft that could never recover when hosted CI passed.
The 2026-10-20 run produced PR #218 this way: its full local suite failed in an unrelated Idea Forge fixture's temporary clone, and the
PR needed a manual merge. A hosted wait expiring also deferred all progress until the following night. The shim reset the worktree before
the Python runner took its lock, allowing an overlapping invocation to erase an active session's work.

## Decision
Hosted CI is the merge authority for allowed nightly changes. Local checks still run, with a process-tree deadline, and their failures
remain in the PR body. Only refused paths produce drafts; legacy local-failure drafts can recover once hosted checks pass. Missing checks,
unknown mergeability, incomplete file lists and missing ownership data never authorize a merge. Merging pins the inspected head SHA.

A separate systemd timer settles every 15 minutes without starting a model. It retries failed workflow jobs once per head and updates a
branch behind main with an expected-head guard. Once per night a capped repair session may resolve a failed or conflicting PR in a
temporary worktree, preserving both histories and pushing with a lease. It gets at most two attempts per PR. Failed work is preserved;
refused paths and intentional drafts retain maintainer control. Merge-off disables merge and recovery writes.

RedEngineGames' path filters can omit a documentation-only PR. After five minutes without checks, dispatch its existing read-only
`checks.yml` validation on the daily branch once per head. A dispatch is never a passing result; the actual checks must finish green.
The fallback neither dispatches nor retries release/deployment workflows.

The shim takes the same lock as direct run/settle commands before fetching or resetting. Bootstrap failures fail the service. Each new
session gets a distinct daily branch, so a rerun cannot replace a previous run. The nightly timer catches missed runs after downtime.

## Consequences
Routine local flakiness, slow CI and conflicts no longer imply a manual merge. A consistently failing change still cannot merge: after
the bounded recovery attempts its PR and logs remain available. Repair sessions have a separate default $5 cap and 30-minute deadline;
the ordinary improvement budget stays unchanged. Both maintainers can still stop a PR by closing it.
