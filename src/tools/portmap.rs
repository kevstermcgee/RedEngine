//! `red_engine2 portmap` and `red_server --upnp`: host a game from a home PC by opening its UDP port on the router (ADR 0031).
//! The protocol work is `net::upnp`; this is the part a person touches: find the router, say what is and is not possible from here, make or
//! remove the mapping, keep it alive while the server runs, and print the address to give a friend.

use crate::net::upnp::{self, Igd, Mapping, UpnpError, DEFAULT_LEASE_SECS, SSDP_ADDR};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// What to do and to which router.
#[derive(Debug, Clone)]
pub struct Options {
    /// The UDP port (external and internal are the same).
    pub port: u16,
    /// Lease, seconds.
    pub lease: u32,
    /// Ask this gateway instead of searching the network.
    pub router: Option<IpAddr>,
    /// Accept a router that only makes permanent mappings.
    pub allow_permanent: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { port: crate::net::DEFAULT_PORT, lease: DEFAULT_LEASE_SECS, router: None, allow_permanent: false }
    }
}

/// A router that answered, and where this machine stands relative to it.
#[derive(Debug, Clone)]
pub struct Found {
    /// The router.
    pub igd: Igd,
    /// This machine's LAN address towards it.
    pub local: Ipv4Addr,
    /// The router's public address, if it says.
    pub external: Option<String>,
}

impl Found {
    /// The address a friend types to join (`None` when the router does not know its public address).
    pub fn join_address(&self, port: u16) -> Option<String> {
        self.external.as_ref().map(|ip| format!("{ip}:{port}"))
    }

    /// Why a friend outside the LAN still could not join even with the mapping, if that is the case (a carrier-grade NAT).
    pub fn reachability_warning(&self) -> Option<String> {
        let ip = self.external.as_deref()?;
        upnp::is_private_or_cgnat(ip).then(|| {
            format!("the router's own \"public\" address is {ip}, which is a private / carrier-grade NAT address: the internet cannot reach it whatever is mapped. Ask the ISP for a public address, or host on a cloud machine (docs/HOSTING.md).")
        })
    }
}

/// Finds the router.
pub fn find(opts: &Options) -> Result<Found, UpnpError> {
    let target: SocketAddr = match opts.router {
        Some(ip) => SocketAddr::new(ip, 1900),
        None => SSDP_ADDR.to_socket_addrs().ok().and_then(|mut i| i.next()).ok_or_else(|| UpnpError::Io("cannot form the SSDP address".to_string()))?,
    };
    let igd = Igd::discover(target, Duration::from_secs(3))?;
    let router_ip = igd
        .router
        .split(':')
        .next()
        .and_then(|h| h.parse::<IpAddr>().ok())
        .or(opts.router)
        .ok_or_else(|| UpnpError::Protocol(format!("cannot tell the router's address from '{}'", igd.router)))?;
    let local =
        upnp::local_ip_towards(router_ip).ok_or_else(|| UpnpError::Io("cannot tell which of this machine's addresses reaches the router".to_string()))?;
    let external = igd.external_ip().ok().flatten();
    Ok(Found { igd, local, external })
}

fn describe(m: &Mapping) -> String {
    format!(
        "{}:{} ('{}', {})",
        m.internal_client,
        m.internal_port,
        m.description,
        if m.lease_secs == 0 { "permanent".to_string() } else { format!("{} s left", m.lease_secs) }
    )
}

/// `portmap status`: the router, this machine, the public address, and what is mapped on the port.
pub fn status(opts: &Options) -> Result<String, UpnpError> {
    let f = find(opts)?;
    let mut s = format!("router: {} (control {}{})\nthis machine: {}\n", f.igd.router, f.igd.control.host, f.igd.control.path, f.local);
    s.push_str(&format!("public address: {}\n", f.external.as_deref().unwrap_or("the router does not say")));
    if let Some(w) = f.reachability_warning() {
        s.push_str(&format!("WARNING: {w}\n"));
    }
    match f.igd.mapping(opts.port)? {
        None => s.push_str(&format!("UDP {}: not mapped\n", opts.port)),
        Some(m) if upnp::is_ours(&m, f.local, opts.port) => s.push_str(&format!("UDP {}: mapped by this tool to {}\n", opts.port, describe(&m))),
        Some(m) => s.push_str(&format!("UDP {}: mapped by something else to {} (left alone)\n", opts.port, describe(&m))),
    }
    Ok(s)
}

/// `portmap enable`: makes the mapping and returns the report a host wants.
pub fn enable(opts: &Options) -> Result<String, UpnpError> {
    let f = find(opts)?;
    let m = f.igd.enable(opts.port, f.local, opts.lease, opts.allow_permanent)?;
    let mut s = format!("mapped UDP {} -> {}\n", opts.port, describe(&m));
    match f.join_address(opts.port) {
        Some(a) => s.push_str(&format!("friends join with:  re2 --connect {a}\n")),
        None => s.push_str("the router did not report a public address: look it up (search \"what is my IP\") and give friends that address and the port\n"),
    }
    if let Some(w) = f.reachability_warning() {
        s.push_str(&format!("WARNING: {w}\n"));
    }
    if m.lease_secs == 0 {
        s.push_str(&format!("NOTE: this mapping is permanent. Remove it when you are done: red_engine2 portmap remove --port {}\n", opts.port));
    } else {
        s.push_str(&format!("the router forgets it after {} s; `portmap keep` (or red_server --upnp) renews it while the server runs\n", m.lease_secs));
    }
    Ok(s)
}

/// `portmap remove`.
pub fn remove(opts: &Options) -> Result<String, UpnpError> {
    let f = find(opts)?;
    Ok(if f.igd.remove(opts.port, f.local)? {
        format!("removed the UDP {} mapping made by this tool\n", opts.port)
    } else {
        format!("no mapping of ours on UDP {}\n", opts.port)
    })
}

/// A live mapping that renews itself and removes itself: what `red_server --upnp` and `portmap keep` hold while the server runs.
pub struct Keeper {
    found: Found,
    opts: Options,
    next_renew: Instant,
}

impl Keeper {
    /// Maps the port now.
    pub fn start(opts: &Options) -> Result<(Keeper, String), UpnpError> {
        let report = enable(opts)?;
        let found = find(opts)?;
        let renew_in = Duration::from_secs((opts.lease.max(120) / 2) as u64);
        Ok((Keeper { found, opts: opts.clone(), next_renew: Instant::now() + renew_in }, report))
    }

    /// Renews the lease when it is half spent. Returns an error line if the router stopped cooperating (the caller prints it and carries on).
    pub fn tick(&mut self) -> Option<String> {
        if Instant::now() < self.next_renew {
            return None;
        }
        self.next_renew = Instant::now() + Duration::from_secs((self.opts.lease.max(120) / 2) as u64);
        match self.found.igd.enable(self.opts.port, self.found.local, self.opts.lease, self.opts.allow_permanent) {
            Ok(_) => None,
            Err(e) => Some(format!("could not renew the UDP {} mapping: {e}", self.opts.port)),
        }
    }

    /// Removes the mapping (best effort).
    pub fn stop(self) -> String {
        match self.found.igd.remove(self.opts.port, self.found.local) {
            Ok(true) => format!("removed the UDP {} mapping", self.opts.port),
            Ok(false) => format!("no mapping of ours left on UDP {}", self.opts.port),
            Err(e) => format!("could not remove the UDP {} mapping: {e}", self.opts.port),
        }
    }
}

/// `portmap keep`: map, then renew until `stop` is set, then remove.
pub fn keep(opts: &Options, stop: &AtomicBool) -> Result<String, UpnpError> {
    let (mut k, report) = Keeper::start(opts)?;
    print!("{report}");
    println!("keeping the mapping alive; press Ctrl-C to remove it and exit");
    while !stop.load(Ordering::Relaxed) {
        if let Some(e) = k.tick() {
            eprintln!("{e}");
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(format!("{}\n", k.stop()))
}
