//! Real-time multiplayer over UDP: an authoritative headless server ([`server`]) and a client library
//! ([`client`]) that the graphical game and the headless bot both use.
//!
//! - [`protocol`]: the wire format (fuzz-tested, bounded).
//! - [`server`]: `Server` = the socket and the tick loop around [`crate::sim::match_sim::MatchSim`]: a fixed 60 Hz
//!   tick, 30 Hz snapshots with per-client delta acknowledgement, timeouts, reconnect-with-token. Its parts:
//!   `sessions` (who is connected, resume tokens), `snapshots` (what each client is sent), [`limits`] (packet budgets).
//! - [`client`]: `NetClient` = handshake, redundant input sending, snapshot receiving, reconnect.
//! - [`interp`]: smooth rendering of remote players and props from snapshots.
//! - [`bot`]: a headless scripted client (how multiplayer is proved without a window).
//! - [`session`]: the graphical client's glue (prediction + scene updates), window-free and tested.
//! - [`predict`]: client-side prediction and reconciliation of the local player.
//!
//! Design and limits: ADR 0016. Nothing here touches a window, GPU or audio device.

// Nothing reachable from a UDP packet may panic the process. Test code is exempt; a genuine invariant is written
// as a `let ... else` / `?` with a message, not an `unwrap`.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::todo, clippy::unimplemented, clippy::unreachable))]

pub mod auth;
pub mod bot;
pub mod client;
pub mod interp;
pub mod limits;
pub mod netsim;
pub mod predict;
pub mod protocol;
pub mod server;
pub mod session;
mod sessions;
mod snapshots;
pub mod testkit;
pub mod upnp;

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
