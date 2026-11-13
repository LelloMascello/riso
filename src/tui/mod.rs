//! `riso-tui`: full Ratatui terminal frontend.
//!
//! Toolkit-free (no GTK/GLib): builds on the shared crate-root modules and uses
//! pure ANSI terminal colours so external terminal theme switches apply live.

mod dialog;
mod drop;
mod edit;
mod event;
mod explorer;
mod icons;
mod line_input;
mod prompt;
mod pty;
mod session;
mod theme;

mod term;

use std::io::{self, Stdout, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate, EnterAlternateScreen,
    LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use dialog::{ConfirmKind, Dialog, DialogResult, PromptKind};
use drop::{DragSession, parse_dropped_paths, parse_user_path_list};
use edit::LaunchAction;
use event::{AppEvent, InputThread, OpenWith, SftpMsg};
use explorer::{ExplorerAction, contains};
use session::{Session, SessionState};
use crate::config::HostStore;
use crate::paths;
use crate::remote::shell_quote;
use crate::sftp;

const USAGE: &str = "\
Uso: riso-tui [argomenti-ssh…]

Senza argomenti avvia l'interfaccia interattiva (schede SSH + explorer SFTP).
Con argomenti si connette direttamente, es. `riso-tui -p 2222 utente@host`.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Terminal,
    Explorer,
}

#[derive(Default)]
struct TopBarAreas {
    prev_tab: Rect,
    next_tab: Rect,
    tabs: Vec<(Rect, Rect)>, // (tab_rect, close_rect)
    new_tab: Rect,
    sftp_toggle: Rect,
}

struct Toast {
    msg: String,
    expires: Instant,
}

struct TermGuard;

impl TermGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut out = io::stdout();
        execute!(
            out,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste,
            EnableFocusChange
        )?;
        // DEC private mode 2031: ask modern terminals for live colour-scheme
        // updates; terminals that do not support it ignore the sequence.
        let _ = out.write_all(b"\x1b[?2031h");
        let _ = out.flush();
        Ok(TermGuard)
    }
}

impl Drop for TermGuard {
    fn drop(&mut self) {
        let mut out = io::stdout();
        let _ = out.write_all(b"\x1b[?2031l");
        let _ = out.flush();
        let _ = execute!(
            out,
            DisableFocusChange,
            DisableBracketedPaste,
            DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = disable_raw_mode();
    }
}

/// Runs the terminal application.
pub fn run(args: Vec<String>) -> ExitCode {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    let store = HostStore::new();
    let initial = if args.is_empty() {
        None
    } else {
        let line = args
            .iter()
            .map(|a| shell_quote(a))
            .collect::<Vec<_>>()
            .join(" ");
        match store.resolve(&line) {
            Some(p) => Some(p),
            None => {
                eprintln!("riso-tui: destinazione ssh non valida: {}", args.join(" "));
                return ExitCode::from(2);
            }
        }
    };

    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut out = io::stdout();
        let _ = execute!(
            out,
            DisableFocusChange,
            DisableBracketedPaste,
            DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = disable_raw_mode();
        prev_hook(info);
    }));

    let Ok(_guard) = TermGuard::enter() else {
        eprintln!("riso-tui: impossibile inizializzare il terminale");
        return ExitCode::FAILURE;
    };
    let backend = CrosstermBackend::new(io::stdout());
    let Ok(mut terminal) = Terminal::new(backend) else {
        eprintln!("riso-tui: impossibile avviare ratatui");
        return ExitCode::FAILURE;
    };

    let (tx, rx) = mpsc::channel::<AppEvent>();
    let input = InputThread::spawn(tx.clone());

    let mut app = App::new(store, tx, initial);
    app.run_loop(&mut terminal, &rx, &input);
    app.shutdown();

    ExitCode::SUCCESS
}

struct App {
    store: Rc<HostStore>,
    tx: Sender<AppEvent>,
    sessions: Vec<Session>,
    active: usize,
    focus: Focus,
    show_explorer: bool,
    dialog: Option<Dialog>,
    toast: Option<Toast>,
    drag: Option<DragSession>,
    drag_pos: Option<(u16, u16)>,
    top_areas: TopBarAreas,
    should_quit: bool,
    force_clear: bool,
}

impl App {
    fn new(store: Rc<HostStore>, tx: Sender<AppEvent>, initial: Option<crate::config::ParsedSsh>) -> Self {
        let mut first = Session::new();
        if let Some(parsed) = initial {
            first.start_parsed(parsed, &store, &tx);
        }
        App {
            store,
            tx,
            sessions: vec![first],
            active: 0,
            focus: Focus::Terminal,
            show_explorer: true,
            dialog: None,
            toast: None,
            drag: None,
            drag_pos: None,
            top_areas: TopBarAreas::default(),
            should_quit: false,
            force_clear: false,
        }
    }

    fn active_session(&self) -> &Session {
        &self.sessions[self.active]
    }

    fn active_session_mut(&mut self) -> &mut Session {
        &mut self.sessions[self.active]
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some(Toast {
            msg: msg.into(),
            expires: Instant::now() + Duration::from_millis(3500),
        });
    }

    fn new_tab(&mut self) {
        let s = Session::new();
        self.sessions.push(s);
        self.active = self.sessions.len() - 1;
        self.focus = Focus::Terminal;
    }

    fn select_tab(&mut self, idx: usize) {
        if idx < self.sessions.len() {
            self.active = idx;
            self.sessions[idx].needs_attention = false;
            if !self.sessions[idx].explorer.is_attached() {
                self.focus = Focus::Terminal;
            }
        }
    }

    fn request_close_tab(&mut self, idx: usize) {
        if idx >= self.sessions.len() {
            return;
        }
        if self.sessions[idx].is_busy() {
            let title = self.sessions[idx].tab_title();
            self.dialog = Some(Dialog::confirm(
                "Chiudere la scheda?",
                format!("La sessione «{title}» è attiva."),
                "Chiudi",
                true,
                ConfirmKind::CloseActiveTab(idx),
            ));
        } else {
            self.close_tab_now(idx);
        }
    }

    fn close_tab_now(&mut self, idx: usize) {
        if idx >= self.sessions.len() {
            return;
        }
        let mut s = self.sessions.remove(idx);
        s.shutdown();
        if self.sessions.is_empty() {
            self.should_quit = true;
            return;
        }
        if self.active >= self.sessions.len() {
            self.active = self.sessions.len() - 1;
        } else if idx < self.active {
            self.active -= 1;
        }
        self.sessions[self.active].needs_attention = false;
    }

    fn shutdown(&mut self) {
        for s in &mut self.sessions {
            s.shutdown();
        }
    }

    fn run_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        rx: &Receiver<AppEvent>,
        input: &InputThread,
    ) {
        let mut dirty = true;
        while !self.should_quit {
            if std::mem::take(&mut self.force_clear) {
                clear_screen(terminal);
                dirty = true;
            }
            if dirty {
                self.draw(terminal);
                dirty = false;
            }

            match rx.recv_timeout(Duration::from_millis(120)) {
                Ok(ev) => {
                    dirty |= self.handle_event(ev, terminal, input);
                    while !self.should_quit {
                        match rx.try_recv() {
                            Ok(more) => dirty |= self.handle_event(more, terminal, input),
                            Err(_) => break,
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }

            if self.should_quit {
                break;
            }
            dirty |= self.tick();
        }
    }

    fn tick(&mut self) -> bool {
        let mut changed = false;
        let tx = self.tx.clone();
        for s in &mut self.sessions {
            let was_attached = s.explorer.is_attached();
            s.poll_master(&tx);
            if s.explorer.is_attached() != was_attached {
                changed = true;
            }
            let tr = s.explorer.transfer_remote();
            s.explorer.edits.poll_changes(s.id, tr.as_deref(), &tx);
        }
        if let Some(t) = &self.toast
            && Instant::now() >= t.expires
        {
            self.toast = None;
            changed = true;
        }
        changed
    }

    fn handle_event(
        &mut self,
        ev: AppEvent,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        input: &InputThread,
    ) -> bool {
        match ev {
            AppEvent::Input(iev) => match iev {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    self.handle_key(key);
                    true
                }
                Event::Mouse(m) => {
                    if m.kind == MouseEventKind::Moved && self.drag.is_none() {
                        return false;
                    }
                    self.handle_mouse(m);
                    true
                }
                Event::Paste(text) => {
                    self.handle_paste(text);
                    true
                }
                Event::FocusGained => {
                    self.force_clear = true;
                    true
                }
                Event::Resize(_, _) => true,
                _ => false,
            },
            AppEvent::PtyOutput(sid, bytes) => {
                let active_id = self.active_session().id;
                let tx = self.tx.clone();
                if let Some(s) = self.sessions.iter_mut().find(|s| s.id == sid) {
                    s.on_pty_output(&bytes, sid == active_id, &tx);
                }
                sid == active_id
            }
            AppEvent::PtyClosed(sid) => {
                let active_id = self.active_session().id;
                if let Some(s) = self.sessions.iter_mut().find(|s| s.id == sid) {
                    s.on_pty_closed(sid == active_id);
                    if sid == active_id {
                        self.focus = Focus::Terminal;
                    }
                }
                true
            }
            AppEvent::Sftp(sid, msg) => {
                self.handle_sftp(sid, msg, terminal, input);
                true
            }
        }
    }

    fn handle_sftp(
        &mut self,
        sid: u64,
        msg: SftpMsg,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        input: &InputThread,
    ) {
        match msg {
            SftpMsg::UploadConflicts { files, dest, existing } => {
                if existing.is_empty() {
                    if let Some(s) = self.sessions.iter_mut().find(|s| s.id == sid) {
                        s.explorer.upload_exec(files, dest, &self.tx);
                    }
                } else {
                    self.dialog = Some(Dialog::confirm(
                        "Sostituire i file esistenti?",
                        existing.join(", "),
                        "Sostituisci",
                        true,
                        ConfirmKind::OverwriteUpload { files, dest },
                    ));
                }
            }
            SftpMsg::EditReady { remote, local, how, result } => {
                if let Err(e) = result {
                    self.toast(format!("{}: {e}", sftp::basename(&remote)));
                    return;
                }
                let action = {
                    let Some(s) = self.sessions.iter_mut().find(|s| s.id == sid) else {
                        return;
                    };
                    s.explorer.edits.on_downloaded(remote, local.clone(), how)
                };
                match action {
                    Ok(LaunchAction::Background { argv, cwd }) => {
                        if let Err(e) = edit::spawn_background(&argv, &cwd) {
                            self.toast(e);
                        }
                    }
                    Ok(LaunchAction::Foreground { argv, cwd }) => {
                        self.run_foreground_editor(sid, &local, &argv, &cwd, terminal, input);
                    }
                    Err(e) => self.toast(e),
                }
            }
            other => {
                let tx = self.tx.clone();
                if let Some(s) = self.sessions.iter_mut().find(|s| s.id == sid)
                    && let Some(msg) = s.explorer.handle_sftp(other, &tx)
                {
                    self.toast(msg);
                }
            }
        }
    }

    fn run_foreground_editor(
        &mut self,
        sid: u64,
        local: &Path,
        argv: &[String],
        cwd: &Path,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        input: &InputThread,
    ) {
        input.pause();
        let mut out = io::stdout();
        let _ = execute!(
            out,
            DisableFocusChange,
            DisableBracketedPaste,
            DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = disable_raw_mode();

        let status = Command::new(&argv[0])
            .args(&argv[1..])
            .current_dir(cwd)
            .status();

        let _ = enable_raw_mode();
        let _ = execute!(
            out,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste,
            EnableFocusChange
        );
        clear_screen(terminal);
        input.resume();

        if let Err(e) = status {
            self.toast(format!("Impossibile avviare {}: {e}", argv[0]));
        }
        let tx = self.tx.clone();
        if let Some(s) = self.sessions.iter_mut().find(|s| s.id == sid) {
            let tr = s.explorer.transfer_remote();
            s.explorer.edits.sync_now(sid, local, tr.as_deref(), &tx);
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if let Some(mut dlg) = self.dialog.take() {
            match dlg.handle_key(key) {
                DialogResult::None => self.dialog = Some(dlg),
                DialogResult::Close => {}
                DialogResult::PromptSubmit { kind, value } => self.on_prompt_submit(kind, value),
                DialogResult::ConfirmYes(kind) => self.on_confirm_yes(kind),
                DialogResult::DropUpload { paths, dest } => {
                    let tx = self.tx.clone();
                    self.active_session_mut().explorer.check_upload(paths, dest, &tx);
                }
                DialogResult::DropPasteTerminal(raw) => {
                    self.active_session_mut().feed_text(&raw);
                }
                DialogResult::OpenWithChosen { entry, how } => {
                    let tx = self.tx.clone();
                    if let Err(e) = self.active_session_mut().explorer.open_entry(&entry, how, &tx) {
                        self.toast(e);
                    }
                }
            }
            return;
        }

        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);

        // Global shortcuts (Alt-based so they never collide with remote shell / vim)
        if alt {
            match key.code {
                KeyCode::Char('q') => {
                    self.should_quit = true;
                    return;
                }
                KeyCode::Char('n') => {
                    self.new_tab();
                    return;
                }
                KeyCode::Char('w') => {
                    self.request_close_tab(self.active);
                    return;
                }
                KeyCode::Char('[') => {
                    let n = self.sessions.len();
                    if n > 1 {
                        self.select_tab((self.active + n - 1) % n);
                    }
                    return;
                }
                KeyCode::Char(']') => {
                    let n = self.sessions.len();
                    if n > 1 {
                        self.select_tab((self.active + 1) % n);
                    }
                    return;
                }
                KeyCode::Char(c @ '1'..='9') => {
                    self.select_tab((c as u8 - b'1') as usize);
                    return;
                }
                KeyCode::Char('b') => {
                    self.show_explorer = !self.show_explorer;
                    if !self.show_explorer {
                        self.focus = Focus::Terminal;
                    }
                    return;
                }
                KeyCode::Char('s') | KeyCode::Char('e') => {
                    self.toggle_focus();
                    return;
                }
                KeyCode::Char('r') => {
                    self.force_clear = true;
                    self.toast("Schermo ridisegnato");
                    return;
                }
                KeyCode::Char('?') | KeyCode::Char('/') => {
                    self.dialog = Some(Dialog::Help);
                    return;
                }
                _ => {}
            }
        }
        if key.code == KeyCode::F(1) {
            self.dialog = Some(Dialog::Help);
            return;
        }

        match self.focus {
            Focus::Explorer => {
                if key.code == KeyCode::Tab && !self.active_session().explorer.filtering {
                    self.focus = Focus::Terminal;
                    return;
                }
                let tx = self.tx.clone();
                let act = self.active_session_mut().explorer.handle_key(key, &tx);
                self.handle_explorer_action(act);
            }
            Focus::Terminal => {
                let state = self.active_session().state;
                if state == SessionState::Prompt {
                    let store = self.store.clone();
                    let tx = self.tx.clone();
                    if let Some(err) = self.active_session_mut().handle_prompt_key(key, &store, &tx) {
                        self.toast(err);
                    }
                    return;
                }

                if shift {
                    match key.code {
                        KeyCode::PageUp => {
                            self.active_session_mut().term.scroll_by(10);
                            return;
                        }
                        KeyCode::PageDown => {
                            self.active_session_mut().term.scroll_by(-10);
                            return;
                        }
                        _ => {}
                    }
                }

                let sess = self.active_session_mut();
                let app_cursor = sess.term.screen().application_cursor();
                if let Some(bytes) = term::key_bytes(&key, app_cursor)
                    && let Some(pty) = &mut sess.pty
                {
                    sess.term.reset_scroll();
                    pty.write(&bytes);
                }
            }
        }
    }

    fn toggle_focus(&mut self) {
        if self.focus == Focus::Explorer {
            self.focus = Focus::Terminal;
        } else {
            self.show_explorer = true;
            self.focus = Focus::Explorer;
        }
    }

    fn handle_explorer_action(&mut self, act: ExplorerAction) {
        let tx = self.tx.clone();
        match act {
            ExplorerAction::None => {}
            ExplorerAction::Toast(m) => self.toast(m),
            ExplorerAction::CdTerminal(dir) => {
                self.active_session_mut().cd_terminal(&dir);
                self.focus = Focus::Terminal;
            }
            ExplorerAction::OpenDefault(entry) => {
                if let Err(e) = self
                    .active_session_mut()
                    .explorer
                    .open_entry(&entry, OpenWith::Default, &tx)
                {
                    self.toast(e);
                }
            }
            ExplorerAction::OpenWithPrompt(entry) => {
                self.dialog = Some(Dialog::OpenWithPick { entry, selected: 0 });
            }
            ExplorerAction::RenamePrompt(entry) => {
                let name = entry.name.clone();
                self.dialog = Some(Dialog::prompt("Rinomina", "", &name, PromptKind::Rename(entry)));
            }
            ExplorerAction::NewFilePrompt => {
                self.dialog = Some(Dialog::prompt("Nuovo file", "", "", PromptKind::NewFile));
            }
            ExplorerAction::NewDirPrompt => {
                self.dialog = Some(Dialog::prompt("Nuova cartella", "", "", PromptKind::NewDir));
            }
            ExplorerAction::GoToPathPrompt(cwd) => {
                self.dialog = Some(Dialog::prompt("Vai a percorso", "", &cwd, PromptKind::GoToPath));
            }
            ExplorerAction::UploadPrompt(dest) => {
                self.dialog = Some(Dialog::prompt(
                    "Carica file o cartelle",
                    "Inserisci o trascina qui percorsi locali (separati da spazi)",
                    "",
                    PromptKind::Upload { dest },
                ));
            }
            ExplorerAction::DownloadPrompt(entries) => {
                let default_dir = default_download_dir();
                self.dialog = Some(Dialog::prompt(
                    format!("Scarica {} elementi in", entries.len()),
                    "Inserisci o trascina una cartella locale",
                    &default_dir.to_string_lossy(),
                    PromptKind::Download { entries },
                ));
            }
            ExplorerAction::DeleteConfirm(entries) => {
                let heading = if entries.len() == 1 {
                    format!("Eliminare «{}»?", entries[0].name)
                } else {
                    format!("Eliminare {} elementi?", entries.len())
                };
                self.dialog = Some(Dialog::confirm(
                    heading,
                    "",
                    "Elimina",
                    true,
                    ConfirmKind::Delete(entries),
                ));
            }
            ExplorerAction::StartDrag(entries) => {
                if self.drag.is_none() {
                    let sid = self.active_session().id;
                    self.drag = Some(DragSession {
                        source_session_id: sid,
                        entries,
                    });
                }
            }
        }
    }

    fn on_prompt_submit(&mut self, kind: PromptKind, value: String) {
        let tx = self.tx.clone();
        match kind {
            PromptKind::Rename(entry) => {
                if let Err(e) = self.active_session().explorer.rename(&entry, value, &tx) {
                    self.toast(e);
                }
            }
            PromptKind::NewFile => {
                if let Err(e) = self.active_session().explorer.create_new(value, false, &tx) {
                    self.toast(e);
                }
            }
            PromptKind::NewDir => {
                if let Err(e) = self.active_session().explorer.create_new(value, true, &tx) {
                    self.toast(e);
                }
            }
            PromptKind::GoToPath => {
                self.active_session_mut().explorer.navigate(value, true, &tx);
            }
            PromptKind::Upload { dest } => {
                let paths = parse_user_path_list(&value);
                let missing: Vec<String> = paths
                    .iter()
                    .filter(|p| !p.exists())
                    .map(|p| p.display().to_string())
                    .collect();
                if paths.is_empty() {
                    return;
                }
                if !missing.is_empty() {
                    self.toast(format!("Percorso inesistente: {}", missing[0]));
                    return;
                }
                self.active_session().explorer.check_upload(paths, dest, &tx);
            }
            PromptKind::Download { entries } => {
                let dir = drop::expand_path(value);
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    self.toast(format!("Cartella locale non valida: {e}"));
                    return;
                }
                self.active_session_mut().explorer.download_exec(entries, dir, &tx);
            }
            PromptKind::OpenWithCmd(entry) => {
                if value.is_empty() {
                    self.dialog = Some(Dialog::prompt(
                        format!("Apri {} con…", entry.name),
                        "Es. nvim, helix, bat, less",
                        "",
                        PromptKind::OpenWithCmd(entry),
                    ));
                } else if let Err(e) = self
                    .active_session_mut()
                    .explorer
                    .open_entry(&entry, OpenWith::Command(value), &tx)
                {
                    self.toast(e);
                }
            }
        }
    }

    fn on_confirm_yes(&mut self, kind: ConfirmKind) {
        let tx = self.tx.clone();
        match kind {
            ConfirmKind::Delete(entries) => {
                self.active_session_mut().explorer.delete(entries, &tx);
            }
            ConfirmKind::OverwriteUpload { files, dest } => {
                self.active_session_mut().explorer.upload_exec(files, dest, &tx);
            }
            ConfirmKind::CloseActiveTab(idx) => {
                self.close_tab_now(idx);
            }
        }
    }

    fn handle_paste(&mut self, text: String) {
        if let Some(dlg) = &mut self.dialog {
            dlg.handle_paste(&text);
            return;
        }
        if let Some(dropped) = parse_dropped_paths(&text) {
            let connected = self.active_session().explorer.is_attached();
            if connected {
                if dropped.is_uri || self.focus == Focus::Explorer {
                    let tx = self.tx.clone();
                    let dest = self.active_session().explorer.target_dir();
                    self.active_session().explorer.check_upload(dropped.paths, dest, &tx);
                    return;
                }
                let dest = self.active_session().explorer.target_dir();
                self.dialog = Some(Dialog::DropChoice {
                    raw: text,
                    paths: dropped.paths,
                    dest,
                });
                return;
            } else if dropped.is_uri {
                self.toast("Connettiti prima di caricare file via SFTP");
                return;
            }
        }
        self.active_session_mut().feed_text(&text);
    }

    fn handle_mouse(&mut self, ev: MouseEvent) {
        if self.dialog.is_some() {
            return;
        }
        let (x, y) = (ev.column, ev.row);

        // Handle finish of an in-app Explorer drag-and-drop
        if matches!(ev.kind, MouseEventKind::Up(MouseButton::Left))
            && let Some(drag) = self.drag.take()
        {
            self.drag_pos = None;
            if self.active_session().id == drag.source_session_id {
                self.finish_drag(drag, x, y);
            }
            return;
        }
        if self.drag.is_some() && matches!(ev.kind, MouseEventKind::Drag(MouseButton::Left)) {
            self.drag_pos = Some((x, y));
            return;
        }

        // Top bar clicks
        if matches!(ev.kind, MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Down(MouseButton::Middle))
            && y == 0
        {
            for (i, (tab_r, close_r)) in self.top_areas.tabs.clone().into_iter().enumerate() {
                if contains(close_r, x, y) || (ev.kind == MouseEventKind::Down(MouseButton::Middle) && contains(tab_r, x, y)) {
                    self.request_close_tab(i);
                    return;
                }
                if contains(tab_r, x, y) {
                    self.select_tab(i);
                    return;
                }
            }
            if ev.kind == MouseEventKind::Down(MouseButton::Left) {
                if contains(self.top_areas.prev_tab, x, y) {
                    if self.active > 0 {
                        self.select_tab(self.active - 1);
                    }
                } else if contains(self.top_areas.next_tab, x, y) {
                    if self.active + 1 < self.sessions.len() {
                        self.select_tab(self.active + 1);
                    }
                } else if contains(self.top_areas.new_tab, x, y) {
                    self.new_tab();
                } else if contains(self.top_areas.sftp_toggle, x, y) {
                    self.show_explorer = !self.show_explorer;
                    if !self.show_explorer {
                        self.focus = Focus::Terminal;
                    }
                }
            }
            return;
        }

        // Explorer panel
        if self.show_explorer && contains(self.active_session().explorer.areas.panel, x, y) {
            if matches!(ev.kind, MouseEventKind::Down(_)) {
                self.focus = Focus::Explorer;
            }
            let tx = self.tx.clone();
            let act = self.active_session_mut().explorer.handle_mouse(ev, &tx);
            self.handle_explorer_action(act);
            return;
        }

        // Terminal pane
        let term_area = self.active_session().term_area;
        if contains(term_area, x, y) {
            if matches!(ev.kind, MouseEventKind::Down(_)) {
                self.focus = Focus::Terminal;
            }
            if self.active_session().state == SessionState::Prompt {
                return;
            }
            let inner = term_area;
            if self.active_session().term.wants_mouse() {
                let col = x - inner.x;
                let row = y - inner.y;
                if let Some(bytes) = self.active_session().term.mouse_bytes(&ev, col, row)
                    && let Some(pty) = &mut self.active_session_mut().pty
                {
                    pty.write(&bytes);
                }
            } else {
                match ev.kind {
                    MouseEventKind::ScrollUp => self.active_session_mut().term.scroll_by(3),
                    MouseEventKind::ScrollDown => self.active_session_mut().term.scroll_by(-3),
                    _ => {}
                }
            }
        }
    }

    fn finish_drag(&mut self, drag: DragSession, x: u16, y: u16) {
        let dl_btn = self.active_session().explorer.areas.btn_download;
        let term_area = self.active_session().term_area;
        if self.show_explorer && contains(dl_btn, x, y) {
            self.handle_explorer_action(ExplorerAction::DownloadPrompt(drag.entries));
            return;
        }
        if contains(term_area, x, y) {
            let quoted: Vec<String> = drag.entries.iter().map(|e| shell_quote(&e.path)).collect();
            if !quoted.is_empty() {
                self.active_session_mut()
                    .feed_text(&format!("{} ", quoted.join(" ")));
                self.focus = Focus::Terminal;
            }
            return;
        }
        if self.show_explorer
            && let Some(target) = self.active_session().explorer.entry_at_row(y)
            && target.is_dir
        {
            let paths: Vec<String> = drag.entries.into_iter().map(|e| e.path).collect();
            let tx = self.tx.clone();
            self.active_session_mut()
                .explorer
                .move_paths(paths, target.path, &tx);
        }
    }

    fn draw(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
        let _ = execute!(io::stdout(), BeginSynchronizedUpdate);

        let _ = terminal.draw(|f| {
            let area = f.area();
            if area.height < 4 || area.width < 20 {
                return;
            }
            let rows = Layout::vertical([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .split(area);

            self.render_top_bar(f, rows[0]);

            let active = self.active;
            let focus = self.focus;
            let show_exp = self.show_explorer && rows[2].width >= 56;
            let drag_active = self.drag.is_some();
            let drag_pos = self.drag_pos;

            let (term_area, exp_area) = if show_exp {
                let exp_w = (rows[2].width * 38 / 100).clamp(30, 56);
                let cols = Layout::horizontal([Constraint::Min(24), Constraint::Length(exp_w)])
                    .split(rows[2]);
                (cols[0], Some(cols[1]))
            } else {
                (rows[2], None)
            };

            let split_x = exp_area.map(|ea| ea.x.saturating_sub(rows[1].x) as usize);
            let mut hline = String::with_capacity(rows[1].width as usize * 3);
            for col in 0..rows[1].width as usize {
                hline.push(if split_x == Some(col) { '┬' } else { '─' });
            }
            f.render_widget(Paragraph::new(Span::styled(hline, theme::dim())), rows[1]);

            let drag_hover_row = if drag_active {
                drag_pos.and_then(|(x, y)| exp_area.filter(|r| contains(*r, x, y)).map(|_| y))
            } else {
                None
            };

            let mut cursor = {
                let sess = &mut self.sessions[active];
                let c = sess.render_terminal(
                    f,
                    term_area,
                    focus == Focus::Terminal && self.dialog.is_none(),
                );
                if let Some(ea) = exp_area {
                    sess.explorer.render(
                        f,
                        ea,
                        focus == Focus::Explorer && self.dialog.is_none(),
                        drag_hover_row,
                        drag_active,
                    );
                }
                c
            };

            if self.drag.is_some() || self.toast.is_some() {
                let overlay_row = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
                self.render_overlay_bar(f, overlay_row);
            }

            if let Some(dlg) = &self.dialog {
                cursor = dlg.render(f, area);
            }
            if let Some((cx, cy)) = cursor {
                f.set_cursor_position((cx, cy));
            }
        });
        let _ = execute!(io::stdout(), EndSynchronizedUpdate);
    }

    fn render_top_bar(&mut self, f: &mut ratatui::Frame, area: Rect) {
        self.top_areas = TopBarAreas::default();
        let mut spans: Vec<Span> = Vec::new();
        let mut x = area.x;
        let y = area.y;

        // Left navigation arrows (< >) matching GUI header
        let can_prev = self.active > 0;
        let can_next = self.active + 1 < self.sessions.len();
        self.top_areas.prev_tab = Rect::new(x, y, 3, 1);
        spans.push(Span::styled(
            " ‹ ",
            if can_prev { theme::base() } else { theme::dim() },
        ));
        x += 3;
        self.top_areas.next_tab = Rect::new(x, y, 3, 1);
        spans.push(Span::styled(
            " › ",
            if can_next { theme::base() } else { theme::dim() },
        ));
        x += 3;
        spans.push(Span::styled("│", theme::dim()));
        x += 1;

        // Right-side controls: + and ◫
        let new_lbl = " + ";
        let sftp_lbl = " ◫ ";
        let right_w = (new_lbl.chars().count() + 1 + sftp_lbl.chars().count()) as u16;
        let tabs_max = area.right().saturating_sub(right_w + 1);

        for (i, s) in self.sessions.iter().enumerate() {
            let is_cur = i == self.active;
            let (dot, dot_style) = s.status_dot();
            let mut label = s.tab_title();
            if label.chars().count() > 20 {
                label = format!("{}…", label.chars().take(19).collect::<String>());
            }
            let text_style = if is_cur {
                theme::tab_active()
            } else if s.needs_attention {
                theme::tab_attention()
            } else {
                theme::tab_inactive()
            };
            let has_dot = !dot.is_empty();
            let prefix = format!(" {label} ");
            let close = "× ";
            let dot_w = if has_dot { 2 } else { 1 };
            let w = (dot_w + prefix.chars().count() + close.chars().count()) as u16;
            if x + w > tabs_max {
                break;
            }
            let tab_rect = Rect::new(x, y, w, 1);
            let close_x = x + dot_w as u16 + prefix.chars().count() as u16;
            let close_rect = Rect::new(close_x, y, 2, 1);
            self.top_areas.tabs.push((tab_rect, close_rect));

            spans.push(Span::styled(" ", text_style));
            if has_dot {
                let dot_s = if is_cur { text_style } else { dot_style };
                spans.push(Span::styled(dot, dot_s));
            }
            spans.push(Span::styled(prefix, text_style));
            spans.push(Span::styled(close, text_style));
            x += w;
            if x < tabs_max {
                spans.push(Span::styled("│", theme::dim()));
                x += 1;
            }
        }

        let right_start = area.right().saturating_sub(right_w);
        if right_start > x {
            spans.push(Span::raw(" ".repeat((right_start - x) as usize)));
            x = right_start;
        }
        if x + right_w <= area.right() {
            let nw = new_lbl.chars().count() as u16;
            self.top_areas.new_tab = Rect::new(x, y, nw, 1);
            spans.push(Span::styled(new_lbl, theme::base()));
            x += nw + 1;
            spans.push(Span::raw(" "));

            let sw = sftp_lbl.chars().count() as u16;
            self.top_areas.sftp_toggle = Rect::new(x, y, sw, 1);
            let sftp_style = if self.show_explorer {
                theme::selection(true)
            } else {
                theme::dim()
            };
            spans.push(Span::styled(sftp_lbl, sftp_style));
        }

        f.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn render_overlay_bar(&self, f: &mut ratatui::Frame, area: Rect) {
        if let Some(drag) = &self.drag {
            let msg = format!(
                " Trascinamento ({} elementi): rilascia su cartella (sposta), terminale (percorso) o [↓ Scarica] ",
                drag.entries.len()
            );
            f.render_widget(Paragraph::new(Span::styled(msg, theme::toast())), area);
            return;
        }
        if let Some(t) = &self.toast {
            f.render_widget(
                Paragraph::new(Span::styled(format!(" {} ", t.msg), theme::toast())),
                area,
            );
        }
    }
}

fn clear_screen(terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
    let _ = execute!(io::stdout(), Clear(ClearType::All));
    let _ = terminal.resize(Rect::ZERO);
}

fn default_download_dir() -> PathBuf {
    let dl = paths::home_dir().join("Downloads");
    if dl.is_dir() {
        dl
    } else {
        std::env::current_dir().unwrap_or_else(|_| paths::home_dir())
    }
}
