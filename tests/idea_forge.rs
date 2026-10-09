//! The Idea Forge pipeline (`scripts/idea_forge.py`, docs/IDEA_FORGE.md): forge an idea, run an agent on it, score the game, file the feedback, ship only the game and its feedback.
//! The checks are Python unit tests (`scripts/test_idea_forge.py`) with a stub `claude` and a fake engine, so the loop is observed without a model, a network or a GPU.
//! They need only `python3` and `git`; skipped where `python3` is not installed. They run on Windows too (the shell-script stubs are skipped there), so a path-quoting
//! assumption in a test shows up in the Windows job.

use std::path::Path;
use std::process::Command;

fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|p| Command::new(p).arg("--version").output().is_ok_and(|o| o.status.success()))
}

#[test]
fn the_idea_forge_pipeline_passes_its_checks() {
    let Some(py) = python() else { return };
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/test_idea_forge.py");
    let o = Command::new(py).arg(&script).output().expect("run the idea forge tests");
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert!(o.status.success(), "scripts/test_idea_forge.py failed:\n{text}");
    assert!(text.contains("OK"), "{text}");
}
