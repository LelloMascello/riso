//! Integrated SFTP file explorer.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::editsync::EditSync;
use crate::remote::{Progress, Remote};
use crate::sftp::{self, Sftp};
use crate::ui;

pub const DRAG_PREFIX: &str = "riso-paths:";

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
    fn new(dir: &str, e: &sftp::DirEntry) -> Self {
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

    fn tooltip(&self) -> String {
        let mut s = perm_string(self.perms, self.is_dir, self.is_link);
        if !self.is_dir {
            s.push_str(&format!("  {}", ui::human_size(self.size)));
        }
        if self.mtime > 0
            && let Ok(dt) = glib::DateTime::from_unix_local(self.mtime as i64)
            && let Ok(t) = dt.format("%Y-%m-%d %H:%M")
        {
            s.push_str(&format!("  {t}"));
        }
        s
    }
}

fn perm_string(mode: u32, is_dir: bool, is_link: bool) -> String {
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

fn sort_entries(v: &mut [Entry]) {
    v.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
}

fn entry_of(obj: &glib::Object) -> Option<Entry> {
    let obj = obj.downcast_ref::<glib::BoxedAnyObject>()?;
    let e = obj.borrow::<Entry>();
    Some(e.clone())
}

fn row_entry(row: &glib::Object) -> Option<Entry> {
    let row = row.downcast_ref::<gtk::TreeListRow>()?;
    entry_of(&row.item()?)
}

struct Clipboard {
    paths: Vec<String>,
    cut: bool,
}

pub struct Explorer {
    pub widget: gtk::Box,
    session_id: u64,
    stack: gtk::Stack,
    placeholder_label: gtk::Label,
    path_entry: gtk::Entry,
    list_view: gtk::ListView,
    root_store: gio::ListStore,
    selection: gtk::MultiSelection,
    menu: gtk::PopoverMenu,
    actions: gio::SimpleActionGroup,
    progress_bar: gtk::ProgressBar,

    browse: RefCell<Option<Rc<Remote>>>,
    transfer: RefCell<Option<Rc<Remote>>>,
    cwd: RefCell<String>,
    back: RefCell<Vec<String>>,
    forward: RefCell<Vec<String>>,
    stores: RefCell<HashMap<String, glib::WeakRef<gio::ListStore>>>,
    show_hidden: Cell<bool>,
    follow_terminal: Cell<bool>,
    clipboard: RefCell<Option<Clipboard>>,
    active_transfers: Cell<u32>,
    current_progress: RefCell<Option<Progress>>,
    pub edits: Rc<EditSync>,
    on_cd_terminal: RefCell<Option<Box<dyn Fn(&str)>>>,
    on_cwd_changed: RefCell<Option<Box<dyn Fn(&str)>>>,
}

impl Explorer {
    pub fn new(session_id: u64) -> Rc<Self> {
        let this = Rc::new_cyclic(|weak: &Weak<Explorer>| Self::build(session_id, weak.clone()));
        this.setup();
        this
    }

    fn build(session_id: u64, weak: Weak<Explorer>) -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.add_css_class("explorer-sidebar");

        // -- Explorer Header Bar ---------------------------------------------
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        toolbar.add_css_class("explorer-toolbar");

        let header_icon = gtk::Image::from_icon_name("riso-folder-symbolic");
        header_icon.set_pixel_size(14);
        header_icon.add_css_class("explorer-header-icon");

        let path_entry = gtk::Entry::new();
        path_entry.set_hexpand(true);
        path_entry.set_placeholder_text(Some("/"));
        path_entry.add_css_class("path-entry");

        let mk_btn = |icon: &str, tip: &str, action: &str| {
            let b = gtk::Button::from_icon_name(icon);
            b.add_css_class("flat");
            b.add_css_class("icon-btn");
            b.set_tooltip_text(Some(tip));
            b.set_action_name(Some(action));
            b.set_valign(gtk::Align::Center);
            b
        };
        let up_btn = mk_btn("riso-up-symbolic", "Cartella superiore", "explorer.up");
        let new_file_btn = mk_btn("riso-new-file-symbolic", "Nuovo file", "explorer.new-file");
        let new_dir_btn = mk_btn(
            "riso-new-folder-symbolic",
            "Nuova cartella",
            "explorer.new-folder",
        );
        let refresh_btn = mk_btn("riso-refresh-symbolic", "Aggiorna", "explorer.refresh");

        let opts_model = gio::Menu::new();
        let opts_s1 = gio::Menu::new();
        opts_s1.append(Some("Carica…"), Some("explorer.upload"));
        opts_s1.append(Some("Copia percorso"), Some("explorer.copy-path"));
        opts_s1.append(Some("Apri nel terminale"), Some("explorer.cd-terminal"));
        let opts_s2 = gio::Menu::new();
        opts_s2.append(Some("Mostra file nascosti"), Some("explorer.show-hidden"));
        opts_s2.append(
            Some("Segui directory terminale"),
            Some("explorer.follow-terminal"),
        );
        opts_model.append_section(None, &opts_s1);
        opts_model.append_section(None, &opts_s2);
        let opts_popover = gtk::PopoverMenu::from_model(Some(&opts_model));
        opts_popover.set_has_arrow(false);

        let more_btn = gtk::MenuButton::new();
        more_btn.set_icon_name("riso-dots-symbolic");
        more_btn.set_popover(Some(&opts_popover));
        more_btn.add_css_class("flat");
        more_btn.add_css_class("icon-btn");
        more_btn.set_tooltip_text(Some("Opzioni esplora file"));
        more_btn.set_valign(gtk::Align::Center);

        toolbar.append(&header_icon);
        toolbar.append(&path_entry);
        toolbar.append(&up_btn);
        toolbar.append(&new_file_btn);
        toolbar.append(&new_dir_btn);
        toolbar.append(&refresh_btn);
        toolbar.append(&more_btn);
        widget.append(&toolbar);

        // -- Thin 2px transfer progress line ---------------------------------
        let progress_bar = gtk::ProgressBar::new();
        progress_bar.add_css_class("thin-progress");
        progress_bar.set_visible(false);
        widget.append(&progress_bar);

        // -- Tree ------------------------------------------------------------
        let root_store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let w = weak.clone();
        let tree = gtk::TreeListModel::new(root_store.clone(), false, false, move |item| {
            let entry = entry_of(item)?;
            if !entry.is_dir {
                return None;
            }
            let store = gio::ListStore::new::<glib::BoxedAnyObject>();
            let probe = store.downgrade();
            let w = w.clone();
            glib::idle_add_local_once(move || {
                if let (Some(store), Some(this)) = (probe.upgrade(), w.upgrade()) {
                    this.register_store(&entry.path, &store);
                    this.load_dir(&entry.path, &store);
                }
            });
            Some(store.upcast())
        });
        let selection = gtk::MultiSelection::new(Some(tree));

        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let expander = gtk::TreeExpander::new();
            expander.set_indent_for_icon(true);
            let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let icon = gtk::Image::new();
            icon.set_pixel_size(14);
            icon.add_css_class("explorer-tree-icon");
            let name = gtk::Label::new(None);
            name.set_xalign(0.0);
            name.set_hexpand(true);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let meta = gtk::Label::new(None);
            meta.set_xalign(1.0);
            meta.add_css_class("explorer-entry-meta");
            meta.set_visible(false);
            hbox.append(&icon);
            hbox.append(&name);
            hbox.append(&meta);
            expander.set_child(Some(&hbox));
            item.set_child(Some(&expander));
        });

        let sig_store: Rc<RefCell<HashMap<usize, (glib::WeakRef<gtk::TreeListRow>, glib::SignalHandlerId)>>> =
            Rc::default();
        let sigs_bind = sig_store.clone();
        factory.connect_bind(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let Some(row) = item.item().and_downcast::<gtk::TreeListRow>() else { return };
            let expander = item.child().and_downcast::<gtk::TreeExpander>().unwrap();
            expander.set_list_row(Some(&row));
            let Some(entry) = row.item().and_then(|o| entry_of(&o)) else { return };
            let hbox = expander.child().and_downcast::<gtk::Box>().unwrap();
            let icon = hbox.first_child().and_downcast::<gtk::Image>().unwrap();
            let name = icon.next_sibling().and_downcast::<gtk::Label>().unwrap();
            let meta = name.next_sibling().and_downcast::<gtk::Label>().unwrap();

            if entry.is_dir {
                let set_folder_icon = |ic: &gtk::Image, expanded: bool| {
                    ic.set_icon_name(Some(if expanded {
                        "riso-folder-open-symbolic"
                    } else {
                        "riso-folder-symbolic"
                    }));
                };
                set_folder_icon(&icon, row.is_expanded());
                let ic = icon.downgrade();
                let id = row.connect_expanded_notify(move |r| {
                    if let Some(ic) = ic.upgrade() {
                        ic.set_icon_name(Some(if r.is_expanded() {
                            "riso-folder-open-symbolic"
                        } else {
                            "riso-folder-symbolic"
                        }));
                    }
                });
                let key = item.as_ptr() as usize;
                sigs_bind.borrow_mut().insert(key, (row.downgrade(), id));
                if entry.is_link {
                    meta.set_text("→");
                    meta.set_visible(true);
                } else {
                    meta.set_text("");
                    meta.set_visible(false);
                }
            } else {
                icon.set_icon_name(Some("riso-file-symbolic"));
                meta.set_text(&ui::human_size(entry.size));
                meta.set_visible(true);
            }

            name.set_text(&entry.name);
            if entry.name.starts_with('.') {
                name.add_css_class("hidden-entry");
            } else {
                name.remove_css_class("hidden-entry");
            }
            expander.set_tooltip_text(Some(&entry.tooltip()));
        });

        let sigs_unbind = sig_store;
        factory.connect_unbind(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let key = item.as_ptr() as usize;
            if let Some((row_weak, id)) = sigs_unbind.borrow_mut().remove(&key)
                && let Some(row) = row_weak.upgrade()
            {
                row.disconnect(id);
            }
            if let Some(expander) = item.child().and_downcast::<gtk::TreeExpander>() {
                expander.set_list_row(None);
            }
        });

        let list_view = gtk::ListView::new(Some(selection.clone()), Some(factory));
        list_view.add_css_class("explorer-tree");
        list_view.set_enable_rubberband(true);
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_child(Some(&list_view));
        scrolled.set_vexpand(true);
        scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);

        // -- Minimal text placeholder ----------------------------------------
        let placeholder_label = gtk::Label::new(Some("In attesa di connessione"));
        placeholder_label.add_css_class("muted-label");
        placeholder_label.set_wrap(true);
        placeholder_label.set_justify(gtk::Justification::Center);
        placeholder_label.set_margin_start(16);
        placeholder_label.set_margin_end(16);
        placeholder_label.set_valign(gtk::Align::Center);
        placeholder_label.set_vexpand(true);

        let stack = gtk::Stack::new();
        stack.add_named(&placeholder_label, Some("placeholder"));
        stack.add_named(&scrolled, Some("browser"));
        stack.set_visible_child_name("placeholder");
        widget.append(&stack);

        // -- Context menu ----------------------------------------------------
        let menu_model = gio::Menu::new();
        let s1 = gio::Menu::new();
        s1.append(Some("Apri"), Some("explorer.open"));
        s1.append(Some("Apri con…"), Some("explorer.open-with"));
        s1.append(Some("Apri nel terminale"), Some("explorer.cd-terminal"));
        let s2 = gio::Menu::new();
        s2.append(Some("Nuovo file…"), Some("explorer.new-file"));
        s2.append(Some("Nuova cartella…"), Some("explorer.new-folder"));
        s2.append(Some("Carica…"), Some("explorer.upload"));
        s2.append(Some("Scarica…"), Some("explorer.download"));
        let s3 = gio::Menu::new();
        s3.append(Some("Copia"), Some("explorer.copy"));
        s3.append(Some("Taglia"), Some("explorer.cut"));
        s3.append(Some("Incolla"), Some("explorer.paste"));
        s3.append(Some("Copia percorso"), Some("explorer.copy-path"));
        s3.append(Some("Rinomina…"), Some("explorer.rename"));
        s3.append(Some("Elimina"), Some("explorer.delete"));
        let s4 = gio::Menu::new();
        s4.append(Some("Mostra file nascosti"), Some("explorer.show-hidden"));
        s4.append(
            Some("Segui directory terminale"),
            Some("explorer.follow-terminal"),
        );
        for s in [&s1, &s2, &s3, &s4] {
            menu_model.append_section(None, s);
        }
        let menu = gtk::PopoverMenu::from_model(Some(&menu_model));
        menu.set_parent(&list_view);
        menu.set_has_arrow(false);
        menu.set_halign(gtk::Align::Start);

        let edits = EditSync::new(session_id, &widget);

        Explorer {
            widget,
            session_id,
            stack,
            placeholder_label,
            path_entry,
            list_view,
            root_store,
            selection,
            menu,
            actions: gio::SimpleActionGroup::new(),
            progress_bar,
            browse: RefCell::default(),
            transfer: RefCell::default(),
            cwd: RefCell::new(String::new()),
            back: RefCell::default(),
            forward: RefCell::default(),
            stores: RefCell::default(),
            show_hidden: Cell::new(false),
            follow_terminal: Cell::new(true),
            clipboard: RefCell::default(),
            active_transfers: Cell::new(0),
            current_progress: RefCell::default(),
            edits,
            on_cd_terminal: RefCell::default(),
            on_cwd_changed: RefCell::default(),
        }
    }

    fn setup(self: &Rc<Self>) {
        self.setup_actions();
        self.setup_shortcuts();
        self.setup_events();
        self.setup_dnd();
        self.update_nav_buttons();
        self.set_browsing_enabled(false);
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    pub fn attach(
        self: &Rc<Self>,
        browse: Rc<Remote>,
        transfer: Rc<Remote>,
        initial_dir: Option<String>,
    ) {
        *self.browse.borrow_mut() = Some(browse.clone());
        *self.transfer.borrow_mut() = Some(transfer.clone());
        self.edits.set_remote(Some(transfer));
        self.set_placeholder("Apertura SFTP…");
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let start = initial_dir.unwrap_or_else(|| ".".to_string());
            let res = browse
                .run(move |s| s.realpath(&start).or_else(|_| s.realpath(".")))
                .await;
            let Some(this) = weak.upgrade() else { return };
            match res {
                Ok(path) => {
                    this.set_browsing_enabled(true);
                    this.stack.set_visible_child_name("browser");
                    this.set_cwd(&path, false);
                }
                Err(e) => this.set_placeholder(&format!("SFTP non disponibile\n{e}")),
            }
        });
    }

    pub fn detach(&self, message: &str) {
        if let Some(r) = self.browse.borrow_mut().take() {
            r.close();
        }
        if let Some(r) = self.transfer.borrow_mut().take() {
            r.close();
        }
        self.edits.set_remote(None);
        self.edits.clear();
        self.root_store.remove_all();
        self.stores.borrow_mut().clear();
        self.cwd.borrow_mut().clear();
        self.path_entry.set_text("");
        self.set_browsing_enabled(false);
        self.set_placeholder(message);
        if let Some(f) = self.on_cwd_changed.borrow().as_ref() {
            f("");
        }
    }

    pub fn show_waiting(&self) {
        self.set_placeholder("In attesa di autenticazione…");
    }

    pub fn is_attached(&self) -> bool {
        self.browse.borrow().is_some()
    }

    pub fn cwd(&self) -> String {
        self.cwd.borrow().clone()
    }

    pub fn connect_cd_terminal(&self, f: impl Fn(&str) + 'static) {
        *self.on_cd_terminal.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_cwd_changed(&self, f: impl Fn(&str) + 'static) {
        *self.on_cwd_changed.borrow_mut() = Some(Box::new(f));
    }

    pub fn prompt_upload(self: &Rc<Self>) {
        if self.is_attached() {
            self.choose_upload();
        }
    }

    pub fn terminal_cwd_changed(self: &Rc<Self>, path: &str) {
        if self.follow_terminal.get() && self.is_attached() && *self.cwd.borrow() != path {
            self.navigate(path.to_string());
        }
    }

    // -----------------------------------------------------------------------
    // State helpers
    // -----------------------------------------------------------------------

    fn set_placeholder(&self, text: &str) {
        self.placeholder_label.set_text(text);
        self.stack.set_visible_child_name("placeholder");
        self.path_entry.set_sensitive(false);
    }

    fn set_browsing_enabled(&self, enabled: bool) {
        self.path_entry.set_sensitive(enabled);
        for name in self.actions.list_actions() {
            if matches!(name.as_str(), "show-hidden" | "follow-terminal") {
                continue;
            }
            if let Some(a) = self
                .actions
                .lookup_action(&name)
                .and_downcast::<gio::SimpleAction>()
            {
                a.set_enabled(enabled);
            }
        }
        if enabled {
            self.update_nav_buttons();
            self.update_action_states();
        }
    }

    fn action(&self, name: &str) -> Option<gio::SimpleAction> {
        self.actions
            .lookup_action(name)
            .and_downcast::<gio::SimpleAction>()
    }

    fn update_nav_buttons(&self) {
        let attached = self.is_attached();
        let set = |name: &str, enabled: bool| {
            if let Some(a) = self.action(name) {
                a.set_enabled(attached && enabled);
            }
        };
        set("back", !self.back.borrow().is_empty());
        set("forward", !self.forward.borrow().is_empty());
        set("up", self.cwd.borrow().as_str() != "/");
    }

    fn update_action_states(&self) {
        if !self.is_attached() {
            return;
        }
        let sel = self.selected_entries();
        let n = sel.len();
        let single_file = n == 1 && !sel[0].is_dir;
        let set = |name: &str, enabled: bool| {
            if let Some(a) = self.action(name) {
                a.set_enabled(enabled);
            }
        };
        set("open", n == 1);
        set("open-with", single_file);
        set("download", n >= 1);
        set("copy", n >= 1);
        set("cut", n >= 1);
        set("rename", n == 1);
        set("delete", n >= 1);
        set("paste", self.clipboard.borrow().is_some());
    }

    fn selected_entries(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        let sel = self.selection.selection();
        let n = sel.size();
        for i in 0..n {
            let pos = sel.nth(i as u32);
            if let Some(entry) = self.selection.item(pos).and_then(|r| row_entry(&r)) {
                out.push(entry);
            }
        }
        out
    }

    fn target_dir(&self) -> String {
        let sel = self.selected_entries();
        match sel.as_slice() {
            [e] if e.is_dir => e.path.clone(),
            _ => self.cwd(),
        }
    }

    fn row_at(&self, x: f64, y: f64) -> Option<(u32, Entry)> {
        let widget = self.list_view.pick(x, y, gtk::PickFlags::DEFAULT)?;
        let expander = widget
            .ancestor(gtk::TreeExpander::static_type())
            .and_downcast::<gtk::TreeExpander>()?;
        let row = expander.list_row()?;
        let entry = entry_of(&row.item()?)?;
        Some((row.position(), entry))
    }

    fn window(&self) -> Option<gtk::Window> {
        self.widget.root().and_downcast::<gtk::Window>()
    }

    fn toast(&self, msg: &str) {
        ui::toast(&self.widget, msg);
    }

    // -----------------------------------------------------------------------
    // Loading
    // -----------------------------------------------------------------------

    fn register_store(&self, path: &str, store: &gio::ListStore) {
        let mut stores = self.stores.borrow_mut();
        stores.retain(|_, w| w.upgrade().is_some());
        stores.insert(path.to_string(), store.downgrade());
    }

    fn load_dir(self: &Rc<Self>, path: &str, store: &gio::ListStore) {
        let Some(remote) = self.browse.borrow().clone() else { return };
        let weak = Rc::downgrade(self);
        let store = store.downgrade();
        let path = path.to_string();
        glib::spawn_future_local(async move {
            let p = path.clone();
            let res = remote.run(move |s| s.readdir(&p)).await;
            let (Some(this), Some(store)) = (weak.upgrade(), store.upgrade()) else { return };
            match res {
                Ok(list) => {
                    let hidden = this.show_hidden.get();
                    let mut entries: Vec<Entry> = list
                        .iter()
                        .filter(|e| hidden || !e.name.starts_with('.'))
                        .map(|e| Entry::new(&path, e))
                        .collect();
                    sort_entries(&mut entries);
                    merge_into(&store, entries);
                }
                Err(e) => this.toast(&format!("{path}: {e}")),
            }
        });
    }

    fn reload_dir(self: &Rc<Self>, path: &str) {
        if *self.cwd.borrow() == path {
            let store = self.root_store.clone();
            self.load_dir(path, &store);
            return;
        }
        let store = self.stores.borrow().get(path).and_then(|w| w.upgrade());
        if let Some(store) = store {
            self.load_dir(path, &store);
        }
    }

    fn refresh_all(self: &Rc<Self>) {
        let cwd = self.cwd();
        let store = self.root_store.clone();
        self.load_dir(&cwd, &store);
        let stores: Vec<(String, gio::ListStore)> = self
            .stores
            .borrow()
            .iter()
            .filter_map(|(p, w)| w.upgrade().map(|s| (p.clone(), s)))
            .collect();
        for (p, s) in stores {
            self.load_dir(&p, &s);
        }
    }

    fn set_cwd(self: &Rc<Self>, path: &str, record_history: bool) {
        let old = self.cwd();
        if record_history && !old.is_empty() && old != path {
            self.back.borrow_mut().push(old);
            self.forward.borrow_mut().clear();
        }
        *self.cwd.borrow_mut() = path.to_string();
        self.path_entry.set_text(path);
        self.path_entry.set_tooltip_text(Some(path));
        self.root_store.remove_all();
        self.stores.borrow_mut().clear();
        let store = self.root_store.clone();
        self.load_dir(path, &store);
        self.update_nav_buttons();
        if let Some(f) = self.on_cwd_changed.borrow().as_ref() {
            f(path);
        }
    }

    pub fn navigate(self: &Rc<Self>, path: String) {
        self.navigate_with(path, true);
    }

    fn navigate_with(self: &Rc<Self>, path: String, record_history: bool) {
        let Some(remote) = self.browse.borrow().clone() else { return };
        let weak = Rc::downgrade(self);
        let base = self.cwd();
        glib::spawn_future_local(async move {
            let target = if path.starts_with('/') || base.is_empty() {
                path.clone()
            } else if path == "~" {
                ".".to_string()
            } else if let Some(rest) = path.strip_prefix("~/") {
                rest.to_string()
            } else {
                sftp::join(&base, &path)
            };
            let res = remote
                .run(move |s| {
                    let real = s.realpath(&target)?;
                    if !s.stat(&real)?.is_dir() {
                        return Err(io::Error::other("Non è una cartella"));
                    }
                    Ok(real)
                })
                .await;
            let Some(this) = weak.upgrade() else { return };
            match res {
                Ok(real) => this.set_cwd(&real, record_history),
                Err(e) => {
                    this.toast(&format!("{path}: {e}"));
                    this.path_entry.set_text(&this.cwd());
                }
            }
        });
    }

    // -----------------------------------------------------------------------
    // Transfers
    // -----------------------------------------------------------------------

    async fn run_transfer<T, F>(self: &Rc<Self>, job: F) -> io::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Sftp, &Progress) -> io::Result<T> + Send + 'static,
    {
        let Some(remote) = self.transfer.borrow().clone() else {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "Non connesso"));
        };
        let progress = Progress::default();
        self.begin_transfer(&progress);
        let p = progress.clone();
        let res = remote.run(move |s| job(s, &p)).await;
        self.end_transfer();
        res
    }

    fn begin_transfer(self: &Rc<Self>, progress: &Progress) {
        self.progress_bar.set_fraction(0.0);
        *self.current_progress.borrow_mut() = Some(progress.clone());
        self.progress_bar.set_visible(true);
        let n = self.active_transfers.get() + 1;
        self.active_transfers.set(n);
        if n == 1 {
            let weak = Rc::downgrade(self);
            glib::timeout_add_local(Duration::from_millis(100), move || {
                let Some(this) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                if this.active_transfers.get() == 0 {
                    this.progress_bar.set_visible(false);
                    return glib::ControlFlow::Break;
                }
                match this
                    .current_progress
                    .borrow()
                    .as_ref()
                    .and_then(|p| p.fraction())
                {
                    Some(f) => this.progress_bar.set_fraction(f),
                    None => this.progress_bar.pulse(),
                }
                glib::ControlFlow::Continue
            });
        }
    }

    fn end_transfer(&self) {
        self.active_transfers
            .set(self.active_transfers.get().saturating_sub(1));
    }

    pub fn upload(self: &Rc<Self>, files: Vec<PathBuf>, dest_dir: String) {
        if files.is_empty() || !self.is_attached() {
            return;
        }
        let this = self.clone();
        glib::spawn_future_local(async move {
            let names: Vec<String> = files
                .iter()
                .filter_map(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()))
                .collect();
            let (d, n) = (dest_dir.clone(), names.clone());
            let Some(browse) = this.browse.borrow().clone() else { return };
            let existing = browse
                .run(move |s| {
                    let mut ex = Vec::new();
                    for name in &n {
                        if s.exists(&sftp::join(&d, name))? {
                            ex.push(name.clone());
                        }
                    }
                    Ok(ex)
                })
                .await
                .unwrap_or_default();

            if !existing.is_empty() {
                let body = existing.join(", ");
                if !ui::confirm(
                    &this.widget,
                    "Sostituire i file esistenti?",
                    &body,
                    "Sostituisci",
                    true,
                )
                .await
                {
                    return;
                }
            }

            let total: u64 = files.iter().map(|f| local_size(f)).sum();
            let d = dest_dir.clone();
            let res = this
                .run_transfer(move |s, p| {
                    p.set_total(total);
                    for (f, name) in files.iter().zip(&names) {
                        s.upload_recursive(f, &sftp::join(&d, name), &mut |n| p.add(n))?;
                    }
                    Ok(())
                })
                .await;
            if let Err(e) = res {
                this.toast(&format!("Caricamento fallito: {e}"));
            }
            this.reload_dir(&dest_dir);
        });
    }

    fn download_selected(self: &Rc<Self>) {
        let entries = self.selected_entries();
        if entries.is_empty() {
            return;
        }
        let this = self.clone();
        glib::spawn_future_local(async move {
            let dialog = gtk::FileDialog::new();
            dialog.set_title("Scarica in");
            dialog.set_accept_label(Some("Scarica"));
            let window = this.window();
            let Ok(folder) = dialog.select_folder_future(window.as_ref()).await else { return };
            let Some(dest) = folder.path() else { return };
            let res = this
                .run_transfer(move |s, p| {
                    let mut total = 0;
                    for e in &entries {
                        total += s.total_size(&e.path).unwrap_or(0);
                    }
                    p.set_total(total);
                    for e in &entries {
                        s.download_recursive(&e.path, &dest.join(&e.name), &mut |n| p.add(n))?;
                    }
                    Ok(())
                })
                .await;
            if let Err(e) = res {
                this.toast(&format!("Download fallito: {e}"));
            }
        });
    }

    fn choose_upload(self: &Rc<Self>) {
        let dest = self.target_dir();
        let this = self.clone();
        glib::spawn_future_local(async move {
            let dialog = gtk::FileDialog::new();
            dialog.set_title("Carica");
            dialog.set_accept_label(Some("Carica"));
            let window = this.window();
            let Ok(files) = dialog.open_multiple_future(window.as_ref()).await else { return };
            let paths: Vec<PathBuf> = (0..files.n_items())
                .filter_map(|i| files.item(i).and_downcast::<gio::File>())
                .filter_map(|f| f.path())
                .collect();
            this.upload(paths, dest);
        });
    }

    // -----------------------------------------------------------------------
    // File operations
    // -----------------------------------------------------------------------

    fn open_entry(self: &Rc<Self>, entry: &Entry, ask_app: bool) {
        if entry.is_dir && !ask_app {
            self.navigate(entry.path.clone());
        } else {
            let edits = self.edits.clone();
            let path = entry.path.clone();
            glib::spawn_future_local(async move { edits.open(path, ask_app).await });
        }
    }

    fn set_clipboard(&self, cut: bool) {
        let paths: Vec<String> = self.selected_entries().into_iter().map(|e| e.path).collect();
        if paths.is_empty() {
            return;
        }
        *self.clipboard.borrow_mut() = Some(Clipboard { paths, cut });
        self.update_action_states();
    }

    fn paste(self: &Rc<Self>) {
        let Some(clip) = self.clipboard.borrow_mut().take() else { return };
        let dest = self.target_dir();
        if clip.cut {
            self.move_paths(clip.paths, dest);
            self.update_action_states();
            return;
        }
        let paths = clip.paths.clone();
        *self.clipboard.borrow_mut() = Some(clip);
        let this = self.clone();
        glib::spawn_future_local(async move {
            let d = dest.clone();
            let res = this
                .run_transfer(move |s, p| {
                    let mut total = 0;
                    for src in &paths {
                        total += s.total_size(src).unwrap_or(0);
                    }
                    p.set_total(total);
                    for src in &paths {
                        let target = unique_destination(s, &d, &sftp::basename(src))?;
                        s.copy_recursive(src, &target, &mut |n| p.add(n))?;
                    }
                    Ok(())
                })
                .await;
            if let Err(e) = res {
                this.toast(&format!("Copia fallita: {e}"));
            }
            this.reload_dir(&dest);
        });
    }

    fn move_paths(self: &Rc<Self>, paths: Vec<String>, dest: String) {
        let Some(remote) = self.browse.borrow().clone() else { return };
        let paths: Vec<String> = paths
            .into_iter()
            .filter(|p| sftp::parent(p) != dest && *p != dest && !dest.starts_with(&format!("{p}/")))
            .collect();
        if paths.is_empty() {
            return;
        }
        let this = self.clone();
        glib::spawn_future_local(async move {
            let (p, d) = (paths.clone(), dest.clone());
            let res = remote
                .run(move |s| {
                    for src in &p {
                        let target = sftp::join(&d, &sftp::basename(src));
                        if s.exists(&target)? {
                            return Err(io::Error::other(format!(
                                "{} esiste già",
                                sftp::basename(src)
                            )));
                        }
                        s.rename(src, &target)?;
                    }
                    Ok(())
                })
                .await;
            if let Err(e) = res {
                this.toast(&format!("Spostamento fallito: {e}"));
            }
            let mut dirs: Vec<String> = paths.iter().map(|p| sftp::parent(p)).collect();
            dirs.push(dest);
            dirs.sort();
            dirs.dedup();
            for d in dirs {
                this.reload_dir(&d);
            }
        });
    }

    fn rename_selected(self: &Rc<Self>) {
        let sel = self.selected_entries();
        let [entry] = sel.as_slice() else { return };
        let entry = entry.clone();
        let this = self.clone();
        glib::spawn_future_local(async move {
            let Some(new_name) =
                ui::prompt(&this.widget, "Rinomina", &entry.name, "Rinomina", !entry.is_dir).await
            else {
                return;
            };
            if new_name == entry.name {
                return;
            }
            if new_name.contains('/') {
                this.toast("Nome non valido");
                return;
            }
            let Some(remote) = this.browse.borrow().clone() else { return };
            let parent = sftp::parent(&entry.path);
            let target = sftp::join(&parent, &new_name);
            let src = entry.path.clone();
            let res = remote
                .run(move |s| {
                    if s.exists(&target)? {
                        return Err(io::Error::other("Nome già esistente"));
                    }
                    s.rename(&src, &target)
                })
                .await;
            if let Err(e) = res {
                this.toast(&format!("Rinomina fallita: {e}"));
            }
            this.reload_dir(&parent);
        });
    }

    fn delete_selected(self: &Rc<Self>) {
        let entries = self.selected_entries();
        if entries.is_empty() {
            return;
        }
        let this = self.clone();
        glib::spawn_future_local(async move {
            let heading = if entries.len() == 1 {
                format!("Eliminare «{}»?", entries[0].name)
            } else {
                format!("Eliminare {} elementi?", entries.len())
            };
            if !ui::confirm(&this.widget, &heading, "", "Elimina", true).await {
                return;
            }
            let paths: Vec<String> = entries.iter().map(|e| e.path.clone()).collect();
            let p2 = paths.clone();
            let res = this
                .run_transfer(move |s, _| {
                    for p in &p2 {
                        s.remove_recursive(p)?;
                    }
                    Ok(())
                })
                .await;
            if let Err(e) = res {
                this.toast(&format!("Eliminazione fallita: {e}"));
            }
            let mut dirs: Vec<String> = paths.iter().map(|p| sftp::parent(p)).collect();
            dirs.sort();
            dirs.dedup();
            for d in dirs {
                this.reload_dir(&d);
            }
        });
    }

    fn create_new(self: &Rc<Self>, folder: bool) {
        let dest = self.target_dir();
        let this = self.clone();
        glib::spawn_future_local(async move {
            let heading = if folder { "Nuova cartella" } else { "Nuovo file" };
            let Some(name) = ui::prompt(&this.widget, heading, "", "Crea", false).await else {
                return;
            };
            if name.contains('/') {
                this.toast("Nome non valido");
                return;
            }
            let Some(remote) = this.browse.borrow().clone() else { return };
            let path = sftp::join(&dest, &name);
            let res = remote
                .run(move |s| if folder { s.mkdir(&path) } else { s.create_file(&path) })
                .await;
            if let Err(e) = res {
                this.toast(&format!("{name}: {e}"));
            }
            this.reload_dir(&dest);
        });
    }

    // -----------------------------------------------------------------------
    // Wiring
    // -----------------------------------------------------------------------

    fn setup_actions(self: &Rc<Self>) {
        let add = |name: &str, f: fn(&Rc<Explorer>)| {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(this) = weak.upgrade() {
                    f(&this);
                }
            });
            self.actions.add_action(&action);
        };
        add("back", |this| {
            let prev = this.back.borrow_mut().pop();
            if let Some(prev) = prev {
                this.forward.borrow_mut().push(this.cwd());
                this.navigate_with(prev, false);
            }
        });
        add("forward", |this| {
            let next = this.forward.borrow_mut().pop();
            if let Some(next) = next {
                this.back.borrow_mut().push(this.cwd());
                this.navigate_with(next, false);
            }
        });
        add("up", |this| {
            let cwd = this.cwd();
            if cwd != "/" {
                this.navigate(sftp::parent(&cwd));
            }
        });
        add("refresh", |this| this.refresh_all());
        add("open", |this| {
            if let [e] = this.selected_entries().as_slice() {
                this.open_entry(e, false);
            }
        });
        add("open-with", |this| {
            if let [e] = this.selected_entries().as_slice() {
                this.open_entry(e, true);
            }
        });
        add("download", |this| this.download_selected());
        add("upload", |this| this.choose_upload());
        add("copy", |this| this.set_clipboard(false));
        add("cut", |this| this.set_clipboard(true));
        add("paste", |this| this.paste());
        add("rename", |this| this.rename_selected());
        add("delete", |this| this.delete_selected());
        add("new-folder", |this| this.create_new(true));
        add("new-file", |this| this.create_new(false));
        add("copy-path", |this| {
            let sel = this.selected_entries();
            let text = if sel.is_empty() {
                this.cwd()
            } else {
                sel.iter().map(|e| e.path.as_str()).collect::<Vec<_>>().join("\n")
            };
            this.widget.clipboard().set_text(&text);
        });
        add("cd-terminal", |this| {
            let sel = this.selected_entries();
            let dir = match sel.as_slice() {
                [e] if e.is_dir => e.path.clone(),
                [e] => sftp::parent(&e.path),
                _ => this.cwd(),
            };
            if let Some(f) = this.on_cd_terminal.borrow().as_ref() {
                f(&dir);
            }
        });
        add("focus-path", |this| {
            this.path_entry.grab_focus();
        });

        let hidden = gio::SimpleAction::new_stateful("show-hidden", None, &false.to_variant());
        let weak = Rc::downgrade(self);
        hidden.connect_activate(move |a, _| {
            let Some(this) = weak.upgrade() else { return };
            let v = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
            a.set_state(&v.to_variant());
            this.show_hidden.set(v);
            if this.is_attached() {
                let cwd = this.cwd();
                this.set_cwd(&cwd, false);
            }
        });
        self.actions.add_action(&hidden);

        let follow = gio::SimpleAction::new_stateful("follow-terminal", None, &true.to_variant());
        let weak = Rc::downgrade(self);
        follow.connect_activate(move |a, _| {
            let Some(this) = weak.upgrade() else { return };
            let v = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(true);
            a.set_state(&v.to_variant());
            this.follow_terminal.set(v);
        });
        self.actions.add_action(&follow);

        self.widget.insert_action_group("explorer", Some(&self.actions));
    }

    fn setup_shortcuts(&self) {
        let controller = gtk::ShortcutController::new();
        for (accel, action) in [
            ("Delete", "explorer.delete"),
            ("F2", "explorer.rename"),
            ("F5", "explorer.refresh"),
            ("<Control>r", "explorer.refresh"),
            ("<Control>c", "explorer.copy"),
            ("<Control>x", "explorer.cut"),
            ("<Control>v", "explorer.paste"),
            ("<Control>s", "explorer.download"),
            ("<Control>h", "explorer.show-hidden"),
            ("<Control>l", "explorer.focus-path"),
            ("<Alt>Left", "explorer.back"),
            ("<Alt>Right", "explorer.forward"),
            ("<Alt>Up", "explorer.up"),
            ("BackSpace", "explorer.up"),
        ] {
            controller.add_shortcut(gtk::Shortcut::new(
                gtk::ShortcutTrigger::parse_string(accel),
                Some(gtk::NamedAction::new(action)),
            ));
        }
        self.list_view.add_controller(controller);
    }

    fn setup_events(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.list_view.connect_activate(move |_, pos| {
            let Some(this) = weak.upgrade() else { return };
            if let Some(entry) = this.selection.item(pos).and_then(|r| row_entry(&r)) {
                this.open_entry(&entry, false);
            }
        });

        let weak = Rc::downgrade(self);
        self.selection.connect_selection_changed(move |_, _, _| {
            if let Some(this) = weak.upgrade() {
                this.update_action_states();
            }
        });

        let weak = Rc::downgrade(self);
        self.path_entry.connect_activate(move |e| {
            if let Some(this) = weak.upgrade() {
                let text = e.text().trim().to_string();
                if !text.is_empty() {
                    this.navigate(text);
                }
                this.list_view.grab_focus();
            }
        });

        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_SECONDARY);
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |gesture, _, x, y| {
            let Some(this) = weak.upgrade() else { return };
            if !this.is_attached() {
                return;
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
            match this.row_at(x, y) {
                Some((pos, _)) => {
                    if !this.selection.is_selected(pos) {
                        this.selection.select_item(pos, true);
                    }
                }
                None => {
                    this.selection.unselect_all();
                }
            }
            this.update_action_states();
            this.menu
                .set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            this.menu.popup();
        });
        self.list_view.add_controller(click);

        let menu = self.menu.clone();
        self.widget.connect_destroy(move |_| menu.unparent());
    }

    fn setup_dnd(self: &Rc<Self>) {
        let source = gtk::DragSource::new();
        source.set_actions(gdk::DragAction::MOVE | gdk::DragAction::COPY);
        let weak = Rc::downgrade(self);
        source.connect_prepare(move |_, x, y| {
            let this = weak.upgrade()?;
            let (pos, entry) = this.row_at(x, y)?;
            let paths: Vec<String> = if this.selection.is_selected(pos) {
                this.selected_entries().into_iter().map(|e| e.path).collect()
            } else {
                vec![entry.path]
            };
            let payload = format!("{DRAG_PREFIX}{}\n{}", this.session_id, paths.join("\n"));
            Some(gdk::ContentProvider::for_value(&payload.to_value()))
        });
        self.list_view.add_controller(source);

        let target = gtk::DropTarget::new(
            glib::Type::INVALID,
            gdk::DragAction::COPY | gdk::DragAction::MOVE,
        );
        target.set_types(&[gdk::FileList::static_type(), String::static_type()]);
        target.set_preload(false);
        target.connect_motion(glib::clone!(
            #[weak(rename_to = list_view)]
            self.list_view,
            #[upgrade_or]
            gdk::DragAction::empty(),
            move |t, _, _| {
                let internal = t.current_drop().and_then(|d| d.drag()).is_some();
                if internal {
                    gdk::DragAction::MOVE
                } else {
                    list_view.add_css_class("drop-active");
                    gdk::DragAction::COPY
                }
            }
        ));
        target.connect_leave(glib::clone!(
            #[weak(rename_to = list_view)]
            self.list_view,
            move |_| list_view.remove_css_class("drop-active")
        ));
        let weak = Rc::downgrade(self);
        target.connect_drop(move |_, value, x, y| {
            let Some(this) = weak.upgrade() else { return false };
            this.list_view.remove_css_class("drop-active");
            if !this.is_attached() {
                return false;
            }
            let dest = match this.row_at(x, y) {
                Some((_, e)) if e.is_dir => e.path,
                _ => this.cwd(),
            };
            if let Ok(files) = value.get::<gdk::FileList>() {
                let paths: Vec<PathBuf> = files.files().iter().filter_map(|f| f.path()).collect();
                if paths.is_empty() {
                    return false;
                }
                this.upload(paths, dest);
                return true;
            }
            if let Ok(text) = value.get::<String>()
                && let Some(rest) = text.strip_prefix(DRAG_PREFIX)
            {
                let mut lines = rest.lines();
                let same_session = lines.next() == Some(this.session_id.to_string().as_str());
                if !same_session {
                    return false;
                }
                let paths: Vec<String> = lines.map(str::to_string).collect();
                this.move_paths(paths, dest);
                return true;
            }
            false
        });
        self.list_view.add_controller(target);
    }
}

fn merge_into(store: &gio::ListStore, entries: Vec<Entry>) {
    let wanted: HashMap<&str, &Entry> = entries.iter().map(|e| (e.name.as_str(), e)).collect();
    let mut i = 0;
    while i < store.n_items() {
        let obj = store.item(i).and_downcast::<glib::BoxedAnyObject>().unwrap();
        let keep = {
            let cur = obj.borrow::<Entry>();
            wanted
                .get(cur.name.as_str())
                .is_some_and(|e| e.is_dir == cur.is_dir)
        };
        if keep {
            let name = obj.borrow::<Entry>().name.clone();
            *obj.borrow_mut::<Entry>() = (*wanted[name.as_str()]).clone();
            i += 1;
        } else {
            store.remove(i);
        }
    }
    for (j, e) in entries.iter().enumerate() {
        let j = j as u32;
        let matches = store
            .item(j)
            .and_then(|o| entry_of(&o))
            .is_some_and(|cur| cur.name == e.name);
        if !matches {
            store.insert(j, &glib::BoxedAnyObject::new(e.clone()));
        }
    }
}

fn unique_destination(s: &mut Sftp, dir: &str, name: &str) -> io::Result<String> {
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

fn local_size(path: &std::path::Path) -> u64 {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::read_dir(path)
            .map(|rd| rd.flatten().map(|e| local_size(&e.path())).sum())
            .unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}
