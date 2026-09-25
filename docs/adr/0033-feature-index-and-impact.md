# 0033. A feature index that cannot rot, and `impact`
Status: accepted

## Context
BlueEngine keeps `tools/FEATURES.json`, a hand-written map of feature -> files -> checks. It answers "where is X?" but nothing keeps it
true, and it cannot answer the question an AI actually has after an edit: "I changed this file: what do I have to run?"
Red already answers "where is X?" dynamically (`search`, `src find`); it lacked the second question.

## Decision
`docs/features.json` (compiled into the binary) lists each feature's summary, file patterns, tests, verification commands, docs and
`depends_on`. Two commands read it:
- `red_engine2 features [NAME | WORDS]` lists, shows or searches.
- `red_engine2 impact FILES` (or `--git [REF]` for the working tree plus untracked files) names the features that own the files, the
  features **built on those** (transitively, through `depends_on`), and prints the exact `cargo test --lib -- filters` and
  `cargo test --test suite ...` commands plus the verification commands and the docs and ADRs to keep true. A test file belongs to
  every feature that runs its suite, so editing a test says what it can break.
- **It cannot drift:** `red_engine2 features --check` and `tests/features_index.rs` fail when a listed file pattern matches nothing,
  a test suite has no `tests/<name>.rs`, a `lib:` filter is not a module, a doc does not exist, a dependency is unknown, or **any
  source file, test or bench belongs to no feature**.

## Consequences
Adding a module means adding it to a feature (the failing test says which file), which is the moment to decide what it depends on and
which tests cover it. The index is coarse on purpose (about 30 features): finer ownership would be a second thing to keep in sync with
the module tree. Dependencies are declared, not inferred from `use` edges (`src deps` shows those); a missing `depends_on` shows up as an
`impact` that under-reports, which is a review question, not something to guess.
