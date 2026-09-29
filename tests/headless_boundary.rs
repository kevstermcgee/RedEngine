//! The dedicated server must build without a window, GPU or audio device (`cargo build --no-default-features`).
//! CI builds it on a bare Linux box; this test catches the mistake earlier: any source file outside the
//! `gfx`-gated modules that names a graphics/audio crate would break that build.
//!
//! Which modules are graphics-only is read from `src/lib.rs` and every `mod.rs` under `src/` (`#[cfg(feature = "gfx")] pub mod x;`) and from the `[[bin]]`s of `Cargo.toml` that
//! require the `gfx` feature: there is no list to keep in step (it was a hand-kept list that every new graphics module had to join). The scan itself is
//! `red_engine2::tools::preflight::headless_violations`, the same function `red_engine2 preflight` runs.

use red_engine2::tools::preflight;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn headless_modules_name_no_graphics_or_audio_crates() {
    let bad = preflight::headless_violations(&root());
    assert!(
        bad.is_empty(),
        "these files are built without the `gfx` feature but name a graphics/audio crate:\n  {}\nMove the code into a gfx-gated module (see src/lib.rs) or gate it with #[cfg(feature = \"gfx\")].",
        bad.join("\n  ")
    );
}

#[test]
fn the_graphics_only_set_is_derived_from_the_repository_not_listed() {
    let gated = preflight::gfx_only_paths(&root());
    for expected in ["render.rs", "viewer.rs", "gpu.rs", "bin/re2/", "app/gpu.rs", "app/shell.rs", "app/offscreen.rs", "app/input.rs"] {
        assert!(gated.iter().any(|g| g == expected), "{expected} should be graphics-only, found {gated:?}");
    }
    assert!(!gated.iter().any(|g| g.starts_with("net")), "the network code is headless: {gated:?}");
}
