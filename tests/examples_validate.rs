//! Every bundled example scene must parse and validate cleanly. This catches SPEC/schema drift
//! without needing a GPU (unlike an actual render), so it's safe to run in plain `cargo test`.

use std::fs;
use std::path::Path;

#[test]
fn all_bundled_examples_are_valid_scenes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut checked = 0;
    for entry in fs::read_dir(&dir).expect("examples/ directory must exist") {
        let entry = entry.expect("readable directory entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path:?}: {e}"));
        match forge3d::schema::parse_scene(&text) {
            Ok(_) => checked += 1,
            Err(errs) => panic!("{path:?} failed validation:\n{}", errs.join("\n")),
        }
    }
    assert!(checked > 0, "expected at least one example scene in {dir:?}");
}
