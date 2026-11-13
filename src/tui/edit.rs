//! TUI: local editing of remote files with automatic upload on save.
//!
//! Files are downloaded into a per-session cache directory and monitored via
//! [`FileStamp`] (which handles atomic replace-by-rename saves used by
//! `nvim`, `helix`, `vim`, etc.).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use super::event::{AppEvent, OpenWith, SftpMsg};
use crate::config::split_shell_words;
use crate::files::FileStamp;
use crate::paths;
use crate::remote::Remote;
use crate::sftp;

const KNOWN_TUI_EDITORS: &[&str] = &[
    "nvim", "vim", "vi", "hx", "helix", "nano", "micro", "kak", "ne", "vis", "pico", "joe",
    "mcedit",
];

const DEBOUNCE: Duration = Duration::from_millis(250);

struct Watch {
    remote_path: String,
    last_stamp: Option<FileStamp>,
    pending_since: Option<Instant>,
    uploading: bool,
}

/// How the main loop should spawn the requested editor/application.
pub enum LaunchAction {
    /// Run interactively in the foreground (suspending the TUI).
    Foreground { argv: Vec<String>, cwd: PathBuf },
    /// Spawn detached in the background.
    Background { argv: Vec<String>, cwd: PathBuf },
}

pub struct EditSync {
    base: PathBuf,
    watches: HashMap<PathBuf, Watch>,
}

impl EditSync {
    pub fn new(session_id: u64) -> Self {
        let base = paths::cache_dir()
            .join("riso")
            .join("edit")
            .join(format!("{}-{session_id}", std::process::id()));
        EditSync {
            base,
            watches: HashMap::new(),
        }
    }

    pub fn active_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .watches
            .values()
            .map(|w| sftp::basename(&w.remote_path))
            .collect();
        names.sort();
        names
    }

    pub fn clear(&mut self) {
        self.watches.clear();
        let _ = fs::remove_dir_all(&self.base);
    }

    fn local_path(&self, remote_path: &str) -> PathBuf {
        self.base.join(remote_path.trim_start_matches('/'))
    }

    /// Downloads `remote_path` to the edit cache; emits [`SftpMsg::EditReady`].
    pub fn open(
        &mut self,
        session_id: u64,
        remote_path: String,
        how: OpenWith,
        transfer: &Remote,
        tx: &Sender<AppEvent>,
    ) -> Result<(), String> {
        let local = self.local_path(&remote_path);
        if let Some(dir) = local.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("Errore locale: {e}"))?;
        }
        self.watches.remove(&local);

        let r = remote_path.clone();
        let l = local.clone();
        let tx = tx.clone();
        let queued = transfer.spawn_with(
            move |s| s.download_file(&r, &l, &mut |_| {}),
            move |res| {
                let _ = tx.send(AppEvent::Sftp(
                    session_id,
                    SftpMsg::EditReady {
                        remote: remote_path,
                        local,
                        how,
                        result: res.map_err(|e| e.to_string()),
                    },
                ));
            },
        );
        if !queued {
            return Err("Canale SFTP non disponibile".to_string());
        }
        Ok(())
    }

    /// Registers a watch once the download completes and prepares the launch.
    pub fn on_downloaded(
        &mut self,
        remote_path: String,
        local: PathBuf,
        how: OpenWith,
    ) -> Result<LaunchAction, String> {
        self.watches.insert(
            local.clone(),
            Watch {
                remote_path,
                last_stamp: FileStamp::of(&local),
                pending_since: None,
                uploading: false,
            },
        );
        prepare_launch(&local, &how)
    }

    /// Checks watched files for local edits and uploads changes after a short
    /// debounce window.
    pub fn poll_changes(
        &mut self,
        session_id: u64,
        transfer: Option<&Remote>,
        tx: &Sender<AppEvent>,
    ) {
        let Some(transfer) = transfer else { return };
        let now = Instant::now();
        for (local, w) in &mut self.watches {
            if w.uploading {
                continue;
            }
            let Some(current) = FileStamp::of(local) else {
                w.pending_since = None;
                continue;
            };
            if Some(current) == w.last_stamp {
                w.pending_since = None;
                continue;
            }
            let since = *w.pending_since.get_or_insert(now);
            if now.duration_since(since) < DEBOUNCE {
                continue;
            }
            w.pending_since = None;
            w.uploading = true;
            let l = local.clone();
            let r = w.remote_path.clone();
            let key = local.clone();
            let tx = tx.clone();
            if !transfer.spawn_with(
                move |s| s.upload_file(&l, &r, &mut |_| {}),
                move |res| {
                    let _ = tx.send(AppEvent::Sftp(
                        session_id,
                        SftpMsg::EditSynced {
                            local: key,
                            stamp: current,
                            result: res.map_err(|e| e.to_string()),
                        },
                    ));
                },
            ) {
                w.uploading = false;
            }
        }
    }

    /// Immediately uploads a watched file if it changed (called right after a
    /// foreground editor exits).
    pub fn sync_now(
        &mut self,
        session_id: u64,
        local: &Path,
        transfer: Option<&Remote>,
        tx: &Sender<AppEvent>,
    ) {
        let Some(transfer) = transfer else { return };
        let Some(w) = self.watches.get_mut(local) else { return };
        if w.uploading {
            return;
        }
        let Some(current) = FileStamp::of(local) else { return };
        if Some(current) == w.last_stamp {
            return;
        }
        w.pending_since = None;
        w.uploading = true;
        let l = local.to_path_buf();
        let key = l.clone();
        let r = w.remote_path.clone();
        let tx = tx.clone();
        if !transfer.spawn_with(
            move |s| s.upload_file(&l, &r, &mut |_| {}),
            move |res| {
                let _ = tx.send(AppEvent::Sftp(
                    session_id,
                    SftpMsg::EditSynced {
                        local: key,
                        stamp: current,
                        result: res.map_err(|e| e.to_string()),
                    },
                ));
            },
        ) {
            w.uploading = false;
        }
    }

    pub fn on_synced(&mut self, local: &Path, stamp: FileStamp, ok: bool) {
        if let Some(w) = self.watches.get_mut(local) {
            w.uploading = false;
            if ok {
                w.last_stamp = Some(stamp);
            }
        }
    }
}

impl Drop for EditSync {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn prepare_launch(local: &Path, how: &OpenWith) -> Result<LaunchAction, String> {
    let cwd = local.parent().unwrap_or_else(|| Path::new("/")).to_path_buf();
    let file_str = local.to_string_lossy().into_owned();
    match how {
        OpenWith::Default => Ok(LaunchAction::Background {
            argv: vec!["xdg-open".to_string(), file_str],
            cwd,
        }),
        OpenWith::Editor => {
            let cmd_str = resolve_editor().ok_or_else(|| {
                "Nessun editor trovato ($VISUAL, $EDITOR, nvim, vim, nano, vi)".to_string()
            })?;
            let mut argv = split_shell_words(&cmd_str)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| "Comando editor non valido".to_string())?;
            argv.push(file_str);
            Ok(LaunchAction::Foreground { argv, cwd })
        }
        OpenWith::Command(cmd_str) => {
            let mut argv = split_shell_words(cmd_str)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| "Comando non valido".to_string())?;
            let is_tui = is_tui_command(&argv[0]);
            argv.push(file_str);
            if is_tui {
                Ok(LaunchAction::Foreground { argv, cwd })
            } else {
                Ok(LaunchAction::Background { argv, cwd })
            }
        }
    }
}

pub fn spawn_background(argv: &[String], cwd: &Path) -> Result<(), String> {
    let Some((prog, args)) = argv.split_first() else {
        return Err("Comando vuoto".to_string());
    };
    Command::new(prog)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Impossibile avviare {prog}: {e}"))
}

fn resolve_editor() -> Option<String> {
    for var in ["VISUAL", "EDITOR"] {
        if let Ok(val) = std::env::var(var) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    for cand in ["nvim", "vim", "nano", "hx", "helix", "micro", "vi"] {
        if in_path(cand) {
            return Some(cand.to_string());
        }
    }
    None
}

fn is_tui_command(prog: &str) -> bool {
    let bin = Path::new(prog)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(prog);
    KNOWN_TUI_EDITORS.contains(&bin)
}

fn in_path(bin: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file())
}
