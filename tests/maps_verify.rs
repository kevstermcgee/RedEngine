//! The four game maps (`house`, `school`, `office`, `store`) must stay playable: each one carries its
//! own `"checks"` block (lint budget, real-physics walks between rooms, object assertions), and this
//! runs them all — without the golden-image views, which need a GPU. If one of these fails, an edit
//! sealed a door, blocked a route or left a lint error; run `red_engine2 verify examples/<map>.json`
//! for the evidence.

use std::path::Path;

use red_engine2::tools::verify::{run, Options};

fn verify(map: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples").join(map);
    let report = run(&path, &Options { skip_views: true, ..Default::default() }).unwrap_or_else(|e| panic!("{map}: {e}"));
    assert_eq!(report.failed(), 0, "{map} failed its own checks:\n{}", report.render());
}

#[test]
fn the_school_passes_its_own_checks() {
    verify("school.json");
}

#[test]
fn the_office_passes_its_own_checks() {
    verify("office.json");
}

#[test]
fn the_store_passes_its_own_checks() {
    verify("store.json");
}
