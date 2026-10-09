# 2026-10-09. The launchpad reads git as data and compares content
Status: accepted
Summary: Changed files come from NUL-delimited porcelain status with no cap or skipped kind, fingerprinted by content, and resume compares content (not paths) and says how the branch moved

## Context
`scripts/launchpad.py` records what a task was made from (`identity`) and `resume` compares it with now to decide which recorded results still apply.
The identity of an engine checkout came from `git status --porcelain` parsed as text: `red_resolve._git` ends in `.strip()`, which removes the leading
space of the *first* line, and `line[3:]` then cut a path whose status column was ` M` (an unstaged edit) from `src/lib.rs` to `rc/lib.rs`. A path that
does not exist hashes to `None`, whatever the file holds, so two different edits of one file recorded the same identity, and the same wrong path made
`recorded_results` skip its "a changed file is newer than the record" check without saying so. Reading the code for the same assumption found more places where a change was silently
left out: untracked *directories* were one `dir/` entry (git's default), files over 4 MB and everything after the 400th changed file were skipped, a rename's old name was dropped,
a game project's pinned engine was not part of the identity at all, and a file being rewritten during the scan was recorded as whichever bytes were read first.

## Decision
- **Git is read as data.** `red_resolve.git_status` runs `status --porcelain=v1 -z --untracked-files=all` and `parse_status_z` splits it: no quoting, no stripping, no fixed-width
  guess about a path; a rename gives both names; non-UTF-8 names go through `os.fsdecode`. `_git` is documented as single-value only. Paths are kept as git writes them (`/`).
- **The identity is content, with no cap.** `launchpad.path_fingerprint` is a content hash, or `deleted`, `dir:<head>`, `symlink:<hash>`, `stat:<size>:<mtime>` (above 256 MB: still part of the
  identity, never skipped) or `unstable`. `engine_inputs` repeats the scan (three times at most) until HEAD, git's list and every file's size and mtime agree before and after it
  was read; `settled` says whether they did and an unsettled scan is itself reported as a change ("source still changing").
- **Resume compares content, not paths.** `diff_identity` treats a path missing from `files` as clean: an edit recorded at the start and committed unchanged since is not an edit,
  an edit undone is not an edit, an edit committed with other content is. A recorded path that is clean now is fingerprinted too (`tracked`) so that can be told.
  A game project's own inputs are compared as before and its pinned engine's working tree is now compared too ("engine source").
- **The branch moving is reported as such** (`head_relation`): advanced (somebody committed: how many commits, which files), rewound, diverged (history rewritten) or unknown (the
  recorded revision is gone), from read-only git.
- **One next action.** Nothing changed: the plan's own first step stands. Something changed: `since_start` lists what is reused (workflow, notes, owners, a fresh CLI: nothing is
  rebuilt to resume) and what is invalidated, and `next_action` becomes one correction: wait (files still changing), read the other commits, or the cheap re-check (`scripts/dev iterate`,
  `scripts/red check`); the plan's own is kept as `planned_next_action`.
- A task recorded by the old launchpad (no `identity_version`) cannot be compared (its paths may be corrupt): it is reported as changed, with the reason, instead of being trusted.

## Consequences
- No relevant modification can be left out of a task's identity because of its path, size, kind or position in the list. `scripts/test_launchpad_git.py` (run by `tests/launchpad.rs`)
  makes every kind of change in real repositories, reproduces the original `rc/lib.rs`, and covers interruption and another agent advancing the branch.
- The scan hashes every changed file (streamed, so memory does not grow): a checkout with gigabytes of uncommitted binaries pays for it at `start` and `resume` (read-only, no builds).
- `dirty_files` (the "N uncommitted" figure, compared with what `scripts/dev` recorded at build time) still counts git's default listing, one entry per new directory, so it is unchanged.
- `src/tools/package.rs::git_info` reads `status --porcelain` as text too (quoted names, ` -> ` in a name); it feeds a package manifest, not a decision, and is left alone here.
