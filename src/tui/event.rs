//! TUI: events delivered to the main loop, and the terminal input thread.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use ratatui::crossterm::event::{self, Event};

use crate::files::{Entry, FileStamp};

/// Everything the main loop reacts to.
pub enum AppEvent {
    /// Key, mouse, paste, focus or resize event from the host terminal.
    Input(Event),
    /// Bytes produced by the `ssh` process of a session.
    PtyOutput(u64, Vec<u8>),
    /// The `ssh` process of a session closed its terminal.
    PtyClosed(u64),
    /// Result of an SFTP job of a session.
    Sftp(u64, SftpMsg),
}

/// Results of background SFTP jobs (see `explorer.rs` / `edit.rs`).
pub enum SftpMsg {
    /// Directory resolved + listed in a single background job.
    DirLoaded {
        nav_seq: u64,
        resolved: String,
        entries: Vec<Entry>,
        record_history: bool,
    },
    /// Navigation/listing error.
    NavError {
        nav_seq: u64,
        initial: bool,
        error: String,
    },
    /// Names among the files to upload that already exist in `dest`.
    UploadConflicts {
        files: Vec<PathBuf>,
        dest: String,
        existing: Vec<String>,
    },
    /// A file operation finished; `reload` lists directories to refresh.
    OpDone {
        error_prefix: String,
        result: Result<String, String>,
        reload: Vec<String>,
    },
    /// A remote file was downloaded into the local edit cache.
    EditReady {
        remote: String,
        local: PathBuf,
        how: OpenWith,
        result: Result<(), String>,
    },
    /// A locally edited file was uploaded back.
    EditSynced {
        local: PathBuf,
        stamp: FileStamp,
        result: Result<(), String>,
    },
}

/// How a remote file is opened locally.
#[derive(Clone, Debug)]
pub enum OpenWith {
    /// `$VISUAL` / `$EDITOR`, in the foreground of this terminal.
    Editor,
    /// The desktop default application (`xdg-open`).
    Default,
    /// A user-supplied command line.
    Command(String),
}

/// Reads host terminal events on a dedicated thread, so the main loop can
/// block on a single channel. It can be paused while a foreground program
/// (e.g. the user's editor) owns the terminal.
pub struct InputThread {
    paused: Arc<AtomicBool>,
    ack: Receiver<()>,
}

impl InputThread {
    pub fn spawn(tx: Sender<AppEvent>) -> Self {
        let paused = Arc::new(AtomicBool::new(false));
        let (ack_tx, ack) = mpsc::channel();
        let flag = paused.clone();
        std::thread::spawn(move || {
            let mut acked = false;
            loop {
                if flag.load(Ordering::Acquire) {
                    if !acked {
                        acked = ack_tx.send(()).is_ok();
                    }
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                acked = false;
                match event::poll(Duration::from_millis(50)) {
                    Ok(true) => match event::read() {
                        Ok(ev) => {
                            if tx.send(AppEvent::Input(ev)).is_err() {
                                return;
                            }
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(10)),
                    },
                    Ok(false) => {}
                    Err(_) => std::thread::sleep(Duration::from_millis(50)),
                }
            }
        });
        InputThread { paused, ack }
    }

    /// Stops reading and waits until the thread has acknowledged it.
    pub fn pause(&self) {
        while self.ack.try_recv().is_ok() {}
        self.paused.store(true, Ordering::Release);
        let _ = self.ack.recv_timeout(Duration::from_millis(500));
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::Release);
    }
}
