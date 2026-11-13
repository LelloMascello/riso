//! Main application window: workspace with ultra-slim titlebar,
//! rectangular bordered tab bar, and right SFTP explorer dock.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gio, glib};

use crate::config::{HostStore, ParsedSsh};
use crate::gui::session::Session;

pub struct AppWindow {
    pub window: adw::ApplicationWindow,
    toast_overlay: adw::ToastOverlay,
    tab_view: adw::TabView,
    tab_scroll: gtk::ScrolledWindow,
    tab_strip: gtk::Box,
    tab_context_popover: gtk::PopoverMenu,
    dock_box: gtk::Box,
    explorer_stack: gtk::Stack,
    sidebar_btn: gtk::Button,
    title_ctx: gtk::Label,
    store: Rc<HostStore>,
    tabs: RefCell<HashMap<adw::Bin, Rc<Session>>>,
    tab_widgets: RefCell<HashMap<adw::TabPage, gtk::Box>>,
    context_page: RefCell<Option<adw::TabPage>>,
    closing_window: Cell<bool>,
}

impl AppWindow {
    pub fn new(app: &adw::Application, initial_targets: &[String]) -> Rc<Self> {
        let store = HostStore::new();

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("riso")
            .default_width(1280)
            .default_height(780)
            .width_request(640)
            .height_request(420)
            .build();

        // -- Tab View --------------------------------------------------------
        let tab_view = adw::TabView::new();
        tab_view.remove_shortcuts(adw::TabViewShortcuts::ALL_SHORTCUTS);
        tab_view.add_shortcuts(
            adw::TabViewShortcuts::CONTROL_PAGE_UP | adw::TabViewShortcuts::CONTROL_PAGE_DOWN,
        );
        tab_view.set_vexpand(true);
        tab_view.set_hexpand(true);

        let tab_menu = gio::Menu::new();
        tab_menu.append(Some("Chiudi"), Some("win.close-context-tab"));
        tab_menu.append(Some("Chiudi le altre"), Some("win.close-other-tabs"));
        tab_view.set_menu_model(Some(&tab_menu));

        // -- Row 1: Ultra-Slim Titlebar --------------------------------------
        let start_controls = gtk::WindowControls::new(gtk::PackType::Start);
        let end_controls = gtk::WindowControls::new(gtk::PackType::End);

        let app_menu = gio::Menu::new();
        let m_tabs = gio::Menu::new();
        m_tabs.append(Some("Nuova connessione"), Some("win.new-tab"));
        m_tabs.append(Some("Chiudi scheda"), Some("win.close-tab"));
        let m_nav = gio::Menu::new();
        m_nav.append(Some("Scheda successiva"), Some("win.next-tab"));
        m_nav.append(Some("Scheda precedente"), Some("win.prev-tab"));
        m_nav.append(Some("Pannello file SFTP"), Some("win.toggle-sidebar"));
        let m_quit = gio::Menu::new();
        m_quit.append(Some("Esci"), Some("app.quit"));
        app_menu.append_section(None, &m_tabs);
        app_menu.append_section(None, &m_nav);
        app_menu.append_section(None, &m_quit);

        let app_popover = gtk::PopoverMenu::from_model(Some(&app_menu));
        app_popover.set_has_arrow(false);

        let menu_btn = gtk::MenuButton::new();
        menu_btn.set_icon_name("riso-menu-symbolic");
        menu_btn.set_popover(Some(&app_popover));
        menu_btn.add_css_class("flat");
        menu_btn.add_css_class("titlebar-btn");
        menu_btn.set_tooltip_text(Some("Menu"));
        menu_btn.set_valign(gtk::Align::Center);

        let title_app = gtk::Label::new(Some("riso"));
        title_app.add_css_class("riso-title-app");

        let title_sep = gtk::Label::new(Some("/"));
        title_sep.add_css_class("riso-title-sep");

        let title_ctx = gtk::Label::new(Some("ssh"));
        title_ctx.add_css_class("riso-title-ctx");
        title_ctx.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let title_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        title_spacer.set_hexpand(true);

        let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        title_row.add_css_class("riso-titlebar");
        title_row.append(&start_controls);
        title_row.append(&menu_btn);
        title_row.append(&title_app);
        title_row.append(&title_sep);
        title_row.append(&title_ctx);
        title_row.append(&title_spacer);
        title_row.append(&end_controls);

        let title_handle = gtk::WindowHandle::new();
        title_handle.set_child(Some(&title_row));

        // -- Row 2 Left: Tab Bar + Session Workspace -------------------------
        let prev_btn = gtk::Button::from_icon_name("riso-chevron-left-symbolic");
        prev_btn.add_css_class("flat");
        prev_btn.add_css_class("topbar-btn");
        prev_btn.set_action_name(Some("win.prev-tab"));
        prev_btn.set_tooltip_text(Some("Scheda precedente (Ctrl+Shift+Tab)"));
        prev_btn.set_valign(gtk::Align::Center);

        let next_btn = gtk::Button::from_icon_name("riso-chevron-right-symbolic");
        next_btn.add_css_class("flat");
        next_btn.add_css_class("topbar-btn");
        next_btn.set_action_name(Some("win.next-tab"));
        next_btn.set_tooltip_text(Some("Scheda successiva (Ctrl+Tab)"));
        next_btn.set_valign(gtk::Align::Center);

        let nav_box = gtk::Box::new(gtk::Orientation::Horizontal, 1);
        nav_box.add_css_class("riso-tab-nav");
        nav_box.append(&prev_btn);
        nav_box.append(&next_btn);

        let tab_strip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tab_strip.add_css_class("riso-tabstrip");

        let tab_filler = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tab_filler.add_css_class("riso-tab-filler");
        tab_filler.set_hexpand(true);

        let tab_strip_wrap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tab_strip_wrap.set_hexpand(true);
        tab_strip_wrap.append(&tab_strip);
        tab_strip_wrap.append(&tab_filler);

        let tab_scroll = gtk::ScrolledWindow::new();
        tab_scroll.add_css_class("riso-tab-scroll");
        tab_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        tab_scroll.set_propagate_natural_height(true);
        tab_scroll.set_has_frame(false);
        tab_scroll.set_hexpand(true);
        tab_scroll.set_vexpand(false);
        tab_scroll.set_child(Some(&tab_strip_wrap));

        let ts_scroll = tab_scroll.clone();
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        wheel.connect_scroll(move |_, _, dy| {
            let adj = ts_scroll.hadjustment();
            let max = (adj.upper() - adj.page_size()).max(0.0);
            if max > 0.0 {
                adj.set_value((adj.value() + dy * 36.0).clamp(0.0, max));
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        tab_scroll.add_controller(wheel);

        let tab_context_popover = gtk::PopoverMenu::from_model(Some(&tab_menu));
        tab_context_popover.set_has_arrow(false);
        tab_context_popover.set_halign(gtk::Align::Start);
        tab_context_popover.set_parent(&tab_strip);
        let tcp = tab_context_popover.clone();
        tab_strip.connect_destroy(move |_| tcp.unparent());

        let add_btn = gtk::Button::from_icon_name("riso-plus-symbolic");
        add_btn.add_css_class("flat");
        add_btn.add_css_class("topbar-btn");
        add_btn.set_action_name(Some("win.new-tab"));
        add_btn.set_tooltip_text(Some("Nuova connessione (Ctrl+Shift+T)"));
        add_btn.set_valign(gtk::Align::Center);

        let sidebar_btn = gtk::Button::from_icon_name("riso-sidebar-symbolic");
        sidebar_btn.add_css_class("flat");
        sidebar_btn.add_css_class("topbar-btn");
        sidebar_btn.add_css_class("active");
        sidebar_btn.set_action_name(Some("win.toggle-sidebar"));
        sidebar_btn.set_tooltip_text(Some("Esplora file (Ctrl+Shift+B)"));
        sidebar_btn.set_valign(gtk::Align::Center);

        let tab_actions = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        tab_actions.add_css_class("riso-tab-actions");
        tab_actions.append(&add_btn);
        tab_actions.append(&sidebar_btn);

        let tab_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tab_row.add_css_class("riso-tabbar-row");
        tab_row.append(&nav_box);
        tab_row.append(&tab_scroll);
        tab_row.append(&tab_actions);

        let main_pane = gtk::Box::new(gtk::Orientation::Vertical, 0);
        main_pane.set_hexpand(true);
        main_pane.set_vexpand(true);
        main_pane.append(&tab_row);
        main_pane.append(&tab_view);

        // -- Row 2 Right: SFTP Explorer Dock ---------------------------------
        let explorer_stack = gtk::Stack::new();
        explorer_stack.set_transition_type(gtk::StackTransitionType::None);
        explorer_stack.set_vexpand(true);
        explorer_stack.set_hexpand(true);

        let dock_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        dock_box.add_css_class("riso-dock");
        dock_box.set_size_request(200, -1);
        dock_box.append(&explorer_stack);

        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.add_css_class("riso-paned");
        paned.set_start_child(Some(&main_pane));
        paned.set_end_child(Some(&dock_box));
        paned.set_resize_start_child(true);
        paned.set_resize_end_child(false);
        paned.set_shrink_start_child(false);
        paned.set_shrink_end_child(false);
        paned.set_vexpand(true);
        paned.set_hexpand(true);

        // Ensure ~290px right dock width on first real allocation across both
        // tiling WMs (Hyprland/Sway) and floating desktops.
        let initial_sized = Cell::new(false);
        paned.add_tick_callback(move |p, _| {
            let w = p.width();
            if w > 0 && !initial_sized.replace(true) {
                let dock_w = 290.min(w / 3).max(200);
                p.set_position((w - dock_w).max(220));
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });

        let main_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        main_box.append(&title_handle);
        main_box.append(&paned);

        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&main_box));
        window.set_content(Some(&toast_overlay));

        let this = Rc::new_cyclic(|_weak: &Weak<Self>| AppWindow {
            window,
            toast_overlay,
            tab_view,
            tab_scroll,
            tab_strip,
            tab_context_popover,
            dock_box,
            explorer_stack,
            sidebar_btn,
            title_ctx,
            store,
            tabs: RefCell::default(),
            tab_widgets: RefCell::default(),
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
            this.open_prompt_tab();
        } else {
            let mut opened = 0;
            for arg in initial_targets {
                if let Some(parsed) = this.store.resolve(arg) {
                    this.open_ssh_tab(parsed);
                    opened += 1;
                }
            }
            if opened == 0 {
                this.open_prompt_tab();
            }
        }

        this
    }

    pub fn open_prompt_tab(self: &Rc<Self>) -> adw::TabPage {
        let (slot, page, session) = self.create_session_tab();
        self.tabs.borrow_mut().insert(slot, session.clone());
        self.explorer_stack
            .set_visible_child(&session.explorer.widget);
        self.tab_view.set_selected_page(&page);
        session.start_prompt();
        session.focus();
        self.sync_chrome();
        page
    }

    pub fn open_ssh_tab(self: &Rc<Self>, parsed: ParsedSsh) -> adw::TabPage {
        let (slot, page, session) = self.create_session_tab();
        self.tabs.borrow_mut().insert(slot, session.clone());
        self.explorer_stack
            .set_visible_child(&session.explorer.widget);
        self.tab_view.set_selected_page(&page);
        session.start_with_target(parsed);
        session.focus();
        self.sync_chrome();
        page
    }

    fn create_session_tab(self: &Rc<Self>) -> (adw::Bin, adw::TabPage, Rc<Session>) {
        let slot = adw::Bin::new();
        let weak = Rc::downgrade(self);
        let session = Session::new(self.store.clone(), move || {
            if let Some(this) = weak.upgrade() {
                this.sync_chrome();
            }
        });
        self.explorer_stack.add_child(&session.explorer.widget);
        slot.set_child(Some(&session.widget));
        let page = self.tab_view.append(&slot);
        session.set_page(&page);
        (slot, page, session)
    }

    fn selected_session(&self) -> Option<Rc<Session>> {
        let page = self.tab_view.selected_page()?;
        let slot = page.child().downcast::<adw::Bin>().ok()?;
        self.tabs.borrow().get(&slot).cloned()
    }

    fn sync_chrome(&self) {
        if self.closing_window.get() {
            return;
        }
        let Some(session) = self.selected_session() else {
            self.window.set_title(Some("riso"));
            self.title_ctx.set_text("ssh");
            return;
        };

        self.window.set_title(Some(&session.window_title()));
        self.title_ctx.set_text(&session.titlebar_context());
    }

    fn toggle_dock(&self) {
        let next = !self.dock_box.is_visible();
        self.dock_box.set_visible(next);
        if next {
            self.sidebar_btn.add_css_class("active");
        } else {
            self.sidebar_btn.remove_css_class("active");
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
            this.open_prompt_tab();
        });
        add("toggle-sidebar", |this| {
            this.toggle_dock();
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

    fn build_tab_item(self: &Rc<Self>, page: &adw::TabPage) -> gtk::Box {
        let tab_box = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        tab_box.add_css_class("riso-tab");

        let dot = gtk::Label::new(Some("•"));
        dot.add_css_class("riso-tab-dot");
        dot.set_valign(gtk::Align::Center);

        let sync_dot = |p: &adw::TabPage, d: &gtk::Label| {
            if p.is_loading() {
                d.add_css_class("loading");
                d.set_visible(true);
            } else if p.needs_attention() {
                d.remove_css_class("loading");
                d.set_visible(true);
            } else {
                d.remove_css_class("loading");
                d.set_visible(false);
            }
        };
        sync_dot(page, &dot);

        let label = gtk::Label::new(Some(&page.title()));
        label.add_css_class("riso-tab-label");
        label.set_valign(gtk::Align::Center);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_max_width_chars(28);

        let sync_tip = |p: &adw::TabPage, tb: &gtk::Box| match p.tooltip().filter(|t| !t.is_empty()) {
            Some(tip) => tb.set_tooltip_markup(Some(tip.as_str())),
            None => tb.set_tooltip_markup(None),
        };
        sync_tip(page, &tab_box);

        let close_btn = gtk::Button::from_icon_name("riso-close-symbolic");
        close_btn.add_css_class("flat");
        close_btn.add_css_class("riso-tab-close");
        close_btn.set_valign(gtk::Align::Center);
        close_btn.set_can_focus(false);
        close_btn.set_tooltip_text(Some("Chiudi scheda (Ctrl+Shift+W)"));

        let view_weak = self.tab_view.downgrade();
        let page_weak = page.downgrade();
        close_btn.connect_clicked(move |_| {
            if let (Some(view), Some(p)) = (view_weak.upgrade(), page_weak.upgrade()) {
                view.close_page(&p);
            }
        });

        let lbl_weak = label.downgrade();
        page.connect_title_notify(move |p| {
            if let Some(lbl) = lbl_weak.upgrade() {
                lbl.set_text(&p.title());
            }
        });

        let tb_weak = tab_box.downgrade();
        page.connect_tooltip_notify(move |p| {
            if let Some(tb) = tb_weak.upgrade() {
                sync_tip(p, &tb);
            }
        });

        let dot_weak = dot.downgrade();
        page.connect_loading_notify(move |p| {
            if let Some(d) = dot_weak.upgrade() {
                sync_dot(p, &d);
            }
        });

        let dot_weak = dot.downgrade();
        page.connect_needs_attention_notify(move |p| {
            if let Some(d) = dot_weak.upgrade() {
                sync_dot(p, &d);
            }
        });

        let click = gtk::GestureClick::new();
        click.set_button(0);
        let weak = Rc::downgrade(self);
        let page_weak = page.downgrade();
        let close_weak = close_btn.downgrade();
        click.connect_pressed(move |gesture, _, x, y| {
            let Some(this) = weak.upgrade() else { return };
            let Some(p) = page_weak.upgrade() else { return };
            if let Some(cb) = close_weak.upgrade()
                && let Some(tb) = gesture.widget()
                && tb
                    .pick(x, y, gtk::PickFlags::DEFAULT)
                    .is_some_and(|w| w == cb || w.is_ancestor(&cb))
            {
                return;
            }
            match gesture.current_button() {
                gtk::gdk::BUTTON_PRIMARY => {
                    this.tab_view.set_selected_page(&p);
                }
                gtk::gdk::BUTTON_MIDDLE => {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    this.tab_view.close_page(&p);
                }
                gtk::gdk::BUTTON_SECONDARY => {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    *this.context_page.borrow_mut() = Some(p);
                    if let Some(tb) = gesture.widget()
                        && let Some(pt) = tb.compute_point(
                            &this.tab_strip,
                            &gtk::graphene::Point::new(x as f32, y as f32),
                        )
                    {
                        this.tab_context_popover.set_pointing_to(Some(
                            &gtk::gdk::Rectangle::new(pt.x() as i32, pt.y() as i32, 1, 1),
                        ));
                        this.tab_context_popover.popup();
                    }
                }
                _ => {}
            }
        });
        tab_box.add_controller(click);

        tab_box.append(&dot);
        tab_box.append(&label);
        tab_box.append(&close_btn);
        tab_box
    }

    fn sync_tab_strip_order(&self) {
        let n = self.tab_view.n_pages();
        let widgets = self.tab_widgets.borrow();
        let mut prev: Option<gtk::Box> = None;
        for i in 0..n {
            let page = self.tab_view.nth_page(i);
            if let Some(w) = widgets.get(&page) {
                self.tab_strip.reorder_child_after(w, prev.as_ref());
                prev = Some(w.clone());
            }
        }
    }

    fn sync_tab_selection(&self) {
        let selected = self.tab_view.selected_page();
        let mut active_widget: Option<gtk::Box> = None;
        for (page, w) in self.tab_widgets.borrow().iter() {
            if selected.as_ref() == Some(page) {
                w.add_css_class("active");
                active_widget = Some(w.clone());
            } else {
                w.remove_css_class("active");
            }
        }
        if let Some(w) = active_widget {
            let scroll = self.tab_scroll.clone();
            let strip = self.tab_strip.clone();
            glib::idle_add_local_once(move || {
                if let Some(bounds) = w.compute_bounds(&strip) {
                    let adj = scroll.hadjustment();
                    let left = bounds.x() as f64;
                    let right = (bounds.x() + bounds.width()) as f64;
                    let cur = adj.value();
                    let page_sz = adj.page_size();
                    if page_sz > 0.0 {
                        if left < cur {
                            adj.set_value(left);
                        } else if right > cur + page_sz {
                            adj.set_value(right - page_sz);
                        }
                    }
                }
            });
        }
    }

    fn setup_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.tab_view.connect_setup_menu(move |_, page| {
            if let Some(this) = weak.upgrade() {
                *this.context_page.borrow_mut() = page.cloned();
            }
        });

        let weak = Rc::downgrade(self);
        self.tab_view.connect_page_attached(move |_, page, _| {
            let Some(this) = weak.upgrade() else { return };
            let item = this.build_tab_item(page);
            this.tab_strip.append(&item);
            this.tab_widgets
                .borrow_mut()
                .insert(page.clone(), item);
            this.sync_tab_strip_order();
            this.sync_tab_selection();
        });

        let weak = Rc::downgrade(self);
        self.tab_view.connect_page_reordered(move |_, _, _| {
            if let Some(this) = weak.upgrade() {
                this.sync_tab_strip_order();
            }
        });

        let weak = Rc::downgrade(self);
        self.tab_view.connect_selected_page_notify(move |view| {
            let Some(this) = weak.upgrade() else { return };
            this.sync_tab_selection();
            let Some(page) = view.selected_page() else {
                this.sync_chrome();
                return;
            };
            page.set_needs_attention(false);
            if let Ok(slot) = page.child().downcast::<adw::Bin>() {
                let session = this.tabs.borrow().get(&slot).cloned();
                if let Some(session) = session {
                    this.explorer_stack
                        .set_visible_child(&session.explorer.widget);
                    session.focus();
                }
            }
            this.sync_chrome();
        });

        let weak = Rc::downgrade(self);
        self.tab_view.connect_close_page(move |view, page| {
            if let Some(this) = weak.upgrade() {
                let removed = page
                    .child()
                    .downcast::<adw::Bin>()
                    .ok()
                    .and_then(|slot| this.tabs.borrow_mut().remove(&slot));
                if let Some(session) = removed {
                    this.explorer_stack.remove(&session.explorer.widget);
                    session.shutdown();
                }
            }
            view.close_page_finish(page, true);
            glib::Propagation::Stop
        });

        let weak = Rc::downgrade(self);
        self.tab_view.connect_page_detached(move |view, page, _| {
            let Some(this) = weak.upgrade() else { return };
            if let Some(item) = this.tab_widgets.borrow_mut().remove(page) {
                this.tab_strip.remove(&item);
            }
            if view.n_pages() == 0 && !this.closing_window.get() {
                this.window.close();
            } else {
                this.sync_tab_selection();
                this.sync_chrome();
            }
        });

        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(this) = weak.upgrade() {
                this.closing_window.set(true);
                let sessions: Vec<Rc<Session>> =
                    this.tabs.borrow_mut().drain().map(|(_, s)| s).collect();
                for session in sessions {
                    session.shutdown();
                }
            }
            glib::Propagation::Proceed
        });
    }
}

