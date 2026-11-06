//! Minimal synchronous SFTP v3 client (draft-ietf-secsh-filexfer-02).
//!
//! The client talks the SFTP protocol over any byte stream; RISO uses the
//! stdin/stdout of an `ssh -s ... sftp` child process that is multiplexed over
//! the control master created by the terminal session, so authentication only
//! happens once (in the terminal) and works for passwords, keys, 2FA, etc.

use std::collections::VecDeque;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::FileExt;
use std::path::Path;

// Packet types
const FXP_INIT: u8 = 1;
const FXP_VERSION: u8 = 2;
const FXP_OPEN: u8 = 3;
const FXP_CLOSE: u8 = 4;
const FXP_READ: u8 = 5;
const FXP_WRITE: u8 = 6;
const FXP_LSTAT: u8 = 7;
const FXP_SETSTAT: u8 = 9;
const FXP_OPENDIR: u8 = 11;
const FXP_READDIR: u8 = 12;
const FXP_REMOVE: u8 = 13;
const FXP_MKDIR: u8 = 14;
const FXP_RMDIR: u8 = 15;
const FXP_REALPATH: u8 = 16;
const FXP_STAT: u8 = 17;
const FXP_RENAME: u8 = 18;
const FXP_STATUS: u8 = 101;
const FXP_HANDLE: u8 = 102;
const FXP_DATA: u8 = 103;
const FXP_NAME: u8 = 104;
const FXP_ATTRS: u8 = 105;

// Attribute flags
const ATTR_SIZE: u32 = 0x1;
const ATTR_UIDGID: u32 = 0x2;
const ATTR_PERMISSIONS: u32 = 0x4;
const ATTR_ACMODTIME: u32 = 0x8;
const ATTR_EXTENDED: u32 = 0x8000_0000;

// Open flags
const OPEN_READ: u32 = 0x1;
const OPEN_WRITE: u32 = 0x2;
const OPEN_CREAT: u32 = 0x8;
const OPEN_TRUNC: u32 = 0x10;
const OPEN_EXCL: u32 = 0x20;

// Status codes
const FX_OK: u32 = 0;
const FX_EOF: u32 = 1;
const FX_NO_SUCH_FILE: u32 = 2;
const FX_PERMISSION_DENIED: u32 = 3;

const CHUNK: u32 = 32 * 1024;
const MAX_IN_FLIGHT: usize = 16;

const S_IFMT: u32 = 0o170000;
const S_IFDIR: u32 = 0o040000;
const S_IFLNK: u32 = 0o120000;

#[derive(Debug, Clone, Default)]
pub struct Attrs {
    pub size: Option<u64>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub permissions: Option<u32>,
    pub atime: Option<u32>,
    pub mtime: Option<u32>,
}

impl Attrs {
    pub fn is_dir(&self) -> bool {
        self.permissions.is_some_and(|p| p & S_IFMT == S_IFDIR)
    }
    pub fn is_symlink(&self) -> bool {
        self.permissions.is_some_and(|p| p & S_IFMT == S_IFLNK)
    }
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub attrs: Attrs,
}

/// Progress callback: receives the number of bytes just transferred.
pub type Progress<'a> = &'a mut dyn FnMut(u64);

pub struct Sftp {
    writer: Box<dyn Write + Send>,
    reader: Box<dyn Read + Send>,
    next_id: u32,
}

// ---------------------------------------------------------------------------
// Wire encoding helpers
// ---------------------------------------------------------------------------

struct Buf(Vec<u8>);

impl Buf {
    fn new(kind: u8) -> Self {
        // 4 bytes placeholder for the length
        Buf(vec![0, 0, 0, 0, kind])
    }
    fn u32(mut self, v: u32) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn u64(mut self, v: u64) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn bytes(mut self, v: &[u8]) -> Self {
        self = self.u32(v.len() as u32);
        self.0.extend_from_slice(v);
        self
    }
    fn str(self, v: &str) -> Self {
        self.bytes(v.as_bytes())
    }
    fn attrs(mut self, a: &Attrs) -> Self {
        let mut flags = 0;
        if a.size.is_some() {
            flags |= ATTR_SIZE;
        }
        if a.uid.is_some() && a.gid.is_some() {
            flags |= ATTR_UIDGID;
        }
        if a.permissions.is_some() {
            flags |= ATTR_PERMISSIONS;
        }
        if a.atime.is_some() && a.mtime.is_some() {
            flags |= ATTR_ACMODTIME;
        }
        self = self.u32(flags);
        if let Some(s) = a.size {
            self = self.u64(s);
        }
        if let (Some(u), Some(g)) = (a.uid, a.gid) {
            self = self.u32(u).u32(g);
        }
        if let Some(p) = a.permissions {
            self = self.u32(p);
        }
        if let (Some(at), Some(mt)) = (a.atime, a.mtime) {
            self = self.u32(at).u32(mt);
        }
        self
    }
    fn finish(mut self) -> Vec<u8> {
        let len = (self.0.len() - 4) as u32;
        self.0[..4].copy_from_slice(&len.to_be_bytes());
        self.0
    }
}

struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if self.data.len() < n {
            return Err(proto_err("pacchetto SFTP troncato"));
        }
        let (a, b) = self.data.split_at(n);
        self.data = b;
        Ok(a)
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bytes(&mut self) -> io::Result<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    fn string(&mut self) -> io::Result<String> {
        Ok(String::from_utf8_lossy(self.bytes()?).into_owned())
    }
    fn attrs(&mut self) -> io::Result<Attrs> {
        let flags = self.u32()?;
        let mut a = Attrs::default();
        if flags & ATTR_SIZE != 0 {
            a.size = Some(self.u64()?);
        }
        if flags & ATTR_UIDGID != 0 {
            a.uid = Some(self.u32()?);
            a.gid = Some(self.u32()?);
        }
        if flags & ATTR_PERMISSIONS != 0 {
            a.permissions = Some(self.u32()?);
        }
        if flags & ATTR_ACMODTIME != 0 {
            a.atime = Some(self.u32()?);
            a.mtime = Some(self.u32()?);
        }
        if flags & ATTR_EXTENDED != 0 {
            let count = self.u32()?;
            for _ in 0..count {
                self.bytes()?;
                self.bytes()?;
            }
        }
        Ok(a)
    }
}

fn proto_err(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

fn status_error(code: u32, msg: String) -> io::Error {
    let kind = match code {
        FX_NO_SUCH_FILE => io::ErrorKind::NotFound,
        FX_PERMISSION_DENIED => io::ErrorKind::PermissionDenied,
        FX_EOF => io::ErrorKind::UnexpectedEof,
        _ => io::ErrorKind::Other,
    };
    let msg = match (code, msg.is_empty()) {
        (FX_NO_SUCH_FILE, _) => "File o directory inesistente".to_string(),
        (FX_PERMISSION_DENIED, _) => "Permesso negato".to_string(),
        (_, false) => msg,
        (_, true) => format!("Errore SFTP (codice {code})"),
    };
    io::Error::new(kind, msg)
}

/// A decoded response packet.
struct Packet {
    kind: u8,
    id: u32,
    body: Vec<u8>,
}

impl Packet {
    fn reader(&self) -> Reader<'_> {
        Reader { data: &self.body }
    }

    /// Converts a STATUS packet into `Ok(())` / `Err`.
    fn status(&self) -> io::Result<()> {
        if self.kind != FXP_STATUS {
            return Err(proto_err("risposta SFTP inattesa"));
        }
        let mut r = self.reader();
        let code = r.u32()?;
        let msg = r.string().unwrap_or_default();
        if code == FX_OK {
            Ok(())
        } else {
            Err(status_error(code, msg))
        }
    }

    fn expect(&self, kind: u8) -> io::Result<Reader<'_>> {
        if self.kind == kind {
            Ok(self.reader())
        } else if self.kind == FXP_STATUS {
            self.status()?;
            Err(proto_err("risposta SFTP inattesa"))
        } else {
            Err(proto_err("risposta SFTP inattesa"))
        }
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

impl Sftp {
    /// Performs the SFTP handshake over the given stream.
    pub fn new(writer: impl Write + Send + 'static, reader: impl Read + Send + 'static) -> io::Result<Self> {
        let mut s = Sftp {
            writer: Box::new(writer),
            reader: Box::new(reader),
            next_id: 1,
        };
        let init = Buf::new(FXP_INIT).u32(3).finish();
        s.writer.write_all(&init)?;
        s.writer.flush()?;
        let (kind, _) = s.read_raw()?;
        if kind != FXP_VERSION {
            return Err(proto_err("handshake SFTP fallito"));
        }
        Ok(s)
    }

    fn read_raw(&mut self) -> io::Result<(u8, Vec<u8>)> {
        let mut len = [0u8; 4];
        self.reader.read_exact(&mut len)?;
        let len = u32::from_be_bytes(len) as usize;
        if len == 0 || len > 4 * 1024 * 1024 {
            return Err(proto_err("lunghezza pacchetto SFTP non valida"));
        }
        let mut data = vec![0u8; len];
        self.reader.read_exact(&mut data)?;
        let kind = data[0];
        data.remove(0);
        Ok((kind, data))
    }

    fn read_packet(&mut self) -> io::Result<Packet> {
        let (kind, mut body) = self.read_raw()?;
        if body.len() < 4 {
            return Err(proto_err("pacchetto SFTP troncato"));
        }
        let id = u32::from_be_bytes(body[..4].try_into().unwrap());
        body.drain(..4);
        Ok(Packet { kind, id, body })
    }

    fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    /// Sends a request (built by `build`, which receives the request id) and
    /// returns its id.
    fn send(&mut self, kind: u8, build: impl FnOnce(Buf) -> Buf) -> io::Result<u32> {
        let id = self.alloc_id();
        let pkt = build(Buf::new(kind).u32(id)).finish();
        self.writer.write_all(&pkt)?;
        self.writer.flush()?;
        Ok(id)
    }

    fn request(&mut self, kind: u8, build: impl FnOnce(Buf) -> Buf) -> io::Result<Packet> {
        let id = self.send(kind, build)?;
        let pkt = self.read_packet()?;
        if pkt.id != id {
            return Err(proto_err("id risposta SFTP non corrispondente"));
        }
        Ok(pkt)
    }

    // -- Basic operations ---------------------------------------------------

    pub fn realpath(&mut self, path: &str) -> io::Result<String> {
        let pkt = self.request(FXP_REALPATH, |b| b.str(path))?;
        let mut r = pkt.expect(FXP_NAME)?;
        let count = r.u32()?;
        if count == 0 {
            return Err(proto_err("realpath vuoto"));
        }
        r.string()
    }

    pub fn stat(&mut self, path: &str) -> io::Result<Attrs> {
        let pkt = self.request(FXP_STAT, |b| b.str(path))?;
        pkt.expect(FXP_ATTRS)?.attrs()
    }

    pub fn lstat(&mut self, path: &str) -> io::Result<Attrs> {
        let pkt = self.request(FXP_LSTAT, |b| b.str(path))?;
        pkt.expect(FXP_ATTRS)?.attrs()
    }

    pub fn exists(&mut self, path: &str) -> io::Result<bool> {
        match self.lstat(path) {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    pub fn setstat(&mut self, path: &str, attrs: &Attrs) -> io::Result<()> {
        self.request(FXP_SETSTAT, |b| b.str(path).attrs(attrs))?.status()
    }

    pub fn mkdir(&mut self, path: &str) -> io::Result<()> {
        self.request(FXP_MKDIR, |b| b.str(path).attrs(&Attrs::default()))?
            .status()
    }

    pub fn rmdir(&mut self, path: &str) -> io::Result<()> {
        self.request(FXP_RMDIR, |b| b.str(path))?.status()
    }

    pub fn remove(&mut self, path: &str) -> io::Result<()> {
        self.request(FXP_REMOVE, |b| b.str(path))?.status()
    }

    pub fn rename(&mut self, from: &str, to: &str) -> io::Result<()> {
        self.request(FXP_RENAME, |b| b.str(from).str(to))?.status()
    }

    /// Creates a new empty file (fails if it already exists).
    pub fn create_file(&mut self, path: &str) -> io::Result<()> {
        let attrs = Attrs {
            permissions: Some(0o644),
            ..Default::default()
        };
        let handle = self.open(path, OPEN_WRITE | OPEN_CREAT | OPEN_EXCL, &attrs)?;
        self.close(&handle)
    }

    fn close(&mut self, handle: &[u8]) -> io::Result<()> {
        self.request(FXP_CLOSE, |b| b.bytes(handle))?.status()
    }

    fn open(&mut self, path: &str, flags: u32, attrs: &Attrs) -> io::Result<Vec<u8>> {
        let pkt = self.request(FXP_OPEN, |b| b.str(path).u32(flags).attrs(attrs))?;
        Ok(pkt.expect(FXP_HANDLE)?.bytes()?.to_vec())
    }

    /// Lists a directory (without `.` and `..`). Symlinks pointing to
    /// directories are reported with directory permissions.
    pub fn readdir(&mut self, path: &str) -> io::Result<Vec<DirEntry>> {
        let pkt = self.request(FXP_OPENDIR, |b| b.str(path))?;
        let handle = pkt.expect(FXP_HANDLE)?.bytes()?.to_vec();
        let mut entries = Vec::new();
        let result = (|| loop {
            let pkt = self.request(FXP_READDIR, |b| b.bytes(&handle))?;
            if pkt.kind == FXP_STATUS {
                let mut r = pkt.reader();
                let code = r.u32()?;
                if code == FX_EOF {
                    return Ok(());
                }
                return Err(status_error(code, r.string().unwrap_or_default()));
            }
            let mut r = pkt.expect(FXP_NAME)?;
            let count = r.u32()?;
            for _ in 0..count {
                let name = r.string()?;
                let _longname = r.bytes()?;
                let attrs = r.attrs()?;
                if name != "." && name != ".." {
                    entries.push(DirEntry { name, attrs });
                }
            }
        })();
        let _ = self.close(&handle);
        result?;

        // Resolve symlinks so that links to directories can be browsed.
        let base = path.trim_end_matches('/');
        for e in entries.iter_mut().filter(|e| e.attrs.is_symlink()) {
            if let Ok(target) = self.stat(&format!("{base}/{}", e.name)) {
                if target.is_dir() {
                    e.attrs.permissions = target.permissions;
                    e.attrs.size = target.size;
                }
            }
        }
        Ok(entries)
    }

    // -- Transfers ----------------------------------------------------------

    /// Downloads a remote file to a local path using pipelined reads.
    pub fn download_file(&mut self, remote: &str, local: &Path, progress: Progress) -> io::Result<()> {
        let attrs = self.stat(remote)?;
        let handle = self.open(remote, OPEN_READ, &Attrs::default())?;
        let result = (|| {
            let file = fs::File::create(local)?;
            let size = attrs.size.unwrap_or(0);
            // Queue of (request id, offset, len) in flight.
            let mut in_flight: VecDeque<(u32, u64, u32)> = VecDeque::new();
            // Pending ranges to request (used for short reads).
            let mut retry: VecDeque<(u64, u32)> = VecDeque::new();
            let mut next_offset = 0u64;
            let mut eof = false;
            loop {
                while in_flight.len() < MAX_IN_FLIGHT && (!retry.is_empty() || !eof) {
                    let (off, len) = if let Some(r) = retry.pop_front() {
                        r
                    } else {
                        // Keep requesting past the known size until EOF in
                        // case the file grows; stop eagerly when the size is
                        // known and reached.
                        if next_offset >= size && !in_flight.is_empty() {
                            break;
                        }
                        let r = (next_offset, CHUNK);
                        next_offset += CHUNK as u64;
                        r
                    };
                    let id = self.send(FXP_READ, |b| b.bytes(&handle).u64(off).u32(len))?;
                    in_flight.push_back((id, off, len));
                }
                let Some((id, off, len)) = in_flight.pop_front() else {
                    break;
                };
                let pkt = self.read_packet()?;
                if pkt.id != id {
                    return Err(proto_err("id risposta SFTP non corrispondente"));
                }
                if pkt.kind == FXP_STATUS {
                    let mut r = pkt.reader();
                    let code = r.u32()?;
                    if code == FX_EOF {
                        eof = true;
                        continue;
                    }
                    return Err(status_error(code, r.string().unwrap_or_default()));
                }
                let data = pkt.expect(FXP_DATA)?.bytes()?;
                file.write_all_at(data, off)?;
                progress(data.len() as u64);
                if data.is_empty() {
                    eof = true;
                } else if (data.len() as u32) < len {
                    retry.push_back((off + data.len() as u64, len - data.len() as u32));
                }
            }
            file.sync_all()?;
            Ok(())
        })();
        let _ = self.close(&handle);
        result
    }

    /// Uploads a local file to a remote path using pipelined writes.
    pub fn upload_file(&mut self, local: &Path, remote: &str, progress: Progress) -> io::Result<()> {
        let mut file = fs::File::open(local)?;
        let meta = file.metadata()?;
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            meta.permissions().mode() & 0o777
        };
        let attrs = Attrs {
            permissions: Some(mode),
            ..Default::default()
        };
        let handle = self.open(remote, OPEN_WRITE | OPEN_CREAT | OPEN_TRUNC, &attrs)?;
        let result = (|| {
            let mut in_flight: VecDeque<(u32, u64)> = VecDeque::new();
            let mut offset = 0u64;
            let mut buf = vec![0u8; CHUNK as usize];
            let mut done = false;
            loop {
                while !done && in_flight.len() < MAX_IN_FLIGHT {
                    let n = read_full(&mut file, &mut buf)?;
                    if n == 0 {
                        done = true;
                        break;
                    }
                    let id = self.send(FXP_WRITE, |b| b.bytes(&handle).u64(offset).bytes(&buf[..n]))?;
                    in_flight.push_back((id, n as u64));
                    offset += n as u64;
                }
                let Some((id, n)) = in_flight.pop_front() else {
                    break;
                };
                let pkt = self.read_packet()?;
                if pkt.id != id {
                    return Err(proto_err("id risposta SFTP non corrispondente"));
                }
                pkt.status()?;
                progress(n);
            }
            Ok(())
        })();
        let _ = self.close(&handle);
        result
    }

    /// Copies a remote file to another remote path (streamed through the
    /// client, as SFTP v3 has no server side copy).
    pub fn copy_file(&mut self, from: &str, to: &str, progress: Progress) -> io::Result<()> {
        let tmp = std::env::temp_dir().join(format!(
            "riso-copy-{}-{}",
            std::process::id(),
            self.alloc_id()
        ));
        let result = self
            .download_file(from, &tmp, &mut |_| {})
            .and_then(|_| self.upload_file(&tmp, to, progress));
        let _ = fs::remove_file(&tmp);
        if result.is_ok() {
            if let Ok(a) = self.stat(from) {
                let _ = self.setstat(to, &Attrs { permissions: a.permissions.map(|p| p & 0o7777), ..Default::default() });
            }
        }
        result
    }

    // -- Recursive helpers --------------------------------------------------

    /// Total size in bytes of a remote path (recursive).
    pub fn total_size(&mut self, path: &str) -> io::Result<u64> {
        let a = self.lstat(path)?;
        if a.is_dir() {
            let mut sum = 0;
            for e in self.readdir(path)? {
                if e.attrs.is_symlink() {
                    continue;
                }
                sum += self.total_size(&join(path, &e.name))?;
            }
            Ok(sum)
        } else {
            Ok(a.size.unwrap_or(0))
        }
    }

    pub fn remove_recursive(&mut self, path: &str) -> io::Result<()> {
        let a = self.lstat(path)?;
        if a.is_dir() {
            for e in self.readdir(path)? {
                let child = join(path, &e.name);
                if e.attrs.is_symlink() {
                    self.remove(&child)?;
                } else {
                    self.remove_recursive(&child)?;
                }
            }
            self.rmdir(path)
        } else {
            self.remove(path)
        }
    }

    pub fn download_recursive(&mut self, remote: &str, local: &Path, progress: Progress) -> io::Result<()> {
        let a = self.stat(remote)?;
        if a.is_dir() {
            fs::create_dir_all(local)?;
            for e in self.readdir(remote)? {
                self.download_recursive(&join(remote, &e.name), &local.join(&e.name), progress)?;
            }
            Ok(())
        } else {
            self.download_file(remote, local, progress)
        }
    }

    pub fn upload_recursive(&mut self, local: &Path, remote: &str, progress: Progress) -> io::Result<()> {
        let meta = fs::metadata(local)?;
        if meta.is_dir() {
            if !self.exists(remote)? {
                self.mkdir(remote)?;
            }
            for entry in fs::read_dir(local)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                self.upload_recursive(&entry.path(), &join(remote, &name), progress)?;
            }
            Ok(())
        } else {
            self.upload_file(local, remote, progress)
        }
    }

    pub fn copy_recursive(&mut self, from: &str, to: &str, progress: Progress) -> io::Result<()> {
        let a = self.stat(from)?;
        if a.is_dir() {
            if to == from || to.starts_with(&format!("{}/", from.trim_end_matches('/'))) {
                return Err(io::Error::other("Impossibile copiare una cartella dentro sé stessa"));
            }
            if !self.exists(to)? {
                self.mkdir(to)?;
            }
            for e in self.readdir(from)? {
                self.copy_recursive(&join(from, &e.name), &join(to, &e.name), progress)?;
            }
            Ok(())
        } else {
            self.copy_file(from, to, progress)
        }
    }
}

fn read_full(file: &mut fs::File, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match file.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// Joins a remote directory path and a name.
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Returns the parent of a remote path (`/` for top-level entries).
pub fn parent(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => trimmed[..i].to_string(),
    }
}

/// Returns the final component of a remote path.
pub fn basename(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(i) => trimmed[i + 1..].to_string(),
        None => trimmed.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_helpers() {
        assert_eq!(join("/", "etc"), "/etc");
        assert_eq!(join("/home/u", "a"), "/home/u/a");
        assert_eq!(parent("/home/u"), "/home");
        assert_eq!(parent("/home"), "/");
        assert_eq!(parent("/"), "/");
        assert_eq!(basename("/home/u/file.txt"), "file.txt");
        assert_eq!(basename("/home/u/"), "u");
    }

    #[test]
    fn attrs_roundtrip() {
        let a = Attrs {
            size: Some(42),
            uid: Some(1),
            gid: Some(2),
            permissions: Some(0o100644),
            atime: Some(10),
            mtime: Some(20),
        };
        let pkt = Buf::new(0).attrs(&a).finish();
        let mut r = Reader { data: &pkt[5..] };
        let b = r.attrs().unwrap();
        assert_eq!(b.size, Some(42));
        assert_eq!(b.permissions, Some(0o100644));
        assert_eq!(b.mtime, Some(20));
        assert!(!b.is_dir());
    }

    #[test]
    fn openssh_sftp_server_roundtrip() {
        let server_bin = Path::new("/usr/lib/ssh/sftp-server");
        if !server_bin.is_file() {
            return;
        }
        let mut child = std::process::Command::new(server_bin)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn sftp-server");

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut sftp = Sftp::new(stdin, std::io::BufReader::new(stdout)).expect("sftp handshake");

        let work = std::env::temp_dir().join(format!("riso-sftp-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&work);
        fs::create_dir_all(&work).unwrap();
        let work_str = work.to_str().unwrap();

        let real = sftp.realpath(work_str).unwrap();
        assert!(!real.is_empty());

        let sub = join(&real, "cartella");
        sftp.mkdir(&sub).unwrap();
        assert!(sftp.stat(&sub).unwrap().is_dir());

        let empty_file = join(&sub, "vuoto.txt");
        sftp.create_file(&empty_file).unwrap();
        assert!(sftp.exists(&empty_file).unwrap());

        // Multi-chunk pipelined upload & download (~300 KB)
        let src_local = work.join("local_src.bin");
        let dst_local = work.join("local_dst.bin");
        let payload: Vec<u8> = (0..300_000u32).map(|i| (i.wrapping_mul(31) & 0xff) as u8).collect();
        fs::write(&src_local, &payload).unwrap();

        let remote_bin = join(&sub, "dati.bin");
        let mut up_bytes = 0u64;
        sftp.upload_file(&src_local, &remote_bin, &mut |n| up_bytes += n)
            .unwrap();
        assert_eq!(up_bytes, payload.len() as u64);

        let mut down_bytes = 0u64;
        sftp.download_file(&remote_bin, &dst_local, &mut |n| down_bytes += n)
            .unwrap();
        assert_eq!(down_bytes, payload.len() as u64);
        assert_eq!(fs::read(&dst_local).unwrap(), payload);

        let entries = sftp.readdir(&sub).unwrap();
        assert_eq!(entries.len(), 2);

        let copy_sub = join(&real, "cartella_copia");
        sftp.copy_recursive(&sub, &copy_sub, &mut |_| {}).unwrap();
        assert_eq!(sftp.total_size(&copy_sub).unwrap(), payload.len() as u64);

        let renamed = join(&real, "cartella_rinominata");
        sftp.rename(&copy_sub, &renamed).unwrap();
        assert!(!sftp.exists(&copy_sub).unwrap());
        assert!(sftp.exists(&renamed).unwrap());

        sftp.remove_recursive(&renamed).unwrap();
        sftp.remove_recursive(&sub).unwrap();
        assert!(!sftp.exists(&sub).unwrap());

        drop(sftp);
        let _ = child.kill();
        let _ = child.wait();
        let _ = fs::remove_dir_all(&work);
    }
}
