//! TUI: drag-and-drop helpers — parsing dropped local file paths / `file://`
//! URIs received via bracketed paste, and tracking in-app mouse drags from
//! the SFTP explorer.

use std::path::PathBuf;

use super::term::percent_decode;
use crate::config::split_shell_words;
use crate::files::Entry;
use crate::paths;

#[derive(Debug, Clone)]
pub struct DroppedFiles {
    pub paths: Vec<PathBuf>,
    /// True when the payload used `file://` URIs (unambiguously a file drop).
    pub is_uri: bool,
}

/// Active mouse drag initiated from an Explorer row.
#[derive(Clone)]
pub struct DragSession {
    pub source_session_id: u64,
    pub entries: Vec<Entry>,
}

/// Parses a bracketed-paste payload into existing local filesystem paths if
/// every non-empty item in the payload refers to an existing local path.
pub fn parse_dropped_paths(raw: &str) -> Option<DroppedFiles> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 1. `text/uri-list` (`file:///...` lines or whitespace-separated URIs).
    if trimmed.contains("file://") {
        let mut paths = Vec::new();
        for line in trimmed.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            for token in line.split_whitespace() {
                let p = parse_file_uri(token)?;
                if !p.exists() {
                    return None;
                }
                paths.push(p);
            }
        }
        return (!paths.is_empty()).then_some(DroppedFiles { paths, is_uri: true });
    }

    // 2. Shell-quoted or escaped paths (e.g. `'/home/user/my file'` or `/home/user/my\ file`).
    if !trimmed.contains('\n')
        && let Some(words) = split_shell_words(trimmed)
        && !words.is_empty()
        && words.iter().all(|w| looks_like_local_path(w))
    {
        let paths: Vec<PathBuf> = words.into_iter().map(expand_path).collect();
        if paths.iter().all(|p| p.exists()) {
            return Some(DroppedFiles {
                paths,
                is_uri: false,
            });
        }
    }

    // 3. Newline-separated raw paths.
    let mut paths = Vec::new();
    for line in trimmed.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        let clean = unquote_simple(l);
        if !looks_like_local_path(clean) {
            return None;
        }
        let p = expand_path(clean.to_string());
        if !p.exists() {
            return None;
        }
        paths.push(p);
    }

    (!paths.is_empty()).then_some(DroppedFiles {
        paths,
        is_uri: false,
    })
}

/// Parses a user-entered path list (used in the Upload dialog; also accepts
/// `file://` URIs and shell-quoted lists).
pub fn parse_user_path_list(raw: &str) -> Vec<PathBuf> {
    if let Some(d) = parse_dropped_paths(raw) {
        return d.paths;
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if let Some(words) = split_shell_words(trimmed) {
        return words.into_iter().map(expand_path).collect();
    }
    vec![expand_path(trimmed.to_string())]
}

pub fn expand_path(s: String) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        paths::home_dir().join(rest)
    } else if s == "~" {
        paths::home_dir()
    } else {
        PathBuf::from(s)
    }
}

fn parse_file_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // `file:///abs/path` or `file://localhost/abs/path`
    let path_part = if rest.starts_with('/') {
        rest
    } else {
        let (host, _p) = rest.split_once('/')?;
        if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
            return None;
        }
        // Keep the leading `/`
        &rest[host.len()..]
    };
    let decoded = percent_decode(path_part);
    Some(PathBuf::from(decoded))
}

fn looks_like_local_path(s: &str) -> bool {
    s.starts_with('/') || s.starts_with("~/") || s.starts_with("./") || s.starts_with("../")
}

fn unquote_simple(s: &str) -> &str {
    if ((s.starts_with('\'') && s.ends_with('\'')) || (s.starts_with('"') && s.ends_with('"')))
        && s.len() >= 2
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_file_uris_and_shell_paths() {
        let dir = std::env::temp_dir().join(format!("riso-drop-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f1 = dir.join("hello world.txt");
        let f2 = dir.join("plain.txt");
        std::fs::write(&f1, b"a").unwrap();
        std::fs::write(&f2, b"b").unwrap();

        let uri = format!(
            "file://{}\r\nfile://localhost{}",
            f1.display().to_string().replace(' ', "%20"),
            f2.display()
        );
        let parsed = parse_dropped_paths(&uri).unwrap();
        assert!(parsed.is_uri);
        assert_eq!(parsed.paths, vec![f1.clone(), f2.clone()]);

        let quoted = format!("'{}' '{}' ", f1.display(), f2.display());
        let parsed = parse_dropped_paths(&quoted).unwrap();
        assert!(!parsed.is_uri);
        assert_eq!(parsed.paths, vec![f1.clone(), f2.clone()]);

        assert!(parse_dropped_paths("ssh leo@host").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
