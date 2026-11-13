//! TUI: integrated SFTP file explorer dock.
//!
//! Shares the same dual-channel architecture as `riso-gui` (`browse` for
//! listings and metadata, `transfer` for uploads/downloads/copies/deletes)
//! and supports keyboard navigation, mouse clicks, drag-and-drop move/insert/
//! download/upload, and live local-edit synchronization.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::edit::EditSync;
use super::event::{AppEvent, OpenWith, SftpMsg};
use super::{icons, theme};
use crate::files::{Entry, human_size, local_size, sort_entries, unique_destination};
use crate::remote::{Progress, Remote};
use crate::sftp;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    #[default]
    Name,
    SizeDesc,
    ModifiedDesc,
    NameDesc,
}

impl SortMode {
    fn next(self) -> Self {
        match self {
            SortMode::Name => SortMode::SizeDesc,
            SortMode::SizeDesc => SortMode::ModifiedDesc,
            SortMode::ModifiedDesc => SortMode::NameDesc,
            SortMode::NameDesc => SortMode::Name,
        }
    }

    fn label(self) -> &'static str {
        match self {
            SortMode::Name => "Nome ↑",
            SortMode::SizeDesc => "Dim ↓",
            SortMode::ModifiedDesc => "Mod ↓",
            SortMode::NameDesc => "Nome ↓",
        }
    }
}

#[derive(Clone)]
pub struct Clipboard {
    pub paths: Vec<String>,
    pub cut: bool,
}

#[derive(Default, Clone, Copy)]
pub struct ExplorerAreas {
    pub panel: Rect,
    pub path_bar: Rect,
    pub list: Rect,
    pub btn_back: Rect,
    pub btn_forward: Rect,
    pub btn_up: Rect,
    pub btn_refresh: Rect,
    pub btn_hidden: Rect,
    pub btn_follow: Rect,
    pub btn_cd: Rect,
    pub btn_download: Rect,
    pub btn_upload: Rect,
}

pub enum ExplorerAction {
    None,
    Toast(String),
    CdTerminal(String),
    OpenDefault(Entry),
    OpenWithPrompt(Entry),
    RenamePrompt(Entry),
    NewFilePrompt,
    NewDirPrompt,
    DeleteConfirm(Vec<Entry>),
    UploadPrompt(String),
    DownloadPrompt(Vec<Entry>),
    GoToPathPrompt(String),
    /// Emitted when user starts dragging a row with the mouse.
    StartDrag(Vec<Entry>),
}

pub struct Explorer {
    session_id: u64,
    browse: Option<Rc<Remote>>,
    transfer: Option<Rc<Remote>>,

    pub cwd: String,
    entries: Vec<Entry>,
    pub cursor: usize,
    scroll: usize,
    marked: HashSet<String>,
    back: Vec<String>,
    forward: Vec<String>,
    dir_cursors: HashMap<String, String>,
    dir_cache: HashMap<String, Vec<Entry>>,
    nav_seq: u64,
    loading: bool,
    sort_mode: SortMode,
    pending_g: bool,
    visual: Option<(usize, HashSet<String>)>,

    pub show_hidden: bool,
    pub follow_terminal: bool,
    pub filter: String,
    pub filtering: bool,

    pub clipboard: Option<Clipboard>,
    active_transfers: u32,
    current_progress: Option<Progress>,
    pub edits: EditSync,

    placeholder: String,
    last_click: Option<(Instant, usize)>,
    mouse_down_row: Option<usize>,
    pub areas: ExplorerAreas,
}

impl Explorer {
    pub fn new(session_id: u64) -> Self {
        Explorer {
            session_id,
            browse: None,
            transfer: None,
            cwd: String::new(),
            entries: Vec::new(),
            cursor: 0,
            scroll: 0,
            marked: HashSet::new(),
            back: Vec::new(),
            forward: Vec::new(),
            dir_cursors: HashMap::new(),
            dir_cache: HashMap::new(),
            nav_seq: 0,
            loading: false,
            sort_mode: SortMode::Name,
            pending_g: false,
            visual: None,
            show_hidden: false,
            follow_terminal: true,
            filter: String::new(),
            filtering: false,
            clipboard: None,
            active_transfers: 0,
            current_progress: None,
            edits: EditSync::new(session_id),
            placeholder: "In attesa di connessione".to_string(),
            last_click: None,
            mouse_down_row: None,
            areas: ExplorerAreas::default(),
        }
    }

    pub fn is_attached(&self) -> bool {
        self.browse.is_some()
    }

    pub fn transfer_remote(&self) -> Option<Rc<Remote>> {
        self.transfer.clone()
    }

    pub fn attach(
        &mut self,
        browse: Rc<Remote>,
        transfer: Rc<Remote>,
        initial_dir: Option<String>,
        tx: &Sender<AppEvent>,
    ) {
        self.browse = Some(browse.clone());
        self.transfer = Some(transfer);
        self.back.clear();
        self.forward.clear();
        self.marked.clear();
        self.entries.clear();
        self.dir_cursors.clear();
        self.dir_cache.clear();
        self.nav_seq = self.nav_seq.wrapping_add(1);
        self.loading = true;
        self.pending_g = false;
        self.visual = None;
        self.last_click = None;
        self.mouse_down_row = None;
        self.cursor = 0;
        self.scroll = 0;
        self.placeholder = "Caricamento…".to_string();

        let start = initial_dir.unwrap_or_else(|| ".".to_string());
        let sid = self.session_id;
        let seq = self.nav_seq;
        let tx = tx.clone();
        browse.spawn_with(
            move |s| {
                let real = s.realpath(&start).or_else(|_| s.realpath("."))?;
                let raw = s.readdir(&real)?;
                let entries = raw.iter().map(|e| Entry::new(&real, e)).collect();
                Ok((real, entries))
            },
            move |res| {
                let msg = match res {
                    Ok((resolved, entries)) => SftpMsg::DirLoaded {
                        nav_seq: seq,
                        resolved,
                        entries,
                        record_history: false,
                    },
                    Err(e) => SftpMsg::NavError {
                        nav_seq: seq,
                        initial: true,
                        error: e.to_string(),
                    },
                };
                let _ = tx.send(AppEvent::Sftp(sid, msg));
            },
        );
    }

    pub fn detach(&mut self, message: &str) {
        self.browse = None;
        self.transfer = None;
        self.edits.clear();
        self.entries.clear();
        self.marked.clear();
        self.back.clear();
        self.forward.clear();
        self.dir_cursors.clear();
        self.dir_cache.clear();
        self.nav_seq = self.nav_seq.wrapping_add(1);
        self.loading = false;
        self.pending_g = false;
        self.visual = None;
        self.last_click = None;
        self.mouse_down_row = None;
        self.cwd.clear();
        self.filter.clear();
        self.filtering = false;
        self.clipboard = None;
        self.active_transfers = 0;
        self.current_progress = None;
        self.cursor = 0;
        self.scroll = 0;
        self.placeholder = message.to_string();
    }

    pub fn show_waiting(&mut self) {
        self.detach("In attesa di connessione");
    }

    pub fn visible_entries(&self) -> Vec<&Entry> {
        let q = self.filter.trim().to_lowercase();
        self.entries
            .iter()
            .filter(|e| self.show_hidden || !e.name.starts_with('.'))
            .filter(|e| q.is_empty() || e.name.to_lowercase().contains(&q))
            .collect()
    }

    pub fn selected_entries(&self) -> Vec<Entry> {
        let visible = self.visible_entries();
        if !self.marked.is_empty() {
            let marked: Vec<Entry> = visible
                .iter()
                .filter(|e| self.marked.contains(&e.path))
                .map(|e| (*e).clone())
                .collect();
            if !marked.is_empty() {
                return marked;
            }
        }
        visible.get(self.cursor).map(|e| vec![(*e).clone()]).unwrap_or_default()
    }

    pub fn entry_at_row(&self, screen_y: u16) -> Option<Entry> {
        let list = self.areas.list;
        if screen_y < list.y || screen_y >= list.bottom() {
            return None;
        }
        let idx = self.scroll + (screen_y - list.y) as usize;
        self.visible_entries().get(idx).map(|e| (*e).clone())
    }

    pub fn target_dir(&self) -> String {
        let vis = self.visible_entries();
        if self.marked.is_empty()
            && let Some(e) = vis.get(self.cursor)
            && e.is_dir
        {
            return e.path.clone();
        }
        self.cwd.clone()
    }

    fn set_cursor(&mut self, idx: usize) {
        self.cursor = idx;
        self.clamp_cursor();
        self.sync_visual_marks();
    }

    fn sync_visual_marks(&mut self) {
        let Some((anchor, base)) = &self.visual else { return };
        let mut next = base.clone();
        let visible = self.visible_entries();
        if !visible.is_empty() {
            let lo = (*anchor).min(self.cursor).min(visible.len() - 1);
            let hi = (*anchor).max(self.cursor).min(visible.len() - 1);
            for e in &visible[lo..=hi] {
                next.insert(e.path.clone());
            }
        }
        self.marked = next;
    }

    fn apply_sort(&mut self) {
        match self.sort_mode {
            SortMode::Name => sort_entries(&mut self.entries),
            SortMode::NameDesc => {
                self.entries.sort_by(|a, b| {
                    b.is_dir
                        .cmp(&a.is_dir)
                        .then_with(|| b.name.to_lowercase().cmp(&a.name.to_lowercase()))
                        .then_with(|| b.name.cmp(&a.name))
                });
            }
            SortMode::SizeDesc => {
                self.entries.sort_by(|a, b| {
                    b.is_dir
                        .cmp(&a.is_dir)
                        .then_with(|| b.size.cmp(&a.size))
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
            }
            SortMode::ModifiedDesc => {
                self.entries.sort_by(|a, b| {
                    b.is_dir
                        .cmp(&a.is_dir)
                        .then_with(|| b.mtime.cmp(&a.mtime))
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
            }
        }
    }

    fn remember_current_cursor(&mut self) {
        if !self.cwd.is_empty()
            && let Some(e) = self.visible_entries().get(self.cursor)
        {
            self.dir_cursors.insert(self.cwd.clone(), e.name.clone());
        }
    }

    fn clamp_cursor(&mut self) {
        let len = self.visible_entries().len();
        if len == 0 {
            self.cursor = 0;
            self.scroll = 0;
        } else {
            self.cursor = self.cursor.min(len - 1);
            let h = self.areas.list.height.max(1) as usize;
            if self.cursor < self.scroll {
                self.scroll = self.cursor;
            } else if self.cursor >= self.scroll + h {
                self.scroll = self.cursor + 1 - h;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Navigation & listing
    // -----------------------------------------------------------------------

    pub fn navigate(&mut self, path: String, record_history: bool, tx: &Sender<AppEvent>) {
        self.navigate_with(path, true, record_history, tx);
    }

    pub fn navigate_with(
        &mut self,
        path: String,
        resolve_realpath: bool,
        record_history: bool,
        tx: &Sender<AppEvent>,
    ) {
        let Some(browse) = self.browse.clone() else { return };
        self.nav_seq = self.nav_seq.wrapping_add(1);
        let seq = self.nav_seq;

        let mut job_record_history = record_history;
        if !resolve_realpath && let Some(cached) = self.dir_cache.get(&path).cloned() {
            // Instant local switch from cache; silently revalidate in background.
            self.apply_directory(path.clone(), cached, record_history);
            job_record_history = false;
        } else {
            self.loading = true;
        }

        let sid = self.session_id;
        let tx = tx.clone();
        browse.spawn_with(
            move |s| {
                let real = if resolve_realpath { s.realpath(&path)? } else { path };
                let raw = s.readdir(&real)?;
                let entries = raw.iter().map(|e| Entry::new(&real, e)).collect();
                Ok((real, entries))
            },
            move |res| {
                let msg = match res {
                    Ok((resolved, entries)) => SftpMsg::DirLoaded {
                        nav_seq: seq,
                        resolved,
                        entries,
                        record_history: job_record_history,
                    },
                    Err(e) => SftpMsg::NavError {
                        nav_seq: seq,
                        initial: false,
                        error: e.to_string(),
                    },
                };
                let _ = tx.send(AppEvent::Sftp(sid, msg));
            },
        );
    }

    pub fn refresh(&mut self, tx: &Sender<AppEvent>) {
        if self.cwd.is_empty() {
            return;
        }
        let cwd = self.cwd.clone();
        self.dir_cache.remove(&cwd);
        let Some(browse) = self.browse.clone() else { return };
        self.nav_seq = self.nav_seq.wrapping_add(1);
        let seq = self.nav_seq;
        self.loading = true;
        let sid = self.session_id;
        let tx = tx.clone();
        browse.spawn_with(
            move |s| {
                let raw = s.readdir(&cwd)?;
                let entries = raw.iter().map(|e| Entry::new(&cwd, e)).collect();
                Ok((cwd, entries))
            },
            move |res| {
                let msg = match res {
                    Ok((resolved, entries)) => SftpMsg::DirLoaded {
                        nav_seq: seq,
                        resolved,
                        entries,
                        record_history: false,
                    },
                    Err(e) => SftpMsg::NavError {
                        nav_seq: seq,
                        initial: false,
                        error: e.to_string(),
                    },
                };
                let _ = tx.send(AppEvent::Sftp(sid, msg));
            },
        );
    }

    pub fn terminal_cwd_changed(&mut self, path: &str, tx: &Sender<AppEvent>) {
        if self.follow_terminal && self.is_attached() && self.cwd != path {
            self.navigate_with(path.to_string(), true, true, tx);
        }
    }

    pub fn go_back(&mut self, tx: &Sender<AppEvent>) {
        if let Some(prev) = self.back.pop() {
            if !self.cwd.is_empty() {
                self.forward.push(self.cwd.clone());
            }
            self.navigate_with(prev, false, false, tx);
        }
    }

    pub fn go_forward(&mut self, tx: &Sender<AppEvent>) {
        if let Some(next) = self.forward.pop() {
            if !self.cwd.is_empty() {
                self.back.push(self.cwd.clone());
            }
            self.navigate_with(next, false, false, tx);
        }
    }

    pub fn go_up(&mut self, tx: &Sender<AppEvent>) {
        if !self.cwd.is_empty() && self.cwd != "/" {
            self.navigate_with(sftp::parent(&self.cwd), false, true, tx);
        }
    }

    fn apply_directory(&mut self, resolved: String, entries: Vec<Entry>, record_history: bool) {
        let same_dir = self.cwd == resolved;
        let target_name = if same_dir {
            self.visible_entries().get(self.cursor).map(|e| e.name.clone())
        } else {
            self.remember_current_cursor();
            if !self.cwd.is_empty() && sftp::parent(&self.cwd) == resolved {
                self.dir_cursors
                    .insert(resolved.clone(), sftp::basename(&self.cwd));
            }
            if record_history && !self.cwd.is_empty() {
                self.back.push(self.cwd.clone());
                self.forward.clear();
            }
            self.marked.clear();
            self.visual = None;
            self.pending_g = false;
            self.last_click = None;
            self.mouse_down_row = None;
            self.filter.clear();
            self.filtering = false;
            self.cursor = 0;
            self.scroll = 0;
            self.dir_cursors.get(&resolved).cloned()
        };

        self.cwd = resolved;
        self.entries = entries;
        self.loading = false;
        self.apply_sort();
        let valid: HashSet<&str> = self.entries.iter().map(|e| e.path.as_str()).collect();
        self.marked.retain(|p| valid.contains(p.as_str()));

        self.cursor = 0;
        self.scroll = 0;
        if let Some(name) = target_name
            && let Some(idx) = self.visible_entries().iter().position(|e| e.name == name)
        {
            self.cursor = idx;
        }
        self.clamp_cursor();
    }

    // -----------------------------------------------------------------------
    // SFTP message handling
    // -----------------------------------------------------------------------

    pub fn handle_sftp(&mut self, msg: SftpMsg, tx: &Sender<AppEvent>) -> Option<String> {
        match msg {
            SftpMsg::DirLoaded {
                nav_seq,
                resolved,
                entries,
                record_history,
            } => {
                if !self.is_attached() {
                    return None;
                }
                if self.dir_cache.len() >= 64 && !self.dir_cache.contains_key(&resolved) {
                    self.dir_cache.clear();
                }
                self.dir_cache.insert(resolved.clone(), entries.clone());
                if nav_seq == self.nav_seq {
                    self.apply_directory(resolved, entries, record_history);
                }
                None
            }
            SftpMsg::NavError {
                nav_seq,
                initial,
                error,
            } => {
                if nav_seq == self.nav_seq {
                    self.loading = false;
                }
                if initial || self.cwd.is_empty() {
                    self.detach(&format!("SFTP non disponibile\n{error}"));
                    None
                } else {
                    Some(error)
                }
            }
            SftpMsg::OpDone { error_prefix, result, reload } => {
                self.end_transfer();
                for dir in &reload {
                    self.dir_cache.remove(dir);
                }
                if reload.iter().any(|d| d == &self.cwd) {
                    self.refresh(tx);
                }
                match result {
                    Ok(msg) if !msg.is_empty() => Some(msg),
                    Ok(_) => None,
                    Err(e) => Some(format!("{error_prefix}: {e}")),
                }
            }
            SftpMsg::EditSynced { local, stamp, result } => {
                self.edits.on_synced(&local, stamp, result.is_ok());
                match result {
                    Ok(()) => None,
                    Err(e) => Some(format!("Sync {}: {e}", local.file_name().unwrap_or_default().to_string_lossy())),
                }
            }
            SftpMsg::UploadConflicts { .. } | SftpMsg::EditReady { .. } => None,
        }
    }

    // -----------------------------------------------------------------------
    // File operations
    // -----------------------------------------------------------------------

    fn begin_transfer(&mut self) -> Progress {
        let p = Progress::default();
        self.current_progress = Some(p.clone());
        self.active_transfers += 1;
        p
    }

    fn end_transfer(&mut self) {
        self.active_transfers = self.active_transfers.saturating_sub(1);
        if self.active_transfers == 0 {
            self.current_progress = None;
        }
    }

    pub fn open_entry(&mut self, entry: &Entry, how: OpenWith, tx: &Sender<AppEvent>) -> Result<(), String> {
        let Some(transfer) = &self.transfer else {
            return Err("Non connesso".to_string());
        };
        self.edits.open(self.session_id, entry.path.clone(), how, transfer, tx)
    }

    pub fn check_upload(&self, files: Vec<PathBuf>, dest_dir: String, tx: &Sender<AppEvent>) {
        if files.is_empty() {
            return;
        }
        let Some(browse) = &self.browse else { return };
        let names: Vec<String> = files
            .iter()
            .filter_map(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        let d = dest_dir.clone();
        let sid = self.session_id;
        let tx = tx.clone();
        browse.spawn_with(
            move |s| {
                let mut ex = Vec::new();
                for name in &names {
                    if s.exists(&sftp::join(&d, name))? {
                        ex.push(name.clone());
                    }
                }
                Ok(ex)
            },
            move |res| {
                let existing = res.unwrap_or_default();
                let _ = tx.send(AppEvent::Sftp(
                    sid,
                    SftpMsg::UploadConflicts {
                        files,
                        dest: dest_dir,
                        existing,
                    },
                ));
            },
        );
    }

    pub fn upload_exec(&mut self, files: Vec<PathBuf>, dest_dir: String, tx: &Sender<AppEvent>) {
        let Some(transfer) = self.transfer.clone() else { return };
        let p = self.begin_transfer();
        let total: u64 = files.iter().map(|f| local_size(f)).sum();
        let d = dest_dir.clone();
        let sid = self.session_id;
        let tx = tx.clone();
        let count = files.len();
        let queued = transfer.spawn_with(
            move |s| {
                p.set_total(total);
                for f in &files {
                    let Some(name) = f.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                        continue;
                    };
                    s.upload_recursive(f, &sftp::join(&d, &name), &mut |n| p.add(n))?;
                }
                Ok(if count == 1 {
                    "Caricamento completato".to_string()
                } else {
                    format!("Caricati {count} elementi")
                })
            },
            move |res| {
                let _ = tx.send(AppEvent::Sftp(
                    sid,
                    SftpMsg::OpDone {
                        error_prefix: "Caricamento fallito".to_string(),
                        result: res.map_err(|e| e.to_string()),
                        reload: vec![dest_dir],
                    },
                ));
            },
        );
        if !queued {
            self.end_transfer();
        }
    }

    pub fn download_exec(&mut self, entries: Vec<Entry>, local_dir: PathBuf, tx: &Sender<AppEvent>) {
        if entries.is_empty() {
            return;
        }
        let Some(transfer) = self.transfer.clone() else { return };
        let p = self.begin_transfer();
        let sid = self.session_id;
        let tx = tx.clone();
        let dest_label = local_dir.display().to_string();
        let queued = transfer.spawn_with(
            move |s| {
                let mut total = 0;
                for e in &entries {
                    total += s.total_size(&e.path).unwrap_or(0);
                }
                p.set_total(total);
                for e in &entries {
                    s.download_recursive(&e.path, &local_dir.join(&e.name), &mut |n| p.add(n))?;
                }
                Ok(format!("Scaricato in {dest_label}"))
            },
            move |res| {
                let _ = tx.send(AppEvent::Sftp(
                    sid,
                    SftpMsg::OpDone {
                        error_prefix: "Download fallito".to_string(),
                        result: res.map_err(|e| e.to_string()),
                        reload: Vec::new(),
                    },
                ));
            },
        );
        if !queued {
            self.end_transfer();
        }
    }

    pub fn set_clipboard(&mut self, cut: bool) -> Option<String> {
        let paths: Vec<String> = self.selected_entries().into_iter().map(|e| e.path).collect();
        if paths.is_empty() {
            return None;
        }
        let n = paths.len();
        self.clipboard = Some(Clipboard { paths, cut });
        Some(if cut {
            format!("Tagliati {n} elementi")
        } else {
            format!("Copiati {n} elementi")
        })
    }

    pub fn paste(&mut self, tx: &Sender<AppEvent>) {
        let Some(clip) = self.clipboard.take() else { return };
        let dest = self.target_dir();
        if clip.cut {
            self.move_paths(clip.paths, dest, tx);
            return;
        }
        let paths = clip.paths.clone();
        self.clipboard = Some(clip);
        let Some(transfer) = self.transfer.clone() else { return };
        let p = self.begin_transfer();
        let d = dest.clone();
        let sid = self.session_id;
        let tx = tx.clone();
        let queued = transfer.spawn_with(
            move |s| {
                let mut total = 0;
                for src in &paths {
                    total += s.total_size(src).unwrap_or(0);
                }
                p.set_total(total);
                for src in &paths {
                    let target = unique_destination(s, &d, &sftp::basename(src))?;
                    s.copy_recursive(src, &target, &mut |n| p.add(n))?;
                }
                Ok(String::new())
            },
            move |res| {
                let _ = tx.send(AppEvent::Sftp(
                    sid,
                    SftpMsg::OpDone {
                        error_prefix: "Copia fallita".to_string(),
                        result: res.map_err(|e| e.to_string()),
                        reload: vec![dest],
                    },
                ));
            },
        );
        if !queued {
            self.end_transfer();
        }
    }

    pub fn move_paths(&mut self, paths: Vec<String>, dest: String, tx: &Sender<AppEvent>) {
        let Some(browse) = &self.browse else { return };
        let paths: Vec<String> = paths
            .into_iter()
            .filter(|p| sftp::parent(p) != dest && *p != dest && !dest.starts_with(&format!("{p}/")))
            .collect();
        if paths.is_empty() {
            return;
        }
        let mut reload: Vec<String> = paths.iter().map(|p| sftp::parent(p)).collect();
        reload.push(dest.clone());
        reload.sort();
        reload.dedup();
        let p_in = paths;
        let d = dest;
        let sid = self.session_id;
        let tx = tx.clone();
        browse.spawn_with(
            move |s| {
                for src in &p_in {
                    let target = sftp::join(&d, &sftp::basename(src));
                    if s.exists(&target)? {
                        return Err(io::Error::other(format!(
                            "{} esiste già",
                            sftp::basename(src)
                        )));
                    }
                    s.rename(src, &target)?;
                }
                Ok(String::new())
            },
            move |res| {
                let _ = tx.send(AppEvent::Sftp(
                    sid,
                    SftpMsg::OpDone {
                        error_prefix: "Spostamento fallito".to_string(),
                        result: res.map_err(|e| e.to_string()),
                        reload,
                    },
                ));
            },
        );
    }

    pub fn rename(&self, entry: &Entry, new_name: String, tx: &Sender<AppEvent>) -> Result<(), String> {
        if new_name == entry.name {
            return Ok(());
        }
        if new_name.contains('/') || new_name.is_empty() {
            return Err("Nome non valido".to_string());
        }
        let Some(browse) = &self.browse else { return Err("Non connesso".to_string()) };
        let parent = sftp::parent(&entry.path);
        let target = sftp::join(&parent, &new_name);
        let src = entry.path.clone();
        let sid = self.session_id;
        let tx = tx.clone();
        browse.spawn_with(
            move |s| {
                if s.exists(&target)? {
                    return Err(io::Error::other("Nome già esistente"));
                }
                s.rename(&src, &target)?;
                Ok(String::new())
            },
            move |res| {
                let _ = tx.send(AppEvent::Sftp(
                    sid,
                    SftpMsg::OpDone {
                        error_prefix: "Rinomina fallita".to_string(),
                        result: res.map_err(|e| e.to_string()),
                        reload: vec![parent],
                    },
                ));
            },
        );
        Ok(())
    }

    pub fn delete(&mut self, entries: Vec<Entry>, tx: &Sender<AppEvent>) {
        if entries.is_empty() {
            return;
        }
        let Some(transfer) = self.transfer.clone() else { return };
        let p = self.begin_transfer();
        let _ = p;
        let paths: Vec<String> = entries.iter().map(|e| e.path.clone()).collect();
        let mut reload: Vec<String> = paths.iter().map(|p| sftp::parent(p)).collect();
        reload.sort();
        reload.dedup();
        let sid = self.session_id;
        let tx = tx.clone();
        let queued = transfer.spawn_with(
            move |s| {
                for p in &paths {
                    s.remove_recursive(p)?;
                }
                Ok(String::new())
            },
            move |res| {
                let _ = tx.send(AppEvent::Sftp(
                    sid,
                    SftpMsg::OpDone {
                        error_prefix: "Eliminazione fallita".to_string(),
                        result: res.map_err(|e| e.to_string()),
                        reload,
                    },
                ));
            },
        );
        if !queued {
            self.end_transfer();
        }
    }

    pub fn create_new(&self, name: String, folder: bool, tx: &Sender<AppEvent>) -> Result<(), String> {
        if name.contains('/') || name.is_empty() {
            return Err("Nome non valido".to_string());
        }
        let Some(browse) = &self.browse else { return Err("Non connesso".to_string()) };
        let dest = self.target_dir();
        let path = sftp::join(&dest, &name);
        let sid = self.session_id;
        let tx = tx.clone();
        browse.spawn_with(
            move |s| {
                if folder {
                    s.mkdir(&path)?;
                } else {
                    s.create_file(&path)?;
                }
                Ok(String::new())
            },
            move |res| {
                let _ = tx.send(AppEvent::Sftp(
                    sid,
                    SftpMsg::OpDone {
                        error_prefix: name,
                        result: res.map_err(|e| e.to_string()),
                        reload: vec![dest],
                    },
                ));
            },
        );
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Input handling
    // -----------------------------------------------------------------------

    pub fn handle_key(
        &mut self,
        key: ratatui::crossterm::event::KeyEvent,
        tx: &Sender<AppEvent>,
    ) -> ExplorerAction {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};
        if !self.is_attached() {
            return ExplorerAction::None;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        if self.filtering {
            match key.code {
                KeyCode::Esc => {
                    self.filtering = false;
                    self.filter.clear();
                    self.clamp_cursor();
                }
                KeyCode::Enter => {
                    self.filtering = false;
                }
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.clamp_cursor();
                }
                KeyCode::Char(c) if !ctrl => {
                    self.filter.push(c);
                    self.cursor = 0;
                    self.clamp_cursor();
                }
                _ => {}
            }
            return ExplorerAction::None;
        }

        let len = self.visible_entries().len();
        let page = self.areas.list.height.max(1) as usize;

        if self.pending_g {
            self.pending_g = false;
            match key.code {
                KeyCode::Char('g') | KeyCode::Home => {
                    self.set_cursor(0);
                    return ExplorerAction::None;
                }
                KeyCode::Char('h') => {
                    self.navigate(".".to_string(), true, tx);
                    return ExplorerAction::None;
                }
                KeyCode::Char('/') | KeyCode::Char('r') => {
                    self.navigate("/".to_string(), true, tx);
                    return ExplorerAction::None;
                }
                KeyCode::Char('G') | KeyCode::End => {
                    if len > 0 {
                        self.set_cursor(len - 1);
                    }
                    return ExplorerAction::None;
                }
                KeyCode::Esc => return ExplorerAction::None,
                _ => {}
            }
        }

        if ctrl {
            match key.code {
                KeyCode::Char('u') => {
                    self.set_cursor(self.cursor.saturating_sub((page / 2).max(1)));
                    return ExplorerAction::None;
                }
                KeyCode::Char('d') => {
                    if len > 0 {
                        self.set_cursor((self.cursor + (page / 2).max(1)).min(len - 1));
                    }
                    return ExplorerAction::None;
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                if len > 0 {
                    self.set_cursor((self.cursor + 1).min(len - 1));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.set_cursor(self.cursor.saturating_sub(1));
            }
            KeyCode::PageDown => {
                if len > 0 {
                    self.set_cursor((self.cursor + page).min(len - 1));
                }
            }
            KeyCode::PageUp => {
                self.set_cursor(self.cursor.saturating_sub(page));
            }
            KeyCode::Home => {
                self.set_cursor(0);
            }
            KeyCode::Char('g') => {
                self.pending_g = true;
            }
            KeyCode::End | KeyCode::Char('G') => {
                if len > 0 {
                    self.set_cursor(len - 1);
                }
            }
            KeyCode::Char('~') => self.navigate(".".to_string(), true, tx),
            KeyCode::Char('-') => self.go_back(tx),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                if let Some(entry) = self.visible_entries().get(self.cursor).map(|e| (*e).clone()) {
                    if entry.is_dir {
                        self.navigate_with(entry.path, entry.is_link, true, tx);
                    } else if let Err(e) = self.open_entry(&entry, OpenWith::Editor, tx) {
                        return ExplorerAction::Toast(e);
                    }
                }
            }
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => self.go_up(tx),
            KeyCode::Char('[') => self.go_back(tx),
            KeyCode::Char(']') => self.go_forward(tx),
            KeyCode::Char('R') | KeyCode::F(5) => self.refresh(tx),
            KeyCode::Char('.') => {
                self.show_hidden = !self.show_hidden;
                self.clamp_cursor();
                self.sync_visual_marks();
            }
            KeyCode::Char(',') => {
                let cur_name = self.visible_entries().get(self.cursor).map(|e| e.name.clone());
                self.sort_mode = self.sort_mode.next();
                self.apply_sort();
                if let Some(name) = cur_name
                    && let Some(idx) = self.visible_entries().iter().position(|e| e.name == name)
                {
                    self.cursor = idx;
                }
                self.clamp_cursor();
                return ExplorerAction::Toast(format!("Ordine: {}", self.sort_mode.label()));
            }
            KeyCode::Char('f') => {
                self.follow_terminal = !self.follow_terminal;
                return ExplorerAction::Toast(if self.follow_terminal {
                    "Segui terminale: attivo".to_string()
                } else {
                    "Segui terminale: disattivato".to_string()
                });
            }
            KeyCode::Char('c') => {
                if !self.cwd.is_empty() {
                    return ExplorerAction::CdTerminal(self.cwd.clone());
                }
            }
            KeyCode::Char('/') => {
                self.filtering = true;
            }
            KeyCode::Char('P') => return ExplorerAction::GoToPathPrompt(self.cwd.clone()),
            KeyCode::Char('v') => {
                if self.visual.take().is_none() && len > 0 {
                    self.visual = Some((self.cursor, self.marked.clone()));
                    self.sync_visual_marks();
                }
            }
            KeyCode::Char(' ') => {
                if let Some(entry) = self.visible_entries().get(self.cursor) {
                    let path = entry.path.clone();
                    if !self.marked.remove(&path) {
                        self.marked.insert(path);
                    }
                    if len > 0 {
                        self.set_cursor((self.cursor + 1).min(len - 1));
                    }
                }
            }
            KeyCode::Char('a') => {
                self.visual = None;
                let all: Vec<String> = self.visible_entries().iter().map(|e| e.path.clone()).collect();
                if !all.is_empty() && all.iter().all(|p| self.marked.contains(p)) {
                    self.marked.clear();
                } else {
                    self.marked.extend(all);
                }
            }
            KeyCode::Esc => {
                if let Some((_, prev_marks)) = self.visual.take() {
                    self.marked = prev_marks;
                } else if !self.filter.is_empty() {
                    self.filter.clear();
                    self.clamp_cursor();
                } else {
                    self.marked.clear();
                }
            }
            KeyCode::Char('o') => {
                if let Some(e) = self.visible_entries().get(self.cursor).map(|e| (*e).clone())
                    && !e.is_dir
                {
                    return ExplorerAction::OpenDefault(e);
                }
            }
            KeyCode::Char('O') => {
                if let Some(e) = self.visible_entries().get(self.cursor).map(|e| (*e).clone()) {
                    return ExplorerAction::OpenWithPrompt(e);
                }
            }
            KeyCode::Char('y') => {
                if let Some(msg) = self.set_clipboard(false) {
                    return ExplorerAction::Toast(msg);
                }
            }
            KeyCode::Char('x') => {
                if let Some(msg) = self.set_clipboard(true) {
                    return ExplorerAction::Toast(msg);
                }
            }
            KeyCode::Char('Y') | KeyCode::Char('X') => {
                self.clipboard = None;
            }
            KeyCode::Char('p') => self.paste(tx),
            KeyCode::Char('r') | KeyCode::F(2) => {
                if let [e] = self.selected_entries().as_slice() {
                    return ExplorerAction::RenamePrompt(e.clone());
                }
            }
            KeyCode::Char('n') => return ExplorerAction::NewFilePrompt,
            KeyCode::Char('N') | KeyCode::Char('M') => return ExplorerAction::NewDirPrompt,
            KeyCode::Char('d') | KeyCode::Delete => {
                let sel = self.selected_entries();
                if !sel.is_empty() {
                    return ExplorerAction::DeleteConfirm(sel);
                }
            }
            KeyCode::Char('u') => return ExplorerAction::UploadPrompt(self.target_dir()),
            KeyCode::Char('D') => {
                let sel = self.selected_entries();
                if !sel.is_empty() {
                    return ExplorerAction::DownloadPrompt(sel);
                }
            }
            _ => {}
        }
        ExplorerAction::None
    }

    pub fn handle_mouse(
        &mut self,
        ev: ratatui::crossterm::event::MouseEvent,
        tx: &Sender<AppEvent>,
    ) -> ExplorerAction {
        use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
        if !self.is_attached() {
            return ExplorerAction::None;
        }
        let (x, y) = (ev.column, ev.row);
        match ev.kind {
            MouseEventKind::ScrollUp => {
                self.cursor = self.cursor.saturating_sub(3);
                self.clamp_cursor();
            }
            MouseEventKind::ScrollDown => {
                let len = self.visible_entries().len();
                if len > 0 {
                    self.cursor = (self.cursor + 3).min(len - 1);
                    self.clamp_cursor();
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.mouse_down_row = None;
                if contains(self.areas.btn_back, x, y) {
                    self.go_back(tx);
                } else if contains(self.areas.btn_forward, x, y) {
                    self.go_forward(tx);
                } else if contains(self.areas.btn_up, x, y) {
                    self.go_up(tx);
                } else if contains(self.areas.btn_refresh, x, y) {
                    self.refresh(tx);
                } else if contains(self.areas.btn_hidden, x, y) {
                    self.show_hidden = !self.show_hidden;
                    self.clamp_cursor();
                } else if contains(self.areas.btn_follow, x, y) {
                    self.follow_terminal = !self.follow_terminal;
                } else if contains(self.areas.btn_cd, x, y) {
                    if !self.cwd.is_empty() {
                        return ExplorerAction::CdTerminal(self.cwd.clone());
                    }
                } else if contains(self.areas.btn_download, x, y) {
                    let sel = self.selected_entries();
                    if !sel.is_empty() {
                        return ExplorerAction::DownloadPrompt(sel);
                    }
                } else if contains(self.areas.btn_upload, x, y) {
                    return ExplorerAction::UploadPrompt(self.target_dir());
                } else if contains(self.areas.path_bar, x, y) {
                    return ExplorerAction::GoToPathPrompt(self.cwd.clone());
                } else if contains(self.areas.list, x, y) {
                    let idx = self.scroll + (y - self.areas.list.y) as usize;
                    let len = self.visible_entries().len();
                    if idx < len {
                        self.cursor = idx;
                        self.clamp_cursor();
                        self.mouse_down_row = Some(idx);
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let down_row = self.mouse_down_row.take();
                if contains(self.areas.list, x, y) {
                    let idx = self.scroll + (y - self.areas.list.y) as usize;
                    if down_row == Some(idx)
                        && let Some(entry) = self.visible_entries().get(idx).map(|e| (*e).clone())
                    {
                        self.cursor = idx;
                        self.clamp_cursor();
                        if ev.modifiers.contains(KeyModifiers::CONTROL) {
                            if !self.marked.remove(&entry.path) {
                                self.marked.insert(entry.path);
                            }
                            self.last_click = None;
                        } else if entry.is_dir {
                            // Single click explores directories immediately
                            self.last_click = None;
                            self.navigate_with(entry.path, entry.is_link, true, tx);
                        } else {
                            // Double click opens files
                            let now = Instant::now();
                            let dbl = self.last_click.is_some_and(|(t, i)| {
                                i == idx && now.duration_since(t).as_millis() < 450
                            });
                            if dbl {
                                self.last_click = None;
                                if let Err(e) = self.open_entry(&entry, OpenWith::Editor, tx) {
                                    return ExplorerAction::Toast(e);
                                }
                            } else {
                                self.last_click = Some((now, idx));
                            }
                        }
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if contains(self.areas.list, x, y) => {
                self.mouse_down_row = None;
                let sel = self.selected_entries();
                if !sel.is_empty() {
                    return ExplorerAction::StartDrag(sel);
                }
            }
            _ => {}
        }
        ExplorerAction::None
    }

    // -----------------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------------

    pub fn render(
        &mut self,
        f: &mut Frame,
        area: Rect,
        focused: bool,
        drag_hover_row: Option<u16>,
        drag_active: bool,
    ) {
        self.areas = ExplorerAreas::default();
        self.areas.panel = area;

        let block = Block::new()
            .borders(Borders::LEFT)
            .border_style(theme::divider(focused));
        let inner = block.inner(area);
        f.render_widget(block, area);

        if inner.width < 4 || inner.height < 3 {
            return;
        }

        if !self.is_attached() {
            let lines: Vec<Line> = self
                .placeholder
                .lines()
                .map(|l| Line::from(Span::styled(l.to_string(), theme::dim())))
                .collect();
            let top = inner.y + inner.height / 3;
            let p_area = Rect::new(inner.x, top, inner.width, inner.height.saturating_sub(top - inner.y));
            f.render_widget(Paragraph::new(lines).centered(), p_area);
            return;
        }

        let has_progress = self.active_transfers > 0;
        let active_edits = self.edits.active_names();
        let has_edits = !active_edits.is_empty();
        let has_filter = self.filtering || !self.filter.is_empty();

        let mut constraints = vec![
            Constraint::Length(1), // path bar
            Constraint::Length(1), // toolbar
            Constraint::Length(1), // separator
        ];
        if has_filter {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Min(1)); // list
        if has_progress {
            constraints.push(Constraint::Length(1));
        }
        if has_edits {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Length(1)); // Yazi status bar

        let chunks = Layout::vertical(constraints).split(inner);
        let mut ci = 0;

        // 1. Path bar
        self.areas.path_bar = chunks[ci];
        let tb = icons::toolbar();
        let max_path_cols = (chunks[ci].width as usize).saturating_sub(tb.dir.width() + 3);
        let path_disp = truncate_left_cols(&self.cwd, max_path_cols);
        let path_line = Line::from(vec![
            Span::styled(format!(" {} ", tb.dir), theme::directory()),
            Span::styled(path_disp, theme::accent().add_modifier(Modifier::BOLD)),
        ]);
        f.render_widget(Paragraph::new(path_line), chunks[ci]);
        ci += 1;

        // 2. Toolbar
        self.render_toolbar(f, chunks[ci], drag_active);
        ci += 1;

        // 3. Thin divider
        let rule = "─".repeat(chunks[ci].width as usize);
        f.render_widget(Paragraph::new(Span::styled(rule, theme::dim())), chunks[ci]);
        ci += 1;

        // Optional filter bar
        if has_filter {
            let fl = Line::from(vec![
                Span::styled(" /", theme::key()),
                Span::raw(self.filter.clone()),
                if self.filtering { Span::styled("_", theme::accent()) } else { Span::raw("") },
            ]);
            f.render_widget(Paragraph::new(fl), chunks[ci]);
            ci += 1;
        }

        // 4. List
        let list_area = chunks[ci];
        self.areas.list = list_area;
        ci += 1;
        self.clamp_cursor();
        self.render_list(f, list_area, focused, drag_hover_row);

        // 5. Progress bar
        if has_progress {
            let ratio = self
                .current_progress
                .as_ref()
                .and_then(|p| p.fraction())
                .unwrap_or(0.0);
            let label = format!("Trasferimento… {}%", (ratio * 100.0).round() as u32);
            f.render_widget(
                Gauge::default()
                    .gauge_style(theme::gauge())
                    .ratio(ratio.clamp(0.0, 1.0))
                    .label(label),
                chunks[ci],
            );
            ci += 1;
        }

        // 6. Active edits bar
        if has_edits {
            let txt = format!(" ✎ {}", active_edits.join(", "));
            f.render_widget(Paragraph::new(Span::styled(txt, theme::ok())), chunks[ci]);
            ci += 1;
        }

        // 7. Yazi-style bottom status line (perms, size, badges, pos/total)
        self.render_status_bar(f, chunks[ci]);
    }

    fn render_toolbar(&mut self, f: &mut Frame, area: Rect, drag_active: bool) {
        let mut x = area.x;
        let y = area.y;
        let max_x = area.right();

        let mut spans: Vec<Span> = Vec::new();
        let mut add_btn = |label: &str, style: ratatui::style::Style, slot: &mut Rect| {
            let w = label.width() as u16;
            if x + w <= max_x {
                *slot = Rect::new(x, y, w, 1);
                spans.push(Span::styled(label.to_string(), style));
                x += w;
            }
        };

        let tb = icons::toolbar();
        let back_s = if self.back.is_empty() { theme::dim() } else { theme::key() };
        let fwd_s = if self.forward.is_empty() { theme::dim() } else { theme::key() };
        let up_s = if self.cwd == "/" { theme::dim() } else { theme::key() };
        let hid_s = if self.show_hidden { theme::accent() } else { theme::dim() };
        let fol_s = if self.follow_terminal { theme::ok() } else { theme::dim() };
        let dl_s = if drag_active { theme::drop_target() } else { theme::key() };

        add_btn(&format!(" {} ", tb.back), back_s, &mut self.areas.btn_back);
        add_btn(&format!(" {} ", tb.forward), fwd_s, &mut self.areas.btn_forward);
        add_btn(&format!(" {} ", tb.up), up_s, &mut self.areas.btn_up);
        add_btn(&format!(" {} ", tb.refresh), theme::key(), &mut self.areas.btn_refresh);
        add_btn(
            &format!(" {} ", if self.show_hidden { tb.hidden_on } else { tb.hidden_off }),
            hid_s,
            &mut self.areas.btn_hidden,
        );
        add_btn(&format!(" {} ", tb.follow), fol_s, &mut self.areas.btn_follow);
        add_btn(&format!(" {} ", tb.terminal), theme::key(), &mut self.areas.btn_cd);
        let dl_lbl = if drag_active {
            format!(" {} Scarica ", tb.download)
        } else {
            format!(" {} ", tb.download)
        };
        add_btn(&dl_lbl, dl_s, &mut self.areas.btn_download);
        add_btn(&format!(" {} ", tb.upload), theme::key(), &mut self.areas.btn_upload);

        f.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn render_list(
        &self,
        f: &mut Frame,
        area: Rect,
        focused: bool,
        drag_hover_row: Option<u16>,
    ) {
        let visible = self.visible_entries();
        if visible.is_empty() {
            let msg = if self.loading {
                "  Caricamento…"
            } else {
                "  (vuota)"
            };
            f.render_widget(Paragraph::new(Span::styled(msg, theme::dim())), area);
            return;
        }
        let cut_set: HashSet<&str> = self
            .clipboard
            .as_ref()
            .filter(|c| c.cut)
            .map(|c| c.paths.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let copy_set: HashSet<&str> = self
            .clipboard
            .as_ref()
            .filter(|c| !c.cut)
            .map(|c| c.paths.iter().map(String::as_str).collect())
            .unwrap_or_default();

        let h = area.height as usize;
        let w = area.width as usize;
        let mut lines: Vec<Line> = Vec::with_capacity(h);

        for (offset, entry) in visible.iter().skip(self.scroll).take(h).enumerate() {
            let idx = self.scroll + offset;
            let is_cur = idx == self.cursor;
            let is_marked = self.marked.contains(&entry.path);
            let is_cut = cut_set.contains(entry.path.as_str());
            let is_yanked = copy_set.contains(entry.path.as_str());
            let is_hover = drag_hover_row == Some(area.y + offset as u16) && entry.is_dir;

            let (icon, icon_base_style) = icons::for_entry(entry);
            let mark = if is_marked { "▎" } else { " " };
            let size_txt = if entry.is_dir {
                String::new()
            } else {
                human_size(entry.size)
            };

            let name_style = if is_hover {
                theme::drop_target()
            } else if is_cut {
                theme::cut()
            } else if is_marked {
                theme::marked()
            } else if entry.is_dir {
                theme::directory()
            } else if entry.is_link {
                theme::symlink()
            } else if entry.perms & 0o111 != 0 {
                theme::executable()
            } else if entry.name.starts_with('.') {
                theme::hidden()
            } else {
                theme::base()
            };

            let icon_w = icon.width();
            let prefix_w = mark.width() + icon_w + 1; // mark + icon + space
            let right_w = if size_txt.is_empty() { 0 } else { size_txt.width() + 1 };
            let name_max = w.saturating_sub(prefix_w + right_w);
            let name_disp = truncate_right_cols(&entry.name, name_max);
            let name_cols = name_disp.width();
            let pad = w.saturating_sub(prefix_w + name_cols + size_txt.width());

            let row_mod = if is_cur { theme::selection(focused) } else { theme::base() };
            let mark_style = if is_marked {
                theme::marked().patch(row_mod)
            } else if is_yanked {
                theme::ok().patch(row_mod)
            } else {
                theme::dim().patch(row_mod)
            };
            let icon_style = if is_hover || is_cut || is_marked {
                name_style.patch(row_mod)
            } else {
                icon_base_style.patch(row_mod)
            };
            let main_style = if is_cur {
                name_style.patch(row_mod)
            } else {
                name_style
            };
            let size_style = theme::dim().patch(row_mod);

            lines.push(Line::from(vec![
                Span::styled(mark, mark_style),
                Span::styled(format!("{icon} "), icon_style.add_modifier(Modifier::empty())),
                Span::styled(name_disp, main_style),
                Span::styled(" ".repeat(pad), row_mod),
                Span::styled(size_txt, size_style),
            ]));
        }
        f.render_widget(Paragraph::new(lines), area);
    }

    fn render_status_bar(&self, f: &mut Frame, area: Rect) {
        let visible = self.visible_entries();
        let total = visible.len();
        let pos = if total == 0 {
            "0/0".to_string()
        } else {
            format!("{}/{}", self.cursor + 1, total)
        };

        let mut right_parts: Vec<String> = Vec::new();
        if self.loading {
            right_parts.push("…".to_string());
        }
        if self.visual.is_some() {
            right_parts.push("VISUAL".to_string());
        } else if self.pending_g {
            right_parts.push("g…".to_string());
        }
        if self.sort_mode != SortMode::Name {
            right_parts.push(self.sort_mode.label().to_string());
        }
        right_parts.push(pos);
        let right = format!("{} ", right_parts.join("  "));

        let mut left_parts: Vec<String> = Vec::new();
        if let Some(cur) = visible.get(self.cursor) {
            left_parts.push(cur.perm_string());
            if !cur.is_dir {
                left_parts.push(human_size(cur.size));
            }
        }
        if !self.marked.is_empty() {
            left_parts.push(format!("{} sel", self.marked.len()));
        }
        if let Some(clip) = &self.clipboard {
            left_parts.push(format!(
                "{}:{}",
                if clip.cut { "x" } else { "y" },
                clip.paths.len()
            ));
        }

        let left_raw = if left_parts.is_empty() {
            String::new()
        } else {
            format!(" {}", left_parts.join("  "))
        };

        let w = area.width as usize;
        let right_w = right.width();
        let max_left = w.saturating_sub(right_w + 1);
        let left = truncate_right_cols(&left_raw, max_left);
        let pad = w.saturating_sub(left.width() + right_w);

        let line = Line::from(vec![
            Span::styled(left, theme::dim()),
            Span::raw(" ".repeat(pad)),
            Span::styled(
                right,
                if self.visual.is_some() || self.pending_g {
                    theme::accent()
                } else {
                    theme::dim()
                },
            ),
        ]);
        f.render_widget(Paragraph::new(line), area);
    }
}

fn truncate_right_cols(s: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }
    if s.width() <= max_cols {
        return s.to_string();
    }
    if max_cols == 1 {
        return "…".to_string();
    }
    let mut out = String::new();
    let mut cols = 0;
    let limit = max_cols - 1;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if cols + cw > limit {
            break;
        }
        out.push(ch);
        cols += cw;
    }
    out.push('…');
    out
}

fn truncate_left_cols(s: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }
    if s.width() <= max_cols {
        return s.to_string();
    }
    if max_cols == 1 {
        return "…".to_string();
    }
    let limit = max_cols - 1;
    let mut rev_chars: Vec<char> = Vec::new();
    let mut cols = 0;
    for ch in s.chars().rev() {
        let cw = ch.width().unwrap_or(0);
        if cols + cw > limit {
            break;
        }
        rev_chars.push(ch);
        cols += cw;
    }
    rev_chars.reverse();
    let mut out = String::from('…');
    out.extend(rev_chars);
    out
}

pub fn contains(r: Rect, x: u16, y: u16) -> bool {
    r.width > 0 && r.height > 0 && x >= r.x && x < r.right() && y >= r.y && y < r.bottom()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_entry(name: &str, is_dir: bool, size: u64, mtime: u32) -> Entry {
        Entry {
            name: name.to_string(),
            path: format!("/remote/{name}"),
            is_dir,
            is_link: false,
            size,
            mtime,
            perms: if is_dir { 0o755 } else { 0o644 },
        }
    }

    #[test]
    fn sort_modes_keep_directories_first() {
        let mut exp = Explorer::new(1);
        exp.entries = vec![
            mk_entry("b.txt", false, 50, 100),
            mk_entry("alpha", true, 0, 10),
            mk_entry("a.txt", false, 200, 300),
        ];

        exp.sort_mode = SortMode::Name;
        exp.apply_sort();
        let names: Vec<&str> = exp.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alpha", "a.txt", "b.txt"]);

        exp.sort_mode = SortMode::SizeDesc;
        exp.apply_sort();
        let names: Vec<&str> = exp.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alpha", "a.txt", "b.txt"]);

        exp.sort_mode = SortMode::NameDesc;
        exp.apply_sort();
        let names: Vec<&str> = exp.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alpha", "b.txt", "a.txt"]);
    }

    #[test]
    fn visual_selection_tracks_range_and_restores_on_cancel() {
        let mut exp = Explorer::new(1);
        exp.entries = vec![
            mk_entry("one", false, 1, 1),
            mk_entry("two", false, 2, 2),
            mk_entry("three", false, 3, 3),
        ];
        exp.marked.insert("/remote/one".to_string());
        exp.visual = Some((1, exp.marked.clone()));
        exp.set_cursor(2);
        assert!(exp.marked.contains("/remote/one"));
        assert!(exp.marked.contains("/remote/two"));
        assert!(exp.marked.contains("/remote/three"));

        if let Some((_, prev)) = exp.visual.take() {
            exp.marked = prev;
        }
        assert_eq!(exp.marked.len(), 1);
        assert!(exp.marked.contains("/remote/one"));
    }

    #[test]
    fn col_truncation_keeps_tail_for_paths_and_head_for_names() {
        assert_eq!(truncate_left_cols("/home/leo/projects/riso", 10), "…ects/riso");
        assert_eq!(truncate_right_cols("very_long_filename.rs", 10), "very_long…");
        assert_eq!(truncate_right_cols("short.rs", 10), "short.rs");
    }
}
