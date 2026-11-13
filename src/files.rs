//! Frontend-agnostic model of remote directory entries and file helpers
//! shared by `riso-gui` and `riso-tui`.

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::SystemTime;

use crate::sftp::{self, Sftp};

/// A single row of a remote directory listing.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
    pub mtime: u32,
    pub perms: u32,
}

impl Entry {
    pub fn new(dir: &str, e: &sftp::DirEntry) -> Self {
        Entry {
            name: e.name.clone(),
            path: sftp::join(dir, &e.name),
            is_dir: e.attrs.is_dir(),
            is_link: e.attrs.is_symlink(),
            size: e.attrs.size.unwrap_or(0),
            mtime: e.attrs.mtime.unwrap_or(0),
            perms: e.attrs.permissions.unwrap_or(0),
        }
    }

    /// `ls -l` style permission string, e.g. `drwxr-xr-x`.
    pub fn perm_string(&self) -> String {
        perm_string(self.perms, self.is_dir, self.is_link)
    }
}

pub fn perm_string(mode: u32, is_dir: bool, is_link: bool) -> String {
    let mut s = String::with_capacity(10);
    s.push(if is_link || mode & 0o170000 == 0o120000 {
        'l'
    } else if is_dir {
        'd'
    } else {
        '-'
    });
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 7;
        s.push(if bits & 4 != 0 { 'r' } else { '-' });
        s.push(if bits & 2 != 0 { 'w' } else { '-' });
        s.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    s
}

/// Directories first, then case-insensitive by name.
pub fn sort_entries(v: &mut [Entry]) {
    v.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Returns a remote path inside `dir` that does not exist yet, appending
/// ` (copia)`, ` (copia 2)`, … to `name` when needed.
pub fn unique_destination(s: &mut Sftp, dir: &str, name: &str) -> io::Result<String> {
    let candidate = sftp::join(dir, name);
    if !s.exists(&candidate)? {
        return Ok(candidate);
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 1.. {
        let suffix = if n == 1 {
            " (copia)".to_string()
        } else {
            format!(" (copia {n})")
        };
        let candidate = sftp::join(dir, &format!("{stem}{suffix}{ext}"));
        if !s.exists(&candidate)? {
            return Ok(candidate);
        }
    }
    unreachable!()
}

/// Identity of a local file version, used to detect saves (including atomic
/// replace-by-rename, which changes the inode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    mtime: Option<SystemTime>,
    len: u64,
    ino: u64,
}

impl FileStamp {
    /// Stamp of a regular file, `None` if missing or not a file.
    pub fn of(path: &Path) -> Option<Self> {
        let m = fs::metadata(path).ok()?;
        if !m.is_file() {
            return None;
        }
        Some(FileStamp {
            mtime: m.modified().ok(),
            len: m.len(),
            ino: m.ino(),
        })
    }
}

/// Human readable size with SI units, e.g. `512 B`, `1.2 kB`, `3.4 MB`.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub fn human_size(n: u64) -> String {
    const UNITS: [&str; 6] = ["kB", "MB", "GB", "TB", "PB", "EB"];
    if n < 1000 {
        return format!("{n} B");
    }
    let mut v = n as f64 / 1000.0;
    let mut unit = 0;
    while v >= 1000.0 && unit < UNITS.len() - 1 {
        v /= 1000.0;
        unit += 1;
    }
    format!("{v:.1} {}", UNITS[unit])
}

/// Recursive size of a local file or directory (symlinks are not followed).
pub fn local_size(path: &Path) -> u64 {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::read_dir(path)
            .map(|rd| rd.flatten().map(|e| local_size(&e.path())).sum())
            .unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perm_string_formats_modes() {
        assert_eq!(perm_string(0o755, true, false), "drwxr-xr-x");
        assert_eq!(perm_string(0o644, false, false), "-rw-r--r--");
        assert_eq!(perm_string(0o120777, false, false), "lrwxrwxrwx");
    }

    #[test]
    fn human_size_uses_si_units() {
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(1200), "1.2 kB");
        assert_eq!(human_size(3_400_000), "3.4 MB");
    }
}
