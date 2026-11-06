//! Saved connections store, minimal connection editor, and VS Code Quick-Pick
//! connection launcher.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::config::{self, Connection};
use crate::ui;

pub struct ConnStore {
    list: RefCell<Vec<Connection>>,
    listeners: RefCell<Vec<Box<dyn Fn() -> bool>>>,
}

impl ConnStore {
    pub fn new() -> Rc<Self> {
        Rc::new(ConnStore {
            list: RefCell::new(config::load_connections()),
            listeners: RefCell::default(),
        })
    }

    pub fn list(&self) -> Vec<Connection> {
        self.list.borrow().clone()
    }

    pub fn upsert(&self, mut conn: Connection) -> Connection {
        {
            let mut list = self.list.borrow_mut();
            if conn.id == 0 {
                conn.id = config::new_id();
                list.push(conn.clone());
            } else if let Some(slot) = list.iter_mut().find(|c| c.id == conn.id) {
                *slot = conn.clone();
            } else {
                list.push(conn.clone());
            }
            let _ = config::save_connections(&list);
        }
        self.notify();
        conn
    }

    pub fn remove(&self, id: u64) {
        {
            let mut list = self.list.borrow_mut();
            list.retain(|c| c.id != id);
            let _ = config::save_connections(&list);
        }
        self.notify();
    }

    pub fn resolve_quick(&self, input: &str) -> Option<Connection> {
        let trimmed = input.trim();
        let bare = trimmed.strip_prefix("ssh ").unwrap_or(trimmed).trim();
        if bare.is_empty() {
            return None;
        }
        for c in self.list.borrow().iter() {
            if (!c.name.trim().is_empty() && c.name.trim().eq_ignore_ascii_case(bare))
                || c.subtitle().eq_ignore_ascii_case(bare)
                || (c.user.is_empty() && c.host.eq_ignore_ascii_case(bare))
            {
                return Some(c.clone());
            }
        }
        Connection::parse_quick(trimmed)
    }

    pub fn subscribe(&self, f: impl Fn() -> bool + 'static) {
        self.listeners.borrow_mut().push(Box::new(f));
    }

    fn notify(&self) {
        self.listeners.borrow_mut().retain(|f| f());
    }
}

// ---------------------------------------------------------------------------
// Minimal Connection Editor Dialog
// ---------------------------------------------------------------------------

pub fn edit_dialog(
    parent: &impl IsA<gtk::Widget>,
    store: Rc<ConnStore>,
    existing: Option<Connection>,
    on_saved: impl Fn(Connection) + 'static,
) {
    let init = existing.unwrap_or_default();

    let dialog = adw::Dialog::new();
    dialog.set_content_width(400);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.add_css_class("vs-form-dialog");

    let mk_field = |placeholder: &str, value: &str| {
        let e = gtk::Entry::new();
        e.set_placeholder_text(Some(placeholder));
        e.set_text(value);
        e.add_css_class("vs-field");
        e
    };

    let host_entry = mk_field("Host o IP (obbligatorio)", &init.host);
    let user_hbox = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let user_entry = mk_field("Utente", &init.user);
    user_entry.set_hexpand(true);
    let port_entry = mk_field(
        "Porta (22)",
        &init
            .port
            .filter(|p| *p != 22)
            .map(|p| p.to_string())
            .unwrap_or_default(),
    );
    port_entry.set_width_chars(9);
    port_entry.set_max_width_chars(9);
    user_hbox.append(&user_entry);
    user_hbox.append(&port_entry);

    let name_entry = mk_field("Alias / etichetta (opzionale)", &init.name);

    let key_hbox = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let key_entry = mk_field("Chiave privata SSH (opzionale)", &init.identity_file);
    key_entry.set_hexpand(true);
    let key_btn = gtk::Button::from_icon_name("vs-folder-symbolic");
    key_btn.add_css_class("flat");
    key_btn.add_css_class("vs-icon-btn");
    key_btn.set_tooltip_text(Some("Sfoglia…"));
    key_hbox.append(&key_entry);
    key_hbox.append(&key_btn);

    let dir_entry = mk_field("Percorso remoto iniziale (opzionale)", &init.initial_dir);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    actions.set_margin_top(4);
    let cancel_btn = gtk::Button::with_label("Annulla");
    cancel_btn.add_css_class("vs-btn");
    let save_btn = gtk::Button::with_label("Salva");
    save_btn.add_css_class("vs-btn-primary");
    save_btn.set_sensitive(!init.host.trim().is_empty());
    actions.append(&cancel_btn);
    actions.append(&save_btn);

    root.append(&host_entry);
    root.append(&user_hbox);
    root.append(&name_entry);
    root.append(&key_hbox);
    root.append(&dir_entry);
    root.append(&actions);
    dialog.set_child(Some(&root));

    host_entry.connect_changed(glib::clone!(
        #[weak]
        save_btn,
        move |e| save_btn.set_sensitive(!e.text().trim().is_empty())
    ));

    cancel_btn.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        move |_| {
            dialog.close();
        }
    ));

    let parent_win = parent.root().and_downcast::<gtk::Window>();
    key_btn.connect_clicked(glib::clone!(
        #[weak]
        key_entry,
        move |_| {
            let win = parent_win.clone();
            glib::spawn_future_local(async move {
                let fd = gtk::FileDialog::new();
                fd.set_title("Chiave privata SSH");
                let ssh_dir = glib::home_dir().join(".ssh");
                if ssh_dir.is_dir() {
                    fd.set_initial_folder(Some(&gio::File::for_path(&ssh_dir)));
                }
                if let Ok(file) = fd.open_future(win.as_ref()).await
                    && let Some(path) = file.path()
                {
                    key_entry.set_text(&path.display().to_string());
                }
            });
        }
    ));

    let id = init.id;
    let on_saved = Rc::new(on_saved);
    let do_save: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak]
        dialog,
        #[weak]
        host_entry,
        #[weak]
        user_entry,
        #[weak]
        port_entry,
        #[weak]
        name_entry,
        #[weak]
        key_entry,
        #[weak]
        dir_entry,
        #[weak]
        root,
        move || {
            let host = host_entry.text().trim().to_string();
            if host.is_empty() {
                return;
            }
            let port_txt = port_entry.text();
            let port_txt = port_txt.trim();
            let port = if port_txt.is_empty() {
                None
            } else {
                match port_txt.parse::<u16>() {
                    Ok(22) => None,
                    Ok(p) if p > 0 => Some(p),
                    _ => {
                        ui::toast(&root, "Porta non valida");
                        return;
                    }
                }
            };
            let conn = Connection {
                id,
                name: name_entry.text().trim().to_string(),
                host,
                user: user_entry.text().trim().to_string(),
                port,
                identity_file: key_entry.text().trim().to_string(),
                initial_dir: dir_entry.text().trim().to_string(),
            };
            let saved = store.upsert(conn);
            on_saved(saved);
            dialog.close();
        }
    ));

    let s = do_save.clone();
    save_btn.connect_clicked(move |_| s());
    for e in [&host_entry, &user_entry, &port_entry, &name_entry, &key_entry, &dir_entry] {
        let s = do_save.clone();
        e.connect_activate(move |_| s());
    }

    dialog.present(Some(parent));
    host_entry.grab_focus();
}

// ---------------------------------------------------------------------------
// VS Code Quick-Pick Launcher
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct HostItem {
    primary: String,
    secondary: String,
    complete_text: String,
    conn: Connection,
    saved_id: Option<u64>,
}

pub fn build_launcher(
    store: Rc<ConnStore>,
    on_connect: impl Fn(Connection) + 'static,
) -> (gtk::Widget, gtk::Entry) {
    let on_connect: Rc<dyn Fn(Connection)> = Rc::new(on_connect);

    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.set_hexpand(true);
    outer.set_vexpand(true);
    outer.add_css_class("vs-launcher-bg");

    let palette = gtk::Box::new(gtk::Orientation::Vertical, 0);
    palette.add_css_class("vs-palette");
    palette.set_halign(gtk::Align::Center);
    palette.set_valign(gtk::Align::Start);
    palette.set_margin_top(64);
    palette.set_size_request(460, -1);

    // -- Top Input Row -------------------------------------------------------
    let input_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    input_row.add_css_class("vs-palette-input-row");

    let entry = gtk::Entry::new();
    entry.set_hexpand(true);
    entry.set_placeholder_text(Some("utente@host[:porta]"));
    entry.add_css_class("vs-palette-entry");

    let add_btn = gtk::Button::from_icon_name("vs-plus-symbolic");
    add_btn.add_css_class("flat");
    add_btn.add_css_class("vs-icon-btn");
    add_btn.set_tooltip_text(Some("Salva connessione"));

    let go_btn = gtk::Button::from_icon_name("vs-arrow-right-symbolic");
    go_btn.add_css_class("flat");
    go_btn.add_css_class("vs-icon-btn");
    go_btn.set_tooltip_text(Some("Connetti"));

    input_row.append(&entry);
    input_row.append(&add_btn);
    input_row.append(&go_btn);
    palette.append(&input_row);

    // -- Filtered Host List --------------------------------------------------
    let list_box = gtk::ListBox::new();
    list_box.set_selection_mode(gtk::SelectionMode::Single);
    list_box.add_css_class("vs-palette-list");
    list_box.set_can_focus(false);

    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
    scrolled.set_max_content_height(320);
    scrolled.set_propagate_natural_height(true);
    scrolled.set_child(Some(&list_box));
    palette.append(&scrolled);

    outer.append(&palette);

    let visible_items: Rc<RefCell<Vec<HostItem>>> = Rc::default();
    let user_navigated = Rc::new(Cell::new(false));
    let suppress_filter = Rc::new(Cell::new(false));

    let refresh_list: Rc<dyn Fn(&gtk::ListBox, &gtk::ScrolledWindow, &str)> = {
        let store = store.clone();
        let visible_items = visible_items.clone();
        let outer_weak = outer.downgrade();
        Rc::new(move |lb, sc, filter| {
            while let Some(child) = lb.first_child() {
                lb.remove(&child);
            }
            let items = gather_items(&store, filter);
            sc.set_visible(!items.is_empty());

            for item in &items {
                let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                row_box.add_css_class("vs-palette-row");

                let primary = gtk::Label::new(Some(&item.primary));
                primary.set_xalign(0.0);
                primary.set_ellipsize(gtk::pango::EllipsizeMode::End);
                row_box.append(&primary);

                if !item.secondary.is_empty() {
                    let sec = gtk::Label::new(Some(&item.secondary));
                    sec.add_css_class("vs-muted");
                    sec.set_xalign(0.0);
                    sec.set_hexpand(true);
                    sec.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    row_box.append(&sec);
                } else {
                    primary.set_hexpand(true);
                }

                if let Some(id) = item.saved_id {
                    let edit_btn = gtk::Button::from_icon_name("vs-edit-symbolic");
                    edit_btn.add_css_class("flat");
                    edit_btn.add_css_class("vs-row-btn");
                    edit_btn.set_tooltip_text(Some("Modifica"));

                    let del_btn = gtk::Button::from_icon_name("vs-trash-symbolic");
                    del_btn.add_css_class("flat");
                    del_btn.add_css_class("vs-row-btn");
                    del_btn.set_tooltip_text(Some("Elimina"));

                    let c_edit = item.conn.clone();
                    let st_edit = store.clone();
                    let ow = outer_weak.clone();
                    edit_btn.connect_clicked(move |btn| {
                        if let Some(o) = ow.upgrade() {
                            edit_dialog(&o, st_edit.clone(), Some(c_edit.clone()), |_| {});
                        } else {
                            edit_dialog(btn, st_edit.clone(), Some(c_edit.clone()), |_| {});
                        }
                    });

                    let st_del = store.clone();
                    let label = item.primary.clone();
                    let ow = outer_weak.clone();
                    del_btn.connect_clicked(move |btn| {
                        let st_del = st_del.clone();
                        let label = label.clone();
                        let anchor: gtk::Widget = ow
                            .upgrade()
                            .map(|o| o.upcast())
                            .unwrap_or_else(|| btn.clone().upcast());
                        glib::spawn_future_local(async move {
                            if ui::confirm(&anchor, &format!("Eliminare «{label}»?"), "", "Elimina", true).await {
                                st_del.remove(id);
                            }
                        });
                    });

                    row_box.append(&edit_btn);
                    row_box.append(&del_btn);
                }

                let row = gtk::ListBoxRow::new();
                row.set_can_focus(false);
                row.set_child(Some(&row_box));
                lb.append(&row);
            }

            let select_first = filter.trim().is_empty() && !items.is_empty();
            *visible_items.borrow_mut() = items;
            if select_first {
                lb.select_row(lb.row_at_index(0).as_ref());
            } else {
                lb.unselect_all();
            }
        })
    };

    refresh_list(&list_box, &scrolled, "");

    let lb_w = list_box.downgrade();
    let sc_w = scrolled.downgrade();
    let e_w = entry.downgrade();
    let rf = refresh_list.clone();
    store.subscribe(move || {
        let (Some(lb), Some(sc), Some(e)) = (lb_w.upgrade(), sc_w.upgrade(), e_w.upgrade()) else {
            return false;
        };
        rf(&lb, &sc, &e.text());
        true
    });

    let rf_chg = refresh_list.clone();
    let nav_chg = user_navigated.clone();
    let sup_chg = suppress_filter.clone();
    entry.connect_changed(glib::clone!(
        #[weak]
        list_box,
        #[weak]
        scrolled,
        move |e| {
            nav_chg.set(false);
            if !sup_chg.get() {
                rf_chg(&list_box, &scrolled, &e.text());
            }
        }
    ));

    let vis_act = visible_items.clone();
    let cb_act = on_connect.clone();
    list_box.connect_row_activated(move |_, row| {
        let idx = row.index();
        if idx >= 0
            && let Some(item) = vis_act.borrow().get(idx as usize).cloned()
        {
            cb_act(item.conn);
        }
    });

    // Keyboard Up / Down / Tab navigation
    let key = gtk::EventControllerKey::new();
    key.set_propagation_phase(gtk::PropagationPhase::Capture);
    let vis_k = visible_items.clone();
    let nav_k = user_navigated.clone();
    let sup_k = suppress_filter.clone();
    key.connect_key_pressed(glib::clone!(
        #[weak]
        list_box,
        #[weak]
        entry,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, k, _, _| match k {
            gdk::Key::Down | gdk::Key::Up => {
                let len = vis_k.borrow().len() as i32;
                if len == 0 {
                    return glib::Propagation::Proceed;
                }
                let next = match list_box.selected_row() {
                    Some(r) => {
                        let cur = r.index();
                        if k == gdk::Key::Down {
                            (cur + 1).rem_euclid(len)
                        } else {
                            (cur - 1).rem_euclid(len)
                        }
                    }
                    None => {
                        if k == gdk::Key::Down {
                            0
                        } else {
                            len - 1
                        }
                    }
                };
                if let Some(r) = list_box.row_at_index(next) {
                    list_box.select_row(Some(&r));
                    nav_k.set(true);
                }
                glib::Propagation::Stop
            }
            gdk::Key::Tab => {
                let target_idx = list_box
                    .selected_row()
                    .map(|r| r.index())
                    .unwrap_or(0);
                if target_idx >= 0
                    && let Some(item) = vis_k.borrow().get(target_idx as usize).cloned()
                {
                    sup_k.set(true);
                    entry.set_text(&item.complete_text);
                    entry.set_position(-1);
                    sup_k.set(false);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
            _ => glib::Propagation::Proceed,
        }
    ));
    entry.add_controller(key);

    let do_connect: Rc<dyn Fn()> = {
        let store = store.clone();
        let on_connect = on_connect.clone();
        let vis = visible_items.clone();
        let nav = user_navigated.clone();
        Rc::new(glib::clone!(
            #[weak]
            entry,
            #[weak]
            list_box,
            #[weak]
            outer,
            move || {
                let txt = entry.text();
                let trimmed = txt.trim();
                if (nav.get() || trimmed.is_empty())
                    && let Some(row) = list_box.selected_row()
                    && let Some(item) = vis.borrow().get(row.index() as usize).cloned()
                {
                    on_connect(item.conn);
                    return;
                }
                if trimmed.is_empty() {
                    return;
                }
                match store.resolve_quick(trimmed) {
                    Some(conn) => on_connect(conn),
                    None => ui::toast(&outer, "Formato non valido (utente@host[:porta])"),
                }
            }
        ))
    };

    let dc1 = do_connect.clone();
    entry.connect_activate(move |_| dc1());
    let dc2 = do_connect;
    go_btn.connect_clicked(move |_| dc2());

    add_btn.connect_clicked(glib::clone!(
        #[weak]
        entry,
        #[weak]
        outer,
        move |_| {
            let preset = Connection::parse_quick(&entry.text());
            edit_dialog(&outer, store.clone(), preset, |_| {});
        }
    ));

    (outer.upcast(), entry)
}

fn gather_items(store: &ConnStore, filter: &str) -> Vec<HostItem> {
    let q = filter
        .trim()
        .strip_prefix("ssh ")
        .unwrap_or(filter.trim())
        .trim()
        .to_lowercase();

    let mut out = Vec::new();
    for c in store.list() {
        let sub = c.subtitle();
        let has_alias = !c.name.trim().is_empty() && c.name.trim() != sub;
        let primary = if has_alias {
            c.name.trim().to_string()
        } else {
            sub.clone()
        };
        let secondary = if has_alias { sub.clone() } else { String::new() };
        if !q.is_empty()
            && !primary.to_lowercase().contains(&q)
            && !sub.to_lowercase().contains(&q)
        {
            continue;
        }
        out.push(HostItem {
            primary,
            secondary,
            complete_text: sub,
            saved_id: Some(c.id),
            conn: c,
        });
    }

    for host in config::ssh_config_hosts() {
        if (!q.is_empty() && !host.to_lowercase().contains(&q))
            || out
                .iter()
                .any(|x| x.conn.host.eq_ignore_ascii_case(&host) || x.primary.eq_ignore_ascii_case(&host))
        {
            continue;
        }
        out.push(HostItem {
            primary: host.clone(),
            secondary: "ssh config".to_string(),
            complete_text: host.clone(),
            saved_id: None,
            conn: Connection {
                host,
                ..Default::default()
            },
        });
    }
    out
}
