//! UPnP port mapping (ADR 0031): open the game's UDP port on a home router so friends can join a server running on a home PC, and close it
//! again. Standard library only; the same code serves `red_engine2 portmap` and `red_server --upnp`.
//!
//! **Discovery, not guessing.** The router is found with SSDP (`M-SEARCH` to 239.255.255.250:1900 for an Internet Gateway Device), its
//! description document is fetched from the `LOCATION` it answered with, and the `controlURL` of its `WANIPConnection` (or
//! `WANPPPConnection`) service is read from that document: routers put the control endpoint at different ports and paths, so nothing is
//! hard-coded. A gateway address can also be given (`--router`) and is then asked for its description directly.
//!
//! **Safe by construction.** A mapping is only ever created for *this machine's* address, always with a lease (a router that insists on a
//! permanent one is refused unless the caller opts in, because a permanent hole is forgotten and left open), always described as ours, and a
//! mapping that exists but is not ours (a different description or a different internal host) is never overwritten and never deleted.
//!
//! What it cannot do: reach a router that has UPnP switched off, a carrier-grade NAT (the router's "external" address is private), or a
//! network with no router. `status` says which.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

/// The multicast address SSDP searches go to.
pub const SSDP_ADDR: &str = "239.255.255.250:1900";
/// The description every mapping made here carries, so it can be recognised (and only its own removed) later.
pub const DESCRIPTION: &str = "Red Engine game server";
/// Default lease, seconds. A router forgets an unrenewed mapping after this.
pub const DEFAULT_LEASE_SECS: u32 = 3600;

const MAX_RESPONSE: usize = 256 * 1024;

/// A device description's service that can map ports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    /// Where SOAP requests go (`host:port`).
    pub host: String,
    /// The path of the control endpoint.
    pub path: String,
    /// The service type to name in requests (`urn:schemas-upnp-org:service:WANIPConnection:1`).
    pub service: String,
}

/// A mapping as the router reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    /// The LAN host traffic is sent to.
    pub internal_client: String,
    /// The port on that host.
    pub internal_port: u16,
    /// The description it was made with.
    pub description: String,
    /// Seconds left on the lease (`0` = permanent).
    pub lease_secs: u32,
    /// Whether the router has it switched on.
    pub enabled: bool,
}

/// A router's answer, or why there was none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpnpError {
    /// No router answered discovery.
    NoRouter(String),
    /// A network or HTTP problem.
    Io(String),
    /// The router's documents were not what UPnP promises.
    Protocol(String),
    /// The router refused with a UPnP error code and text.
    Refused(u32, String),
    /// We declined to act (someone else's mapping; a permanent lease).
    Declined(String),
}

impl std::fmt::Display for UpnpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpnpError::NoRouter(s) | UpnpError::Io(s) | UpnpError::Protocol(s) | UpnpError::Declined(s) => f.write_str(s),
            UpnpError::Refused(code, text) => write!(f, "the router refused (UPnP error {code}: {})", explain_code(*code, text)),
        }
    }
}

impl std::error::Error for UpnpError {}

fn io_err(what: &str, e: impl std::fmt::Display) -> UpnpError {
    UpnpError::Io(format!("{what}: {e}"))
}

/// What a UPnP error code means for a person with a router.
pub fn explain_code(code: u32, router_text: &str) -> String {
    let known = match code {
        714 => "there is no such mapping",
        718 => "that external port is already mapped to another device",
        724 => "this router needs the external and internal port to be the same",
        725 => "this router only accepts permanent mappings",
        726 | 727 => "this router only accepts wildcard remote hosts / external ports",
        402 | 501 => "the router rejected the request",
        606 => "the router requires authorisation for this action",
        _ => "",
    };
    match (known.is_empty(), router_text.is_empty()) {
        (true, true) => "no details".to_string(),
        (true, false) => router_text.to_string(),
        (false, true) => known.to_string(),
        (false, false) => format!("{known}; the router says '{router_text}'"),
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------
// discovery
// ---------------------------------------------------------------------------------------------------------------------------------

/// The `M-SEARCH` for an Internet Gateway Device.
pub fn msearch(target: &str) -> String {
    format!("M-SEARCH * HTTP/1.1\r\nHOST: {SSDP_ADDR}\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {target}\r\n\r\n")
}

/// The header `name` of an SSDP/HTTP response (case-insensitive), trimmed.
pub fn header<'a>(response: &'a str, name: &str) -> Option<&'a str> {
    response.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

/// Sends an `M-SEARCH` to `target` (the multicast address, or a gateway's own address) and returns the first `LOCATION` that answers.
pub fn discover_location(target: SocketAddr, timeout: Duration) -> Result<String, UpnpError> {
    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| io_err("cannot open a UDP socket", e))?;
    let _ = sock.set_multicast_ttl_v4(2);
    let end = Instant::now() + timeout;
    let mut buf = [0u8; 2048];
    for st in ["urn:schemas-upnp-org:device:InternetGatewayDevice:1", "urn:schemas-upnp-org:device:InternetGatewayDevice:2", "upnp:rootdevice"] {
        if sock.send_to(msearch_bytes(st).as_slice(), target).is_err() {
            continue;
        }
        let slice_end = (Instant::now() + timeout / 3).min(end);
        while Instant::now() < slice_end {
            let left = slice_end.saturating_duration_since(Instant::now()).max(Duration::from_millis(20));
            if sock.set_read_timeout(Some(left)).is_err() {
                break;
            }
            match sock.recv_from(&mut buf) {
                Ok((n, _)) => {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    if let Some(loc) = header(&text, "location") {
                        return Ok(loc.to_string());
                    }
                }
                Err(_) => break,
            }
        }
    }
    Err(UpnpError::NoRouter("no router answered UPnP discovery (UPnP may be switched off in the router, or there is no router on this network)".to_string()))
}

fn msearch_bytes(st: &str) -> Vec<u8> {
    msearch(st).into_bytes()
}

// ---------------------------------------------------------------------------------------------------------------------------------
// a minimal HTTP client (routers speak HTTP/1.x over a plain TCP connection)
// ---------------------------------------------------------------------------------------------------------------------------------

/// Splits `http://host:port/path` into `(host:port, path)`.
pub fn split_url(url: &str) -> Result<(String, String), UpnpError> {
    let rest = url.strip_prefix("http://").ok_or_else(|| UpnpError::Protocol(format!("'{url}' is not an http:// address")))?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let host = if host.contains(':') { host.to_string() } else { format!("{host}:80") };
    Ok((host, path.to_string()))
}

/// One HTTP request; returns `(status, body)`. Handles `Content-Length`, chunked and connection-close bodies, and caps the size.
pub fn http(host: &str, method: &str, path: &str, extra_headers: &[(&str, String)], body: &str, timeout: Duration) -> Result<(u16, String), UpnpError> {
    let addr = host
        .to_socket_addrs()
        .map_err(|e| io_err(&format!("cannot resolve {host}"), e))?
        .next()
        .ok_or_else(|| UpnpError::Io(format!("no address for {host}")))?;
    let mut s = TcpStream::connect_timeout(&addr, timeout).map_err(|e| io_err(&format!("cannot connect to {host}"), e))?;
    let _ = s.set_read_timeout(Some(timeout));
    let _ = s.set_write_timeout(Some(timeout));
    let mut req = format!("{method} {path} HTTP/1.1\r\nHOST: {host}\r\nConnection: close\r\nUser-Agent: red-engine/0.1 UPnP/1.1\r\n");
    for (k, v) in extra_headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
    s.write_all(req.as_bytes()).map_err(|e| io_err("cannot send to the router", e))?;
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match s.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&chunk[..n]);
                if raw.len() > MAX_RESPONSE {
                    return Err(UpnpError::Protocol("the router's answer is implausibly large".to_string()));
                }
            }
            Err(e) if raw.is_empty() => return Err(io_err("no answer from the router", e)),
            Err(_) => break, // a timeout after some data: use what came
        }
    }
    parse_http_response(&raw)
}

/// Parses a raw HTTP response into `(status, body)`.
pub fn parse_http_response(raw: &[u8]) -> Result<(u16, String), UpnpError> {
    let text = String::from_utf8_lossy(raw);
    let (head, body) = text.split_once("\r\n\r\n").ok_or_else(|| UpnpError::Protocol("the router's answer is not HTTP".to_string()))?;
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or_else(|| UpnpError::Protocol("the router's answer has no status".to_string()))?;
    let chunked = header(head, "transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    let body = if chunked {
        dechunk(body)
    } else if let Some(n) = header(head, "content-length").and_then(|v| v.parse::<usize>().ok()) {
        body.chars().take(n).collect::<String>()
    } else {
        body.to_string()
    };
    Ok((status, body))
}

fn dechunk(mut s: &str) -> String {
    let mut out = String::new();
    while let Some((size_line, rest)) = s.split_once("\r\n") {
        let Ok(n) = usize::from_str_radix(size_line.split(';').next().unwrap_or("").trim(), 16) else { break };
        if n == 0 {
            break;
        }
        let take: String = rest.chars().take(n).collect();
        let bytes = take.len();
        out.push_str(&take);
        s = rest.get(bytes..).unwrap_or("").trim_start_matches("\r\n");
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------------------------
// the device description and SOAP
// ---------------------------------------------------------------------------------------------------------------------------------

/// The text of the first `<tag>...</tag>` in `xml`.
pub fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim())
}

/// Finds the WAN connection service in a device description and resolves its control URL against `base` (the description's own address).
pub fn find_control(description: &str, base_host: &str) -> Result<Control, UpnpError> {
    let url_base = tag(description, "URLBase");
    for block in description.split("<service>").skip(1) {
        let block = block.split("</service>").next().unwrap_or("");
        let Some(service) = tag(block, "serviceType") else { continue };
        if !(service.contains("WANIPConnection") || service.contains("WANPPPConnection")) {
            continue;
        }
        let Some(control) = tag(block, "controlURL") else { continue };
        let (host, path) = if control.starts_with("http://") {
            split_url(control)?
        } else if let Some(b) = url_base.filter(|b| b.starts_with("http://")) {
            let (h, p) = split_url(b)?;
            (h, join_path(&p, control))
        } else {
            (base_host.to_string(), join_path("/", control))
        };
        return Ok(Control { host, path, service: service.to_string() });
    }
    Err(UpnpError::Protocol("the router has no WANIPConnection / WANPPPConnection service (it cannot map ports)".to_string()))
}

fn join_path(base: &str, rel: &str) -> String {
    if rel.starts_with('/') {
        rel.to_string()
    } else {
        format!("{}/{}", base.trim_end_matches('/'), rel)
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The SOAP envelope for `action` with `args` (in order).
pub fn envelope(service: &str, action: &str, args: &[(&str, String)]) -> String {
    let mut inner = String::new();
    for (k, v) in args {
        inner.push_str(&format!("<{k}>{}</{k}>", xml_escape(v)));
    }
    format!(
        "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{service}\">{inner}</u:{action}></s:Body></s:Envelope>"
    )
}

/// A router found and ready to be asked.
#[derive(Debug, Clone)]
pub struct Igd {
    /// Where to send SOAP.
    pub control: Control,
    /// The router's address as it answered (for messages).
    pub router: String,
    /// Network timeout per request.
    pub timeout: Duration,
}

impl Igd {
    /// Finds the router: SSDP to `search` (the multicast group, or a gateway's own `ip:1900`), then its description.
    pub fn discover(search: SocketAddr, timeout: Duration) -> Result<Igd, UpnpError> {
        let location = discover_location(search, timeout)?;
        Igd::from_location(&location, Duration::from_secs(4))
    }

    /// Reads the router's description at `location` (`http://host:port/path`).
    pub fn from_location(location: &str, timeout: Duration) -> Result<Igd, UpnpError> {
        let (host, path) = split_url(location)?;
        let (status, body) = http(&host, "GET", &path, &[], "", timeout)?;
        if status != 200 {
            return Err(UpnpError::Protocol(format!("the router's description at {location} answered HTTP {status}")));
        }
        Ok(Igd { control: find_control(&body, &host)?, router: host, timeout })
    }

    fn call(&self, action: &str, args: &[(&str, String)]) -> Result<String, UpnpError> {
        let body = envelope(&self.control.service, action, args);
        let (status, text) = http(
            &self.control.host,
            "POST",
            &self.control.path,
            &[("Content-Type", "text/xml; charset=\"utf-8\"".to_string()), ("SOAPAction", format!("\"{}#{action}\"", self.control.service))],
            &body,
            self.timeout,
        )?;
        if let Some(code) = tag(&text, "errorCode") {
            return Err(UpnpError::Refused(code.parse().unwrap_or(0), tag(&text, "errorDescription").unwrap_or("").to_string()));
        }
        if status >= 400 {
            return Err(UpnpError::Refused(status as u32, String::new()));
        }
        Ok(text)
    }

    /// The router's public address (`None` if it will not say).
    pub fn external_ip(&self) -> Result<Option<String>, UpnpError> {
        let r = self.call("GetExternalIPAddress", &[])?;
        Ok(tag(&r, "NewExternalIPAddress").filter(|s| !s.is_empty()).map(str::to_string))
    }

    /// The UDP mapping on external port `port`, if there is one.
    pub fn mapping(&self, port: u16) -> Result<Option<Mapping>, UpnpError> {
        let args = [("NewRemoteHost", String::new()), ("NewExternalPort", port.to_string()), ("NewProtocol", "UDP".to_string())];
        match self.call("GetSpecificPortMappingEntry", &args) {
            Ok(r) => Ok(Some(Mapping {
                internal_client: tag(&r, "NewInternalClient").unwrap_or("").to_string(),
                internal_port: tag(&r, "NewInternalPort").and_then(|p| p.parse().ok()).unwrap_or(0),
                description: tag(&r, "NewPortMappingDescription").unwrap_or("").to_string(),
                lease_secs: tag(&r, "NewLeaseDuration").and_then(|p| p.parse().ok()).unwrap_or(0),
                enabled: tag(&r, "NewEnabled").is_none_or(|e| e.trim() == "1"),
            })),
            Err(UpnpError::Refused(714, _)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Maps UDP `port` to `local:port` with a lease, unless the port is already mapped to something that is not ours. Renewing our own
    /// mapping is fine. A router that insists on a permanent lease is refused unless `allow_permanent`.
    pub fn enable(&self, port: u16, local: Ipv4Addr, lease_secs: u32, allow_permanent: bool) -> Result<Mapping, UpnpError> {
        if let Some(m) = self.mapping(port)? {
            if !is_ours(&m, local, port) {
                return Err(UpnpError::Declined(format!(
                    "UDP {port} is already mapped to {}:{} ('{}'): leaving it alone. Pick another port with --port.",
                    m.internal_client, m.internal_port, m.description
                )));
            }
        }
        let args = [
            ("NewRemoteHost", String::new()),
            ("NewExternalPort", port.to_string()),
            ("NewProtocol", "UDP".to_string()),
            ("NewInternalPort", port.to_string()),
            ("NewInternalClient", local.to_string()),
            ("NewEnabled", "1".to_string()),
            ("NewPortMappingDescription", DESCRIPTION.to_string()),
            ("NewLeaseDuration", lease_secs.to_string()),
        ];
        match self.call("AddPortMapping", &args) {
            Ok(_) => {}
            Err(UpnpError::Refused(725, _)) if allow_permanent => {
                let mut permanent = args.to_vec();
                if let Some(l) = permanent.iter_mut().find(|a| a.0 == "NewLeaseDuration") {
                    l.1 = "0".to_string();
                }
                self.call("AddPortMapping", &permanent)?;
            }
            Err(UpnpError::Refused(725, _)) => {
                return Err(UpnpError::Declined(
                    "this router only accepts permanent port mappings, which would stay open after the game ends. Refusing; use --allow-permanent to accept it and run `portmap remove` when done.".to_string(),
                ));
            }
            Err(e) => return Err(e),
        }
        // Trust, then verify: read it back.
        let m = self.mapping(port)?.ok_or_else(|| UpnpError::Protocol("the router accepted the mapping but does not list it".to_string()))?;
        if !is_ours(&m, local, port) || !m.enabled {
            return Err(UpnpError::Protocol(format!(
                "the router's mapping is not what was asked for: {}:{} '{}'",
                m.internal_client, m.internal_port, m.description
            )));
        }
        if m.lease_secs == 0 && !allow_permanent {
            let _ = self.remove(port, local);
            return Err(UpnpError::Declined("the router made the mapping permanent; it was removed again instead of leaving a hole open".to_string()));
        }
        Ok(m)
    }

    /// Removes UDP `port` if (and only if) the mapping is ours. `Ok(false)` when there was nothing of ours to remove.
    pub fn remove(&self, port: u16, local: Ipv4Addr) -> Result<bool, UpnpError> {
        match self.mapping(port)? {
            None => Ok(false),
            Some(m) if !is_ours(&m, local, port) => {
                Err(UpnpError::Declined(format!("UDP {port} belongs to another mapping ('{}' -> {}): leaving it alone.", m.description, m.internal_client)))
            }
            Some(_) => {
                self.call(
                    "DeletePortMappingEntry",
                    &[("NewRemoteHost", String::new()), ("NewExternalPort", port.to_string()), ("NewProtocol", "UDP".to_string())],
                )?;
                Ok(true)
            }
        }
    }
}

/// Whether `m` is a mapping made here: our description, to this machine, to the same port.
pub fn is_ours(m: &Mapping, local: Ipv4Addr, port: u16) -> bool {
    m.description == DESCRIPTION && m.internal_client == local.to_string() && m.internal_port == port
}

/// The address of this machine on the interface that reaches `peer` (no packet is sent: a UDP `connect` only picks the route).
pub fn local_ip_towards(peer: IpAddr) -> Option<Ipv4Addr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect(SocketAddr::new(peer, 1900)).ok()?;
    match sock.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if !v4.is_unspecified() => Some(v4),
        _ => None,
    }
}

/// Whether an address is private (RFC 1918 / CGNAT): a router whose "external" address is one of these cannot be reached from the internet.
pub fn is_private_or_cgnat(ip: &str) -> bool {
    let Ok(v4) = ip.parse::<Ipv4Addr>() else { return false };
    let o = v4.octets();
    v4.is_private() || v4.is_loopback() || v4.is_link_local() || (o[0] == 100 && (64..128).contains(&o[1]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    const DESCRIPTION_XML: &str = r#"<?xml version="1.0"?><root><URLBase>http://127.0.0.1:PORT/</URLBase><device><serviceList>
        <service><serviceType>urn:schemas-upnp-org:service:Layer3Forwarding:1</serviceType><controlURL>/l3f</controlURL></service>
        <service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType><controlURL>ctl/IPConn</controlURL></service>
        </serviceList></device></root>"#;

    /// A pretend router: serves its description and a stateful SOAP endpoint, on loopback.
    struct FakeRouter {
        addr: SocketAddr,
        table: Arc<Mutex<HashMap<u16, Mapping>>>,
        only_permanent: Arc<Mutex<bool>>,
    }

    fn fake_router() -> FakeRouter {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let table: Arc<Mutex<HashMap<u16, Mapping>>> = Arc::new(Mutex::new(HashMap::new()));
        let only_permanent = Arc::new(Mutex::new(false));
        let (t2, p2) = (table.clone(), only_permanent.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { break };
                let mut buf = vec![0u8; 16384];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let reply = |status: &str, body: String| format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: text/xml\r\n\r\n{body}", body.len());
                let response = if req.starts_with("GET /rootDesc.xml") {
                    reply("200 OK", DESCRIPTION_XML.replace("PORT", &addr.port().to_string()))
                } else if req.starts_with("POST /ctl/IPConn") {
                    let action =
                        req.split("SOAPAction: \"").nth(1).and_then(|r| r.split('#').nth(1)).and_then(|r| r.split('"').next()).unwrap_or("").to_string();
                    let port: u16 = tag(&req, "NewExternalPort").and_then(|p| p.parse().ok()).unwrap_or(0);
                    let fault = |code: u32, text: &str| {
                        reply("500 Internal Server Error", format!("<s:Envelope><s:Body><s:Fault><detail><UPnPError><errorCode>{code}</errorCode><errorDescription>{text}</errorDescription></UPnPError></detail></s:Fault></s:Body></s:Envelope>"))
                    };
                    let ok =
                        |inner: &str| reply("200 OK", format!("<s:Envelope><s:Body><u:{action}Response>{inner}</u:{action}Response></s:Body></s:Envelope>"));
                    let mut t = t2.lock().unwrap();
                    match action.as_str() {
                        "GetExternalIPAddress" => ok("<NewExternalIPAddress>203.0.113.7</NewExternalIPAddress>"),
                        "GetSpecificPortMappingEntry" => match t.get(&port) {
                            None => fault(714, "NoSuchEntryInArray"),
                            Some(m) => ok(&format!(
                                "<NewInternalPort>{}</NewInternalPort><NewInternalClient>{}</NewInternalClient><NewEnabled>{}</NewEnabled><NewPortMappingDescription>{}</NewPortMappingDescription><NewLeaseDuration>{}</NewLeaseDuration>",
                                m.internal_port, m.internal_client, m.enabled as u8, m.description, m.lease_secs
                            )),
                        },
                        "AddPortMapping" => {
                            let lease: u32 = tag(&req, "NewLeaseDuration").and_then(|p| p.parse().ok()).unwrap_or(0);
                            if *p2.lock().unwrap() && lease != 0 {
                                fault(725, "OnlyPermanentLeasesSupported")
                            } else {
                                t.insert(
                                    port,
                                    Mapping {
                                        internal_client: tag(&req, "NewInternalClient").unwrap_or("").to_string(),
                                        internal_port: tag(&req, "NewInternalPort").and_then(|p| p.parse().ok()).unwrap_or(0),
                                        description: tag(&req, "NewPortMappingDescription").unwrap_or("").to_string(),
                                        lease_secs: lease,
                                        enabled: true,
                                    },
                                );
                                ok("")
                            }
                        }
                        "DeletePortMappingEntry" => {
                            if t.remove(&port).is_some() {
                                ok("")
                            } else {
                                fault(714, "NoSuchEntryInArray")
                            }
                        }
                        _ => fault(401, "Invalid Action"),
                    }
                } else {
                    reply("404 Not Found", String::new())
                };
                let _ = s.write_all(response.as_bytes());
            }
        });
        FakeRouter { addr, table, only_permanent }
    }

    fn igd(r: &FakeRouter) -> Igd {
        Igd::from_location(&format!("http://{}/rootDesc.xml", r.addr), Duration::from_secs(3)).unwrap()
    }

    const ME: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 50);

    #[test]
    fn the_control_url_is_read_from_the_description_in_every_form_a_router_writes_it() {
        let rel = "<service><serviceType>urn:x:service:WANIPConnection:2</serviceType><controlURL>/upnp/control/WANIPConn1</controlURL></service>";
        let c = find_control(rel, "192.168.0.1:5431").unwrap();
        assert_eq!((c.host.as_str(), c.path.as_str()), ("192.168.0.1:5431", "/upnp/control/WANIPConn1"));
        assert!(c.service.ends_with("WANIPConnection:2"), "the service version the router offers is the one we name");
        let abs = "<service><serviceType>urn:x:service:WANPPPConnection:1</serviceType><controlURL>http://10.0.0.1:49152/ctl</controlURL></service>";
        assert_eq!(find_control(abs, "ignored:1").unwrap().host, "10.0.0.1:49152");
        let based = "<URLBase>http://192.168.2.1:2555/</URLBase><service><serviceType>urn:x:service:WANIPConnection:1</serviceType><controlURL>ctl/IPConn</controlURL></service>";
        let c = find_control(based, "ignored:1").unwrap();
        assert_eq!((c.host.as_str(), c.path.as_str()), ("192.168.2.1:2555", "/ctl/IPConn"));
        assert!(matches!(find_control("<service><serviceType>urn:x:service:Layer3Forwarding:1</serviceType></service>", "h:1"), Err(UpnpError::Protocol(_))));
    }

    #[test]
    fn ssdp_responses_and_http_bodies_parse_in_any_case_and_framing() {
        let resp = "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=120\r\nLocation: http://192.168.0.1:1900/rootDesc.xml\r\nST: upnp:rootdevice\r\n\r\n";
        assert_eq!(header(resp, "location"), Some("http://192.168.0.1:1900/rootDesc.xml"));
        assert_eq!(header(resp, "LOCATION"), header(resp, "location"));
        assert!(msearch("upnp:rootdevice").contains("MAN: \"ssdp:discover\"") && msearch("x").ends_with("\r\n\r\n"));
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        assert_eq!(parse_http_response(chunked).unwrap(), (200, "hello world".to_string()));
        assert_eq!(parse_http_response(b"HTTP/1.1 500 X\r\nContent-Length: 2\r\n\r\nokEXTRA").unwrap(), (500, "ok".to_string()));
        assert!(parse_http_response(b"garbage").is_err());
        assert_eq!(split_url("http://192.168.0.1/desc.xml").unwrap(), ("192.168.0.1:80".to_string(), "/desc.xml".to_string()));
        assert!(split_url("https://x/").is_err());
    }

    #[test]
    fn discovery_finds_the_router_that_answers_and_says_so_when_none_does() {
        let responder = UdpSocket::bind("127.0.0.1:0").unwrap();
        let target = responder.local_addr().unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            if let Ok((_, from)) = responder.recv_from(&mut buf) {
                let _ = responder.send_to(b"HTTP/1.1 200 OK\r\nlocation: http://127.0.0.1:9/desc.xml\r\n\r\n", from);
            }
        });
        assert_eq!(discover_location(target, Duration::from_secs(2)).unwrap(), "http://127.0.0.1:9/desc.xml");
        let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
        let e = discover_location(silent.local_addr().unwrap(), Duration::from_millis(300)).unwrap_err();
        assert!(matches!(e, UpnpError::NoRouter(_)) && e.to_string().contains("switched off"), "{e}");
    }

    #[test]
    fn enable_maps_verifies_and_remove_deletes_only_what_is_ours() {
        let r = fake_router();
        let g = igd(&r);
        assert_eq!(g.external_ip().unwrap().as_deref(), Some("203.0.113.7"));
        assert_eq!(g.mapping(27015).unwrap(), None);
        let m = g.enable(27015, ME, 3600, false).unwrap();
        assert_eq!((m.internal_client.as_str(), m.internal_port, m.description.as_str(), m.lease_secs), ("192.168.1.50", 27015, DESCRIPTION, 3600));
        // Renewing our own mapping is allowed.
        g.enable(27015, ME, 1800, false).unwrap();
        assert_eq!(r.table.lock().unwrap().get(&27015).unwrap().lease_secs, 1800);
        assert!(g.remove(27015, ME).unwrap());
        assert!(!g.remove(27015, ME).unwrap(), "nothing of ours left");
        assert!(r.table.lock().unwrap().is_empty());
    }

    #[test]
    fn a_mapping_that_is_not_ours_is_never_overwritten_or_deleted() {
        let r = fake_router();
        r.table.lock().unwrap().insert(
            27015,
            Mapping { internal_client: "192.168.1.99".into(), internal_port: 27015, description: "Someone's NAS".into(), lease_secs: 0, enabled: true },
        );
        let g = igd(&r);
        for e in [g.enable(27015, ME, 3600, false).unwrap_err(), g.remove(27015, ME).unwrap_err()] {
            assert!(matches!(e, UpnpError::Declined(_)) && e.to_string().contains("leaving it alone"), "{e}");
        }
        assert_eq!(r.table.lock().unwrap().get(&27015).unwrap().description, "Someone's NAS", "untouched");
        // Same description but another host on the LAN is also not ours.
        r.table.lock().unwrap().insert(
            27016,
            Mapping { internal_client: "192.168.1.77".into(), internal_port: 27016, description: DESCRIPTION.into(), lease_secs: 100, enabled: true },
        );
        assert!(matches!(g.remove(27016, ME), Err(UpnpError::Declined(_))));
    }

    #[test]
    fn a_router_that_only_accepts_permanent_mappings_is_refused_unless_the_caller_opts_in() {
        let r = fake_router();
        *r.only_permanent.lock().unwrap() = true;
        let g = igd(&r);
        let e = g.enable(27015, ME, 3600, false).unwrap_err();
        assert!(matches!(e, UpnpError::Declined(_)) && e.to_string().contains("permanent"), "{e}");
        assert!(r.table.lock().unwrap().is_empty(), "nothing was left open");
        let m = g.enable(27015, ME, 3600, true).unwrap();
        assert_eq!(m.lease_secs, 0, "opted in: permanent, and the caller was told to remove it later");
    }

    #[test]
    fn router_errors_are_explained_in_plain_words() {
        assert!(explain_code(718, "").contains("already mapped"));
        assert!(explain_code(725, "OnlyPermanentLeasesSupported").contains("permanent"));
        assert_eq!(explain_code(999, "vendor text"), "vendor text");
        assert!(UpnpError::Refused(718, String::new()).to_string().contains("718"));
        assert!(is_private_or_cgnat("192.168.1.1") && is_private_or_cgnat("100.64.3.4") && is_private_or_cgnat("10.1.2.3"));
        assert!(!is_private_or_cgnat("203.0.113.7") && !is_private_or_cgnat("not an ip"));
    }

    #[test]
    fn the_local_address_is_the_one_on_the_interface_that_reaches_the_router() {
        let ip = local_ip_towards("127.0.0.1".parse().unwrap()).unwrap();
        assert_eq!(ip, Ipv4Addr::LOCALHOST);
    }
}
