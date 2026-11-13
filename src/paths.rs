//! Frontend-agnostic XDG / user directory and identity helpers.
//!
//! These replace the `glib::*_dir()` helpers in shared code so that
//! `riso-tui` does not have to link against GLib.

use std::ffi::CStr;
use std::path::PathBuf;

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

fn passwd_entry() -> Option<(String, PathBuf)> {
    // SAFETY: getpwuid returns a pointer to static storage or NULL; the
    // fields are copied out immediately.
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() {
            return None;
        }
        let name = CStr::from_ptr((*pw).pw_name).to_string_lossy().into_owned();
        let dir = CStr::from_ptr((*pw).pw_dir).to_string_lossy().into_owned();
        Some((name, PathBuf::from(dir)))
    }
}

/// The user's home directory (`$HOME`, falling back to the passwd entry).
pub fn home_dir() -> PathBuf {
    env_path("HOME")
        .or_else(|| passwd_entry().map(|(_, d)| d))
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// `$XDG_CONFIG_HOME` or `~/.config`.
pub fn config_dir() -> PathBuf {
    env_path("XDG_CONFIG_HOME").unwrap_or_else(|| home_dir().join(".config"))
}

/// `$XDG_CACHE_HOME` or `~/.cache`.
pub fn cache_dir() -> PathBuf {
    env_path("XDG_CACHE_HOME").unwrap_or_else(|| home_dir().join(".cache"))
}

/// `$XDG_RUNTIME_DIR`, falling back to the cache directory (like GLib).
pub fn runtime_dir() -> PathBuf {
    env_path("XDG_RUNTIME_DIR").unwrap_or_else(cache_dir)
}

/// Login name of the current user.
pub fn user_name() -> String {
    passwd_entry()
        .map(|(n, _)| n)
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "user".to_string())
}

/// Host name of this machine (may be fully qualified).
pub fn host_name() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: buf is valid for buf.len() bytes; gethostname NUL-terminates
    // on success when the name fits.
    let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
    if ok {
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        let name = String::from_utf8_lossy(&buf[..end]).into_owned();
        if !name.is_empty() {
            return name;
        }
    }
    "localhost".to_string()
}
