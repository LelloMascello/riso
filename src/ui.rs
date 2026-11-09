//! Terminal widget construction and minimal UI dialogs.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib, pango};
use vte4::prelude::*;

pub type PasteHook = Rc<RefCell<Option<Box<dyn Fn() -> bool>>>>;

pub fn new_terminal() -> (vte4::Terminal, PasteHook) {
    let term = vte4::Terminal::new();
    let paste_hook: PasteHook = Rc::default();
    term.set_hexpand(true);
    term.set_vexpand(true);
    term.set_scrollback_lines(10_000);
    term.set_mouse_autohide(true);
    term.set_audible_bell(false);
    term.set_enable_fallback_scrolling(true);
    term.set_scroll_on_keystroke(true);
    term.set_font(Some(&pango::FontDescription::from_string("Monospace 10.5")));
    term.set_cell_height_scale(1.08);
    term.add_css_class("session-terminal");
    crate::theme::bind_terminal(&term);
    term.set_margin_start(12);
    term.set_margin_end(6);
    term.set_margin_top(8);
    term.set_margin_bottom(4);

    let actions = gio::SimpleActionGroup::new();
    let copy = gio::SimpleAction::new("copy", None);
    copy.connect_activate(glib::clone!(
        #[weak]
        term,
        move |_, _| term.copy_clipboard_format(vte4::Format::Text)
    ));
    let paste = gio::SimpleAction::new("paste", None);
    let ph = paste_hook.clone();
    paste.connect_activate(glib::clone!(
        #[weak]
        term,
        move |_, _| {
            let handled = ph.borrow().as_ref().is_some_and(|f| f());
            if !handled {
                term.paste_clipboard();
            }
        }
    ));
    let select_all = gio::SimpleAction::new("select-all", None);
    select_all.connect_activate(glib::clone!(
        #[weak]
        term,
        move |_, _| term.select_all()
    ));
    actions.add_action(&copy);
    actions.add_action(&paste);
    actions.add_action(&select_all);
    term.insert_action_group("term", Some(&actions));

    let shortcuts = gtk::ShortcutController::new();
    shortcuts.set_propagation_phase(gtk::PropagationPhase::Capture);
    for (accel, action) in [
        ("<Control><Shift>c", "term.copy"),
        ("<Control><Shift>v", "term.paste"),
        ("<Shift>Insert", "term.paste"),
        ("<Control><Shift>a", "term.select-all"),
    ] {
        shortcuts.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string(accel),
            Some(gtk::NamedAction::new(action)),
        ));
    }
    term.add_controller(shortcuts);

    term.connect_selection_changed(glib::clone!(
        #[weak]
        copy,
        move |t| copy.set_enabled(t.has_selection())
    ));
    copy.set_enabled(false);

    let menu = gio::Menu::new();
    menu.append(Some("Copia"), Some("term.copy"));
    menu.append(Some("Incolla"), Some("term.paste"));
    menu.append(Some("Seleziona tutto"), Some("term.select-all"));
    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.set_parent(&term);
    popover.set_has_arrow(false);
    popover.set_halign(gtk::Align::Start);
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_SECONDARY);
    click.connect_pressed(glib::clone!(
        #[weak]
        popover,
        #[weak]
        term,
        #[weak]
        copy,
        move |_, _, x, y| {
            copy.set_enabled(term.has_selection());
            popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.popup();
        }
    ));
    term.add_controller(click);
    term.connect_destroy(move |_| popover.unparent());

    (term, paste_hook)
}

pub fn spawn(term: &vte4::Terminal, argv: &[String], cwd: Option<&str>) {
    let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let program = argv.first().cloned().unwrap_or_default();
    term.spawn_async(
        vte4::PtyFlags::DEFAULT,
        cwd,
        &argv_refs,
        &[],
        glib::SpawnFlags::SEARCH_PATH,
        || {},
        -1,
        None::<&gio::Cancellable>,
        glib::clone!(
            #[weak]
            term,
            move |res| {
                if let Err(e) = res {
                    let msg = format!("\r\nImpossibile avviare {program}: {}\r\n", e.message());
                    term.feed(msg.as_bytes());
                }
            }
        ),
    );
}

pub fn toast(widget: &impl IsA<gtk::Widget>, msg: &str) {
    let _ = widget.activate_action("win.toast", Some(&msg.to_variant()));
}

pub async fn confirm(
    widget: &impl IsA<gtk::Widget>,
    heading: &str,
    body: &str,
    accept: &str,
    destructive: bool,
) -> bool {
    let dialog = adw::AlertDialog::new(Some(heading), (!body.is_empty()).then_some(body));
    dialog.add_responses(&[("cancel", "Annulla"), ("ok", accept)]);
    dialog.set_response_appearance(
        "ok",
        if destructive {
            adw::ResponseAppearance::Destructive
        } else {
            adw::ResponseAppearance::Suggested
        },
    );
    dialog.set_default_response(Some(if destructive { "cancel" } else { "ok" }));
    dialog.set_close_response("cancel");
    dialog.choose_future(Some(widget)).await == "ok"
}

pub async fn prompt(
    widget: &impl IsA<gtk::Widget>,
    heading: &str,
    initial: &str,
    accept: &str,
    select_stem: bool,
) -> Option<String> {
    let dialog = adw::AlertDialog::new(Some(heading), None);
    dialog.add_responses(&[("cancel", "Annulla"), ("ok", accept)]);
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("cancel");
    let entry = gtk::Entry::new();
    entry.add_css_class("riso-dialog-entry");
    entry.set_text(initial);
    entry.set_activates_default(true);
    dialog.set_extra_child(Some(&entry));
    dialog.set_response_enabled("ok", !initial.trim().is_empty());
    entry.connect_changed(glib::clone!(
        #[weak]
        dialog,
        move |e| dialog.set_response_enabled("ok", !e.text().trim().is_empty())
    ));
    let stem_len = if select_stem {
        match initial.rfind('.') {
            Some(i) if i > 0 => i as i32,
            _ => -1,
        }
    } else {
        -1
    };
    entry.connect_map(move |e| {
        e.grab_focus();
        e.select_region(0, stem_len);
    });
    let res = dialog.choose_future(Some(widget)).await;
    let text = entry.text().trim().to_string();
    (res == "ok" && !text.is_empty()).then_some(text)
}

pub fn human_size(n: u64) -> String {
    glib::format_size(n).to_string()
}
