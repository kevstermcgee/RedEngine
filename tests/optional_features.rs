//! Commands whose cargo feature is not in this build say so, with the way out, instead of failing obscurely (ADR 2026-10-07-tooling-dependencies-are-optional-features).
//! The default build has `mcp` and no `video`; `--features video` and `--no-default-features` are checked by CI building each.

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_red_engine2");

fn run(args: &[&str]) -> (bool, String) {
    let o = Command::new(BIN).args(args).output().unwrap();
    (o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

#[cfg(all(feature = "gfx", not(feature = "video")))]
#[test]
fn mp4_export_names_the_feature_it_needs_and_png_output_is_unaffected() {
    let (ok, text) = run(&["render", "examples/test_lab.json", "out/never.mp4"]);
    assert!(!ok);
    assert!(text.contains("feature `video`") && text.contains("--features video") && text.contains("PNG"), "{text}");
}

#[cfg(not(feature = "mcp"))]
#[test]
fn mcp_names_the_feature_it_needs() {
    let (ok, text) = run(&["mcp"]);
    assert!(!ok);
    assert!(text.contains("feature `mcp`"), "{text}");
}

#[test]
fn the_binary_runs_in_every_feature_set() {
    let (ok, text) = run(&["describe", "--brief"]);
    assert!(ok, "{text}");
}
