//! RedEngine 2D: a declarative 2D game, simulated deterministically with no GPU, drawn by a CPU renderer, played natively (headless) or in a browser (WebAssembly).
//!
//! A game is one JSON file (`*.game2d.json`): capabilities, a virtual screen, pixel-art sprites, sounds as voice descriptions, variables, prefabs, a scene or tile map,
//! a HUD, rules and scripted checks. [`game::parse`] validates it strictly (every typo is an error with a fix); [`sim::Game`] runs it at a fixed 60 Hz; [`render`] draws
//! a frame into RGBA pixels. The browser player (`web`, built for `wasm32`) and the CLI (`red_engine2 sim|verify|frame|web|publish`) drive the same `Game`, so a
//! scripted playthrough gives the same state hash in both.
//!
//! **Ownership.** This crate owns 2D presentation and 2D simulation semantics only. It shares, by including the engine's own source files (no copies), the pieces that are
//! not about 2D: field validation (`fields`, `suggest`), the rule expression language (`rules_expr`) and the audio description and synthesis (`voice_spec`, `dsp`,
//! `score`, `audio_fx`, `audio_analysis`, `synth`). It does not link the 3D renderer, the authoritative simulation or any networking.

// Shared with the main crate: one file, two crates (see `Cargo.toml` of the root for how `red_engine2` depends on this one).
#[path = "../../../src/audio_analysis.rs"]
pub mod audio_analysis;
#[path = "../../../src/audio_fx.rs"]
pub mod audio_fx;
#[path = "../../../src/dsp.rs"]
pub mod dsp;
#[path = "../../../src/fields.rs"]
pub mod fields;
#[path = "../../../src/sim/rules_expr.rs"]
pub mod rules_expr;
#[path = "../../../src/score.rs"]
pub mod score;
#[path = "../../../src/suggest.rs"]
pub mod suggest;
#[path = "../../../src/synth.rs"]
pub mod synth;
#[path = "../../../src/voice_spec.rs"]
pub mod voice_spec;

/// `rules_expr` suggests names through `crate::prefabs::suggest` in the main crate; here it is the same function.
pub mod prefabs {
    pub use crate::suggest::suggest;
}

pub mod caps;
pub mod controls;
pub(crate) mod effects;
pub mod font;
pub mod game;
pub mod game3d;
pub mod host;
pub mod raster3d;
pub mod reference;
pub mod render;
pub mod script;
pub mod sim;
pub mod sound;
#[cfg(target_arch = "wasm32")]
pub mod web;

/// The shared files' own unit tests reach for a few items of the main crate; the 2D crate's test build supplies just enough of them (the catalog of built-in 3D game sounds is
/// the main crate's, and its golden test runs there).
#[cfg(test)]
pub mod sfx {
    /// The main crate's built-in sound catalog; empty here.
    pub fn voice_catalog() -> Vec<(String, crate::dsp::Voice)> {
        Vec::new()
    }
}
