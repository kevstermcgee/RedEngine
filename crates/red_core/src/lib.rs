//! Red Engine's headless core (`red_core`): the code the engine crate (`red_engine2`), the 2D crate (`red2d`) and game crates share, so each
//! is compiled once and has one type (ADR 2026-10-13-red-core-is-one-crate-the-engine-and-its-games-share). It grows module by module toward the
//! headless server's whole tree (ADR 2026-10-13-the-next-phase, phase 1 item 3); a module moves here only when everything it uses is here already.
//! The engine re-exports every module at its old path (`red_engine2::dsp`, `red_engine2::sim::rules_expr`), so no caller changes.
//!
//! Today the scene format and everything it builds on (`schema`, `strict`, `prefabs`, `macros`, `style`, the scene's blocks: `atmosphere`, `ambience`,
//! `daycycle`, `terrain`, `procgen::world`, ...), the simulation (`sim::*`: rules, matches, karts, combat, bots, traces and checksums), the physics
//! (`physics`, `collide`, `player`, `props`, `hit`, `characters`), the wire protocol (`net::protocol`, `net::interp`, `net::happenings`) and the
//! networking that needs no socket of its own (`net::{limits, netsim, upnp, portmap, join_code, auth, relay}`), with the synthesizer, the rules'
//! expression language, strict fields, colours, easing and crypto under them. Two optional features belong to builds that draw or play sound:
//! `shader-check` (naga: a game's sky is checked against the engine's shaders) and `decode` (symphonia: the player's own music).

pub mod ambience;
pub mod arsenal;
pub mod atmosphere;
pub mod audio_analysis;
pub mod audio_fx;
pub mod characters;
pub mod collide;
pub mod color;
pub mod costumes;
pub mod crypto;
pub mod daycycle;
pub mod dsp;
pub mod easing;
pub mod expanse;
pub mod feel;
pub mod fields;
pub mod font_glyphs;
pub mod geometry;
pub mod hit;
pub mod hud_config;
pub mod killcam;
pub mod listen;
pub mod mixer;
pub mod nature;
pub mod physics;
pub mod player;
pub mod prefabs;
pub mod props;
pub mod scene_macros;
pub mod schema;
pub mod score;
pub mod sfx;
/// Compiles WGSL with naga and refuses what Direct3D would (only with the `shader-check` feature, which the engine's `render` turns on).
#[cfg(feature = "shader-check")]
pub mod shader_check;
pub mod skeleton;
pub mod strict;
pub mod style;
pub mod suggest;
pub mod synth;
pub mod terrain;
pub mod track;
pub mod ui_config;
pub mod uniforms;
pub mod voice_spec;
pub mod weapons;

/// The procedural world: noise, the table of species, the generator and the ground it gives (the streaming and the shapes the renderer draws are the
/// engine crate's, which re-exports these at `red_engine2::procgen::*`).
pub mod procgen {
    pub mod flora;
    pub mod ground;
    pub mod noise;
    pub mod world;

    pub use flora::{Climate, Kind, Species, SpeciesId};
    pub use ground::ProcgenGround;
    pub use world::{Biome, ChunkId, Config, Plant, Trunk, World, CHUNK};
}

/// The simulation (the engine crate keeps `sim::env`, `sim::replay` and `sim::scenario`, and re-exports these at `red_engine2::sim::*`).
pub mod sim {
    // Nothing reachable from a UDP packet may panic the process (the engine's `sim` and `net` hold the same rule). Test code is exempt.
    #![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::todo, clippy::unimplemented, clippy::unreachable))]
    pub mod ai;
    pub mod approach;
    pub mod change;
    pub mod checksum;
    pub mod clock;
    pub mod combat;
    pub mod combat_cfg;
    pub mod components;
    pub mod entities;
    pub mod flow;
    pub mod interact;
    pub mod interest;
    pub mod items;
    pub mod kart;
    pub mod kit;
    pub mod match_sim;
    pub mod objective;
    pub mod objective_run;
    pub mod ordnance;
    pub mod player;
    pub mod race;
    pub mod rules;
    pub mod rules_expr;
    pub mod rules_run;
    pub mod scratch;
    pub mod shooter;
    pub mod snapshot;
    pub mod spawns;
    pub mod statics;
    pub mod trace;
}

/// The network: the wire protocol, the server, the client and their QUIC transport (`quic`: quinn, rustls, tokio; the engine turns it on, the 2D crate
/// has no network), and the pieces that need no socket of their own. The engine crate keeps the hosting from a game, the bots and the client
/// session, and re-exports these at `red_engine2::net::*`.
pub mod net {
    // Nothing reachable from a UDP packet may panic the process. Test code is exempt.
    #![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::todo, clippy::unimplemented, clippy::unreachable))]
    pub mod auth;
    #[cfg(feature = "quic")]
    pub mod client;
    pub mod happenings;
    pub mod interp;
    pub mod join_code;
    pub mod limits;
    pub mod netsim;
    pub mod portmap;
    pub mod protocol;
    #[cfg(feature = "quic")]
    pub mod quic;
    pub mod relay;
    pub mod relay_server;
    #[cfg(feature = "quic")]
    pub mod server;
    #[cfg(feature = "quic")]
    mod sessions;
    #[cfg(feature = "quic")]
    mod snapshots;
    #[cfg(feature = "quic")]
    pub mod transport;
    pub mod upnp;

    /// The per-client prop selection, exposed only so `benches/settled_world.rs` can time it without a socket.
    #[cfg(feature = "quic")]
    #[doc(hidden)]
    pub use snapshots::props_to_send;

    /// The default UDP port of a Red server.
    pub const DEFAULT_PORT: u16 = 27015;

    /// A 32-bit hash of a map file's text (line endings ignored), sent in the join request so a client
    /// with a different copy of the map is refused instead of desyncing silently.
    pub fn map_hash(text: &str) -> u32 {
        let mut h = 0x811c_9dc5u32;
        for b in text.bytes().filter(|b| *b != b'\r') {
            h ^= b as u32;
            h = h.wrapping_mul(0x0100_0193);
        }
        h
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn map_hash_ignores_line_endings_but_not_content() {
            assert_eq!(map_hash("a\nb\n"), map_hash("a\r\nb\r\n"));
            assert_ne!(map_hash("a\nb\n"), map_hash("a\nc\n"));
        }
    }
}
