//! Unified SSH session tab: starts directly with an empty SFTP explorer
//! ("In attesa di connessione") and a VTE4 terminal displaying a fixed,
//! non-deletable `[<user>@<host> ~]$ ssh ` prompt ready for `user@hostname`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, glib};
use vte4::prelude::*;

use crate::config::{self, HostStore, ParsedSsh};
use crate::explorer::{self, Explorer};
use crate::remote::{Remote, SshTarget, shell_quote};
use crate::ui;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Prompt,
    Connecting,
    Connected,
    ShuttingDown,
}

struct PromptState {
    prefix: String,
    input: Vec<char>,
    cursor: usize,
    history_draft: Option<String>,
    history_idx: Option<usize>,
    suggestions: Vec<String>,
    suggest_navigated: bool,
}

impl PromptState {
    fn new() -> Self {
        PromptState {
            prefix: config::local_prompt_prefix(),
            input: Vec::new(),
            cursor: 0,
            history_draft: None,
            history_idx: None,
            suggestions: Vec::new(),
            suggest_navigated: false,
        }
    }

    fn text(&self) -> String {
        self.input.iter().collect()
    }

    fn cursor_visual_cols(&self) -> usize {
        str_cols(&self.prefix) + chars_cols(&self.input[..self.cursor])
    }

    fn full_visual_cols(&self) -> usize {
        str_cols(&self.prefix) + chars_cols(&self.input)
    }
}

pub struct Session {
    pub widget: gtk::Box,
    pub terminal: vte4::Terminal,
    store: Rc<HostStore>,
    parsed: RefCell<Option<ParsedSsh>>,
    target: RefCell<Option<SshTarget>>,
    explorer: Rc<Explorer>,
    page: RefCell<Option<adw::TabPage>>,
    on_title_changed: Box<dyn Fn()>,

    status_dot: gtk::Label,
    status_label: gtk::Label,
    reconnect_btn: gtk::Button,
    sync_label: gtk::Label,

    prompt: RefCell<PromptState>,
    suggest_popover: gtk::Popover,
    suggest_list: gtk::ListBox,

    state: Cell<State>,
    poll: RefCell<Option<glib::SourceId>>,
}

impl Session {
    pub fn new(store: Rc<HostStore>, on_title_changed: impl Fn() + 'static) -> Rc<Self> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let (terminal, paste_hook) = ui::new_terminal();
        let explorer = Explorer::new(id);

        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.add_css_class("session-view");

        let term_overlay = gtk::Overlay::new();
        term_overlay.set_child(Some(&terminal_scroller(&terminal)));
        let drop_overlay = build_drop_overlay();
        term_overlay.add_overlay(&drop_overlay);
        setup_terminal_drop(&term_overlay, &terminal, &explorer, &drop_overlay);

        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.set_start_child(Some(&explorer.widget));
        paned.set_end_child(Some(&term_overlay));
        paned.set_position(260);
        paned.set_resize_start_child(false);
        paned.set_shrink_start_child(false);
        paned.set_shrink_end_child(false);
        paned.set_vexpand(true);
        explorer.widget.set_size_request(180, -1);
        widget.append(&paned);

        // Minimal 22px status strip (no repeated host or path)
        let status = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        status.add_css_class("statusbar");

        let status_dot = gtk::Label::new(Some("●"));
        status_dot.add_css_class("status-dot");
        status_dot.set_visible(false);
        let status_label = gtk::Label::new(None);

        let reconnect_btn = gtk::Button::with_label("Riconnetti");
        reconnect_btn.add_css_class("flat");
        reconnect_btn.add_css_class("status-btn");
        reconnect_btn.set_visible(false);

        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);

        let sync_label = gtk::Label::new(None);
        sync_label.set_visible(false);
        sync_label.add_css_class("sync-badge");

        status.append(&status_dot);
        status.append(&status_label);
        status.append(&reconnect_btn);
        status.append(&spacer);
        status.append(&sync_label);
        widget.append(&status);

        // Inline host completion popover anchored to the terminal cursor
        let suggest_list = gtk::ListBox::new();
        suggest_list.set_selection_mode(gtk::SelectionMode::Single);
        suggest_list.add_css_class("suggest-list");
        suggest_list.set_can_focus(false);

        let suggest_popover = gtk::Popover::new();
        suggest_popover.add_css_class("suggest-popover");
        suggest_popover.set_has_arrow(false);
        suggest_popover.set_autohide(false);
        suggest_popover.set_can_focus(false);
        suggest_popover.set_position(gtk::PositionType::Bottom);
        suggest_popover.set_halign(gtk::Align::Start);
        suggest_popover.set_child(Some(&suggest_list));
        suggest_popover.set_parent(&terminal);

        let sp = suggest_popover.clone();
        terminal.connect_destroy(move |_| sp.unparent());

        let this = Rc::new(Session {
            widget,
            terminal,
            store,
            parsed: RefCell::new(None),
            target: RefCell::new(None),
            explorer,
            page: RefCell::default(),
            on_title_changed: Box::new(on_title_changed),
            status_dot,
            status_label,
            reconnect_btn,
            sync_label,
            prompt: RefCell::new(PromptState::new()),
            suggest_popover,
            suggest_list,
            state: Cell::new(State::Prompt),
            poll: RefCell::default(),
        });

        let weak = Rc::downgrade(&this);
        *paste_hook.borrow_mut() = Some(Box::new(move || {
            let Some(this) = weak.upgrade() else {
                return false;
            };
            if this.state.get() == State::Prompt {
                this.paste_into_prompt();
                true
            } else {
                false
            }
        }));

        this.connect_signals();
        this.setup_prompt_controller();
        this
    }

    pub fn tab_title(&self) -> String {
        if let Some(parsed) = self.parsed.borrow().as_ref() {
            let name = parsed.conn.name.trim();
            if !name.is_empty() {
                return name.to_string();
            }
        }
        if let Some(target) = self.target.borrow().as_ref() {
            return target.label();
        }
        "Nuova connessione".to_string()
    }

    pub fn window_title(&self) -> String {
        if self.target.borrow().is_some() {
            self.tab_title()
        } else {
            "SSH".to_string()
        }
    }

    pub fn set_page(&self, page: &adw::TabPage) {
        *self.page.borrow_mut() = Some(page.clone());
        self.refresh_titles();
    }

    fn refresh_titles(&self) {
        if let Some(page) = self.page.borrow().as_ref() {
            let title = self.tab_title();
            page.set_title(&title);
            let tooltip = self
                .target
                .borrow()
                .as_ref()
                .map(SshTarget::label)
                .filter(|lbl| lbl != &title)
                .unwrap_or_default();
            page.set_tooltip(&glib::markup_escape_text(&tooltip));
        }
        (self.on_title_changed)();
    }

    pub fn toggle_explorer(&self) {
        self.explorer
            .widget
            .set_visible(!self.explorer.widget.is_visible());
    }

    /// Starts the tab in waiting-for-connection mode with `[user@host ~]$ ssh `.
    pub fn start_prompt(&self) {
        self.enter_prompt();
    }

    /// Starts the tab directly connecting to `parsed` (used for CLI arguments).
    pub fn start_with_target(self: &Rc<Self>, parsed: ParsedSsh) {
        let prefix = self.prompt.borrow().prefix.clone();
        let line = format!("{prefix}{}\r\n", parsed.raw_command);
        self.terminal.feed(line.as_bytes());
        self.begin_connection(parsed);
    }

    pub fn focus(&self) {
        self.terminal.grab_focus();
    }

    pub fn shutdown(&self) {
        self.state.set(State::ShuttingDown);
        self.suggest_popover.popdown();
        if let Some(id) = self.poll.borrow_mut().take() {
            id.remove();
        }
        self.explorer.detach("Sessione chiusa");
        if let Some(t) = self.target.borrow().as_ref() {
            t.exit_master();
        }
    }

    // -----------------------------------------------------------------------
    // Fixed `[user@host ~]$ ssh ` Prompt Line Editor
    // -----------------------------------------------------------------------

    fn enter_prompt(&self) {
        self.state.set(State::Prompt);
        self.suggest_popover.popdown();
        let (col, _) = self.terminal.cursor_position();
        if col > 0 {
            self.terminal.feed(b"\r\n");
        }
        let prefix = {
            let mut p = self.prompt.borrow_mut();
            p.input.clear();
            p.cursor = 0;
            p.history_draft = None;
            p.history_idx = None;
            p.suggestions.clear();
            p.suggest_navigated = false;
            p.prefix.clone()
        };
        self.terminal.feed(prefix.as_bytes());
    }

    fn mutate_prompt<F>(&self, f: F)
    where
        F: FnOnce(&mut PromptState),
    {
        let (old_vis_cols, prefix, full_text, cursor_text, full_vis_cols, need_reposition) = {
            let mut p = self.prompt.borrow_mut();
            let old_vis_cols = p.cursor_visual_cols();
            f(&mut p);
            p.cursor = p.cursor.min(p.input.len());
            let full_text: String = p.input.iter().collect();
            let cursor_text: String = p.input[..p.cursor].iter().collect();
            let full_vis_cols = p.full_visual_cols();
            let need_reposition = p.cursor < p.input.len();
            (
                old_vis_cols,
                p.prefix.clone(),
                full_text,
                cursor_text,
                full_vis_cols,
                need_reposition,
            )
        };

        let cols = self.terminal.column_count().max(1) as usize;
        let (cur_col, _) = self.terminal.cursor_position();
        let up = rows_above_origin(old_vis_cols, cur_col.max(0) as usize, cols);

        let mut seq = String::with_capacity(prefix.len() + full_text.len() + 16);
        if up > 0 {
            seq.push_str(&format!("\x1b[{up}A"));
        }
        seq.push_str("\r\x1b[J");
        seq.push_str(&prefix);
        seq.push_str(&full_text);
        self.terminal.feed(seq.as_bytes());

        if need_reposition {
            let (end_col, _) = self.terminal.cursor_position();
            let end_up = rows_above_origin(full_vis_cols, end_col.max(0) as usize, cols);
            let mut pos_seq = String::with_capacity(prefix.len() + cursor_text.len() + 16);
            if end_up > 0 {
                pos_seq.push_str(&format!("\x1b[{end_up}A"));
            }
            pos_seq.push('\r');
            pos_seq.push_str(&prefix);
            pos_seq.push_str(&cursor_text);
            self.terminal.feed(pos_seq.as_bytes());
        }
    }

    fn setup_prompt_controller(self: &Rc<Self>) {
        let focus_ctl = gtk::EventControllerFocus::new();
        let sp = self.suggest_popover.downgrade();
        focus_ctl.connect_leave(move |_| {
            if let Some(sp) = sp.upgrade() {
                sp.popdown();
            }
        });
        self.terminal.add_controller(focus_ctl);

        let weak = Rc::downgrade(self);
        self.suggest_list.connect_row_activated(move |_, row| {
            let Some(this) = weak.upgrade() else { return };
            if this.state.get() != State::Prompt {
                return;
            }
            let idx = row.index();
            if idx >= 0 {
                this.accept_suggestion(idx as usize);
                this.submit_prompt();
            }
            this.terminal.grab_focus();
        });

        let key = gtk::EventControllerKey::new();
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        key.connect_key_pressed(move |_, k, _, mods| {
            let Some(this) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if this.state.get() != State::Prompt {
                return glib::Propagation::Proceed;
            }
            this.handle_prompt_key(k, mods)
        });
        self.terminal.add_controller(key);
    }

    fn handle_prompt_key(
        self: &Rc<Self>,
        key: gdk::Key,
        mods: gdk::ModifierType,
    ) -> glib::Propagation {
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = mods.contains(gdk::ModifierType::ALT_MASK);
        let super_m = mods.contains(gdk::ModifierType::SUPER_MASK);

        // Let app / window / clipboard shortcuts propagate untouched
        if ctrl && shift {
            return glib::Propagation::Proceed;
        }
        if ctrl
            && matches!(
                key,
                gdk::Key::Tab
                    | gdk::Key::ISO_Left_Tab
                    | gdk::Key::Page_Up
                    | gdk::Key::Page_Down
                    | gdk::Key::KP_Page_Up
                    | gdk::Key::KP_Page_Down
            )
        {
            return glib::Propagation::Proceed;
        }
        if super_m {
            return glib::Propagation::Proceed;
        }

        if ctrl {
            return match key.to_lower() {
                gdk::Key::a => {
                    self.mutate_prompt(|p| p.cursor = 0);
                    glib::Propagation::Stop
                }
                gdk::Key::e => {
                    self.mutate_prompt(|p| p.cursor = p.input.len());
                    glib::Propagation::Stop
                }
                gdk::Key::b | gdk::Key::Left | gdk::Key::KP_Left => {
                    self.mutate_prompt(|p| {
                        if key == gdk::Key::b {
                            p.cursor = p.cursor.saturating_sub(1);
                        } else {
                            p.cursor = prev_word_boundary(&p.input, p.cursor);
                        }
                    });
                    glib::Propagation::Stop
                }
                gdk::Key::f | gdk::Key::Right | gdk::Key::KP_Right => {
                    self.mutate_prompt(|p| {
                        if key == gdk::Key::f {
                            p.cursor = (p.cursor + 1).min(p.input.len());
                        } else {
                            p.cursor = next_word_boundary(&p.input, p.cursor);
                        }
                    });
                    glib::Propagation::Stop
                }
                gdk::Key::u => {
                    self.mutate_prompt(|p| {
                        p.input.drain(..p.cursor);
                        p.cursor = 0;
                        p.history_idx = None;
                    });
                    self.refresh_suggestions(false);
                    glib::Propagation::Stop
                }
                gdk::Key::k => {
                    self.mutate_prompt(|p| {
                        p.input.truncate(p.cursor);
                        p.history_idx = None;
                    });
                    self.refresh_suggestions(false);
                    glib::Propagation::Stop
                }
                gdk::Key::w | gdk::Key::BackSpace => {
                    self.mutate_prompt(|p| {
                        let start = prev_word_boundary(&p.input, p.cursor);
                        p.input.drain(start..p.cursor);
                        p.cursor = start;
                        p.history_idx = None;
                    });
                    self.refresh_suggestions(false);
                    glib::Propagation::Stop
                }
                gdk::Key::v => {
                    self.paste_into_prompt();
                    glib::Propagation::Stop
                }
                gdk::Key::c => {
                    self.suggest_popover.popdown();
                    self.mutate_prompt(|p| p.cursor = p.input.len());
                    self.terminal.feed(b"^C\r\n");
                    self.enter_prompt();
                    glib::Propagation::Stop
                }
                gdk::Key::l => {
                    self.suggest_popover.popdown();
                    let (prefix, full, cursor_txt, need_pos, full_cols) = {
                        let p = self.prompt.borrow();
                        (
                            p.prefix.clone(),
                            p.text(),
                            p.input[..p.cursor].iter().collect::<String>(),
                            p.cursor < p.input.len(),
                            p.full_visual_cols(),
                        )
                    };
                    let mut seq = format!("\x1b[2J\x1b[H{prefix}{full}");
                    self.terminal.feed(seq.as_bytes());
                    if need_pos {
                        let cols = self.terminal.column_count().max(1) as usize;
                        let (end_col, _) = self.terminal.cursor_position();
                        let up = rows_above_origin(full_cols, end_col.max(0) as usize, cols);
                        seq.clear();
                        if up > 0 {
                            seq.push_str(&format!("\x1b[{up}A"));
                        }
                        seq.push('\r');
                        seq.push_str(&prefix);
                        seq.push_str(&cursor_txt);
                        self.terminal.feed(seq.as_bytes());
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Stop,
            };
        }

        if alt && key == gdk::Key::BackSpace {
            self.mutate_prompt(|p| {
                let start = prev_word_boundary(&p.input, p.cursor);
                p.input.drain(start..p.cursor);
                p.cursor = start;
                p.history_idx = None;
            });
            self.refresh_suggestions(false);
            return glib::Propagation::Stop;
        }

        match key {
            gdk::Key::Return | gdk::Key::KP_Enter => {
                let navigated = self.prompt.borrow().suggest_navigated;
                if self.suggest_popover.is_visible()
                    && navigated
                    && let Some(row) = self.suggest_list.selected_row()
                    && row.index() >= 0
                {
                    self.accept_suggestion(row.index() as usize);
                }
                self.submit_prompt();
                glib::Propagation::Stop
            }
            gdk::Key::BackSpace => {
                // `ssh ` prefix is strictly non-deletable when cursor == 0
                let can_delete = self.prompt.borrow().cursor > 0;
                if can_delete {
                    self.mutate_prompt(|p| {
                        p.cursor -= 1;
                        p.input.remove(p.cursor);
                        p.history_idx = None;
                    });
                    self.refresh_suggestions(false);
                }
                glib::Propagation::Stop
            }
            gdk::Key::Delete | gdk::Key::KP_Delete => {
                let can_delete = {
                    let p = self.prompt.borrow();
                    p.cursor < p.input.len()
                };
                if can_delete {
                    self.mutate_prompt(|p| {
                        p.input.remove(p.cursor);
                        p.history_idx = None;
                    });
                    self.refresh_suggestions(false);
                }
                glib::Propagation::Stop
            }
            gdk::Key::Left | gdk::Key::KP_Left => {
                if self.prompt.borrow().cursor > 0 {
                    self.mutate_prompt(|p| p.cursor -= 1);
                }
                glib::Propagation::Stop
            }
            gdk::Key::Right | gdk::Key::KP_Right => {
                let can_move = {
                    let p = self.prompt.borrow();
                    p.cursor < p.input.len()
                };
                if can_move {
                    self.mutate_prompt(|p| p.cursor += 1);
                }
                glib::Propagation::Stop
            }
            gdk::Key::Home | gdk::Key::KP_Home => {
                self.mutate_prompt(|p| p.cursor = 0);
                glib::Propagation::Stop
            }
            gdk::Key::End | gdk::Key::KP_End => {
                self.mutate_prompt(|p| p.cursor = p.input.len());
                glib::Propagation::Stop
            }
            gdk::Key::Escape => {
                if self.suggest_popover.is_visible() {
                    self.suggest_popover.popdown();
                }
                glib::Propagation::Stop
            }
            gdk::Key::Tab | gdk::Key::ISO_Left_Tab => {
                if self.suggest_popover.is_visible() {
                    let idx = self
                        .suggest_list
                        .selected_row()
                        .map(|r| r.index().max(0) as usize)
                        .unwrap_or(0);
                    self.accept_suggestion(idx);
                } else {
                    self.refresh_suggestions(true);
                }
                glib::Propagation::Stop
            }
            gdk::Key::Down | gdk::Key::KP_Down => {
                if self.suggest_popover.is_visible() {
                    self.step_suggestion(1);
                } else {
                    self.step_history(false);
                }
                glib::Propagation::Stop
            }
            gdk::Key::Up | gdk::Key::KP_Up => {
                if self.suggest_popover.is_visible() {
                    self.step_suggestion(-1);
                } else {
                    self.step_history(true);
                }
                glib::Propagation::Stop
            }
            _ => {
                if let Some(ch) = key.to_unicode()
                    && ch >= ' '
                    && ch != '\x7f'
                    && (!alt || !ch.is_ascii_alphabetic())
                {
                    self.mutate_prompt(|p| {
                        p.input.insert(p.cursor, ch);
                        p.cursor += 1;
                        p.history_idx = None;
                    });
                    self.refresh_suggestions(false);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Stop
            }
        }
    }

    fn paste_into_prompt(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let clipboard = self.terminal.clipboard();
        glib::spawn_future_local(async move {
            let Ok(Some(raw)) = clipboard.read_text_future().await else {
                return;
            };
            let Some(this) = weak.upgrade() else { return };
            if this.state.get() != State::Prompt {
                return;
            }
            let cleaned = raw
                .replace(['\r', '\n'], " ")
                .trim()
                .to_string();
            if cleaned.is_empty() {
                return;
            }
            this.mutate_prompt(|p| {
                let to_insert = if p.input.is_empty() {
                    cleaned.strip_prefix("ssh ").unwrap_or(&cleaned).trim_start()
                } else {
                    cleaned.as_str()
                };
                for ch in to_insert.chars().filter(|&c| c >= ' ' && c != '\x7f') {
                    p.input.insert(p.cursor, ch);
                    p.cursor += 1;
                }
                p.history_idx = None;
            });
            this.refresh_suggestions(false);
        });
    }

    fn refresh_suggestions(&self, explicit_tab: bool) {
        let token_info = {
            let p = self.prompt.borrow();
            active_host_token(&p.input, p.cursor)
        };
        let Some((tok_start, tok_end, token)) = token_info else {
            self.suggest_popover.popdown();
            return;
        };
        if token.is_empty() && !explicit_tab {
            self.suggest_popover.popdown();
            return;
        }

        let hits = self.store.completions(&token);
        if hits.is_empty()
            || (!explicit_tab && hits.len() == 1 && hits[0].eq_ignore_ascii_case(&token))
        {
            self.suggest_popover.popdown();
            return;
        }
        if explicit_tab && hits.len() == 1 {
            let only = hits[0].clone();
            self.mutate_prompt(|p| {
                p.input.splice(tok_start..tok_end, only.chars());
                p.cursor = tok_start + only.chars().count();
            });
            self.suggest_popover.popdown();
            return;
        }

        while let Some(child) = self.suggest_list.first_child() {
            self.suggest_list.remove(&child);
        }
        for item in &hits {
            let lbl = gtk::Label::new(Some(item));
            lbl.set_xalign(0.0);
            let row = gtk::ListBoxRow::new();
            row.set_can_focus(false);
            row.set_child(Some(&lbl));
            self.suggest_list.append(&row);
        }
        if let Some(first) = self.suggest_list.row_at_index(0) {
            self.suggest_list.select_row(Some(&first));
        }

        let token_col_offset = {
            let mut p = self.prompt.borrow_mut();
            p.suggestions = hits;
            p.suggest_navigated = false;
            str_cols(&p.prefix) + chars_cols(&p.input[..tok_start])
        };

        // Anchor the popover right beneath the start of the active token
        let cols = self.terminal.column_count().max(1);
        let (_, cur_row) = self.terminal.cursor_position();
        let scroll_top = self
            .terminal
            .vadjustment()
            .map(|a| a.value())
            .unwrap_or(0.0) as i64;
        let vis_row = (cur_row - scroll_top).max(0) as i32;
        let vis_col = (token_col_offset as i64 % cols) as i32;
        let cw = (self.terminal.char_width() as i32).max(8);
        let ch = (self.terminal.char_height() as i32).max(16);

        self.suggest_popover
            .set_pointing_to(Some(&gdk::Rectangle::new(vis_col * cw, vis_row * ch, 1, ch)));
        self.suggest_popover.popup();
    }

    fn step_suggestion(&self, delta: i32) {
        let len = self.prompt.borrow().suggestions.len() as i32;
        if len <= 0 {
            return;
        }
        let cur = self
            .suggest_list
            .selected_row()
            .map(|r| r.index())
            .unwrap_or(0);
        let next = (cur + delta).rem_euclid(len);
        if let Some(row) = self.suggest_list.row_at_index(next) {
            self.suggest_list.select_row(Some(&row));
            self.prompt.borrow_mut().suggest_navigated = true;
        }
    }

    fn accept_suggestion(&self, idx: usize) {
        let chosen = {
            let p = self.prompt.borrow();
            let Some(s) = p.suggestions.get(idx).cloned() else {
                return;
            };
            let Some((start, end, _)) = active_host_token(&p.input, p.cursor) else {
                return;
            };
            (s, start, end)
        };
        let (text, start, end) = chosen;
        self.mutate_prompt(|p| {
            p.input.splice(start..end, text.chars());
            p.cursor = start + text.chars().count();
            p.suggest_navigated = false;
        });
        self.suggest_popover.popdown();
    }

    fn step_history(&self, older: bool) {
        let hist = self.store.history();
        if hist.is_empty() {
            return;
        }
        let replacement: Option<String> = {
            let mut p = self.prompt.borrow_mut();
            if older {
                match p.history_idx {
                    None => {
                        p.history_draft = Some(p.text());
                        p.history_idx = Some(0);
                        Some(hist[0].clone())
                    }
                    Some(i) if i + 1 < hist.len() => {
                        p.history_idx = Some(i + 1);
                        Some(hist[i + 1].clone())
                    }
                    Some(_) => None,
                }
            } else {
                match p.history_idx {
                    Some(0) => {
                        p.history_idx = None;
                        Some(p.history_draft.take().unwrap_or_default())
                    }
                    Some(i) => {
                        p.history_idx = Some(i - 1);
                        Some(hist[i - 1].clone())
                    }
                    None => None,
                }
            }
        };
        if let Some(text) = replacement {
            self.mutate_prompt(|p| {
                p.input = text.chars().collect();
                p.cursor = p.input.len();
            });
        }
    }

    fn submit_prompt(self: &Rc<Self>) {
        self.suggest_popover.popdown();
        let line = self.prompt.borrow().text();
        if line.trim().is_empty() {
            return;
        }
        // Move cursor to end of line before emitting CRLF so wrapped lines stay intact
        self.mutate_prompt(|p| p.cursor = p.input.len());
        self.terminal.feed(b"\r\n");

        match self.store.resolve(&line) {
            Some(parsed) => self.begin_connection(parsed),
            None => {
                self.terminal
                    .feed(b"ssh: formato non valido (usa utente@hostname)\r\n");
                self.enter_prompt();
            }
        }
    }

    fn begin_connection(self: &Rc<Self>, parsed: ParsedSsh) {
        self.store.record(&parsed);
        let target = SshTarget::from_parsed(&parsed);
        *self.parsed.borrow_mut() = Some(parsed);
        *self.target.borrow_mut() = Some(target);
        self.refresh_titles();
        self.connect();
    }

    // -----------------------------------------------------------------------
    // SSH Lifecycle & Multiplexed SFTP Attachment
    // -----------------------------------------------------------------------

    fn connect_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.terminal.connect_child_exited(move |_, status| {
            if let Some(this) = weak.upgrade() {
                this.on_ssh_exited(status);
            }
        });

        let weak = Rc::downgrade(self);
        self.reconnect_btn.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.reconnect();
            }
        });

        let weak = Rc::downgrade(self);
        self.explorer.connect_cd_terminal(move |dir| {
            let Some(this) = weak.upgrade() else { return };
            if this.state.get() != State::Connected {
                return;
            }
            let cmd = format!(" cd -- {}\n", shell_quote(dir));
            this.terminal.feed_child(cmd.as_bytes());
            this.terminal.grab_focus();
        });

        let weak = Rc::downgrade(self);
        self.explorer.edits.connect_changed(move |files| {
            let Some(this) = weak.upgrade() else { return };
            match files {
                [] => this.sync_label.set_visible(false),
                [single] => {
                    this.sync_label.set_text(&format!("⟳ {single}"));
                    this.sync_label.set_visible(true);
                }
                many => {
                    this.sync_label.set_text(&format!("⟳ {} in sync", many.len()));
                    this.sync_label.set_visible(true);
                }
            }
        });

        let weak = Rc::downgrade(self);
        #[allow(deprecated)]
        self.terminal.connect_current_directory_uri_changed(move |term| {
            let Some(this) = weak.upgrade() else { return };
            let Some(uri) = term.current_directory_uri() else { return };
            let Ok((path, _)) = glib::filename_from_uri(&uri) else { return };
            let Some(path) = path.to_str() else { return };
            this.explorer.terminal_cwd_changed(path);
        });
    }

    fn set_status(&self, dot_class: &str, text: &str) {
        for c in ["ok", "wait", "err"] {
            self.status_dot.remove_css_class(c);
        }
        self.status_dot.add_css_class(dot_class);
        self.status_dot.set_visible(true);
        self.status_label.set_text(text);
    }

    fn connect(self: &Rc<Self>) {
        let Some(target) = self.target.borrow().clone() else { return };
        self.suggest_popover.popdown();
        self.state.set(State::Connecting);
        self.reconnect_btn.set_visible(false);
        self.set_status("wait", "Connessione…");
        if let Some(page) = self.page.borrow().as_ref() {
            page.set_loading(true);
            page.set_needs_attention(false);
        }
        self.explorer.show_waiting();
        ui::spawn(&self.terminal, &target.terminal_argv(), None);

        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local(Duration::from_millis(250), move || {
            let Some(this) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if this.state.get() != State::Connecting {
                this.poll.borrow_mut().take();
                return glib::ControlFlow::Break;
            }
            if this
                .target
                .borrow()
                .as_ref()
                .is_some_and(SshTarget::master_ready)
            {
                this.poll.borrow_mut().take();
                this.on_connected();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        if let Some(old) = self.poll.borrow_mut().replace(id) {
            old.remove();
        }
    }

    fn on_connected(self: &Rc<Self>) {
        let Some(target) = self.target.borrow().clone() else { return };
        self.state.set(State::Connected);
        self.set_status("ok", "Connesso");
        if let Some(page) = self.page.borrow().as_ref() {
            page.set_loading(false);
        }
        let browse = Rc::new(Remote::start(&target));
        let transfer = Rc::new(Remote::start(&target));
        let initial = self
            .parsed
            .borrow()
            .as_ref()
            .map(|p| p.conn.initial_dir.trim().to_string())
            .filter(|s| !s.is_empty());
        self.explorer.attach(browse, transfer, initial);
    }

    fn on_ssh_exited(self: &Rc<Self>, status: i32) {
        if self.state.get() == State::ShuttingDown {
            return;
        }
        let was_connected = self.state.get() == State::Connected;
        if let Some(id) = self.poll.borrow_mut().take() {
            id.remove();
        }
        if let Some(t) = self.target.borrow().as_ref() {
            t.exit_master();
        }

        let code = exit_code(status);
        let status_txt = match code.filter(|&c| c != 0) {
            Some(c) => format!("Disconnesso ({c})"),
            None => "Disconnesso".to_string(),
        };
        self.explorer.detach(if was_connected {
            "Disconnesso"
        } else {
            "In attesa di connessione"
        });
        self.set_status("err", &status_txt);
        self.reconnect_btn
            .set_visible(self.parsed.borrow().is_some());
        if let Some(page) = self.page.borrow().as_ref() {
            page.set_loading(false);
            if !page.is_selected() {
                page.set_needs_attention(true);
            }
        }
        self.enter_prompt();
    }

    fn reconnect(self: &Rc<Self>) {
        if self.state.get() != State::Prompt {
            return;
        }
        let Some(parsed) = self.parsed.borrow().clone() else { return };
        self.suggest_popover.popdown();
        let cmd = parsed.raw_command.clone();
        self.mutate_prompt(|p| {
            p.input = cmd.chars().collect();
            p.cursor = p.input.len();
        });
        self.terminal.feed(b"\r\n");
        if let Some(t) = self.target.borrow_mut().as_mut() {
            t.renew_control_path();
        }
        self.connect();
        self.terminal.grab_focus();
    }
}

// ---------------------------------------------------------------------------
// Line-editing & layout helpers
// ---------------------------------------------------------------------------

fn char_cols(c: char) -> usize {
    let u = c as u32;
    if u < 0x20 || u == 0x7f {
        0
    } else if matches!(
        u,
        0x1100..=0x115F
            | 0x2E80..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE6F
            | 0xFF01..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F000..=0x3FFFD
    ) {
        2
    } else {
        1
    }
}

fn str_cols(s: &str) -> usize {
    s.chars().map(char_cols).sum()
}

fn chars_cols(cs: &[char]) -> usize {
    cs.iter().copied().map(char_cols).sum()
}

fn rows_above_origin(vis_cols: usize, cur_col: usize, width: usize) -> usize {
    if vis_cols == 0 || width == 0 {
        return 0;
    }
    if vis_cols.is_multiple_of(width) && cur_col + 1 >= width {
        // VTE pending-wrap state at right margin
        (vis_cols / width).saturating_sub(1)
    } else {
        vis_cols / width
    }
}

fn prev_word_boundary(input: &[char], cursor: usize) -> usize {
    let mut i = cursor.min(input.len());
    while i > 0 && input[i - 1].is_whitespace() {
        i -= 1;
    }
    while i > 0 && !input[i - 1].is_whitespace() {
        i -= 1;
    }
    i
}

fn next_word_boundary(input: &[char], cursor: usize) -> usize {
    let len = input.len();
    let mut i = cursor.min(len);
    while i < len && input[i].is_whitespace() {
        i += 1;
    }
    while i < len && !input[i].is_whitespace() {
        i += 1;
    }
    i
}

/// Extracts `(start_idx, end_idx, prefix_to_cursor)` if the cursor is on a
/// destination token (rather than a flag or an option value like `-p 2222`).
fn active_host_token(input: &[char], cursor: usize) -> Option<(usize, usize, String)> {
    let cursor = cursor.min(input.len());
    let mut start = cursor;
    while start > 0 && !input[start - 1].is_whitespace() {
        start -= 1;
    }
    let mut end = cursor;
    while end < input.len() && !input[end].is_whitespace() {
        end += 1;
    }
    if start < end && input[start] == '-' {
        return None;
    }
    // Check if immediately preceded by a 1-arg flag such as `-p` or `-i`
    if start > 0 {
        let mut pe = start;
        while pe > 0 && input[pe - 1].is_whitespace() {
            pe -= 1;
        }
        let mut ps = pe;
        while ps > 0 && !input[ps - 1].is_whitespace() {
            ps -= 1;
        }
        if ps < pe {
            let prev: String = input[ps..pe].iter().collect();
            if matches!(
                prev.as_str(),
                "-p" | "-i"
                    | "-l"
                    | "-b"
                    | "-c"
                    | "-D"
                    | "-E"
                    | "-e"
                    | "-F"
                    | "-I"
                    | "-J"
                    | "-L"
                    | "-m"
                    | "-O"
                    | "-o"
                    | "-Q"
                    | "-R"
                    | "-S"
                    | "-W"
                    | "-w"
            ) {
                return None;
            }
        }
    }
    let prefix: String = input[start..cursor].iter().collect();
    Some((start, end, prefix))
}

// ---------------------------------------------------------------------------
// Terminal drag-and-drop & scroller
// ---------------------------------------------------------------------------

fn build_drop_overlay() -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.add_css_class("drop-overlay");
    outer.set_visible(false);
    outer.set_can_target(false);
    outer.set_hexpand(true);
    outer.set_vexpand(true);

    let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    card.add_css_class("drop-card");
    card.set_halign(gtk::Align::Center);
    card.set_valign(gtk::Align::Center);
    card.set_vexpand(true);

    let icon = gtk::Image::from_icon_name("riso-upload-symbolic");
    icon.set_pixel_size(24);
    let title = gtk::Label::new(Some("Rilascia per caricare"));
    title.add_css_class("muted-label");

    card.append(&icon);
    card.append(&title);
    outer.append(&card);
    outer
}

fn setup_terminal_drop(
    overlay: &gtk::Overlay,
    terminal: &vte4::Terminal,
    explorer: &Rc<Explorer>,
    drop_overlay: &gtk::Box,
) {
    let target = gtk::DropTarget::new(
        glib::Type::INVALID,
        gdk::DragAction::COPY | gdk::DragAction::MOVE,
    );
    target.set_types(&[gdk::FileList::static_type(), String::static_type()]);
    target.set_preload(false);

    let exp = Rc::downgrade(explorer);
    target.connect_motion(glib::clone!(
        #[weak]
        drop_overlay,
        #[upgrade_or]
        gdk::DragAction::empty(),
        move |t, _, _| {
            let Some(exp) = exp.upgrade() else {
                return gdk::DragAction::empty();
            };
            if t.current_drop().and_then(|d| d.drag()).is_some() {
                return gdk::DragAction::COPY;
            }
            if !exp.is_attached() {
                return gdk::DragAction::empty();
            }
            drop_overlay.set_visible(true);
            gdk::DragAction::COPY
        }
    ));

    target.connect_leave(glib::clone!(
        #[weak]
        drop_overlay,
        move |_| drop_overlay.set_visible(false)
    ));

    let exp = Rc::downgrade(explorer);
    target.connect_drop(glib::clone!(
        #[weak]
        drop_overlay,
        #[weak]
        terminal,
        #[upgrade_or]
        false,
        move |_, value, _, _| {
            drop_overlay.set_visible(false);
            let Some(exp) = exp.upgrade() else { return false };
            if let Ok(files) = value.get::<gdk::FileList>() {
                if !exp.is_attached() {
                    return false;
                }
                let paths: Vec<std::path::PathBuf> =
                    files.files().iter().filter_map(|f| f.path()).collect();
                if paths.is_empty() {
                    return false;
                }
                exp.upload(paths, exp.cwd());
                return true;
            }
            if let Ok(text) = value.get::<String>()
                && let Some(rest) = text.strip_prefix(explorer::DRAG_PREFIX)
            {
                if !exp.is_attached() {
                    return false;
                }
                let quoted: Vec<String> = rest.lines().skip(1).map(shell_quote).collect();
                if !quoted.is_empty() {
                    let joined = format!("{} ", quoted.join(" "));
                    terminal.feed_child(joined.as_bytes());
                    terminal.grab_focus();
                    return true;
                }
            }
            false
        }
    ));
    overlay.add_controller(target);
}

fn terminal_scroller(terminal: &vte4::Terminal) -> gtk::ScrolledWindow {
    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_child(Some(terminal));
    scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
    scrolled.set_hexpand(true);
    scrolled.set_vexpand(true);
    scrolled.add_css_class("terminal-box");
    scrolled
}

fn exit_code(status: i32) -> Option<i32> {
    if status & 0x7f == 0 {
        Some((status >> 8) & 0xff)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_token_extraction_skips_flags() {
        let cs: Vec<char> = "leo@arch".chars().collect();
        assert_eq!(
            active_host_token(&cs, cs.len()),
            Some((0, 8, "leo@arch".to_string()))
        );

        let cs: Vec<char> = "-p 2222".chars().collect();
        assert_eq!(active_host_token(&cs, cs.len()), None);

        let cs: Vec<char> = "-p 2222 ho".chars().collect();
        assert_eq!(
            active_host_token(&cs, cs.len()),
            Some((8, 10, "ho".to_string()))
        );
    }

    #[test]
    fn word_boundaries_stay_inside_editable_input() {
        let cs: Vec<char> = "-p 2222 leo@host".chars().collect();
        assert_eq!(prev_word_boundary(&cs, 0), 0);
        assert_eq!(prev_word_boundary(&cs, 16), 8);
        assert_eq!(prev_word_boundary(&cs, 8), 3);
        assert_eq!(prev_word_boundary(&cs, 3), 0);
    }
}
