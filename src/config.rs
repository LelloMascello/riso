//! Persistent configuration: saved connections.

use std::io;
use std::path::PathBuf;

use gtk::glib;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Connection {
    pub id: u64,
    /// Display name (defaults to the host).
    pub name: String,
    /// Host name, IP address or `~/.ssh/config` alias.
    pub host: String,
    /// Remote user (empty: ssh default).
    pub user: String,
    pub port: Option<u16>,
    /// Private key path (empty: ssh default / agent).
    pub identity_file: String,
    /// Directory shown in the file explorer after connecting (empty: home).
    pub initial_dir: String,
}

impl Connection {
    pub fn subtitle(&self) -> String {
        let mut s = String::new();
        if !self.user.is_empty() {
            s.push_str(&self.user);
            s.push('@');
        }
        s.push_str(&self.host);
        if let Some(p) = self.port.filter(|p| *p != 22) {
            s.push_str(&format!(":{p}"));
        }
        s
    }

    /// Parses a quick-connect string: `[user@]host[:port]`.
    pub fn parse_quick(input: &str) -> Option<Connection> {
        let input = input.trim();
        let input = input.strip_prefix("ssh ").unwrap_or(input).trim();
        if input.is_empty() || input.contains(char::is_whitespace) {
            return None;
        }
        let (user, rest) = match input.rsplit_once('@') {
            Some((u, r)) => (u.to_string(), r),
            None => (String::new(), input),
        };
        // Bracketed IPv6: [::1]:22
        let (host, port) = if let Some(r) = rest.strip_prefix('[') {
            let (h, after) = r.split_once(']')?;
            let port = after.strip_prefix(':').and_then(|p| p.parse().ok());
            (h.to_string(), port)
        } else if rest.matches(':').count() == 1 {
            let (h, p) = rest.split_once(':').unwrap();
            (h.to_string(), Some(p.parse().ok()?))
        } else {
            (rest.to_string(), None)
        };
        if host.is_empty() {
            return None;
        }
        Some(Connection {
            host,
            user,
            port,
            ..Default::default()
        })
    }
}

fn config_file() -> PathBuf {
    glib::user_config_dir().join("riso").join("connections.json")
}

pub fn load_connections() -> Vec<Connection> {
    std::fs::read_to_string(config_file())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_connections(conns: &[Connection]) -> io::Result<()> {
    let path = config_file();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(conns).map_err(io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(tmp, path)
}

pub fn new_id() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// Host aliases declared in `~/.ssh/config` (without wildcards).
pub fn ssh_config_hosts() -> Vec<String> {
    let path = glib::home_dir().join(".ssh").join("config");
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut hosts = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else { continue };
        if !key.eq_ignore_ascii_case("host") {
            continue;
        }
        for h in parts {
            if h.starts_with('#') {
                break;
            }
            if !h.contains(['*', '?', '!']) && !hosts.iter().any(|x| x == h) {
                hosts.push(h.to_string());
            }
        }
    }
    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_connect_parsing() {
        let c = Connection::parse_quick("leo@example.com:2222").unwrap();
        assert_eq!((c.user.as_str(), c.host.as_str(), c.port), ("leo", "example.com", Some(2222)));
        let c = Connection::parse_quick("ssh myhost").unwrap();
        assert_eq!((c.user.as_str(), c.host.as_str(), c.port), ("", "myhost", None));
        let c = Connection::parse_quick("root@[::1]:22").unwrap();
        assert_eq!((c.host.as_str(), c.port), ("::1", Some(22)));
        let c = Connection::parse_quick("fe80::1").unwrap();
        assert_eq!(c.host, "fe80::1");
        assert!(Connection::parse_quick("").is_none());
        assert!(Connection::parse_quick("host:abc").is_none());
    }
}
