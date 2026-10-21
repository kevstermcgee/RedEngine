//! The home-hosting deploy scripts (`deploy/red-portmap.py`, `deploy/red-selfcheck.py`; docs/HOSTING.md "Operating the box"): permanent UPnP mappings against a fake router that
//! ignores an identical re-add like the real one, and the five-minute self-check against a fake relay, a fake QUIC listener and a fake machine. The checks are Python unit tests
//! (`scripts/test_red_deploy.py`, standard library only, no network, no systemd); skipped where `python3` is not installed. They run on Windows too (the hosted python job).

use std::path::Path;
use std::process::Command;

fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|p| Command::new(p).arg("--version").output().is_ok_and(|o| o.status.success()))
}

#[test]
fn the_deploy_scripts_keep_the_mappings_permanent_and_the_selfcheck_names_what_is_wrong() {
    let Some(py) = python() else { return };
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/test_red_deploy.py");
    let o = Command::new(py).arg(&script).output().expect("run the deploy script tests");
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert!(o.status.success(), "scripts/test_red_deploy.py failed:\n{text}");
    assert!(text.contains("OK"), "{text}");
}
