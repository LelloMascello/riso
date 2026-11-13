//! SSH connection description and background SFTP workers.
//!
//! The terminal runs the system `ssh` binary as an OpenSSH *control master*.
//! Once the user has authenticated in the terminal, multiplexed `ssh -s sftp`
//! channels are opened over the same connection so no second authentication is
//! needed.

use std::cell::RefCell;
use std::io;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use crate::config::{Connection, ParsedSsh};
use crate::paths;
use crate::sftp::Sftp;

static CONTROL_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Everything needed to invoke `ssh` for a given connection.
#[derive(Debug, Clone)]
pub struct SshTarget {
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
    pub extra_args: Vec<String>,
    pub control_path: PathBuf,
}

impl SshTarget {
    pub fn from_parsed(parsed: &ParsedSsh) -> Self {
        let mut t = Self::from_connection(&parsed.conn);
        t.extra_args = parsed.extra_args.clone();
        t
    }

    pub fn from_connection(conn: &Connection) -> Self {
        let non_empty = |s: &str| {
            let s = s.trim();
            (!s.is_empty()).then(|| s.to_string())
        };
        SshTarget {
            host: conn.host.trim().to_string(),
            user: non_empty(&conn.user),
            port: conn.port,
            identity_file: non_empty(&conn.identity_file),
            extra_args: Vec::new(),
            control_path: new_control_path(),
        }
    }

    /// Generates a fresh control socket path (used for reconnections).
    pub fn renew_control_path(&mut self) {
        let _ = std::fs::remove_file(&self.control_path);
        self.control_path = new_control_path();
    }

    /// `user@host` (or just `host`).
    pub fn destination(&self) -> String {
        match &self.user {
            Some(u) => format!("{u}@{}", self.host),
            None => self.host.clone(),
        }
    }

    /// Human readable label, e.g. `user@host:2222`.
    pub fn label(&self) -> String {
        match self.port {
            Some(p) if p != 22 => format!("{}:{p}", self.destination()),
            _ => self.destination(),
        }
    }

    fn common_args(&self) -> Vec<String> {
        let mut args = vec![
            "-o".to_string(),
            format!("ControlPath={}", self.control_path.display()),
        ];
        if let Some(p) = self.port {
            args.push("-p".into());
            args.push(p.to_string());
        }
        if let Some(i) = &self.identity_file {
            args.push("-i".into());
            args.push(expand_tilde(i));
        }
        args.extend(self.extra_args.iter().cloned());
        args
    }

    /// Command line run inside the terminal: an interactive session that also
    /// becomes the control master for the SFTP channels.
    pub fn terminal_argv(&self) -> Vec<String> {
        let mut argv = vec![
            "ssh".to_string(),
            "-o".into(),
            "ControlMaster=auto".into(),
            "-o".into(),
            "ControlPersist=5".into(),
        ];
        argv.extend(self.common_args());
        argv.push("--".into());
        argv.push(self.destination());
        argv
    }

    /// True once the control master is up (i.e. the user has authenticated).
    pub fn master_ready(&self) -> bool {
        use std::os::unix::fs::FileTypeExt;
        std::fs::metadata(&self.control_path).is_ok_and(|m| m.file_type().is_socket())
    }

    fn sftp_command(&self) -> Command {
        let mut cmd = Command::new("ssh");
        cmd.args([
            "-o",
            "ControlMaster=no",
            "-o",
            &format!("ControlPath={}", self.control_path.display()),
            "-o",
            "BatchMode=yes",
            "-T",
        ]);
        if let Some(p) = self.port {
            cmd.args(["-p", &p.to_string()]);
        }
        cmd.args(["-s", "--", &self.destination(), "sftp"]);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        cmd
    }

    /// Asks the control master to terminate (non blocking).
    pub fn exit_master(&self) {
        if !self.master_ready() {
            return;
        }
        let mut cmd = Command::new("ssh");
        cmd.args([
            "-o",
            &format!("ControlPath={}", self.control_path.display()),
            "-O",
            "exit",
            "--",
            &self.destination(),
        ]);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Ok(mut child) = cmd.spawn() {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

fn new_control_path() -> PathBuf {
    let dir = paths::runtime_dir().join("riso");
    let _ = std::fs::create_dir_all(&dir);
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let n = CONTROL_COUNTER.fetch_add(1, Ordering::Relaxed);
    dir.join(format!("cm-{}-{n}", std::process::id()))
}

pub fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        format!("{}/{rest}", paths::home_dir().display())
    } else {
        path.to_string()
    }
}

/// Quotes a string for a POSIX shell.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

type Job = Box<dyn FnOnce(Option<&mut Sftp>) + Send>;

/// An SFTP channel served by a dedicated background thread.
pub struct Remote {
    tx: RefCell<Option<mpsc::Sender<Job>>>,
    child: Arc<Mutex<Option<Child>>>,
}

impl Remote {
    pub fn start(target: &SshTarget) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        let child_slot: Arc<Mutex<Option<Child>>> = Arc::default();
        let slot = child_slot.clone();
        let mut cmd = target.sftp_command();
        std::thread::spawn(move || {
            let sftp = (|| -> io::Result<Sftp> {
                let mut child = cmd.spawn()?;
                let stdin = child.stdin.take().unwrap();
                let stdout = child.stdout.take().unwrap();
                *slot.lock().unwrap() = Some(child);
                Sftp::new(stdin, std::io::BufReader::with_capacity(256 * 1024, stdout))
            })();
            let mut sftp = match sftp {
                Ok(s) => Some(s),
                Err(e) => {
                    eprintln!("avvio SFTP fallito: {e}");
                    None
                }
            };
            while let Ok(job) = rx.recv() {
                job(sftp.as_mut());
            }
        });
        Remote {
            tx: RefCell::new(Some(tx)),
            child: child_slot,
        }
    }

    /// Queues `f` on the worker thread without waiting for it. `f` receives
    /// `None` when the SFTP channel could not be opened. Returns `false` when
    /// the worker is gone (the job is dropped).
    ///
    /// Used by frontends without an async executor (riso-tui), which report
    /// results back through their own channel.
    pub fn spawn(&self, f: impl FnOnce(Option<&mut Sftp>) + Send + 'static) -> bool {
        self.tx
            .borrow()
            .as_ref()
            .is_some_and(|tx| tx.send(Box::new(f)).is_ok())
    }

    /// Like [`Remote::spawn`], but maps a missing channel to an error and
    /// hands the result to `done` on the worker thread.
    pub fn spawn_with<T, F, D>(&self, f: F, done: D) -> bool
    where
        F: FnOnce(&mut Sftp) -> io::Result<T> + Send + 'static,
        D: FnOnce(io::Result<T>) + Send + 'static,
    {
        self.spawn(move |sftp| {
            done(match sftp {
                Some(sftp) => f(sftp),
                None => Err(not_available()),
            })
        })
    }

    /// Runs `f` on the worker thread and awaits its result.
    pub async fn run<T, F>(&self, f: F) -> io::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Sftp) -> io::Result<T> + Send + 'static,
    {
        let (s, r) = async_channel::bounded(1);
        let sent = self.spawn_with(f, move |res| {
            let _ = s.send_blocking(res);
        });
        if !sent {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "Connessione chiusa",
            ));
        }
        r.recv().await.unwrap_or_else(|_| {
            Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "Connessione chiusa",
            ))
        })
    }

    /// Terminates the channel.
    pub fn close(&self) {
        self.tx.borrow_mut().take();
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

fn not_available() -> io::Error {
    io::Error::new(io::ErrorKind::NotConnected, "Canale SFTP non disponibile")
}

impl Drop for Remote {
    fn drop(&mut self) {
        self.close();
    }
}

/// Thread-safe progress counter shared between a transfer job and the UI.
#[derive(Clone, Default)]
pub struct Progress {
    pub done: Arc<AtomicU64>,
    pub total: Arc<AtomicU64>,
}

impl Progress {
    pub fn add(&self, n: u64) {
        self.done.fetch_add(n, Ordering::Relaxed);
    }
    pub fn set_total(&self, n: u64) {
        self.total.store(n, Ordering::Relaxed);
    }
    pub fn fraction(&self) -> Option<f64> {
        let total = self.total.load(Ordering::Relaxed);
        (total > 0).then(|| (self.done.load(Ordering::Relaxed) as f64 / total as f64).min(1.0))
    }
}
