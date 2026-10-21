//! Real-time multiplayer over UDP: an authoritative headless server ([`server`]) and a client library
//! ([`client`]) that the graphical game and the headless bot both use.
//!
//! - [`protocol`]: the wire format (fuzz-tested, bounded).
//! - [`arena_wire`]: the loadout shooter's own part of a snapshot (a `protocol::GameMsg` the protocol carries without reading).
//! - [`server`]: `Server` = the socket and the tick loop around [`crate::sim::match_sim::MatchSim`]: a fixed 60 Hz
//!   tick, 30 Hz snapshots with per-client delta acknowledgement, timeouts, reconnect-with-token. Its parts:
//!   `sessions` (who is connected, resume tokens), `snapshots` (what each client is sent), [`limits`] (packet budgets).
//! - [`client`]: `NetClient` = handshake, redundant input sending, snapshot receiving, reconnect.
//! - [`interp`]: smooth rendering of remote players and props from snapshots.
//! - [`host`]: a server on a thread of the current process (`re2 --host`: play against a map's bots with nothing else to start).
//! - [`join`]: JOIN as a library: what a typed relay or direct code becomes (address, pinned identity, join key, relay claim).
//! - [`join_task`]: JOIN on a worker thread (the window keeps drawing; every map is a fresh attempt with a fresh relay claim).
//! - [`bot`]: a headless scripted client (how multiplayer is proved without a window).
//! - [`session`]: the graphical client's glue (prediction + scene updates), window-free and tested.
//! - [`predict`]: client-side prediction and reconciliation of the local player.
//!
//! Design and limits: ADR 0016. Nothing here touches a window, GPU or audio device.

// Nothing reachable from a UDP packet may panic the process. Test code is exempt; a genuine invariant is written
// as a `let ... else` / `?` with a message, not an `unwrap`.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::todo, clippy::unimplemented, clippy::unreachable))]

pub use red_core::net::arena_wire;
pub use red_core::net::auth;
pub mod bot;
pub use red_core::net::client;
pub mod fleet;
pub use red_core::net::game_view;
pub use red_core::net::happenings;
pub mod host;
pub use red_core::net::interp;
#[cfg(not(target_arch = "wasm32"))]
pub mod join;
#[cfg(not(target_arch = "wasm32"))]
pub mod join_task;
pub use red_core::net::join_code;
pub use red_core::net::limits;
pub mod memnet;
pub use red_core::net::netsim;
pub mod predict;
pub use red_core::net::portmap;
pub use red_core::net::protocol;
pub use red_core::net::quic;
pub use red_core::net::refusals;
pub use red_core::net::relay;
pub use red_core::net::relay_server;
pub use red_core::net::server;
pub mod session;
/// The per-client prop selection, exposed only so `benches/settled_world.rs` can time it without a socket.
#[doc(hidden)]
pub use red_core::net::props_to_send;
/// What a snapshot for a viewer carries, built without a server: only for the loadout parity harness (`tests/loadout_parity.rs`).
#[doc(hidden)]
pub use red_core::net::snapshot_view;
pub mod testkit;
pub use red_core::net::transport;
pub use red_core::net::upnp;

/// The default UDP port of a Red server (defined in the core, which [`join_code`] needs it in).
pub use red_core::net::DEFAULT_PORT;

pub use red_core::net::map_hash;
