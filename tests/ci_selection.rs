//! Which hosted CI jobs a change starts (`.github/workflows/ci.yml`, the `what changed` step): the step is run as written, under bash, in a throwaway repository whose second commit changes
//! exactly the named files (`scripts/test_ci_selection.py`, ADR 2026-10-09-ci-runs-the-python-tools-on-windows-when-they-change). Skipped where python3, bash or git is missing.

use std::path::Path;
use std::process::Command;

fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|p| Command::new(p).arg("--version").output().is_ok_and(|o| o.status.success()))
}

#[test]
fn a_change_to_a_python_tool_starts_the_python_job_and_documentation_starts_nothing() {
    let Some(py) = python() else { return };
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/test_ci_selection.py");
    let o = Command::new(py).arg(&script).output().expect("run the CI selection tests");
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert!(o.status.success(), "scripts/test_ci_selection.py failed:\n{text}");
    assert!(text.contains("OK"), "{text}");
}
