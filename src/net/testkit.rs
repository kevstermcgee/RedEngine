//! A bare-socket client that speaks the real handshake, for tests and tools that need to drive a server *below* [`super::client::NetClient`]:
//! send a hand-made packet, send garbage, send a datagram with a wrong tag, watch exactly what comes back. It performs the challenge, the
//! join-key proof and the tagging (ADR 0028) and nothing else: no prediction, no interpolation, no timers.
//!
//! It is used by `tests/net_abuse.rs`, `tests/net_auth.rs`, `tests/net_flow.rs`, `tests/net_budget.rs` and the `net-test` command.

use super::auth::{join_proof, Direction, SessionKey, PROOF_LEN};
use super::protocol::*;
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};

/// A client that owns a socket and a handshake, and nothing more.
pub struct RawClient {
    /// Its socket (public so a test can send raw bytes from this exact source address).
    pub sock: UdpSocket,
    server: SocketAddr,
    map_hash: u32,
    join_key: Option<String>,
    nonce: u64,
    cookie: u64,
    key: Option<SessionKey>,
    /// The token to resume with (`0` = a fresh join).
    pub resume_token: u64,
    /// `0` human, `1` rat.
    pub character: u8,
    /// The name it joins with.
    pub name: String,
    /// The `Welcome` once it arrives.
    pub welcome: Option<Welcome>,
    /// Whether the server said it needs a join key.
    pub requires_key: bool,
    /// The size of the biggest datagram received (what a bandwidth or MTU budget is checked against).
    pub largest_datagram: usize,
}

impl RawClient {
    /// A client for `server` that has not sent anything yet. `nonce` distinguishes handshakes (any distinct non-zero value).
    pub fn new(server: SocketAddr, map_hash: u32, join_key: Option<&str>, nonce: u64) -> std::io::Result<RawClient> {
        let sock = UdpSocket::bind("127.0.0.1:0")?;
        sock.set_nonblocking(true)?;
        Ok(RawClient {
            sock,
            server,
            map_hash,
            join_key: join_key.map(str::to_string),
            nonce: nonce.max(1),
            cookie: 0,
            key: None,
            resume_token: 0,
            character: 0,
            name: String::new(),
            welcome: None,
            requires_key: false,
            largest_datagram: 0,
        })
    }

    /// The bytes of the `Hello` this client would send now (the first one has no cookie; after a `Challenge` it carries the proof).
    pub fn hello_bytes(&self) -> Vec<u8> {
        let proof = match (&self.join_key, self.cookie) {
            (Some(k), c) if c != 0 => join_proof(k.as_bytes(), self.nonce, c, self.map_hash, PROTOCOL_VERSION),
            _ => [0; PROOF_LEN],
        };
        let h = Hello {
            version: PROTOCOL_VERSION,
            map_hash: self.map_hash,
            character: self.character,
            resume_token: self.resume_token,
            client_nonce: self.nonce,
            cookie: self.cookie,
            proof,
            name: self.name.clone(),
        };
        let mut b = Vec::new();
        ClientMsg::Hello(h).encode(&mut b);
        b
    }

    /// Sends the current `Hello`.
    pub fn send_hello(&self) {
        let _ = self.sock.send_to(&self.hello_bytes(), self.server);
    }

    /// Sends bytes exactly as given.
    pub fn send_raw(&self, bytes: &[u8]) {
        let _ = self.sock.send_to(bytes, self.server);
    }

    /// Encodes and sends a message, tagged if it is a kind that carries a tag (a no-op for those until the handshake gave us a key).
    pub fn send(&self, msg: &ClientMsg) {
        let _ = self.sock.send_to(&self.signed_bytes(msg), self.server);
    }

    /// The datagram for `msg`: tagged when the kind needs one and we have the key.
    pub fn signed_bytes(&self, msg: &ClientMsg) -> Vec<u8> {
        let mut b = Vec::new();
        msg.encode(&mut b);
        if let (Some(k), Some(kind)) = (&self.key, peek_kind(&b)) {
            if is_signed(kind) {
                k.sign(Direction::ToServer, &mut b);
            }
        }
        b
    }

    /// Reads everything waiting. A `Challenge` is answered at once with the second `Hello`; a tagged message is verified (and dropped if
    /// the tag is wrong); a `Welcome` is remembered. Returns what was received, in order.
    pub fn poll(&mut self) -> Vec<ServerMsg> {
        let mut out = Vec::new();
        let mut buf = [0u8; 2048];
        loop {
            match self.sock.recv_from(&mut buf) {
                Ok((n, _)) => {
                    self.largest_datagram = self.largest_datagram.max(n);
                    let bytes = &buf[..n];
                    let Some(kind) = peek_kind(bytes) else { continue };
                    let msg = if is_signed(kind) {
                        let Some(body) = self.key.as_ref().and_then(|k| k.verify(Direction::ToClient, bytes)) else { continue };
                        ServerMsg::decode(body)
                    } else {
                        ServerMsg::decode(bytes)
                    };
                    let Ok(msg) = msg else { continue };
                    match &msg {
                        ServerMsg::Challenge { cookie, requires_key } => {
                            self.requires_key = *requires_key;
                            self.cookie = *cookie;
                            self.key = Some(SessionKey::derive(self.join_key.as_deref().unwrap_or("").as_bytes(), self.nonce, *cookie));
                            self.send_hello();
                        }
                        ServerMsg::Welcome(w) => self.welcome = Some(*w),
                        _ => {}
                    }
                    out.push(msg);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::ConnectionReset => continue,
                Err(_) => break,
            }
        }
        out
    }

    /// Runs the whole handshake, calling `pump` whenever the server needs a turn (an in-process server: `|| server.pump(now)`; a remote
    /// one: a short sleep). `Err` carries the rejection or a timeout.
    pub fn handshake(&mut self, mut pump: impl FnMut()) -> Result<Welcome, String> {
        self.send_hello();
        for _ in 0..8 {
            pump();
            for m in self.poll() {
                match m {
                    ServerMsg::Welcome(w) => return Ok(w),
                    ServerMsg::Reject(r) => return Err(format!("rejected: {r:?}")),
                    _ => {}
                }
            }
        }
        Err("no Welcome (handshake timed out)".to_string())
    }
}
