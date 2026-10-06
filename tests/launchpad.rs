//! The AI launchpad and the shared executable resolver (`scripts/launchpad.py`, `scripts/red_resolve.py`; ADR 2026-10-06-an-ai-launchpad-and-one-executable-resolver).
//! The checks are Python unit tests (`scripts/test_launchpad.py`) over throwaway engine checkouts with a fake `cargo` and a fake `red_engine2`, so "discovery never compiles"
//! and "a stale binary is never run" are observed, not assumed. They need only `python3`; skipped where it is not installed.

use std::path::Path;
use std::process::Command;

fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|p| Command::new(p).arg("--version").output().is_ok_and(|o| o.status.success()))
}

#[test]
fn the_launchpad_and_the_resolver_pass_their_checks() {
    let Some(py) = python() else { return };
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/test_launchpad.py");
    let o = Command::new(py).arg(&script).output().expect("run the launchpad tests");
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert!(o.status.success(), "scripts/test_launchpad.py failed:\n{text}");
    assert!(text.contains("OK"), "{text}");
}
