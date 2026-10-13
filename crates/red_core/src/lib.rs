//! Red Engine's headless core (`red_core`): the code the engine crate (`red_engine2`), the 2D crate (`red2d`) and game crates share, so each
//! is compiled once and has one type (ADR 2026-10-13-red-core-is-one-crate-the-engine-and-its-games-share). It grows module by module toward the
//! headless server's whole tree (ADR 2026-10-13-the-next-phase, phase 1 item 3); a module moves here only when everything it uses is here already.
//! The engine re-exports every module at its old path (`red_engine2::dsp`, `red_engine2::sim::rules_expr`), so no caller changes.
//!
//! Today: the synthesizer and its analysis (`synth`, `dsp`, `audio_fx`, `audio_analysis`, `voice_spec`, `score`), the rules' expression language
//! (`sim::rules_expr`), strict field checking (`fields`, `suggest`) and the bitmap font's glyphs (`font_glyphs`).

pub mod audio_analysis;
pub mod audio_fx;
pub mod dsp;
pub mod fields;
pub mod font_glyphs;
pub mod score;
pub mod suggest;
pub mod synth;
pub mod voice_spec;

/// The simulation's pieces that live in the core so far (the rest of `sim` is still in the engine crate).
pub mod sim {
    pub mod rules_expr;
}
