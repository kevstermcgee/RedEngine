//! A join code: everything a friend needs to join a hosted game, as one string they can paste (ADR 2026-09-30-killchain-loadout-shooter).
//!
//! `HOST:PORT#sha256:<fingerprint>#<key>`. The fingerprint pins the host's identity so the connection is encrypted and the right
//! server is verified (QUIC + TLS 1.3); the key is the host's join key (optional). A plain `HOST` or `HOST:PORT` is a server on a trusted
//! network or this machine. Pure parsing and formatting: no sockets.

use super::DEFAULT_PORT;

/// A parsed join code.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JoinCode {
    /// `HOST:PORT` (the default port is added when the code names none).
    pub address: String,
    /// The host's pinned identity, `sha256:<64 hex>`, if the code carries one.
    pub fingerprint: Option<String>,
    /// The join key, if the code carries one.
    pub key: Option<String>,
}

impl JoinCode {
    /// Parses what a person pasted or typed: surrounding spaces and a leading `kc:` are ignored. Errors say what is wrong.
    pub fn parse(text: &str) -> Result<JoinCode, String> {
        let t = text.trim().trim_start_matches("kc:").trim();
        if t.is_empty() {
            return Err("type or paste the join code your friend sent".to_string());
        }
        let mut parts = t.split('#');
        let host = parts.next().unwrap_or("").trim();
        if host.is_empty() || host.contains(char::is_whitespace) {
            return Err(format!("'{host}' is not an address: a join code starts with HOST:PORT"));
        }
        let address = if host.starts_with('[') {
            if host.contains("]:") {
                host.to_string()
            } else {
                format!("{host}:{DEFAULT_PORT}")
            }
        } else if host.matches(':').count() == 1 {
            let (h, p) = host.split_once(':').unwrap_or((host, ""));
            p.parse::<u16>().map_err(|_| format!("'{p}' is not a port number"))?;
            format!("{h}:{p}")
        } else if host.contains(':') {
            return Err("an IPv6 address needs brackets and a port: [::1]:27015".to_string());
        } else {
            format!("{host}:{DEFAULT_PORT}")
        };
        let mut fingerprint = None;
        let mut key = None;
        for p in parts {
            let p = p.trim();
            if p.is_empty() {
                continue;
            }
            if p.starts_with("sha256:") || (p.len() == 64 && p.chars().all(|c| c.is_ascii_hexdigit())) {
                let hex = p.trim_start_matches("sha256:");
                if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err("the identity part of the code is damaged (it should be 64 hexadecimal characters): ask for the code again".to_string());
                }
                fingerprint = Some(format!("sha256:{}", hex.to_ascii_lowercase()));
            } else {
                key = Some(p.to_string());
            }
        }
        Ok(JoinCode { address, fingerprint, key })
    }

    /// The code as one line to give a friend.
    pub fn format(&self) -> String {
        let mut s = self.address.clone();
        if let Some(f) = &self.fingerprint {
            s.push('#');
            s.push_str(f);
        }
        if let Some(k) = &self.key {
            s.push('#');
            s.push_str(k);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn a_code_round_trips_and_the_default_port_is_added() {
        let c = JoinCode { address: "203.0.113.9:27016".into(), fingerprint: Some(FP.into()), key: Some("k7Q2mZ".into()) };
        assert_eq!(JoinCode::parse(&c.format()).unwrap(), c);
        assert_eq!(JoinCode::parse("example.com").unwrap().address, format!("example.com:{DEFAULT_PORT}"));
        assert_eq!(JoinCode::parse("  kc:10.0.0.5:4000  ").unwrap().address, "10.0.0.5:4000");
        assert_eq!(JoinCode::parse(&format!("10.0.0.5#{}", FP.trim_start_matches("sha256:"))).unwrap().fingerprint.as_deref(), Some(FP), "a bare hash is a fingerprint");
        assert_eq!(JoinCode::parse("[::1]").unwrap().address, format!("[::1]:{DEFAULT_PORT}"));
    }

    #[test]
    fn mistakes_are_named() {
        assert!(JoinCode::parse("").unwrap_err().contains("paste"));
        assert!(JoinCode::parse("host:port").unwrap_err().contains("port"));
        assert!(JoinCode::parse("::1").unwrap_err().contains("brackets"));
        assert!(JoinCode::parse("10.0.0.1#sha256:abc").unwrap_err().contains("damaged"));
        assert!(JoinCode::parse("a b").is_err());
    }
}
