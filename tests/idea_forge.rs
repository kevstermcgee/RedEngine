//! The Idea Forge pipeline (`scripts/idea_forge.py`, docs/IDEA_FORGE.md): forge an idea, run an agent on it, score the game, file the feedback, ship only the game and its feedback.
//! The checks are Python unit tests (`scripts/test_idea_forge.py`) with a stub `claude` and a fake engine, so the loop is observed without a model, a network or a GPU.
//! They need only `python3` and `git`; skipped where `python3` is not installed. They run on Windows too (the shell-script stubs are skipped there), so a path-quoting
//! assumption in a test shows up in the Windows job.

use std::path::Path;
use std::process::Command;

fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|p| Command::new(p).arg("--version").output().is_ok_and(|o| o.status.success()))
}

/// Runs one Python test file and returns its combined output; `None` where Python is not installed.
fn run_python_tests(script: &str) -> Option<String> {
    let py = python()?;
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(script);
    let o = Command::new(py).arg(&path).output().expect("run the python tests");
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert!(o.status.success(), "{script} failed:\n{text}");
    assert!(text.contains("OK"), "{text}");
    Some(text)
}

#[test]
fn the_idea_forge_pipeline_passes_its_checks() {
    run_python_tests("scripts/test_idea_forge.py");
}

/// A supervised child (the agent, a gate) stops at its deadline whether or not it prints, in quiet mode too, with its whole process tree, and keeps its partial output
/// (`scripts/proc_supervisor.py`, ADR 2026-10-09-a-child-process-runs-under-a-deadline-it-cannot-miss). Every child is `python -c`, so this runs the same on Linux and Windows.
#[test]
fn a_supervised_process_respects_its_deadline_whatever_it_prints_and_leaves_nothing_running() {
    run_python_tests("scripts/test_proc_supervisor.py");
}
