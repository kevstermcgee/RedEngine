//! JOIN, as a library: what the text a person typed or pasted turns into (ADR 2026-10-09-relay-codes-carry-the-admission-key).
//!
//! Two kinds of code reach this engine: the short relay code `H3PQXR-K7Q2-MZ4P-WTXA` (`net::relay::ShortJoin`) and the long direct one
//! `HOST:PORT#sha256:<fingerprint>#<key>` (`net::join_code::JoinCode`). [`JoinTarget::parse`] tells them apart and
//! [`JoinTarget::client_config`] makes the [`ClientConfig`] either one needs: the address to dial, the **pinned server identity**, the **join
//! key** and, for a relayed game, the claim that locks the relay's pairing to the socket the connection will use. Every graphical or
//! command-line JOIN goes through here, so the hosting side's `LocalHost::share_codes` and this are the two halves of one contract, and
//! `tests/net_join_flow.rs` drives both. No window, no GPU: the pure parts are unit-tested below, the sockets by that test.

use super::client::{ClientConfig, ClientTransportConfig};
use super::join_code::JoinCode;
use super::quic::ServerTrust;
use super::relay::ShortJoin;
use super::relay_server::{resolve_code, ResolveError};
use std::time::Duration;

/// What was typed, understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinTarget {
    /// A relay code, with the host's admission key when the code carried one.
    Relay(ShortJoin),
    /// A direct `HOST:PORT[#fingerprint][#key]` code.
    Direct(JoinCode),
}

/// Why a code could not become a connection attempt. Each variant has one sentence for a screen ([`Display`](std::fmt::Display)) that says
/// what to do next. A *refused* join (wrong key, wrong map, full) is not here: that is the server's answer, `ConnState::Rejected`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinError {
    /// The text is neither kind of code (the message says what is wrong with it).
    BadCode(String),
    /// A relay code was given but relays are switched off on this machine (`RE2_RELAY=off`).
    RelaysOff,
    /// The relay did not resolve the code.
    Relay(ResolveError),
    /// The relay resolved the code but gave no server identity to pin. A relayed connection is never made without one: the relay's address
    /// is not the host, so there would be nothing to check the other end against.
    HostNotVerifiable,
    /// A direct code to another machine with no identity part (`#sha256:...`): refused, because nothing could be encrypted or verified.
    NoIdentity,
    /// The host name in a direct code did not resolve.
    AddressNotFound(String),
}

impl std::fmt::Display for JoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JoinError::BadCode(m) => f.write_str(m),
            JoinError::RelaysOff => {
                f.write_str("Relays are switched off on this PC (RE2_RELAY=off): ask whoever is hosting for the full join code instead.")
            }
            JoinError::Relay(e) => e.fmt(f),
            JoinError::HostNotVerifiable => {
                f.write_str("The relay could not vouch for that host, so the connection cannot be verified. Not connecting: ask your friend for a fresh code.")
            }
            JoinError::NoIdentity => f.write_str(
                "That code has no identity part, so the connection could not be encrypted or checked: ask your friend for the whole code (it is on their lobby screen).",
            ),
            JoinError::AddressNotFound(a) => write!(f, "Cannot find '{a}'. Check the code and your internet connection."),
        }
    }
}

impl std::error::Error for JoinError {}

impl JoinTarget {
    /// Understands what a person typed or pasted: a relay code if it is shaped like one, otherwise a direct code.
    pub fn parse(text: &str) -> Result<JoinTarget, JoinError> {
        if let Some(short) = ShortJoin::parse(text) {
            return Ok(JoinTarget::Relay(short));
        }
        JoinCode::parse(text).map(JoinTarget::Direct).map_err(JoinError::BadCode)
    }

    /// The join key this code carries, if any (for a screen that wants to say "this code has no key").
    pub fn key(&self) -> Option<&str> {
        match self {
            JoinTarget::Relay(s) => s.key.as_deref(),
            JoinTarget::Direct(c) => c.key.as_deref(),
        }
    }

    /// The connection settings this code stands for. `relay` is the relay to resolve a relay code against (`net::relay::relay_from_env`);
    /// a relay code resolves over the network, waiting at most `timeout` for the relay's answer, a direct code only looks up a name.
    ///
    /// The result always pins the host's identity (QUIC + TLS 1.3) unless the host is on this machine (loopback, development UDP), exactly
    /// as `ClientTransportConfig::choose` has always decided; there is no option here that turns encryption off.
    pub fn client_config(&self, relay: Option<&str>, timeout: Duration) -> Result<ClientConfig, JoinError> {
        match self {
            JoinTarget::Relay(short) => {
                let relay = relay.ok_or(JoinError::RelaysOff)?;
                let resolved = resolve_code(relay, short.code, timeout).map_err(JoinError::Relay)?;
                let fingerprint = resolved.fingerprint.clone().ok_or(JoinError::HostNotVerifiable)?;
                let mut cfg = ClientConfig::new(resolved.relay_addr, 0, 0, 0);
                cfg.join_key = short.key.clone();
                cfg.transport = ClientTransportConfig::Quic {
                    trust: ServerTrust::fingerprint(&fingerprint).map_err(|_| JoinError::HostNotVerifiable)?,
                    server_name: "localhost".to_string(),
                    relay_claim: Some(resolved.claim_bytes()),
                };
                Ok(cfg)
            }
            JoinTarget::Direct(code) => {
                let addr = std::net::ToSocketAddrs::to_socket_addrs(&code.address)
                    .ok()
                    .and_then(|mut i| i.next())
                    .ok_or_else(|| JoinError::AddressNotFound(code.address.clone()))?;
                let mut cfg = ClientConfig::new(addr, 0, 0, 0);
                cfg.join_key = code.key.clone();
                cfg.transport =
                    ClientTransportConfig::choose(addr, code.fingerprint.as_deref(), None, Some("localhost"), false).map_err(|_| JoinError::NoIdentity)?;
                Ok(cfg)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::relay::{code_to_string, generate_code, generate_join_key};

    const FP: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn a_relay_code_with_its_key_and_a_direct_code_are_told_apart() {
        let code = generate_code().unwrap();
        let key = generate_join_key().unwrap();
        let shown = ShortJoin { code, key: Some(key.clone()) }.format();
        assert_eq!(JoinTarget::parse(&shown).unwrap(), JoinTarget::Relay(ShortJoin { code, key: Some(key.clone()) }));
        assert_eq!(JoinTarget::parse(&shown).unwrap().key(), Some(key.as_str()));
        // The bare code is a relay code with no key (an open host).
        assert_eq!(JoinTarget::parse(&code_to_string(&code)).unwrap(), JoinTarget::Relay(ShortJoin { code, key: None }));
        let direct = JoinTarget::parse(&format!("203.0.113.9:27015#{FP}#{key}")).unwrap();
        assert!(matches!(&direct, JoinTarget::Direct(c) if c.fingerprint.as_deref() == Some(FP) && c.key.as_deref() == Some(key.as_str())), "{direct:?}");
        assert!(matches!(JoinTarget::parse("not a code").unwrap_err(), JoinError::BadCode(_)));
        assert!(matches!(JoinTarget::parse("").unwrap_err(), JoinError::BadCode(m) if m.contains("paste")));
    }

    #[test]
    fn a_relay_code_needs_a_relay_and_a_direct_code_to_another_machine_needs_an_identity() {
        let relay_code = JoinTarget::parse("H3PQXR-K7Q2-MZ4P-WTXA").unwrap();
        assert_eq!(relay_code.client_config(None, Duration::from_millis(50)).unwrap_err(), JoinError::RelaysOff);
        let bare = JoinTarget::parse("203.0.113.9:27015").unwrap();
        assert_eq!(bare.client_config(None, Duration::from_millis(50)).unwrap_err(), JoinError::NoIdentity, "never plaintext to another machine");
        // The loopback is this machine: development UDP stays available for local tools.
        let local = JoinTarget::parse("127.0.0.1:27015").unwrap().client_config(None, Duration::from_millis(50)).unwrap();
        assert!(matches!(local.transport, ClientTransportConfig::DevUdp));
        // With an identity it is QUIC pinned to it, and the key travels with the config.
        let pinned = JoinTarget::parse(&format!("203.0.113.9:27015#{FP}#SECRETKEY99")).unwrap().client_config(None, Duration::from_millis(50)).unwrap();
        assert!(matches!(pinned.transport, ClientTransportConfig::Quic { relay_claim: None, .. }));
        assert_eq!(pinned.join_key.as_deref(), Some("SECRETKEY99"));
    }

    #[test]
    fn every_error_says_what_to_do() {
        for e in [
            JoinError::RelaysOff,
            JoinError::HostNotVerifiable,
            JoinError::NoIdentity,
            JoinError::AddressNotFound("nowhere.invalid:1".into()),
            JoinError::Relay(ResolveError::CodeNotLive),
        ] {
            let text = e.to_string();
            assert!(text.contains(':') || text.contains("check") || text.contains("ask"), "{e:?} -> {text}");
            assert!(!text.contains("None") && !text.contains("Err("), "{text}");
        }
    }
}
