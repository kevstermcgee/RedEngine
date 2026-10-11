//! `red_engine2 validate` with no paths: every content file in the checkout passes its own checker, in well under the 10 s budget, and a broken file
//! is reported as `file:line: message` (ADR 2026-10-11-one-validate-for-every-content-file).

use std::process::Command;
use std::time::Instant;

fn red() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_red_engine2"));
    c.current_dir(env!("CARGO_MANIFEST_DIR"));
    c
}

#[test]
fn the_whole_checkout_validates_within_the_budget() {
    let t = Instant::now();
    let out = red().arg("validate").output().unwrap();
    let secs = t.elapsed().as_secs_f64();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    assert!(text.contains("0 problem(s)") && text.contains("scene ") && text.contains("2D game "), "{text}");
    // The budget the owner set is 10 s for the whole repository; it measures ~0.2 s, so this only fails on a real regression (not on a busy machine).
    assert!(secs < 10.0, "validate took {secs:.1} s for the whole checkout (budget 10 s)");
}

#[test]
fn a_folder_with_a_broken_file_names_the_file_and_line_and_fails() {
    let dir = std::env::temp_dir().join(format!("re2_content_validate_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ok.json"), r#"{"camera":{"position":[0,2,8]},"objects":[{"id":"a","type":"box","position":[0,0.5,0]}]}"#).unwrap();
    std::fs::write(
        dir.join("bad.json"),
        "{\n  \"camera\": {\"position\": [0, 2, 8]},\n  \"objects\": [\n    {\"id\": \"p\", \"type\": \"prefab\", \"prefab\": \"lamp_postt\", \"position\": [0, 0, 0]}\n  ]\n}",
    )
    .unwrap();
    let out = red().arg("validate").arg(&dir).output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(err.contains("bad.json:4: p.prefab: unknown prefab 'lamp_postt'"), "{err}");
    assert!(err.contains("1 problem(s) in 1 file(s)"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}
