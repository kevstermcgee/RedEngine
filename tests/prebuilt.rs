//! The prebuilt-binary fetch (`scripts/prebuilt.py`, ADR 2026-10-09-prebuilt-binary-when-sources-match): a release built from exactly the checkout's sources
//! is downloaded, checksum-verified and used instead of a 7-minute compile; anything else falls back to building with the reason said.
//! The checks are Python unit tests (`scripts/test_prebuilt.py`) over a throwaway git checkout and a release in a directory (real archives, manifest and checksums, fake binaries), with a
//! fake `cargo` that records a compile, so "no compile" and "falls back cleanly" are observed. They need `python3` and `git`; skipped where python is not installed.

use std::path::Path;
use std::process::Command;

fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|p| Command::new(p).arg("--version").output().is_ok_and(|o| o.status.success()))
}

#[test]
fn a_matching_release_replaces_the_compile_and_anything_else_falls_back() {
    let Some(py) = python() else { return };
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/test_prebuilt.py");
    let o = Command::new(py).arg(&script).output().expect("run the prebuilt tests");
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert!(o.status.success(), "scripts/test_prebuilt.py failed:\n{text}");
    assert!(text.contains("OK"), "{text}");
}
