//! SSH session tab: SFTP explorer + VTE4 terminal + minimal 22px status strip.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, glib};
use vte4::prelude::*;

use crate::config::Connection;
use crate::explorer::{self, Explorer};
use crate::remote::{Remote, SshTarget, shell_quote};
use crate::ui;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Connecting,
    Connected,
    Closed,
}

pub struct Session {
    pub widget: gtk::Box,
    pub terminal: vte4::Terminal,
    conn: Connection,
    target: RefCell<SshTarget>,
    explorer: Rc<Explorer>,
    page: RefCell<Option<adw::TabPage>>,
    status_dot: gtk::Label,
    status_label: gtk::Label,
    reconnect_btn: gtk::Button,
    sync_label: gtk::Label,
    state: Cell<State>,
    poll: RefCell<Option<glib::SourceId>>,
}

impl Session {
    pub fn new(conn: Connection) -> Rc<Self> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let terminal = ui::new_terminal();
        let explorer = Explorer::new(id);
        let target = SshTarget::from_connection(&conn);

        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.add_css_class("vs-session");

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

        // Minimal 22px VS Code status strip (no repeated host or path)
        let status = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        status.add_css_class("vs-statusbar");

        let status_dot = gtk::Label::new(Some("●"));
        status_dot.add_css_class("vs-dot");
        let status_label = gtk::Label::new(None);

        let reconnect_btn = gtk::Button::with_label("Riconnetti");
        reconnect_btn.add_css_class("flat");
        reconnect_btn.add_css_class("vs-status-btn");
        reconnect_btn.set_visible(false);

        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);

        let sync_label = gtk::Label::new(None);
        sync_label.set_visible(false);
        sync_label.add_css_class("vs-sync-badge");

        status.append(&status_dot);
        status.append(&status_label);
        status.append(&reconnect_btn);
        status.append(&spacer);
        status.append(&sync_label);
        widget.append(&status);

        let this = Rc::new(Session {
            widget,
            terminal,
            conn,
            target: RefCell::new(target),
            explorer,
            page: RefCell::default(),
            status_dot,
            status_label,
            reconnect_btn,
            sync_label,
            state: Cell::new(State::Closed),
            poll: RefCell::default(),
        });
        this.connect_signals();
        this
    }

    pub fn title(&self) -> String {
        if self.conn.name.trim().is_empty() {
            self.target.borrow().label()
        } else {
            self.conn.name.trim().to_string()
        }
    }

    pub fn set_page(&self, page: &adw::TabPage) {
        let title = self.title();
        let target_lbl = self.target.borrow().label();
        page.set_title(&title);
        if title != target_lbl {
            page.set_tooltip(&glib::markup_escape_text(&target_lbl));
        }
        *self.page.borrow_mut() = Some(page.clone());
    }

    pub fn toggle_explorer(&self) {
        self.explorer
            .widget
            .set_visible(!self.explorer.widget.is_visible());
    }

    pub fn start(self: &Rc<Self>) {
        self.connect();
    }

    pub fn focus(&self) {
        self.terminal.grab_focus();
    }

    pub fn shutdown(&self) {
        if let Some(id) = self.poll.borrow_mut().take() {
            id.remove();
        }
        self.state.set(State::Closed);
        self.explorer.detach("Sessione chiusa");
        self.target.borrow().exit_master();
    }

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
        self.status_label.set_text(text);
    }

    fn connect(self: &Rc<Self>) {
        let target = self.target.borrow().clone();
        self.state.set(State::Connecting);
        self.reconnect_btn.set_visible(false);
        self.set_status("wait", "Connessione…");
        if let Some(page) = self.page.borrow().as_ref() {
            page.set_loading(true);
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
            if this.target.borrow().master_ready() {
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
        self.state.set(State::Connected);
        self.set_status("ok", "Connesso");
        if let Some(page) = self.page.borrow().as_ref() {
            page.set_loading(false);
        }
        let target = self.target.borrow().clone();
        let browse = Rc::new(Remote::start(&target));
        let transfer = Rc::new(Remote::start(&target));
        let initial = Some(self.conn.initial_dir.trim().to_string()).filter(|s| !s.is_empty());
        self.explorer.attach(browse, transfer, initial);
    }

    fn on_ssh_exited(self: &Rc<Self>, status: i32) {
        if let Some(id) = self.poll.borrow_mut().take() {
            id.remove();
        }
        self.state.set(State::Closed);
        self.target.borrow().exit_master();

        let code = exit_code(status);
        let status_txt = match code.filter(|c| *c != 0) {
            Some(c) => format!("Disconnesso ({c})"),
            None => "Disconnesso".to_string(),
        };
        self.explorer.detach("Disconnesso");
        self.set_status("err", &status_txt);
        self.reconnect_btn.set_visible(true);
        if let Some(page) = self.page.borrow().as_ref() {
            page.set_loading(false);
            if !page.is_selected() {
                page.set_needs_attention(true);
            }
        }
    }

    fn reconnect(self: &Rc<Self>) {
        if self.state.get() != State::Closed {
            return;
        }
        self.target.borrow_mut().renew_control_path();
        self.terminal
            .feed(b"\r\n\x1b[90m--- riconnessione ---\x1b[0m\r\n");
        if let Some(page) = self.page.borrow().as_ref() {
            page.set_needs_attention(false);
        }
        self.connect();
        self.terminal.grab_focus();
    }
}

fn build_drop_overlay() -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.add_css_class("vs-drop-overlay");
    outer.set_visible(false);
    outer.set_can_target(false);
    outer.set_hexpand(true);
    outer.set_vexpand(true);

    let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    card.add_css_class("vs-drop-card");
    card.set_halign(gtk::Align::Center);
    card.set_valign(gtk::Align::Center);
    card.set_vexpand(true);

    let icon = gtk::Image::from_icon_name("vs-upload-symbolic");
    icon.set_pixel_size(24);
    let title = gtk::Label::new(Some("Rilascia per caricare"));
    title.add_css_class("vs-muted");

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
    scrolled.add_css_class("vs-terminal-box");
    scrolled
}

fn exit_code(status: i32) -> Option<i32> {
    if status & 0x7f == 0 {
        Some((status >> 8) & 0xff)
    } else {
        None
    }
}
