//! The AI launchpad and the shared executable resolver (`scripts/launchpad.py`, `scripts/red_resolve.py`; ADR 2026-10-06-an-ai-launchpad-and-one-executable-resolver).
//! The checks are Python unit tests (`scripts/test_launchpad.py`) over throwaway engine checkouts with a fake `cargo` and a fake `red_engine2`, so "discovery never compiles"
//! and "a stale binary is never run" are observed, not assumed. They need only `python3`; skipped where it is not installed. `scripts/test_launchpad_git.py` does the same with real git repositories.

use std::path::Path;
use std::process::Command;

fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|p| Command::new(p).arg("--version").output().is_ok_and(|o| o.status.success()))
}

/// Runs one Python test file and returns its combined output; `None` where Python is not installed.
fn run_python_tests(script: &str) -> Option<String> {
    let py = python()?;
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(script);
    let o = Command::new(py).arg(&path).output().expect("run the launchpad tests");
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert!(o.status.success(), "{script} failed:\n{text}");
    assert!(text.contains("OK"), "{text}");
    Some(text)
}

#[test]
fn the_launchpad_and_the_resolver_pass_their_checks() {
    run_python_tests("scripts/test_launchpad.py");
}

/// Source identity and task recovery against real git repositories: every kind of change, unusual file names, concurrent edits, another agent advancing the branch
/// (`scripts/test_launchpad_git.py`). Needs `git` as well as Python; the file skips itself where git is missing.
#[test]
fn source_identity_and_task_recovery_hold_in_real_repositories() {
    run_python_tests("scripts/test_launchpad_git.py");
}
