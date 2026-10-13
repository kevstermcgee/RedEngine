//! Red Engine's headless core (`red_core`): the code the engine crate (`red_engine2`), the 2D crate (`red2d`) and game crates share, so each
//! is compiled once and has one type (ADR 2026-10-13-red-core-is-one-crate-the-engine-and-its-games-share). It grows module by module toward the
//! headless server's whole tree (ADR 2026-10-13-the-next-phase, phase 1 item 3); a module moves here only when everything it uses is here already.
//! The engine re-exports every module at its old path (`red_engine2::dsp`, `red_engine2::sim::rules_expr`), so no caller changes.
//!
//! Today: the synthesizer and its analysis (`synth`, `dsp`, `audio_fx`, `audio_analysis`, `voice_spec`, `score`), the rules' expression language
//! (`sim::rules_expr`), strict field checking (`fields`, `suggest`), the bitmap font's glyphs (`font_glyphs`), colours and easing curves (`color`, `easing`),
//! the cryptographic primitives (`crypto`), the simulation's foundations (`sim::{clock, scratch, components, change, entities, snapshot, statics, spawns,
//! approach}`) and networking that needs no wire protocol yet (`net::{limits, netsim, upnp, join_code, auth, relay}`).

pub mod audio_analysis;
pub mod audio_fx;
pub mod color;
pub mod crypto;
pub mod dsp;
pub mod easing;
pub mod fields;
pub mod font_glyphs;
pub mod score;
pub mod suggest;
pub mod synth;
pub mod voice_spec;

/// The simulation's pieces that live in the core so far (the rest of `sim` is still in the engine crate, which re-exports these at `red_engine2::sim::*`).
pub mod sim {
    // Nothing reachable from a UDP packet may panic the process (the engine's `sim` and `net` hold the same rule). Test code is exempt.
    #![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::todo, clippy::unimplemented, clippy::unreachable))]
    pub mod approach;
    pub mod change;
    pub mod clock;
    pub mod components;
    pub mod entities;
    pub mod rules_expr;
    pub mod scratch;
    pub mod snapshot;
    pub mod spawns;
    pub mod statics;
}

/// The networking pieces that live in the core so far (the server, the client and the wire protocol are still in the engine crate, which re-exports
/// these at `red_engine2::net::*`).
pub mod net {
    // Nothing reachable from a UDP packet may panic the process. Test code is exempt.
    #![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::todo, clippy::unimplemented, clippy::unreachable))]
    pub mod auth;
    pub mod join_code;
    pub mod limits;
    pub mod netsim;
    pub mod relay;
    pub mod upnp;

    /// The default UDP port of a Red server.
    pub const DEFAULT_PORT: u16 = 27015;
}
