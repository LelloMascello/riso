//! Minimal VS Code-style main window: single unified top tab bar, zero repeated
//! labels, and SSH-only tab management.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gio, glib};

use crate::config::Connection;
use crate::connections::{self, ConnStore};
use crate::session::Session;

enum TabEntry {
    Launcher(gtk::Entry),
    Session(Rc<Session>),
}

pub struct AppWindow {
    pub window: adw::ApplicationWindow,
    toast_overlay: adw::ToastOverlay,
    tab_view: adw::TabView,
    sidebar_btn: gtk::Button,
    store: Rc<ConnStore>,
    tabs: RefCell<HashMap<adw::Bin, TabEntry>>,
    context_page: RefCell<Option<adw::TabPage>>,
    closing_window: Cell<bool>,
}

impl AppWindow {
    pub fn new(app: &adw::Application, initial_targets: &[String]) -> Rc<Self> {
        let store = ConnStore::new();

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("SSH")
            .default_width(1200)
            .default_height(760)
            .width_request(600)
            .height_request(400)
            .build();

        // -- Tab View --------------------------------------------------------
        let tab_view = adw::TabView::new();
        tab_view.remove_shortcuts(adw::TabViewShortcuts::ALL_SHORTCUTS);
        tab_view.add_shortcuts(
            adw::TabViewShortcuts::CONTROL_PAGE_UP | adw::TabViewShortcuts::CONTROL_PAGE_DOWN,
        );

        let tab_menu = gio::Menu::new();
        tab_menu.append(Some("Chiudi"), Some("win.close-context-tab"));
        tab_menu.append(Some("Chiudi le altre"), Some("win.close-other-tabs"));
        tab_view.set_menu_model(Some(&tab_menu));

        // -- Single Unified VS Code Top Bar ----------------------------------
        let sidebar_btn = gtk::Button::from_icon_name("vs-sidebar-symbolic");
        sidebar_btn.add_css_class("flat");
        sidebar_btn.add_css_class("vs-topbar-btn");
        sidebar_btn.set_action_name(Some("win.toggle-sidebar"));
        sidebar_btn.set_tooltip_text(Some("Esplora file (Ctrl+Shift+B)"));
        sidebar_btn.set_valign(gtk::Align::Center);
        sidebar_btn.set_margin_start(6);

        let add_btn = gtk::Button::from_icon_name("vs-plus-symbolic");
        add_btn.add_css_class("flat");
        add_btn.add_css_class("vs-topbar-btn");
        add_btn.set_action_name(Some("win.new-tab"));
        add_btn.set_tooltip_text(Some("Nuova connessione (Ctrl+Shift+T)"));
        add_btn.set_valign(gtk::Align::Center);

        let tab_bar = adw::TabBar::new();
        tab_bar.set_view(Some(&tab_view));
        tab_bar.set_autohide(false);
        tab_bar.set_expand_tabs(false);
        tab_bar.add_css_class("inline");
        tab_bar.add_css_class("vs-tabbar");
        tab_bar.set_hexpand(true);
        tab_bar.set_end_action_widget(Some(&add_btn));

        let start_controls = gtk::WindowControls::new(gtk::PackType::Start);
        let end_controls = gtk::WindowControls::new(gtk::PackType::End);

        let top_row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        top_row.add_css_class("vs-topbar");
        top_row.append(&start_controls);
        top_row.append(&sidebar_btn);
        top_row.append(&tab_bar);
        top_row.append(&end_controls);

        let top_handle = gtk::WindowHandle::new();
        top_handle.set_child(Some(&top_row));

        let main_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        main_box.append(&top_handle);
        main_box.append(&tab_view);
        tab_view.set_vexpand(true);

        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&main_box));
        window.set_content(Some(&toast_overlay));

        let this = Rc::new_cyclic(|_weak: &Weak<Self>| AppWindow {
            window,
            toast_overlay,
            tab_view,
            sidebar_btn,
            store,
            tabs: RefCell::default(),
            context_page: RefCell::default(),
            closing_window: Cell::new(false),
        });

        this.setup_actions();
        this.setup_signals();

        let keep_alive = RefCell::new(Some(this.clone()));
        this.window.connect_destroy(move |_| {
            keep_alive.borrow_mut().take();
        });

        if initial_targets.is_empty() {
            this.open_launcher_tab();
        } else {
            let mut opened = 0;
            for arg in initial_targets {
                if let Some(conn) = this.store.resolve_quick(arg) {
                    this.open_ssh_tab(conn);
                    opened += 1;
                }
            }
            if opened == 0 {
                this.open_launcher_tab();
            }
        }

        this
    }

    pub fn open_launcher_tab(self: &Rc<Self>) -> adw::TabPage {
        let slot = adw::Bin::new();
        let page = self.tab_view.append(&slot);
        page.set_title("Nuova");

        let w = Rc::downgrade(self);
        let s = slot.downgrade();
        let (launcher, entry) = connections::build_launcher(self.store.clone(), move |conn| {
            if let (Some(this), Some(slot)) = (w.upgrade(), s.upgrade()) {
                this.upgrade_slot_to_ssh(&slot, conn);
            }
        });
        slot.set_child(Some(&launcher));
        self.tabs
            .borrow_mut()
            .insert(slot, TabEntry::Launcher(entry.clone()));
        self.tab_view.set_selected_page(&page);
        entry.grab_focus();
        self.sync_ui_state();
        page
    }

    pub fn open_ssh_tab(self: &Rc<Self>, conn: Connection) -> adw::TabPage {
        let slot = adw::Bin::new();
        let session = Session::new(conn);
        slot.set_child(Some(&session.widget));
        let page = self.tab_view.append(&slot);
        session.set_page(&page);
        self.tabs
            .borrow_mut()
            .insert(slot, TabEntry::Session(session.clone()));
        self.tab_view.set_selected_page(&page);
        session.start();
        session.focus();
        self.sync_ui_state();
        page
    }

    fn upgrade_slot_to_ssh(self: &Rc<Self>, slot: &adw::Bin, conn: Connection) {
        let page = self.tab_view.page(slot);
        let session = Session::new(conn);
        slot.set_child(Some(&session.widget));
        session.set_page(&page);
        self.tabs
            .borrow_mut()
            .insert(slot.clone(), TabEntry::Session(session.clone()));
        session.start();
        session.focus();
        self.sync_ui_state();
    }

    fn sync_ui_state(&self) {
        let Some(page) = self.tab_view.selected_page() else {
            self.sidebar_btn.set_visible(false);
            self.window.set_title(Some("SSH"));
            return;
        };
        let Some(slot) = page.child().downcast::<adw::Bin>().ok() else { return };
        match self.tabs.borrow().get(&slot) {
            Some(TabEntry::Session(session)) => {
                self.sidebar_btn.set_visible(true);
                self.window.set_title(Some(&session.title()));
            }
            _ => {
                self.sidebar_btn.set_visible(false);
                self.window.set_title(Some("SSH"));
            }
        }
    }

    fn setup_actions(self: &Rc<Self>) {
        let toast_action = gio::SimpleAction::new("toast", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(self);
        toast_action.connect_activate(move |_, param| {
            let Some(this) = weak.upgrade() else { return };
            let Some(msg) = param.and_then(|v| v.get::<String>()) else { return };
            let toast = adw::Toast::new(&msg);
            toast.set_use_markup(false);
            toast.set_timeout(3);
            this.toast_overlay.add_toast(toast);
        });
        self.window.add_action(&toast_action);

        let add = |name: &str, f: fn(&Rc<AppWindow>)| {
            let a = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            a.connect_activate(move |_, _| {
                if let Some(this) = weak.upgrade() {
                    f(&this);
                }
            });
            self.window.add_action(&a);
        };

        add("new-tab", |this| {
            this.open_launcher_tab();
        });
        add("toggle-sidebar", |this| {
            let Some(page) = this.tab_view.selected_page() else { return };
            let Some(slot) = page.child().downcast::<adw::Bin>().ok() else { return };
            if let Some(TabEntry::Session(session)) = this.tabs.borrow().get(&slot) {
                session.toggle_explorer();
            }
        });
        add("close-tab", |this| {
            if let Some(page) = this.tab_view.selected_page() {
                this.tab_view.close_page(&page);
            }
        });
        add("close-context-tab", |this| {
            let page = this
                .context_page
                .borrow_mut()
                .take()
                .or_else(|| this.tab_view.selected_page());
            if let Some(page) = page {
                this.tab_view.close_page(&page);
            }
        });
        add("close-other-tabs", |this| {
            let page = this
                .context_page
                .borrow_mut()
                .take()
                .or_else(|| this.tab_view.selected_page());
            if let Some(page) = page {
                this.tab_view.close_other_pages(&page);
            }
        });
        add("next-tab", |this| {
            if !this.tab_view.select_next_page() && this.tab_view.n_pages() > 1 {
                let first = this.tab_view.nth_page(0);
                this.tab_view.set_selected_page(&first);
            }
        });
        add("prev-tab", |this| {
            let n = this.tab_view.n_pages();
            if !this.tab_view.select_previous_page() && n > 1 {
                let last = this.tab_view.nth_page(n - 1);
                this.tab_view.set_selected_page(&last);
            }
        });
    }

    fn setup_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.tab_view.connect_setup_menu(move |_, page| {
            if let Some(this) = weak.upgrade() {
                *this.context_page.borrow_mut() = page.cloned();
            }
        });

        let weak = Rc::downgrade(self);
        self.tab_view.connect_selected_page_notify(move |view| {
            let Some(this) = weak.upgrade() else { return };
            this.sync_ui_state();
            let Some(page) = view.selected_page() else { return };
            page.set_needs_attention(false);
            let Some(slot) = page.child().downcast::<adw::Bin>().ok() else { return };
            match this.tabs.borrow().get(&slot) {
                Some(TabEntry::Launcher(entry)) => {
                    entry.grab_focus();
                }
                Some(TabEntry::Session(session)) => {
                    session.focus();
                }
                None => {}
            }
        });

        let weak = Rc::downgrade(self);
        self.tab_view.connect_close_page(move |view, page| {
            if let Some(this) = weak.upgrade()
                && let Some(slot) = page.child().downcast::<adw::Bin>().ok()
                && let Some(TabEntry::Session(session)) = this.tabs.borrow_mut().remove(&slot)
            {
                session.shutdown();
            }
            view.close_page_finish(page, true);
            glib::Propagation::Stop
        });

        let weak = Rc::downgrade(self);
        self.tab_view.connect_page_detached(move |view, _, _| {
            let Some(this) = weak.upgrade() else { return };
            if view.n_pages() == 0 && !this.closing_window.get() {
                this.window.close();
            }
        });

        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(this) = weak.upgrade() {
                this.closing_window.set(true);
                for (_, entry) in this.tabs.borrow_mut().drain() {
                    if let TabEntry::Session(session) = entry {
                        session.shutdown();
                    }
                }
            }
            glib::Propagation::Proceed
        });
    }
}
