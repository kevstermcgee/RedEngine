//! A join code: everything a friend needs to join a hosted game, as one string they can paste (ADR 2026-09-30-killchain-loadout-shooter).
//!
//! `HOST:PORT#sha256:<fingerprint>#<key>`. The fingerprint pins the host's identity so the connection is encrypted and the right
//! server is verified (QUIC + TLS 1.3); the key is the host's join key (optional). A plain `HOST` or `HOST:PORT` is a server on a trusted
//! network or this machine. Pure parsing and formatting: no sockets.
//!
//! People do not paste codes tidily: the box's `join-code.txt` holds a line like `from anywhere (needs UDP 27015 forwarded):  HOST:PORT#...`, a
//! chat message wraps a code, an editor adds a newline. [`extract_code`] finds the code in whatever was pasted (JOIN calls it before it parses
//! anything), and [`JoinCode::parse`] itself accepts `SHA256:` in any case and hexadecimal in either case.

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

/// The code inside what a person pasted or typed, as one line with no label and no surrounding white space (what [`JoinCode::parse`] and
/// `ShortJoin::parse` then read). A label is whatever comes before the last `:` that is followed by white space (`from anywhere (needs UDP 27015
/// forwarded):  HOST:PORT#...`), so a colon inside the code itself (`HOST:PORT`, `sha256:`) is never taken for one. Several lines (a whole
/// `join-code.txt`, a message with a code in it) give one code: the line whose label says "anywhere" if there is one (the code a friend on
/// another network needs), else the first line with a `#` in it (a long code), else the first non-empty line. Text with no label and one
/// line is only trimmed, so `H3PQXR K7Q2 MZ4P WTXA` (a code read out loud) is left for the parser as it is.
pub fn extract_code(text: &str) -> String {
    let mut lines: Vec<(String, String)> = Vec::new(); // (label, code)
    for line in text.lines() {
        let (label, rest) = split_label(line.trim());
        let rest = rest.trim();
        if !rest.is_empty() {
            lines.push((label.to_ascii_lowercase(), rest.to_string()));
        }
    }
    let pick =
        lines.iter().find(|(label, _)| label.contains("anywhere")).or_else(|| lines.iter().find(|(_, code)| code.contains('#'))).or_else(|| lines.first());
    pick.map(|(_, code)| code.clone()).unwrap_or_default()
}

/// `line` split into its label (up to and including the last `:` that is followed by white space) and the rest; no label gives `("", line)`.
fn split_label(line: &str) -> (&str, &str) {
    let mut cut = None;
    for (i, c) in line.char_indices() {
        if c == ':' && line[i + 1..].chars().next().is_some_and(char::is_whitespace) {
            cut = Some(i + 1);
        }
    }
    match cut {
        Some(i) => (&line[..i], &line[i..]),
        None => ("", line),
    }
}

/// Whether `p` starts with `sha256:` in any case.
fn has_sha256_prefix(p: &str) -> bool {
    p.get(..7).is_some_and(|head| head.eq_ignore_ascii_case("sha256:"))
}

impl JoinCode {
    /// Parses what a person pasted or typed: a label before the code (`from anywhere (...):`), surrounding white space (a trailing newline too),
    /// a leading `kc:` and the case of `sha256:` and of the hexadecimal are all ignored; the key is taken exactly as written. Errors say what is
    /// wrong.
    pub fn parse(text: &str) -> Result<JoinCode, String> {
        let text = extract_code(text);
        let t = text.trim();
        let t = if t.get(..3).is_some_and(|head| head.eq_ignore_ascii_case("kc:")) { t[3..].trim() } else { t };
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
            if has_sha256_prefix(p) || (p.len() == 64 && p.chars().all(|c| c.is_ascii_hexdigit())) {
                let hex = if has_sha256_prefix(p) { &p[7..] } else { p };
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
        assert_eq!(
            JoinCode::parse(&format!("10.0.0.5#{}", FP.trim_start_matches("sha256:"))).unwrap().fingerprint.as_deref(),
            Some(FP),
            "a bare hash is a fingerprint"
        );
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
    #[test]
    fn the_identity_is_accepted_in_any_case() {
        let upper = format!("host.example:27015#SHA256:{}#KeyOne", FP.trim_start_matches("sha256:").to_ascii_uppercase());
        let c = JoinCode::parse(&upper).unwrap();
        assert_eq!(
            (c.address.as_str(), c.fingerprint.as_deref(), c.key.as_deref()),
            ("host.example:27015", Some(FP), Some("KeyOne")),
            "the key is kept as written"
        );
        assert_eq!(JoinCode::parse(&format!("h#Sha256:{}", FP.trim_start_matches("sha256:"))).unwrap().fingerprint.as_deref(), Some(FP));
        assert_eq!(
            JoinCode::parse(&format!("h#{}", FP.trim_start_matches("sha256:").to_ascii_uppercase())).unwrap().fingerprint.as_deref(),
            Some(FP),
            "a bare upper-case hash too"
        );
        assert!(JoinCode::parse("h#SHA256:abc").unwrap_err().contains("damaged"));
        assert_eq!(JoinCode::parse("KC:10.0.0.5:4000").unwrap().address, "10.0.0.5:4000");
    }

    #[test]
    fn a_label_white_space_and_a_trailing_newline_around_the_code_are_ignored() {
        let code = format!("red-engine.duckdns.org:27015#{FP}#K7Q2MZ4P");
        let want = JoinCode::parse(&code).unwrap();
        for pasted in [
            format!("from anywhere (needs UDP 27015 forwarded):  {code}"),
            format!("from anywhere (needs UDP 27015 forwarded):  {code}\n"),
            format!("from anywhere (needs UDP 27015 forwarded):\t{code}\r\n"),
            format!("  {code}  \n\n"),
            format!("\n{code}"),
            format!("on this network (LAN): {code}"),
        ] {
            assert_eq!(JoinCode::parse(&pasted).unwrap(), want, "{pasted:?}");
        }
        // Only a label: nothing to join.
        assert!(JoinCode::parse("from anywhere (needs UDP 27015 forwarded):  ").is_err());
    }

    #[test]
    fn a_whole_file_of_codes_gives_the_one_for_friends_far_away() {
        let (lan, far) = (format!("192.168.0.5:27015#{FP}#K7Q2MZ4P"), format!("red-engine.duckdns.org:27015#{FP}#K7Q2MZ4P"));
        let file = format!("on this network (LAN):  {lan}\nfrom anywhere (needs UDP 27015 forwarded):  {far}\n");
        assert_eq!(extract_code(&file), far, "the line that says anywhere wins, whatever its place");
        assert_eq!(JoinCode::parse(&file).unwrap().address, "red-engine.duckdns.org:27015");
        // With no such label the first line with a long code in it is the code.
        assert_eq!(extract_code(&format!("Join us!\n{lan}\nSee you there")), lan);
        // A message with a code inside is not a code unless a line says so; one line is only trimmed.
        assert_eq!(extract_code("  H3PQXR K7Q2 MZ4P WTXA \n"), "H3PQXR K7Q2 MZ4P WTXA", "a code read out loud keeps its spaces for the parser");
        assert_eq!(extract_code(""), "");
        assert_eq!(extract_code("host:27015"), "host:27015", "the colon in HOST:PORT is not a label");
    }
}
