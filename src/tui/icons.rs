//! Yazi-inspired file & directory icons for `riso-tui`.
//!
//! Uses Nerd Font glyphs (rendered via `Symbols Nerd Font` or any Nerd Font
//! fallback in modern terminals) paired with the terminal's native 16 ANSI
//! colours. When running inside a bare Linux console (`TERM=linux`), falls back
//! cleanly to plain Unicode symbols.

use std::sync::OnceLock;

use ratatui::style::Style;

use super::theme;
use crate::files::Entry;

fn use_nerd_fonts() -> bool {
    static NERD: OnceLock<bool> = OnceLock::new();
    *NERD.get_or_init(|| std::env::var_os("TERM").is_none_or(|t| t != "linux"))
}

pub struct ToolbarIcons {
    pub dir: &'static str,
    pub back: &'static str,
    pub forward: &'static str,
    pub up: &'static str,
    pub refresh: &'static str,
    pub hidden_on: &'static str,
    pub hidden_off: &'static str,
    pub follow: &'static str,
    pub terminal: &'static str,
    pub download: &'static str,
    pub upload: &'static str,
}

/// Icons for the explorer header path bar and toolbar.
pub fn toolbar() -> ToolbarIcons {
    if use_nerd_fonts() {
        ToolbarIcons {
            dir: "\u{f024b}",        // 󰉋
            back: "\u{f004d}",       // 󰁍
            forward: "\u{f0054}",    // 󰁔
            up: "\u{f005d}",         // 󰁝
            refresh: "\u{f0450}",    // 󰑐
            hidden_on: "\u{f0208}",  // 󰈈
            hidden_off: "\u{f0209}", // 󰈉
            follow: "\u{f04e6}",     // 󰓦
            terminal: "\u{f489}",    // 
            download: "\u{f01da}",   // 󰇚
            upload: "\u{f0552}",     // 󰕒
        }
    } else {
        ToolbarIcons {
            dir: ">",
            back: "<",
            forward: ">",
            up: "^",
            refresh: "R",
            hidden_on: "*",
            hidden_off: ".",
            follow: "~",
            terminal: "cd",
            download: "v",
            upload: "^",
        }
    }
}

/// Returns `(icon, style)` for an explorer entry.
pub fn for_entry(entry: &Entry) -> (&'static str, Style) {
    let nerd = use_nerd_fonts();
    if entry.is_dir {
        let style = if entry.is_link {
            theme::symlink()
        } else {
            theme::directory()
        };
        if !nerd {
            return ("▸", style);
        }
        let glyph = match entry.name.as_str() {
            ".git" => "\u{e702}",
            ".ssh" => "\u{f0306}",
            _ => "\u{f024b}",
        };
        return (glyph, style);
    }

    if entry.is_link {
        return (if nerd { "\u{f0337}" } else { "→" }, theme::symlink());
    }

    if !nerd {
        if entry.perms & 0o111 != 0 {
            return ("*", theme::executable());
        }
        return ("·", if entry.name.starts_with('.') { theme::hidden() } else { theme::base() });
    }

    let lower = entry.name.to_ascii_lowercase();
    if let Some(res) = by_filename(&lower) {
        return res;
    }
    if let Some((_, ext)) = lower.rsplit_once('.')
        && let Some(res) = by_extension(ext)
    {
        return res;
    }

    if entry.perms & 0o111 != 0 {
        return ("\u{f013}", theme::executable()); // 
    }
    if entry.name.starts_with('.') {
        return ("\u{f06d0}", theme::hidden()); // 󰛐
    }
    ("\u{f0214}", theme::base()) // 󰈔
}

fn by_filename(name: &str) -> Option<(&'static str, Style)> {
    match name {
        "cargo.toml" => Some(("\u{e7a8}", theme::config())),
        "cargo.lock" | "package-lock.json" | "yarn.lock" | "pnpm-lock.yaml" | "poetry.lock" => {
            Some(("\u{f033e}", theme::hidden()))
        }
        "dockerfile" | "containerfile" | "docker-compose.yml" | "docker-compose.yaml" => {
            Some(("\u{f308}", theme::code()))
        }
        ".gitignore" | ".gitattributes" | ".gitmodules" => Some(("\u{e702}", theme::hidden())),
        ".bashrc" | ".bash_profile" | ".zshrc" | ".zprofile" | ".profile" => {
            Some(("\u{e795}", theme::executable()))
        }
        ".env" | ".env.local" | ".env.example" | "makefile" | "cmakelists.txt" => {
            Some(("\u{e615}", theme::config()))
        }
        "id_rsa" | "id_ed25519" | "id_ecdsa" | "authorized_keys" | "known_hosts" => {
            Some(("\u{f0306}", theme::config()))
        }
        "readme.md" | "changelog.md" | "license" | "license.md" | "copying" => {
            Some(("\u{f0219}", theme::base()))
        }
        _ => None,
    }
}

fn by_extension(ext: &str) -> Option<(&'static str, Style)> {
    match ext {
        "rs" => Some(("\u{e7a8}", theme::config())),
        "py" | "pyi" | "pyc" => Some(("\u{e73c}", theme::config())),
        "js" | "mjs" | "cjs" | "jsx" => Some(("\u{e74e}", theme::config())),
        "ts" | "mts" | "cts" | "tsx" => Some(("\u{e628}", theme::code())),
        "c" | "h" => Some(("\u{e61e}", theme::code())),
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" => Some(("\u{e61d}", theme::code())),
        "go" => Some(("\u{e627}", theme::code())),
        "sh" | "bash" | "zsh" | "fish" => Some(("\u{e795}", theme::executable())),
        "html" | "htm" => Some(("\u{e736}", theme::config())),
        "css" | "scss" | "sass" | "less" => Some(("\u{e749}", theme::code())),
        "json" | "jsonc" | "jsonl" => Some(("\u{e60b}", theme::config())),
        "toml" => Some(("\u{e6b2}", theme::config())),
        "yaml" | "yml" => Some(("\u{e6a8}", theme::config())),
        "ini" | "conf" | "cfg" | "service" | "timer" | "socket" | "desktop" => {
            Some(("\u{e615}", theme::config()))
        }
        "md" | "markdown" => Some(("\u{e73e}", theme::base())),
        "txt" | "log" | "csv" | "tsv" => Some(("\u{f0219}", theme::base())),
        "pdf" => Some(("\u{f0226}", theme::archive())),
        "db" | "sqlite" | "sqlite3" | "sql" => Some(("\u{f1c0}", theme::code())),
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "lz4" | "7z" | "rar" | "deb"
        | "rpm" | "iso" => Some(("\u{f05c4}", theme::archive())),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico" | "avif" => {
            Some(("\u{f02e9}", theme::media()))
        }
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "flv" => Some(("\u{f0567}", theme::media())),
        "mp3" | "flac" | "wav" | "ogg" | "m4a" | "opus" | "aac" => {
            Some(("\u{f075a}", theme::media()))
        }
        "lock" => Some(("\u{f033e}", theme::hidden())),
        "pem" | "key" | "pub" | "crt" | "cer" | "gpg" | "asc" => {
            Some(("\u{f0306}", theme::config()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    fn dummy(name: &str, is_dir: bool, is_link: bool, perms: u32) -> Entry {
        Entry {
            name: name.to_string(),
            path: format!("/tmp/{name}"),
            is_dir,
            is_link,
            size: 128,
            mtime: 0,
            perms,
        }
    }

    #[test]
    fn nerd_icons_fit_single_cell_and_classify_entries() {
        let dir = dummy("src", true, false, 0o755);
        let rs = dummy("main.rs", false, false, 0o644);
        let zip = dummy("archive.tar.zst", false, false, 0o644);
        let link = dummy("latest", false, true, 0o777);
        let exec = dummy("run", false, false, 0o755);

        for e in [&dir, &rs, &zip, &link, &exec] {
            let (icon, _) = for_entry(e);
            assert!(!icon.is_empty());
            assert!(icon.width() <= 2, "unexpected width for {icon:?}");
        }
        assert_ne!(for_entry(&dir).0, for_entry(&rs).0);
        assert_ne!(for_entry(&rs).0, for_entry(&zip).0);
        assert_ne!(for_entry(&link).0, for_entry(&exec).0);
    }
}
