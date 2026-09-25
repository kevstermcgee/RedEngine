//! The dedicated server must build without a window, GPU or audio device (`cargo build --no-default-features`).
//! CI builds it on a bare Linux box; this test catches the mistake earlier: any source file outside the
//! `gfx`-gated modules that names a graphics/audio crate would break that build.

use std::path::{Path, PathBuf};

/// Modules gated behind `#[cfg(feature = "gfx")]` in `src/lib.rs`, plus the windowed binary.
const GFX_ONLY: &[&str] = &["audio.rs", "gpu.rs", "menu.rs", "mesh.rs", "overlay.rs", "render.rs", "revolver.rs", "video.rs", "viewer.rs"];
/// Directories that are entirely windowed code (`re2` is a directory binary: main.rs, weapons.rs, avatar.rs, window.rs, win.rs).
const GFX_ONLY_DIRS: &[&str] = &["bin/re2/"];
const BANNED: &[&str] = &["wgpu::", "winit::", "rodio::", "ffmpeg_sidecar", "pollster::"];

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn headless_modules_name_no_graphics_or_audio_crates() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rs_files(&src, &mut files);
    let mut bad = Vec::new();
    for f in files {
        let rel = f.strip_prefix(&src).unwrap().to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
        if GFX_ONLY.contains(&rel.as_str()) || GFX_ONLY_DIRS.iter().any(|d| rel.starts_with(d)) {
            continue;
        }
        let text = std::fs::read_to_string(&f).unwrap();
        for (n, line) in text.lines().enumerate().filter(|(_, l)| !l.trim_start().starts_with("//")) {
            if let Some(b) = BANNED.iter().find(|b| line.contains(*b)) {
                bad.push(format!("src/{rel}:{}: `{b}`", n + 1));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "these files are built without the `gfx` feature but name a graphics/audio crate:\n  {}\nMove the code into a gfx-gated module (see src/lib.rs) or gate it with #[cfg(feature = \"gfx\")].",
        bad.join("\n  ")
    );
}

#[test]
fn every_gfx_only_module_is_gated_in_lib_rs() {
    let lib = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs")).unwrap();
    for m in GFX_ONLY.iter().filter(|m| !m.contains('/')) {
        let name = m.trim_end_matches(".rs");
        let gated = lib.contains(&format!("#[cfg(feature = \"gfx\")]\npub mod {name};"));
        assert!(gated, "src/lib.rs must declare `pub mod {name};` behind #[cfg(feature = \"gfx\")]");
    }
}
