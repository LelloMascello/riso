//! TUI: a single SSH session tab (VT100 terminal with fixed `ssh` prompt + SFTP explorer).

use std::io::Write;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::event::AppEvent;
use super::explorer::Explorer;
use super::prompt::{Prompt, PromptAction};
use super::pty::Pty;
use super::term::Term;
use super::theme;
use crate::config::{HostStore, ParsedSsh};
use crate::remote::{Remote, SshTarget, shell_quote};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Prompt,
    Connecting,
    Connected,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Idle,
    Wait,
    Ok,
    Err,
}

pub struct Session {
    pub id: u64,
    pub state: SessionState,
    pub status_kind: StatusKind,
    pub needs_attention: bool,

    pub prompt: Prompt,
    pub parsed: Option<ParsedSsh>,
    pub target: Option<SshTarget>,
    pub pty: Option<Pty>,
    pub term: Term,
    pub term_area: Rect,

    pub explorer: Explorer,
}

impl Session {
    pub fn new() -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let mut term = Term::new(24, 80);
        let prompt = Prompt::new();
        prompt.write_initial(&mut term);
        Session {
            id,
            state: SessionState::Prompt,
            status_kind: StatusKind::Idle,
            needs_attention: false,
            prompt,
            parsed: None,
            target: None,
            pty: None,
            term,
            term_area: Rect::new(0, 0, 80, 24),
            explorer: Explorer::new(id),
        }
    }

    pub fn is_busy(&self) -> bool {
        self.state != SessionState::Prompt
    }

    pub fn tab_title(&self) -> String {
        if let Some(p) = &self.parsed {
            let alias = p.conn.name.trim();
            if !alias.is_empty() {
                return alias.to_string();
            }
        }
        if let Some(t) = &self.target {
            return t.label();
        }
        "Nuova connessione".to_string()
    }

    pub fn status_dot(&self) -> (&'static str, Style) {
        match self.status_kind {
            StatusKind::Ok => ("●", theme::ok()),
            StatusKind::Wait => ("◐", theme::wait()),
            StatusKind::Err => ("○", theme::error()),
            StatusKind::Idle => ("○", theme::dim()),
        }
    }

    pub fn start_parsed(&mut self, parsed: ParsedSsh, store: &HostStore, tx: &Sender<AppEvent>) {
        store.record(&parsed);
        let target = SshTarget::from_parsed(&parsed);
        // Mirror what was launched when started from CLI arguments.
        if self.prompt.input.is_empty() {
            let cmd = if !parsed.raw_command.trim().is_empty() {
                parsed.raw_command.trim().to_string()
            } else {
                target.label()
            };
            self.term.process(cmd.as_bytes());
            self.term.process(b"\r\n");
        }
        self.parsed = Some(parsed);
        self.target = Some(target);
        self.connect(tx);
    }

    pub fn handle_prompt_key(
        &mut self,
        key: ratatui::crossterm::event::KeyEvent,
        store: &HostStore,
        tx: &Sender<AppEvent>,
    ) -> Option<String> {
        if let PromptAction::Submit(line) = self.prompt.handle_key(key, store, &mut self.term) {
            if let Some(parsed) = store.resolve(&line) {
                self.start_parsed(parsed, store, tx);
            } else {
                let err = format!("Destinazione non valida: {line}");
                self.prompt.reset_into(&mut self.term);
                return Some(err);
            }
        }
        None
    }

    fn connect(&mut self, tx: &Sender<AppEvent>) {
        let Some(target) = self.target.as_mut() else { return };
        target.renew_control_path();
        let argv = target.terminal_argv();
        let rows = self.term_area.height.max(4);
        let cols = self.term_area.width.max(10);

        self.term.reset_scroll();
        self.explorer.show_waiting();

        match Pty::spawn(&argv, rows, cols, self.id, tx.clone()) {
            Ok(pty) => {
                self.pty = Some(pty);
                self.state = SessionState::Connecting;
                self.status_kind = StatusKind::Wait;
            }
            Err(e) => {
                self.state = SessionState::Prompt;
                self.status_kind = StatusKind::Err;
                self.term.process(format!("Impossibile avviare ssh: {e}\r\n").as_bytes());
                self.prompt.reset_into(&mut self.term);
            }
        }
    }

    pub fn poll_master(&mut self, tx: &Sender<AppEvent>) {
        if self.state != SessionState::Connecting {
            return;
        }
        let Some(target) = &self.target else { return };
        if !target.master_ready() {
            return;
        }
        self.state = SessionState::Connected;
        self.status_kind = StatusKind::Ok;
        let browse = Rc::new(Remote::start(target));
        let transfer = Rc::new(Remote::start(target));
        let initial = self
            .parsed
            .as_ref()
            .map(|p| p.conn.initial_dir.trim().to_string())
            .filter(|s| !s.is_empty());
        self.explorer.attach(browse, transfer, initial, tx);
    }

    pub fn on_pty_output(&mut self, bytes: &[u8], is_active: bool, tx: &Sender<AppEvent>) {
        self.term.process(bytes);
        let cb = self.term.callbacks();
        let replies = std::mem::take(&mut cb.replies);
        let cwd = cb.cwd.take();
        let clip = std::mem::take(&mut cb.clipboard);
        let bell = std::mem::replace(&mut cb.bell, false);

        if !replies.is_empty()
            && let Some(pty) = &mut self.pty
        {
            pty.write(&replies);
        }
        if let Some(dir) = cwd {
            self.explorer.terminal_cwd_changed(&dir, tx);
        }
        if !clip.is_empty() {
            let mut out = std::io::stdout().lock();
            for data in clip {
                let _ = out.write_all(b"\x1b]52;c;");
                let _ = out.write_all(&data);
                let _ = out.write_all(b"\x07");
            }
            let _ = out.flush();
        }
        if bell && !is_active {
            self.needs_attention = true;
        }
    }

    pub fn on_pty_closed(&mut self, is_active: bool) {
        let was_connected = self.state == SessionState::Connected;
        let code = self.pty.as_mut().and_then(|p| p.wait()).and_then(|s| s.code());
        self.pty = None;
        if let Some(t) = &self.target {
            t.exit_master();
        }
        self.state = SessionState::Prompt;
        self.explorer.detach(if was_connected {
            "Disconnesso"
        } else {
            "In attesa di connessione"
        });
        let failed = code.is_some_and(|c| c != 0);
        self.status_kind = if failed { StatusKind::Err } else { StatusKind::Idle };
        if !is_active {
            self.needs_attention = true;
        }
        self.prompt.reset_into(&mut self.term);
    }

    pub fn cd_terminal(&mut self, dir: &str) {
        if self.state != SessionState::Connected {
            return;
        }
        let cmd = format!(" cd -- {}\n", shell_quote(dir));
        if let Some(pty) = &mut self.pty {
            pty.write(cmd.as_bytes());
        }
    }

    pub fn feed_text(&mut self, text: &str) {
        match self.state {
            SessionState::Prompt => self.prompt.insert_str(text, &mut self.term),
            SessionState::Connecting | SessionState::Connected => {
                let bytes = self.term.paste_bytes(text);
                if let Some(pty) = &mut self.pty {
                    self.term.reset_scroll();
                    pty.write(&bytes);
                }
            }
        }
    }

    pub fn resize_inner(&mut self, rows: u16, cols: u16) {
        let rows = rows.max(1);
        let cols = cols.max(1);
        if self.term_area.height != rows || self.term_area.width != cols {
            self.term.resize(rows, cols);
            if let Some(pty) = &self.pty {
                pty.resize(rows, cols);
            }
        }
    }

    pub fn shutdown(&mut self) {
        if let Some(mut pty) = self.pty.take() {
            pty.kill();
        }
        if let Some(t) = &self.target {
            t.exit_master();
        }
        self.explorer.detach("Chiuso");
    }

    pub fn render_terminal(&mut self, f: &mut Frame, area: Rect, focused: bool) -> Option<(u16, u16)> {
        if area.width == 0 || area.height == 0 {
            return None;
        }
        self.resize_inner(area.height, area.width);
        self.term_area = area;

        let cur = self.term.render(area, f.buffer_mut());
        if focused { cur } else { None }
    }
}
