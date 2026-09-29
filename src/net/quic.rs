//! The production transport (ADR 0044): Red's datagrams as QUIC DATAGRAM frames (RFC 9221) inside a TLS 1.3 connection, from the
//! maintained `quinn` + `rustls` (ring provider) implementations. Everything cryptographic here is theirs.
//!
//! - **Server identity** is a certificate chain + private key the operator deploys ([`ServerIdentity`], `red_engine2 net-identity`
//!   makes a self-signed one). Clients verify it against a pinned SHA-256 fingerprint or a CA bundle ([`ServerTrust`]); there is no
//!   "accept any certificate" mode, and a failed verification stops the client ([`TransportStatus::Failed`]), never downgrades it.
//! - **Permission to join** stays Red's join key, proved (not sent) with HMAC-SHA256 over TLS exporter keying material, so a proof only
//!   works on the connection it was made for ([`super::auth::join_proof_bound`]). Server identity and join permission are separate.
//! - **Budgets.** A message that fits [`quinn::Connection::max_datagram_size`] goes as one unreliable datagram (snapshots are sized to
//!   fit by the server); a larger one (a full rule state, a long roster) goes on a short unidirectional stream, reliable, at most
//!   [`MAX_STREAM_MESSAGE`] bytes, at most [`MAX_STREAMS_IN_FLIGHT`] at once. Incoming datagrams wait in a bounded queue
//!   ([`INBOUND_QUEUE`]); connections are bounded ([`QuicServerOptions::max_connections`]); unvalidated addresses get a stateless Retry.
//!
//! The async runtime (tokio) lives inside this module on its own threads; the server and client stay synchronous, non-blocking loops.

use super::transport::{ClientTransport, Security, ServerTransport, TransportStats, TransportStatus};
use crate::crypto::{hex, sha256, unhex};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{Connection, Endpoint, TransportConfig, VarInt};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::collections::HashMap;
use std::io::{self, ErrorKind};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

/// ALPN protocol id: a QUIC client for anything else is refused during the TLS handshake.
pub const ALPN: &[u8] = b"red/8";
/// TLS exporter label for the join-proof channel binding (RFC 5705 / RFC 8446 section 7.5).
pub const EXPORTER_LABEL: &[u8] = b"EXPORTER-red-join-v8";
/// Incoming datagrams (plus stream messages) held for the application before new ones are dropped.
pub const INBOUND_QUEUE: usize = 1024;
/// Largest message carried on a stream (oversized for a datagram). Bigger is dropped.
pub const MAX_STREAM_MESSAGE: usize = 8 * 1024;
/// Oversized messages being written at once, per endpoint; more are dropped (they are repeated state).
pub const MAX_STREAMS_IN_FLIGHT: usize = 64;
/// How long a QUIC handshake may take.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// A connection with no traffic at all (not even keep-alives) for this long is closed by QUIC itself.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(10);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn runtime(name: &str) -> io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name(name).enable_all().build()
}

/// A deployment's server identity: a certificate chain and its private key (PEM). Never commit the key.
pub struct ServerIdentity {
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
}

impl ServerIdentity {
    /// Loads `cert` (PEM chain, leaf first) and `key` (PEM private key).
    pub fn load(cert: &Path, key: &Path) -> Result<ServerIdentity, String> {
        let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert)
            .map_err(|e| format!("{}: {e}", cert.display()))?
            .collect::<Result<_, _>>()
            .map_err(|e| format!("{}: {e}", cert.display()))?;
        if chain.is_empty() {
            return Err(format!("{}: no certificate in the file", cert.display()));
        }
        let key = PrivateKeyDer::from_pem_file(key).map_err(|e| format!("{}: {e}", key.display()))?;
        Ok(ServerIdentity { chain, key })
    }

    /// A fresh self-signed identity for `names` (DNS names or IP addresses the clients will use), with its PEM encodings to save.
    pub fn generate(names: &[String]) -> Result<GeneratedIdentity, String> {
        let names = if names.is_empty() { vec!["localhost".to_string()] } else { names.to_vec() };
        let ck = rcgen::generate_simple_self_signed(names).map_err(|e| format!("cannot make a certificate: {e}"))?;
        let key = PrivateKeyDer::try_from(ck.signing_key.serialize_der()).map_err(|e| format!("cannot encode the key: {e}"))?;
        Ok(GeneratedIdentity {
            cert_pem: ck.cert.pem(),
            key_pem: ck.signing_key.serialize_pem(),
            identity: ServerIdentity { chain: vec![ck.cert.der().clone()], key },
        })
    }

    /// `sha256:<hex>` of the leaf certificate: what clients pin (`--server-fingerprint`).
    pub fn fingerprint(&self) -> String {
        fingerprint_of(self.chain[0].as_ref())
    }

    fn clone_key(&self) -> PrivateKeyDer<'static> {
        self.key.clone_key()
    }
}

/// A newly generated identity and the PEM text to store it in (`net-identity`). Write the key with owner-only permissions and never
/// commit it.
pub struct GeneratedIdentity {
    /// The usable identity.
    pub identity: ServerIdentity,
    /// The certificate, PEM.
    pub cert_pem: String,
    /// The private key, PEM (PKCS#8).
    pub key_pem: String,
}

/// `sha256:<hex>` of a DER certificate.
pub fn fingerprint_of(der: &[u8]) -> String {
    format!("sha256:{}", hex(&sha256(der)))
}

/// How a client decides the server is the one it meant. There is deliberately no "trust anything" variant.
#[derive(Clone)]
pub enum ServerTrust {
    /// The server's leaf certificate must have exactly this SHA-256 (`sha256:<64 hex>`, as `red_server` prints it). The TLS handshake
    /// signature is still verified against that certificate.
    Fingerprint([u8; 32]),
    /// The chain must verify to one of these roots for `server_name` (a CA-issued or private-CA certificate).
    Roots(Arc<rustls::RootCertStore>),
}

impl ServerTrust {
    /// Parses `sha256:<hex>` (the prefix is optional; colons are ignored).
    pub fn fingerprint(s: &str) -> Result<ServerTrust, String> {
        let h = s.trim().trim_start_matches("sha256:").replace(':', "");
        let bytes = unhex(&h).filter(|b| b.len() == 32).ok_or_else(|| format!("'{s}' is not a SHA-256 fingerprint (sha256:<64 hex digits>)"))?;
        let mut f = [0u8; 32];
        f.copy_from_slice(&bytes);
        Ok(ServerTrust::Fingerprint(f))
    }

    /// Loads CA roots from a PEM file.
    pub fn roots_file(path: &Path) -> Result<ServerTrust, String> {
        let mut store = rustls::RootCertStore::empty();
        for c in CertificateDer::pem_file_iter(path).map_err(|e| format!("{}: {e}", path.display()))? {
            store.add(c.map_err(|e| format!("{}: {e}", path.display()))?).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        if store.is_empty() {
            return Err(format!("{}: no CA certificate in the file", path.display()));
        }
        Ok(ServerTrust::Roots(Arc::new(store)))
    }
}

/// Pins one leaf certificate by hash but still checks the TLS 1.3 handshake signature with it (so the peer must hold its key).
#[derive(Debug)]
struct PinnedVerifier {
    pin: [u8; 32],
    algs: rustls::crypto::WebPkiSupportedAlgorithms,
    failure: Arc<Mutex<Option<String>>>,
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if crate::crypto::ct_eq(&sha256(end_entity.as_ref()), &self.pin) {
            Ok(ServerCertVerified::assertion())
        } else {
            let got = fingerprint_of(end_entity.as_ref());
            *lock(&self.failure) = Some(format!("server identity mismatch: the server presented {got}, expected sha256:{}", hex(&self.pin)));
            Err(rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure))
        }
    }

    fn verify_tls12_signature(&self, _: &[u8], _: &CertificateDer<'_>, _: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(rustls::PeerIncompatible::Tls12NotOffered))
        // QUIC is TLS 1.3 only
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

/// Wraps the WebPKI verifier to remember why a chain was refused (so the client can report it and stop).
#[derive(Debug)]
struct RecordingVerifier {
    inner: Arc<rustls::client::WebPkiServerVerifier>,
    failure: Arc<Mutex<Option<String>>>,
}

impl ServerCertVerifier for RecordingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.inner.verify_server_cert(end_entity, intermediates, server_name, ocsp, now).inspect_err(|e| {
            *lock(&self.failure) = Some(format!("server identity rejected: {e}"));
        })
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(m, c, d)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(m, c, d)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

fn transport_config(accept_streams: bool) -> TransportConfig {
    let mut t = TransportConfig::default();
    t.max_idle_timeout(IDLE_TIMEOUT.try_into().ok());
    t.keep_alive_interval(Some(Duration::from_secs(2)));
    t.datagram_receive_buffer_size(Some(256 * 1024));
    t.datagram_send_buffer_size(256 * 1024);
    t.max_concurrent_bidi_streams(VarInt::from_u32(0));
    t.max_concurrent_uni_streams(VarInt::from_u32(if accept_streams { 32 } else { 0 }));
    // QUIC datagrams are congestion-controlled (RFC 9221). Loss-based CUBIC collapses its window under the bursty random loss a game
    // link sees and then drops snapshots at the sender; BBR models bandwidth and delay instead (measured in ADR 0044, `net-test`).
    t.congestion_controller_factory(Arc::new(quinn::congestion::BbrConfig::default()));
    // Start at QUIC's guaranteed 1200-byte path (RFC 9000 section 14) so the first packets cross any internet path; MTU discovery
    // raises it later. The datagram budget the server sizes snapshots to therefore starts near 1150 bytes and grows.
    t.initial_mtu(1200);
    t
}

/// What arrives from the async side.
enum Inbound {
    Message(SocketAddr, bytes::Bytes),
}

/// Server-side limits.
#[derive(Debug, Clone, Copy)]
pub struct QuicServerOptions {
    /// Connections (established plus handshaking) accepted at once; more are refused.
    pub max_connections: usize,
}

impl Default for QuicServerOptions {
    fn default() -> Self {
        // Eight players, room for the same number reconnecting or being refused a slot by Red itself.
        QuicServerOptions { max_connections: 16 }
    }
}

struct ServerShared {
    conns: Mutex<HashMap<SocketAddr, Connection>>,
    /// Peers whose connection ended, for [`ServerTransport::take_closed`] (bounded by the connection limit churn).
    closed: Mutex<Vec<SocketAddr>>,
    stats: Mutex<TransportStats>,
    handshaking: AtomicUsize,
    streams_in_flight: AtomicUsize,
}

/// The production server transport. See the module docs.
pub struct QuicServer {
    rt: Option<tokio::runtime::Runtime>,
    endpoint: Endpoint,
    shared: Arc<ServerShared>,
    rx: mpsc::Receiver<Inbound>,
    fingerprint: String,
}

impl QuicServer {
    /// Listens on `bind` with `identity`.
    pub fn bind(bind: SocketAddr, identity: &ServerIdentity, opts: QuicServerOptions) -> io::Result<QuicServer> {
        let bad = |e: String| io::Error::new(ErrorKind::InvalidInput, e);
        let mut tls = rustls::ServerConfig::builder_with_provider(provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| bad(e.to_string()))?
            .with_no_client_auth()
            .with_single_cert(identity.chain.clone(), identity.clone_key())
            .map_err(|e| bad(format!("the server certificate and key do not work together: {e}")))?;
        tls.alpn_protocols = vec![ALPN.to_vec()];
        tls.max_early_data_size = 0; // no 0-RTT: a replayed early Hello must not be possible
        let crypto = QuicServerConfig::try_from(tls).map_err(|e| bad(e.to_string()))?;
        let mut sc = quinn::ServerConfig::with_crypto(Arc::new(crypto));
        sc.transport_config(Arc::new(transport_config(false)));
        let rt = runtime("red-quic-server")?;
        let endpoint = {
            let _g = rt.enter();
            Endpoint::server(sc, bind)?
        };
        let shared = Arc::new(ServerShared {
            conns: Mutex::new(HashMap::new()),
            closed: Mutex::new(Vec::new()),
            stats: Mutex::new(TransportStats::default()),
            handshaking: AtomicUsize::new(0),
            streams_in_flight: AtomicUsize::new(0),
        });
        let (tx, rx) = mpsc::channel(INBOUND_QUEUE);
        rt.spawn(accept_loop(endpoint.clone(), shared.clone(), tx, opts.max_connections.max(1)));
        Ok(QuicServer { rt: Some(rt), endpoint, shared, rx, fingerprint: identity.fingerprint() })
    }

    /// The identity fingerprint clients pin.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Connections currently established.
    pub fn connection_count(&self) -> usize {
        lock(&self.shared.conns).len()
    }
}

async fn accept_loop(endpoint: Endpoint, shared: Arc<ServerShared>, tx: mpsc::Sender<Inbound>, max: usize) {
    while let Some(incoming) = endpoint.accept().await {
        let busy = lock(&shared.conns).len() + shared.handshaking.load(Ordering::Relaxed);
        if busy >= max {
            lock(&shared.stats).connections_refused += 1;
            incoming.refuse();
            continue;
        }
        // Stateless address validation: an address that has not echoed a Retry token costs no connection state (anti-spoofing,
        // anti-amplification: QUIC's own version of Red's address cookie).
        if !incoming.remote_address_validated() {
            let _ = incoming.retry();
            continue;
        }
        shared.handshaking.fetch_add(1, Ordering::Relaxed);
        let (shared, tx) = (shared.clone(), tx.clone());
        tokio::spawn(async move {
            let result = match incoming.accept() {
                Ok(connecting) => tokio::time::timeout(HANDSHAKE_TIMEOUT, connecting).await.ok().and_then(Result::ok),
                Err(_) => None,
            };
            shared.handshaking.fetch_sub(1, Ordering::Relaxed);
            let Some(conn) = result else {
                lock(&shared.stats).handshakes_failed += 1;
                return;
            };
            let key = conn.remote_address();
            if let Some(old) = lock(&shared.conns).insert(key, conn.clone()) {
                old.close(VarInt::from_u32(1), b"replaced by a new connection from the same address");
            }
            lock(&shared.stats).connections_accepted += 1;
            read_connection(conn.clone(), key, tx, shared.clone()).await;
            let mut conns = lock(&shared.conns);
            if conns.get(&key).is_some_and(|c| c.stable_id() == conn.stable_id()) {
                conns.remove(&key);
                let mut closed = lock(&shared.closed);
                if closed.len() < 1024 {
                    closed.push(key);
                }
            }
        });
    }
}

/// Moves one connection's datagrams (and, for a client, oversized stream messages) into the bounded queue until it closes.
async fn read_connection(conn: Connection, key: SocketAddr, tx: mpsc::Sender<Inbound>, shared: Arc<ServerShared>) {
    let dropped = |shared: &ServerShared| lock(&shared.stats).queue_dropped += 1;
    loop {
        tokio::select! {
            d = conn.read_datagram() => match d {
                Ok(bytes) => {
                    if tx.try_send(Inbound::Message(key, bytes)).is_err() {
                        dropped(&shared);
                    }
                }
                Err(_) => return,
            },
            s = conn.accept_uni() => match s {
                Ok(mut recv) => {
                    let (tx, shared) = (tx.clone(), shared.clone());
                    tokio::spawn(async move {
                        if let Ok(Ok(data)) = tokio::time::timeout(Duration::from_secs(5), recv.read_to_end(MAX_STREAM_MESSAGE)).await {
                            if tx.try_send(Inbound::Message(key, data.into())).is_err() {
                                dropped(&shared);
                            }
                        }
                    });
                }
                Err(_) => return,
            },
        }
    }
}

/// Sends `bytes` on `conn`: one datagram when it fits, otherwise one short unidirectional stream (bounded by `in_flight`).
fn send_on(rt: &tokio::runtime::Runtime, conn: &Connection, bytes: &[u8], in_flight: &Arc<ServerShared>) -> io::Result<usize> {
    let _in_runtime = rt.enter();
    let max = conn.max_datagram_size().unwrap_or(0);
    if bytes.len() <= max {
        return conn.send_datagram(bytes::Bytes::copy_from_slice(bytes)).map(|()| bytes.len()).map_err(|e| io::Error::other(e.to_string()));
    }
    if bytes.len() > MAX_STREAM_MESSAGE || in_flight.streams_in_flight.load(Ordering::Relaxed) >= MAX_STREAMS_IN_FLIGHT {
        lock(&in_flight.stats).send_dropped += 1;
        return Err(io::Error::new(ErrorKind::WouldBlock, "oversized message dropped: stream budget spent"));
    }
    in_flight.streams_in_flight.fetch_add(1, Ordering::Relaxed);
    lock(&in_flight.stats).sent_on_stream += 1;
    let (conn, data, shared) = (conn.clone(), bytes.to_vec(), in_flight.clone());
    rt.spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            if let Ok(mut s) = conn.open_uni().await {
                if s.write_all(&data).await.is_ok() {
                    let _ = s.finish();
                }
            }
        })
        .await;
        shared.streams_in_flight.fetch_sub(1, Ordering::Relaxed);
    });
    Ok(bytes.len())
}

fn binding(conn: &Connection) -> Option<[u8; 32]> {
    let mut out = [0u8; 32];
    conn.export_keying_material(&mut out, EXPORTER_LABEL, b"").ok().map(|()| out)
}

impl ServerTransport for QuicServer {
    fn security(&self) -> Security {
        Security::Quic
    }
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.endpoint.local_addr()
    }
    fn recv(&mut self, buf: &mut [u8]) -> Option<(SocketAddr, usize)> {
        loop {
            match self.rx.try_recv() {
                Ok(Inbound::Message(peer, bytes)) => {
                    if bytes.len() > buf.len() {
                        continue; // larger than the application accepts: drop
                    }
                    buf[..bytes.len()].copy_from_slice(&bytes);
                    return Some((peer, bytes.len()));
                }
                Err(_) => return None,
            }
        }
    }
    fn send(&mut self, peer: SocketAddr, bytes: &[u8]) -> io::Result<usize> {
        let Some(conn) = lock(&self.shared.conns).get(&peer).cloned() else {
            return Err(io::Error::new(ErrorKind::NotConnected, "no QUIC connection from that address"));
        };
        let Some(rt) = self.rt.as_ref() else { return Err(io::Error::new(ErrorKind::NotConnected, "shut down")) };
        send_on(rt, &conn, bytes, &self.shared)
    }
    fn max_datagram(&self, peer: SocketAddr) -> usize {
        lock(&self.shared.conns).get(&peer).and_then(Connection::max_datagram_size).unwrap_or(0)
    }
    fn channel_binding(&self, peer: SocketAddr) -> Option<[u8; 32]> {
        lock(&self.shared.conns).get(&peer).and_then(binding)
    }
    fn close_peer(&mut self, peer: SocketAddr, reason: &str) {
        if let Some(c) = lock(&self.shared.conns).remove(&peer) {
            c.close(VarInt::from_u32(0), reason.as_bytes());
        }
    }
    fn shutdown(&mut self) {
        if self.rt.is_none() {
            return;
        }
        // Give the goodbye datagrams just queued a moment to leave: closing the endpoint discards unsent datagrams. The
        // CONNECTION_CLOSE that follows tells every client even if they were lost.
        std::thread::sleep(Duration::from_millis(30));
        self.endpoint.close(VarInt::from_u32(0), b"server shutting down");
        if let Some(rt) = self.rt.take() {
            let ep = self.endpoint.clone();
            rt.block_on(async move {
                let _ = tokio::time::timeout(Duration::from_millis(500), ep.wait_idle()).await;
            });
            rt.shutdown_timeout(Duration::from_millis(200));
        }
    }
    fn stats(&self) -> TransportStats {
        lock(&self.shared.stats).clone()
    }
    fn take_closed(&mut self) -> Vec<SocketAddr> {
        std::mem::take(&mut *lock(&self.shared.closed))
    }
}

impl Drop for QuicServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The production client transport. See the module docs.
pub struct QuicClient {
    rt: Option<tokio::runtime::Runtime>,
    endpoint: Endpoint,
    config: quinn::ClientConfig,
    server: SocketAddr,
    server_name: String,
    conn: Arc<Mutex<Option<Connection>>>,
    status: Arc<Mutex<TransportStatus>>,
    identity_failure: Arc<Mutex<Option<String>>>,
    /// A handshake task is running (so `reconnect` does not start a second one).
    handshaking: Arc<std::sync::atomic::AtomicBool>,
    shared: Arc<ServerShared>,
    rx: mpsc::Receiver<Inbound>,
    tx: mpsc::Sender<Inbound>,
}

impl QuicClient {
    /// Starts connecting to `server`, verifying it with `trust` for `server_name` (the name in the certificate; for a pinned
    /// fingerprint any name works).
    pub fn connect(server: SocketAddr, server_name: &str, trust: ServerTrust) -> io::Result<QuicClient> {
        let failure = Arc::new(Mutex::new(None));
        let builder = rustls::ClientConfig::builder_with_provider(provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| io::Error::other(e.to_string()))?;
        let verifier: Arc<dyn ServerCertVerifier> = match trust {
            ServerTrust::Fingerprint(pin) => Arc::new(PinnedVerifier { pin, algs: provider().signature_verification_algorithms, failure: failure.clone() }),
            ServerTrust::Roots(roots) => {
                let inner =
                    rustls::client::WebPkiServerVerifier::builder_with_provider(roots, provider()).build().map_err(|e| io::Error::other(e.to_string()))?;
                Arc::new(RecordingVerifier { inner, failure: failure.clone() })
            }
        };
        let mut tls = builder.dangerous().with_custom_certificate_verifier(verifier).with_no_client_auth();
        tls.alpn_protocols = vec![ALPN.to_vec()];
        tls.enable_early_data = false;
        let crypto = QuicClientConfig::try_from(tls).map_err(|e| io::Error::other(e.to_string()))?;
        let mut config = quinn::ClientConfig::new(Arc::new(crypto));
        config.transport_config(Arc::new(transport_config(true)));
        let rt = runtime("red-quic-client")?;
        let endpoint = {
            let _g = rt.enter();
            Endpoint::client(super::client::local_bind_for(server))?
        };
        let (tx, rx) = mpsc::channel(INBOUND_QUEUE);
        let mut c = QuicClient {
            rt: Some(rt),
            endpoint,
            config,
            server,
            server_name: if server_name.is_empty() { "localhost".to_string() } else { server_name.to_string() },
            conn: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new(TransportStatus::Connecting)),
            identity_failure: failure,
            handshaking: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            shared: Arc::new(ServerShared {
                conns: Mutex::new(HashMap::new()),
                closed: Mutex::new(Vec::new()),
                stats: Mutex::new(TransportStats::default()),
                handshaking: AtomicUsize::new(0),
                streams_in_flight: AtomicUsize::new(0),
            }),
            rx,
            tx,
        };
        c.start();
        Ok(c)
    }

    fn start(&mut self) {
        let Some(rt) = self.rt.as_ref() else { return };
        if self.handshaking.swap(true, Ordering::AcqRel) {
            return; // one handshake at a time
        }
        *lock(&self.status) = TransportStatus::Connecting;
        let _in_runtime = rt.enter(); // quinn spawns the connection driver on the current runtime
        let connecting = self.endpoint.connect_with(self.config.clone(), self.server, &self.server_name);
        let (conn_slot, status, failure, tx, shared, server, handshaking) = (
            self.conn.clone(),
            self.status.clone(),
            self.identity_failure.clone(),
            self.tx.clone(),
            self.shared.clone(),
            self.server,
            self.handshaking.clone(),
        );
        rt.spawn(async move {
            let result = match connecting {
                Ok(c) => match tokio::time::timeout(HANDSHAKE_TIMEOUT, c).await {
                    Ok(Ok(conn)) => Ok(conn),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err("QUIC handshake timed out".to_string()),
                },
                Err(e) => Err(e.to_string()),
            };
            handshaking.store(false, Ordering::Release);
            match result {
                Ok(conn) => {
                    *lock(&conn_slot) = Some(conn.clone());
                    *lock(&status) = TransportStatus::Ready;
                    read_connection(conn, server, tx, shared).await;
                    let mut st = lock(&status);
                    if *st == TransportStatus::Ready {
                        *st = TransportStatus::Connecting; // the connection closed: the client's reconnect starts a new one
                    }
                }
                Err(e) => {
                    // A refused identity is final (fail closed). Anything else (timeout, refused because full) may be retried.
                    let identity = lock(&failure).clone();
                    *lock(&status) = match identity {
                        Some(why) => TransportStatus::Failed(why),
                        None => TransportStatus::Failed(format!("transient: {e}")),
                    };
                }
            }
        });
    }

    fn connection(&self) -> Option<Connection> {
        lock(&self.conn).clone().filter(|c| c.close_reason().is_none())
    }
}

impl ClientTransport for QuicClient {
    fn security(&self) -> Security {
        Security::Quic
    }
    fn send(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let (Some(conn), Some(rt)) = (self.connection(), self.rt.as_ref()) else {
            return Err(io::Error::new(ErrorKind::NotConnected, "QUIC connection not established"));
        };
        send_on(rt, &conn, bytes, &self.shared)
    }
    fn recv(&mut self, buf: &mut [u8]) -> Option<usize> {
        loop {
            match self.rx.try_recv() {
                Ok(Inbound::Message(_, bytes)) if bytes.len() <= buf.len() => {
                    buf[..bytes.len()].copy_from_slice(&bytes);
                    return Some(bytes.len());
                }
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    }
    fn max_datagram(&self) -> usize {
        self.connection().and_then(|c| c.max_datagram_size()).unwrap_or(0)
    }
    fn channel_binding(&self) -> Option<[u8; 32]> {
        self.connection().as_ref().and_then(binding)
    }
    fn status(&self) -> TransportStatus {
        match lock(&self.status).clone() {
            // A transient failure reads as "still connecting": `reconnect` retries it. Identity failures stay failed.
            TransportStatus::Failed(why) if why.starts_with("transient: ") => TransportStatus::Connecting,
            s => s,
        }
    }
    fn reconnect(&mut self) {
        if matches!(self.status(), TransportStatus::Failed(_)) {
            return; // fail closed: a server that failed identity verification is never retried
        }
        // Whatever is left of the old connection goes (the server went silent: it may have restarted and lost its state), and a new
        // handshake starts unless one is already running.
        if let Some(c) = lock(&self.conn).take() {
            c.close(VarInt::from_u32(0), b"reconnecting");
        }
        self.start();
    }
    fn close(&mut self) {
        if let Some(c) = lock(&self.conn).take() {
            c.close(VarInt::from_u32(0), b"bye");
        }
        if let Some(rt) = self.rt.take() {
            let ep = self.endpoint.clone();
            rt.block_on(async move {
                let _ = tokio::time::timeout(Duration::from_millis(300), ep.wait_idle()).await;
            });
            rt.shutdown_timeout(Duration::from_millis(100));
        }
        *lock(&self.status) = TransportStatus::Failed("closed".to_string());
    }
}

impl Drop for QuicClient {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn wait<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
        let t0 = Instant::now();
        loop {
            if let Some(v) = f() {
                return v;
            }
            assert!(t0.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn fingerprints_parse_and_pem_round_trips() {
        let gen = ServerIdentity::generate(&["localhost".into()]).unwrap();
        let fp = gen.identity.fingerprint();
        assert!(fp.starts_with("sha256:") && fp.len() == 7 + 64);
        assert!(matches!(ServerTrust::fingerprint(&fp), Ok(ServerTrust::Fingerprint(_))));
        assert!(ServerTrust::fingerprint("sha256:1234").is_err());
        let dir = std::env::temp_dir().join(format!("red-quic-{}", crate::crypto::random_u64().unwrap()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("c.pem"), &gen.cert_pem).unwrap();
        std::fs::write(dir.join("k.pem"), &gen.key_pem).unwrap();
        let back = ServerIdentity::load(&dir.join("c.pem"), &dir.join("k.pem")).unwrap();
        assert_eq!(back.fingerprint(), fp);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_pinned_client_exchanges_datagrams_and_oversized_messages_and_shares_the_channel_binding() {
        let id = ServerIdentity::generate(&["localhost".into()]).unwrap().identity;
        let mut server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), &id, QuicServerOptions::default()).unwrap();
        let trust = ServerTrust::fingerprint(&id.fingerprint()).unwrap();
        let mut client = QuicClient::connect(server.local_addr().unwrap(), "localhost", trust).unwrap();
        wait("the handshake", || (client.status() == TransportStatus::Ready).then_some(()));
        assert!(client.security().is_secure());
        client.send(b"hello over quic").unwrap();
        let mut buf = [0u8; 16 * 1024];
        let (peer, n) = wait("a datagram at the server", || server.recv(&mut buf));
        assert_eq!(&buf[..n], b"hello over quic");
        assert_eq!(server.channel_binding(peer), client.channel_binding(), "both ends export the same keying material");
        assert!(server.channel_binding(peer).is_some());
        let limit = server.max_datagram(peer);
        eprintln!("MEASURE quic max_datagram_size on a fresh loopback path: {limit} bytes");
        assert!(limit >= 1100, "QUIC datagram budget {limit}");
        let big = vec![7u8; limit + 500];
        server.send(peer, &big).unwrap();
        let n = wait("the oversized message at the client", || client.recv(&mut buf));
        assert_eq!(&buf[..n], &big[..]);
        assert_eq!(server.stats().sent_on_stream, 1);
    }

    #[test]
    fn a_wrong_pin_fails_closed_and_is_never_retried() {
        let id = ServerIdentity::generate(&["localhost".into()]).unwrap().identity;
        let other = ServerIdentity::generate(&["localhost".into()]).unwrap().identity;
        let server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), &id, QuicServerOptions::default()).unwrap();
        let mut client = QuicClient::connect(server.local_addr().unwrap(), "localhost", ServerTrust::fingerprint(&other.fingerprint()).unwrap()).unwrap();
        let why = wait("the identity failure", || match client.status() {
            TransportStatus::Failed(w) => Some(w),
            _ => None,
        });
        assert!(why.contains("identity mismatch"), "{why}");
        client.reconnect();
        assert!(matches!(client.status(), TransportStatus::Failed(_)), "a failed identity is not retried");
        assert!(client.send(b"x").is_err());
    }
}
