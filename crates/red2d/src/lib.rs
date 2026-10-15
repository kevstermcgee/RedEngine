//! RedEngine 2D: a declarative 2D game, simulated deterministically with no GPU, drawn by a CPU renderer, played natively (a window, or headless for checks).
//!
//! A game is one JSON file (`*.game2d.json`): capabilities, a virtual screen, pixel-art sprites, sounds as voice descriptions, variables, prefabs, a scene or tile map,
//! a HUD, rules and scripted checks. [`game::parse`] validates it strictly (every typo is an error with a fix); [`sim::Game`] runs it at a fixed 60 Hz; [`render`] draws
//! a frame into RGBA pixels. The native player (`red_engine2 play2d`, `re2d`) and the CLI (`red_engine2 sim|verify|frame`) drive the same `Game`, so a
//! scripted playthrough gives the same state hash in both.
//!
//! **Ownership.** This crate owns 2D presentation and 2D simulation semantics only. The pieces that are not about 2D come from the headless core it shares with
//! the engine crate (`red_core`): field validation (`fields`, `suggest`), the rule expression language (`rules_expr`) and the audio description and synthesis
//! (`voice_spec`, `dsp`, `score`, `audio_fx`, `audio_analysis`, `synth`). It does not link the 3D renderer, the authoritative simulation or any networking.

// The headless core, shared with the engine crate: one copy, one set of types (ADR 2026-10-13-red-core-is-one-crate-the-engine-and-its-games-share).
pub use red_core::sim::rules_expr;
pub use red_core::{audio_analysis, audio_fx, dsp, fields, score, suggest, synth, voice_spec};

pub mod caps;
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
pub mod solve;
pub mod sound;
mod talk;
