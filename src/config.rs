//! Persistent host history, `~/.ssh/config` discovery, and `ssh` argument parsing.

use std::cell::RefCell;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

use crate::paths;

const MAX_RECENT_HOSTS: usize = 50;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Connection {
    pub id: u64,
    /// Optional alias / label.
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

    /// Parses a single target token: `[ssh://][user@]host[:port]`.
    pub fn parse_quick(input: &str) -> Option<Connection> {
        let input = input.trim();
        let input = input.strip_prefix("ssh ").unwrap_or(input).trim();
        if input.is_empty() || input.contains(char::is_whitespace) || input.starts_with('-') {
            return None;
        }
        let body = input
            .strip_prefix("ssh://")
            .map(|s| s.trim_end_matches('/'))
            .unwrap_or(input);
        if body.is_empty() || body.contains('/') {
            return None;
        }
        let (user, rest) = match body.rsplit_once('@') {
            Some((u, r)) if !u.is_empty() => (u.to_string(), r),
            Some(_) => return None,
            None => (String::new(), body),
        };
        // Bracketed IPv6: [::1]:22
        let (host, port) = if let Some(r) = rest.strip_prefix('[') {
            let (h, after) = r.split_once(']')?;
            let port = match after.strip_prefix(':') {
                Some(p) => Some(p.parse::<u16>().ok().filter(|&n| n > 0)?),
                None if after.is_empty() => None,
                None => return None,
            };
            (h.to_string(), port)
        } else if rest.matches(':').count() == 1 {
            let (h, p) = rest.split_once(':')?;
            let port = p.parse::<u16>().ok().filter(|&n| n > 0)?;
            (h.to_string(), Some(port))
        } else {
            (rest.to_string(), None)
        };
        if host.is_empty() || host.starts_with('-') {
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

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSsh {
    pub conn: Connection,
    pub extra_args: Vec<String>,
    pub raw_command: String,
}

/// Parses arguments typed after `ssh ` (e.g. `user@host`, `user@host:2222`,
/// `-p 2222 -i ~/.ssh/key user@host`, `-J bastion user@host`).
pub fn parse_ssh_command(input: &str) -> Option<ParsedSsh> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut tokens = split_shell_words(trimmed)?;
    if tokens.len() > 1 && tokens[0] == "ssh" {
        tokens.remove(0);
    }
    if tokens.is_empty() {
        return None;
    }

    const ARG_FLAGS: &[char] = &[
        'b', 'c', 'D', 'E', 'e', 'F', 'I', 'i', 'J', 'L', 'l', 'm', 'O', 'o', 'p', 'Q', 'R', 'S',
        'W', 'w',
    ];

    let mut port_flag: Option<u16> = None;
    let mut identity_flag: Option<String> = None;
    let mut user_flag: Option<String> = None;
    let mut extra_args: Vec<String> = Vec::new();
    let mut destination: Option<String> = None;

    let mut i = 0;
    while i < tokens.len() {
        let tok = &tokens[i];
        if tok == "--" {
            if i + 1 < tokens.len() && destination.is_none() {
                destination = Some(tokens[i + 1].clone());
                i += 2;
                continue;
            }
            return None;
        }
        if let Some(short) = tok.strip_prefix('-').filter(|s| !s.is_empty() && !s.starts_with('-'))
        {
            let mut chars = short.chars();
            let first = chars.next().unwrap();
            let rest: String = chars.collect();
            if ARG_FLAGS.contains(&first) {
                let val = if !rest.is_empty() {
                    rest
                } else {
                    i += 1;
                    tokens.get(i)?.clone()
                };
                match first {
                    'p' => {
                        port_flag = Some(val.parse::<u16>().ok().filter(|&n| n > 0)?);
                    }
                    'i' => {
                        identity_flag = Some(val);
                    }
                    'l' => {
                        user_flag = Some(val);
                    }
                    _ => {
                        extra_args.push(format!("-{first}"));
                        extra_args.push(val);
                    }
                }
            } else {
                extra_args.push(tok.clone());
            }
        } else if tok.starts_with("--") {
            return None;
        } else if destination.is_none() {
            destination = Some(tok.clone());
        } else {
            // Ignore or reject extra remote command tokens: interactive SSH tab expects a host.
            return None;
        }
        i += 1;
    }

    let mut conn = Connection::parse_quick(&destination?)?;
    if let Some(p) = port_flag {
        conn.port = Some(p);
    }
    if let Some(id_file) = identity_flag {
        conn.identity_file = id_file;
    }
    if conn.user.is_empty()
        && let Some(u) = user_flag
    {
        conn.user = u;
    }

    let raw_command = tokens.join(" ");
    Some(ParsedSsh {
        conn,
        extra_args,
        raw_command,
    })
}

/// Splits a command line into words, honouring POSIX-shell quoting.
pub fn split_shell_words(s: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                for nc in chars.by_ref() {
                    if nc == '\'' {
                        break;
                    }
                    cur.push(nc);
                }
            }
            '"' => {
                in_word = true;
                while let Some(nc) = chars.next() {
                    if nc == '"' {
                        break;
                    }
                    if nc == '\\'
                        && let Some(&esc) = chars.peek()
                        && matches!(esc, '"' | '\\' | '$' | '`')
                    {
                        cur.push(chars.next().unwrap());
                        continue;
                    }
                    cur.push(nc);
                }
            }
            '\\' => {
                in_word = true;
                if let Some(nc) = chars.next() {
                    cur.push(nc);
                }
            }
            other => {
                in_word = true;
                cur.push(other);
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    Some(words)
}

/// Builds the fixed local prompt prefix, e.g. `[leo@archlinux ~]$ ssh `.
pub fn local_prompt_prefix() -> String {
    let user = paths::user_name();
    let full_host = paths::host_name();
    let host = full_host
        .split('.')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("localhost");
    format!("[{user}@{host} ~]$ ssh ")
}

// ---------------------------------------------------------------------------
// Persistent Host History & Completion Store
// ---------------------------------------------------------------------------

pub struct HostStore {
    list: RefCell<Vec<Connection>>,
    cmd_history: RefCell<Vec<String>>,
}

impl HostStore {
    pub fn new() -> Rc<Self> {
        let loaded = load_connections();
        let history: Vec<String> = loaded.iter().map(Connection::subtitle).collect();
        Rc::new(HostStore {
            list: RefCell::new(loaded),
            cmd_history: RefCell::new(history),
        })
    }

    #[cfg(test)]
    fn from_list(list: Vec<Connection>) -> Self {
        let history = list.iter().map(Connection::subtitle).collect();
        HostStore {
            list: RefCell::new(list),
            cmd_history: RefCell::new(history),
        }
    }

    pub fn resolve(&self, input: &str) -> Option<ParsedSsh> {
        let mut parsed = parse_ssh_command(input)?;
        if parsed.extra_args.is_empty() {
            for saved in self.list.borrow().iter() {
                let alias_match = !saved.name.trim().is_empty()
                    && parsed.conn.user.is_empty()
                    && parsed.conn.port.is_none()
                    && saved.name.trim().eq_ignore_ascii_case(&parsed.conn.host);
                let host_match = saved.host.eq_ignore_ascii_case(&parsed.conn.host)
                    && (saved.user.eq_ignore_ascii_case(&parsed.conn.user)
                        || (parsed.conn.user.is_empty() && alias_match));
                if alias_match {
                    let mut merged = saved.clone();
                    if let Some(p) = parsed.conn.port {
                        merged.port = Some(p);
                    }
                    if !parsed.conn.identity_file.is_empty() {
                        merged.identity_file = parsed.conn.identity_file.clone();
                    }
                    parsed.conn = merged;
                    break;
                } else if host_match {
                    if parsed.conn.port.is_none() {
                        parsed.conn.port = saved.port;
                    }
                    if parsed.conn.identity_file.is_empty() {
                        parsed.conn.identity_file = saved.identity_file.clone();
                    }
                    if parsed.conn.initial_dir.is_empty() {
                        parsed.conn.initial_dir = saved.initial_dir.clone();
                    }
                    if parsed.conn.name.is_empty() {
                        parsed.conn.name = saved.name.clone();
                    }
                    break;
                }
            }
        }
        Some(parsed)
    }

    pub fn record(&self, parsed: &ParsedSsh) {
        {
            let mut hist = self.cmd_history.borrow_mut();
            let raw = parsed.raw_command.trim().to_string();
            if !raw.is_empty() {
                hist.retain(|h| h != &raw);
                hist.insert(0, raw);
                hist.truncate(MAX_RECENT_HOSTS);
            }
        }

        let mut list = self.list.borrow_mut();
        let mut entry = parsed.conn.clone();
        if let Some(pos) = list.iter().position(|c| {
            c.host.eq_ignore_ascii_case(&entry.host)
                && c.user.eq_ignore_ascii_case(&entry.user)
                && c.port == entry.port
        }) {
            let prev = list.remove(pos);
            if entry.id == 0 {
                entry.id = prev.id;
            }
            if entry.name.is_empty() {
                entry.name = prev.name;
            }
            if entry.identity_file.is_empty() {
                entry.identity_file = prev.identity_file;
            }
            if entry.initial_dir.is_empty() {
                entry.initial_dir = prev.initial_dir;
            }
        } else if entry.id == 0 {
            entry.id = new_id();
        }
        list.insert(0, entry);
        list.truncate(MAX_RECENT_HOSTS);
        let _ = save_connections(&list);
    }

    pub fn history(&self) -> Vec<String> {
        self.cmd_history.borrow().clone()
    }

    /// Returns ranked completion candidates for the target token being typed.
    pub fn completions(&self, token: &str) -> Vec<String> {
        self.completions_with_ssh_hosts(token, &ssh_config_hosts())
    }

    fn completions_with_ssh_hosts(&self, token: &str, ssh_hosts: &[String]) -> Vec<String> {
        let q = token.trim();
        let q_low = q.to_lowercase();
        let user_prefix = q.rsplit_once('@').map(|(u, h)| (format!("{u}@"), h.to_lowercase()));

        let mut raw_candidates: Vec<String> = Vec::new();
        for c in self.list.borrow().iter() {
            let sub = c.subtitle();
            if !sub.is_empty() {
                raw_candidates.push(sub);
            }
            let alias = c.name.trim();
            if !alias.is_empty() && !alias.contains(char::is_whitespace) {
                raw_candidates.push(alias.to_string());
            }
            if let Some((ref uprefix, _)) = user_prefix
                && !c.host.is_empty()
            {
                let mut with_user = format!("{uprefix}{}", c.host);
                if let Some(p) = c.port.filter(|&p| p != 22) {
                    with_user.push_str(&format!(":{p}"));
                }
                raw_candidates.push(with_user);
            }
        }

        for host in ssh_hosts {
            if let Some((ref uprefix, _)) = user_prefix {
                raw_candidates.push(format!("{uprefix}{host}"));
            } else {
                raw_candidates.push(host.clone());
            }
        }

        let mut prefix_hits: Vec<String> = Vec::new();
        let mut substr_hits: Vec<String> = Vec::new();

        for cand in raw_candidates {
            let cand_low = cand.to_lowercase();
            let already = |v: &[String]| v.iter().any(|x| x.eq_ignore_ascii_case(&cand));
            if already(&prefix_hits) || already(&substr_hits) {
                continue;
            }
            if q_low.is_empty() || cand_low.starts_with(&q_low) {
                prefix_hits.push(cand);
            } else if let Some((ref uprefix, ref h_low)) = user_prefix {
                let uprefix_low = uprefix.to_lowercase();
                if cand_low.starts_with(&uprefix_low)
                    && let Some((_, cand_h)) = cand_low.rsplit_once('@')
                    && cand_h.contains(h_low.as_str())
                {
                    substr_hits.push(cand);
                }
            } else if cand_low.contains(&q_low) {
                substr_hits.push(cand);
            }
        }

        prefix_hits.extend(substr_hits);
        prefix_hits.truncate(8);
        prefix_hits
    }
}

fn config_file() -> PathBuf {
    paths::config_dir().join("riso").join("connections.json")
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
    let path = paths::home_dir().join(".ssh").join("config");
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
        assert_eq!(
            (c.user.as_str(), c.host.as_str(), c.port),
            ("leo", "example.com", Some(2222))
        );
        let c = Connection::parse_quick("ssh myhost").unwrap();
        assert_eq!((c.user.as_str(), c.host.as_str(), c.port), ("", "myhost", None));
        let c = Connection::parse_quick("root@[::1]:22").unwrap();
        assert_eq!((c.host.as_str(), c.port), ("::1", Some(22)));
        let c = Connection::parse_quick("fe80::1").unwrap();
        assert_eq!(c.host, "fe80::1");
        assert!(Connection::parse_quick("").is_none());
        assert!(Connection::parse_quick("host:abc").is_none());
        assert!(Connection::parse_quick("-oProxyCommand=evil").is_none());
    }

    #[test]
    fn ssh_command_args_parsing() {
        let p = parse_ssh_command("leo@archlinux").unwrap();
        assert_eq!(p.conn.user, "leo");
        assert_eq!(p.conn.host, "archlinux");
        assert_eq!(p.conn.port, None);
        assert!(p.extra_args.is_empty());

        let p = parse_ssh_command("-p 2222 -i ~/.ssh/id_ed25519 leo@server").unwrap();
        assert_eq!(p.conn.user, "leo");
        assert_eq!(p.conn.host, "server");
        assert_eq!(p.conn.port, Some(2222));
        assert_eq!(p.conn.identity_file, "~/.ssh/id_ed25519");
        assert!(p.extra_args.is_empty());

        let p = parse_ssh_command("-p2200 -l root -J bastion 10.0.0.5").unwrap();
        assert_eq!(p.conn.user, "root");
        assert_eq!(p.conn.host, "10.0.0.5");
        assert_eq!(p.conn.port, Some(2200));
        assert_eq!(p.extra_args, vec!["-J", "bastion"]);

        let p = parse_ssh_command("ssh utente1@hostname1").unwrap();
        assert_eq!(p.conn.subtitle(), "utente1@hostname1");

        assert!(parse_ssh_command("").is_none());
        assert!(parse_ssh_command("-p").is_none());
        assert!(parse_ssh_command("-v").is_none());
    }

    #[test]
    fn host_completions_match_prefix_and_user() {
        let store = HostStore::from_list(vec![Connection {
            user: "utente1".into(),
            host: "hostname1".into(),
            ..Default::default()
        }]);
        let ssh_hosts = vec!["hostname1".to_string(), "hostname2".to_string()];

        let hits = store.completions_with_ssh_hosts("ho", &ssh_hosts);
        assert_eq!(hits[0], "hostname1");
        assert_eq!(hits[1], "hostname2");

        let hits_user = store.completions_with_ssh_hosts("leo@ho", &ssh_hosts);
        assert_eq!(hits_user, vec!["leo@hostname1", "leo@hostname2"]);
    }
}
