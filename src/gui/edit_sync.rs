//! GUI: local editing of remote files with support for terminal editors (`nvim`,
//! `helix`, `vim`, …) and atomic/backup-rename saves.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::files::FileStamp;
use crate::gui::widgets;
use crate::remote::Remote;
use crate::sftp;

const KNOWN_TUI_EDITORS: &[&str] = &[
    "nvim", "vim", "vi", "hx", "helix", "nano", "micro", "kak", "ne", "vis",
];

struct Watch {
    remote_path: String,
    _monitor: gio::FileMonitor,
    last_stamp: Option<FileStamp>,
    pending: Option<glib::SourceId>,
}

pub struct EditSync {
    base: PathBuf,
    widget: gtk::Widget,
    remote: RefCell<Option<Rc<Remote>>>,
    watches: RefCell<HashMap<PathBuf, Watch>>,
    on_changed: RefCell<Option<Box<dyn Fn(&[String])>>>,
}

impl EditSync {
    pub fn new(session_id: u64, widget: &impl IsA<gtk::Widget>) -> Rc<Self> {
        let base = crate::paths::cache_dir()
            .join("riso")
            .join("edit")
            .join(format!("{}-{session_id}", std::process::id()));
        Rc::new(EditSync {
            base,
            widget: widget.clone().upcast(),
            remote: RefCell::default(),
            watches: RefCell::default(),
            on_changed: RefCell::default(),
        })
    }

    pub fn set_remote(&self, remote: Option<Rc<Remote>>) {
        *self.remote.borrow_mut() = remote;
    }

    pub fn connect_changed(&self, f: impl Fn(&[String]) + 'static) {
        *self.on_changed.borrow_mut() = Some(Box::new(f));
    }

    fn notify(&self) {
        let mut names: Vec<String> = self
            .watches
            .borrow()
            .values()
            .map(|w| sftp::basename(&w.remote_path))
            .collect();
        names.sort();
        if let Some(f) = self.on_changed.borrow().as_ref() {
            f(&names);
        }
    }

    pub fn clear(&self) {
        for (_, w) in self.watches.borrow_mut().drain() {
            if let Some(id) = w.pending {
                id.remove();
            }
        }
        let _ = fs::remove_dir_all(&self.base);
        self.notify();
    }

    fn local_path(&self, remote_path: &str) -> PathBuf {
        self.base.join(remote_path.trim_start_matches('/'))
    }

    pub async fn open(self: &Rc<Self>, remote_path: String, ask_app: bool) {
        let Some(remote) = self.remote.borrow().clone() else {
            widgets::toast(&self.widget, "Non connesso");
            return;
        };
        let local = self.local_path(&remote_path);
        if let Some(dir) = local.parent() {
            if let Err(e) = fs::create_dir_all(dir) {
                widgets::toast(&self.widget, &format!("Errore locale: {e}"));
                return;
            }
        }

        if let Some(w) = self.watches.borrow_mut().remove(&local) {
            if let Some(id) = w.pending {
                id.remove();
            }
        }

        let (r, l) = (remote_path.clone(), local.clone());
        if let Err(e) = remote.run(move |s| s.download_file(&r, &l, &mut |_| {})).await {
            widgets::toast(
                &self.widget,
                &format!("{}: {e}", sftp::basename(&remote_path)),
            );
            return;
        }

        if let Err(e) = self.watch(&local, &remote_path) {
            widgets::toast(&self.widget, &format!("Monitoraggio fallito: {e}"));
        }

        if ask_app {
            open_with_dialog(&self.widget, &local).await;
        } else if let Err(e) = launch_default(&local) {
            widgets::toast(&self.widget, &e);
        }
    }

    fn watch(self: &Rc<Self>, local: &Path, remote_path: &str) -> Result<(), glib::Error> {
        let parent = local.parent().unwrap_or(local);
        let target_name = local.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        // Watch the parent directory so atomic saves and backup-renames
        // (e.g. Neovim renaming `file` -> `file~` and creating a new `file`)
        // never detach the monitor.
        let dir_file = gio::File::for_path(parent);
        let monitor = dir_file.monitor_directory(
            gio::FileMonitorFlags::WATCH_MOVES,
            None::<&gio::Cancellable>,
        )?;
        let weak = Rc::downgrade(self);
        let key = local.to_path_buf();
        monitor.connect_changed(move |_, file, other, event| {
            use gio::FileMonitorEvent as E;
            if !matches!(
                event,
                E::Changed | E::ChangesDoneHint | E::Created | E::MovedIn | E::Renamed
            ) {
                return;
            }
            let matches_file = file
                .path()
                .and_then(|p| p.file_name().map(|n| n == target_name))
                .unwrap_or(false);
            let matches_other = other
                .and_then(|o| o.path())
                .and_then(|p| p.file_name().map(|n| n == target_name))
                .unwrap_or(false);
            if (matches_file || matches_other)
                && let Some(this) = weak.upgrade()
            {
                this.schedule_upload(&key);
            }
        });
        self.watches.borrow_mut().insert(
            local.to_path_buf(),
            Watch {
                remote_path: remote_path.to_string(),
                _monitor: monitor,
                last_stamp: FileStamp::of(local),
                pending: None,
            },
        );
        self.notify();
        Ok(())
    }

    fn schedule_upload(self: &Rc<Self>, local: &Path) {
        let mut watches = self.watches.borrow_mut();
        let Some(w) = watches.get_mut(local) else { return };
        if let Some(id) = w.pending.take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let key = local.to_path_buf();
        w.pending = Some(glib::timeout_add_local_once(
            Duration::from_millis(300),
            move || {
                if let Some(this) = weak.upgrade() {
                    if let Some(w) = this.watches.borrow_mut().get_mut(&key) {
                        w.pending = None;
                    }
                    glib::spawn_future_local(async move { this.upload(key).await });
                }
            },
        ));
    }

    async fn upload(self: Rc<Self>, local: PathBuf) {
        let Some(current) = FileStamp::of(&local) else { return };
        let remote_path = {
            let watches = self.watches.borrow();
            let Some(w) = watches.get(&local) else { return };
            if Some(current) == w.last_stamp {
                return;
            }
            w.remote_path.clone()
        };
        let name = sftp::basename(&remote_path);
        let Some(remote) = self.remote.borrow().clone() else {
            widgets::toast(&self.widget, &format!("{name}: non connesso"));
            return;
        };
        let (l, r) = (local.clone(), remote_path);
        match remote.run(move |s| s.upload_file(&l, &r, &mut |_| {})).await {
            Ok(()) => {
                if let Some(w) = self.watches.borrow_mut().get_mut(&local) {
                    w.last_stamp = Some(current);
                }
            }
            Err(e) => widgets::toast(&self.widget, &format!("Sync {name}: {e}")),
        }
    }
}

impl Drop for EditSync {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

// ---------------------------------------------------------------------------
// Application & Terminal Editor Launching
// ---------------------------------------------------------------------------

fn guess_content_type(path: &Path) -> (String, bool) {
    let mut sample = [0u8; 512];
    let n = fs::File::open(path)
        .and_then(|mut f| f.read(&mut sample))
        .unwrap_or(0);
    let slice = &sample[..n];
    let name = path.file_name().and_then(|s| s.to_str());
    let (ctype, _) = gio::content_type_guess(name, Some(slice));
    let looks_text = gio::content_type_is_a(&ctype, "text/plain")
        || (n == 0 && ctype == "application/octet-stream")
        || (n > 0 && !slice.contains(&0));
    (ctype.into(), looks_text)
}

fn launch_default(local: &Path) -> Result<(), String> {
    let (ctype, is_text) = guess_content_type(local);

    if let Some(app) = gio::AppInfo::default_for_type(&ctype, false) {
        return launch_app_info(&app, local);
    }
    if is_text {
        if let Some(app) = gio::AppInfo::default_for_type("text/plain", false) {
            return launch_app_info(&app, local);
        }
        if let Some(editor) = env_editor() {
            return launch_command_line(&editor, local, true);
        }
    }
    Err(format!(
        "Nessuna applicazione associata a {}",
        local.file_name().unwrap_or_default().to_string_lossy()
    ))
}

fn env_editor() -> Option<String> {
    std::env::var("VISUAL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("EDITOR").ok().filter(|s| !s.trim().is_empty()))
}

fn launch_app_info(app: &gio::AppInfo, local: &Path) -> Result<(), String> {
    if is_terminal_app(app) {
        let argv = build_app_argv(app, local);
        return spawn_in_terminal(&argv, local.parent());
    }
    let ctx = gdk::Display::default().map(|d| d.app_launch_context());
    app.launch(&[gio::File::for_path(local)], ctx.as_ref())
        .map_err(|e| format!("Impossibile avviare {}: {}", app.display_name(), e.message()))
}

fn is_terminal_app(app: &gio::AppInfo) -> bool {
    if let Some(id) = app.id() {
        let mut dirs = vec![glib::user_data_dir()];
        dirs.extend(glib::system_data_dirs());
        for dir in dirs {
            let desktop = dir.join("applications").join(id.as_str());
            if desktop.is_file() {
                let kf = glib::KeyFile::new();
                if kf
                    .load_from_file(&desktop, glib::KeyFileFlags::NONE)
                    .is_ok()
                {
                    return kf.boolean("Desktop Entry", "Terminal").unwrap_or(false);
                }
            }
        }
    }
    app.executable()
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|bin| KNOWN_TUI_EDITORS.contains(&bin))
}

fn build_app_argv(app: &gio::AppInfo, local: &Path) -> Vec<String> {
    let file_str = local.to_string_lossy().into_owned();
    if let Some(cmdline) = app.commandline() {
        if let Ok(parts) = glib::shell_parse_argv(&cmdline) {
            let mut argv = Vec::new();
            let mut inserted = false;
            for part in parts {
                let s = part.to_string_lossy();
                match s.as_ref() {
                    "%f" | "%F" | "%u" | "%U" => {
                        argv.push(file_str.clone());
                        inserted = true;
                    }
                    _ if s.starts_with('%') => {}
                    _ => {
                        if s.contains("%f") || s.contains("%F") {
                            argv.push(s.replace("%f", &file_str).replace("%F", &file_str));
                            inserted = true;
                        } else {
                            argv.push(s.into_owned());
                        }
                    }
                }
            }
            if !inserted {
                argv.push(file_str.clone());
            }
            if !argv.is_empty() {
                return argv;
            }
        }
    }
    vec![app.executable().to_string_lossy().into_owned(), file_str]
}

fn launch_command_line(cmd_str: &str, local: &Path, force_terminal_check: bool) -> Result<(), String> {
    let parts = glib::shell_parse_argv(cmd_str)
        .map_err(|e| format!("Comando non valido: {}", e.message()))?;
    let mut argv: Vec<String> = parts
        .into_iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    let Some(first) = argv.first().cloned() else {
        return Err("Comando vuoto".to_string());
    };
    argv.push(local.to_string_lossy().into_owned());

    let bin = Path::new(&first)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&first);
    let is_tui = force_terminal_check && KNOWN_TUI_EDITORS.contains(&bin)
        || gio::AppInfo::all().iter().any(|a| {
            a.executable().file_name().and_then(|n| n.to_str()) == Some(bin)
                && is_terminal_app(a)
        });

    if is_tui {
        spawn_in_terminal(&argv, local.parent())
    } else {
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(dir) = local.parent() {
            cmd.current_dir(dir);
        }
        cmd.spawn()
            .map(|_| ())
            .map_err(|e| format!("Impossibile avviare {first}: {e}"))
    }
}

fn spawn_in_terminal(editor_argv: &[String], cwd: Option<&Path>) -> Result<(), String> {
    let Some((term_bin, prefix_args)) = detect_terminal() else {
        return Err(
            "Nessun terminale trovato ($TERMINAL, kitty, foot, alacritty, wezterm, …)".to_string(),
        );
    };
    let mut cmd = Command::new(&term_bin);
    cmd.args(&prefix_args)
        .args(editor_argv)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("Impossibile avviare il terminale {term_bin}: {e}"))
}

fn detect_terminal() -> Option<(String, Vec<String>)> {
    if let Ok(env_term) = std::env::var("TERMINAL") {
        let env_term = env_term.trim();
        if !env_term.is_empty()
            && let Ok(parts) = glib::shell_parse_argv(env_term)
            && !parts.is_empty()
        {
            let mut iter = parts.into_iter().map(|p| p.to_string_lossy().into_owned());
            let bin = iter.next().unwrap();
            let mut args: Vec<String> = iter.collect();
            let name = Path::new(&bin)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&bin);
            if args.is_empty() {
                args.extend(terminal_exec_flag(name));
            }
            return Some((bin, args));
        }
    }

    for candidate in [
        "xdg-terminal-exec",
        "kitty",
        "foot",
        "alacritty",
        "ghostty",
        "wezterm",
        "ptyxis",
        "kgx",
        "gnome-terminal",
        "konsole",
        "xfce4-terminal",
        "xterm",
    ] {
        if glib::find_program_in_path(candidate).is_some() {
            return Some((candidate.to_string(), terminal_exec_flag(candidate)));
        }
    }
    None
}

fn terminal_exec_flag(term_name: &str) -> Vec<String> {
    match term_name {
        "xdg-terminal-exec" => vec![],
        "kitty" | "gnome-terminal" | "ptyxis" => vec!["--".into()],
        "wezterm" => vec!["start".into(), "--".into()],
        _ => vec!["-e".into()],
    }
}

// ---------------------------------------------------------------------------
// Minimal "Apri con…" quick-pick dialog
// ---------------------------------------------------------------------------

async fn open_with_dialog(parent: &gtk::Widget, local: &Path) {
    let (ctype, is_text) = guess_content_type(local);
    let mut apps = gio::AppInfo::all_for_type(&ctype);
    if is_text {
        for a in gio::AppInfo::all_for_type("text/plain") {
            if !apps.iter().any(|x| x.equal(&a)) {
                apps.push(a);
            }
        }
    }

    let dialog = adw::Dialog::new();
    dialog.set_content_width(400);

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
    vbox.add_css_class("picker-dialog");

    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("Comando o applicazione (es. nvim, helix)…"));
    entry.add_css_class("picker-input");
    vbox.append(&entry);

    let list_box = gtk::ListBox::new();
    list_box.set_selection_mode(gtk::SelectionMode::Single);
    list_box.add_css_class("picker-list");

    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
    scrolled.set_max_content_height(280);
    scrolled.set_propagate_natural_height(true);
    scrolled.set_child(Some(&list_box));
    vbox.append(&scrolled);

    #[derive(Clone)]
    enum Choice {
        Command(String),
        App(gio::AppInfo),
    }

    let mut all_choices: Vec<(String, String, Choice)> = Vec::new();
    if let Some(ed) = env_editor() {
        all_choices.push((ed.clone(), "$EDITOR".to_string(), Choice::Command(ed)));
    }
    for app in apps {
        let name = app.display_name().to_string();
        let exe = app
            .executable()
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let sub = if exe != name.to_lowercase() { exe } else { String::new() };
        all_choices.push((name, sub, Choice::App(app)));
    }

    let visible_choices: Rc<RefCell<Vec<Choice>>> = Rc::default();
    let populate = {
        let all_choices = all_choices.clone();
        let visible_choices = visible_choices.clone();
        move |lb: &gtk::ListBox, filter: &str| {
            while let Some(c) = lb.first_child() {
                lb.remove(&c);
            }
            let q = filter.trim().to_lowercase();
            let mut vis = Vec::new();
            for (title, sub, ch) in &all_choices {
                if !q.is_empty()
                    && !title.to_lowercase().contains(&q)
                    && !sub.to_lowercase().contains(&q)
                {
                    continue;
                }
                let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                hbox.add_css_class("picker-row");
                let lbl = gtk::Label::new(Some(title));
                lbl.set_xalign(0.0);
                lbl.set_hexpand(true);
                hbox.append(&lbl);
                if !sub.is_empty() {
                    let muted = gtk::Label::new(Some(sub));
                    muted.add_css_class("muted-label");
                    hbox.append(&muted);
                }
                let row = gtk::ListBoxRow::new();
                row.set_child(Some(&hbox));
                lb.append(&row);
                vis.push(ch.clone());
            }
            *visible_choices.borrow_mut() = vis;
            if let Some(first) = lb.row_at_index(0) {
                lb.select_row(Some(&first));
            }
        }
    };
    populate(&list_box, "");

    let pop = populate.clone();
    entry.connect_changed(glib::clone!(
        #[weak]
        list_box,
        move |e| pop(&list_box, &e.text())
    ));

    let (tx, rx) = async_channel::bounded::<Option<Choice>>(1);

    let tx_row = tx.clone();
    let vis_row = visible_choices.clone();
    list_box.connect_row_activated(glib::clone!(
        #[weak]
        dialog,
        move |_, row| {
            let idx = row.index();
            if idx >= 0
                && let Some(ch) = vis_row.borrow().get(idx as usize).cloned()
            {
                let _ = tx_row.try_send(Some(ch));
                dialog.close();
            }
        }
    ));

    let tx_ent = tx.clone();
    let vis_ent = visible_choices.clone();
    entry.connect_activate(glib::clone!(
        #[weak]
        list_box,
        #[weak]
        dialog,
        move |e| {
            if let Some(row) = list_box.selected_row()
                && let Some(ch) = vis_ent.borrow().get(row.index() as usize).cloned()
            {
                let _ = tx_ent.try_send(Some(ch));
                dialog.close();
                return;
            }
            let txt = e.text().trim().to_string();
            if !txt.is_empty() {
                let _ = tx_ent.try_send(Some(Choice::Command(txt)));
                dialog.close();
            }
        }
    ));

    let key = gtk::EventControllerKey::new();
    key.set_propagation_phase(gtk::PropagationPhase::Capture);
    let vis_key = visible_choices.clone();
    key.connect_key_pressed(glib::clone!(
        #[weak]
        list_box,
        #[weak]
        dialog,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, k, _, _| match k {
            gdk::Key::Down | gdk::Key::Up => {
                let len = vis_key.borrow().len() as i32;
                if len > 0 {
                    let cur = list_box.selected_row().map(|r| r.index()).unwrap_or(0);
                    let next = if k == gdk::Key::Down {
                        (cur + 1).rem_euclid(len)
                    } else {
                        (cur - 1).rem_euclid(len)
                    };
                    if let Some(r) = list_box.row_at_index(next) {
                        list_box.select_row(Some(&r));
                    }
                }
                glib::Propagation::Stop
            }
            gdk::Key::Escape => {
                dialog.close();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    ));
    entry.add_controller(key);

    let tx_close = tx.clone();
    dialog.connect_closed(move |_| {
        let _ = tx_close.try_send(None);
    });

    dialog.set_child(Some(&vbox));
    dialog.present(Some(parent));
    entry.grab_focus();

    if let Ok(Some(choice)) = rx.recv().await {
        let res = match choice {
            Choice::App(app) => launch_app_info(&app, local),
            Choice::Command(cmd) => launch_command_line(&cmd, local, true),
        };
        if let Err(e) = res {
            widgets::toast(parent, &e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvim_terminal_detection_and_argv() {
        if let Some(app) = gio::AppInfo::default_for_type("text/plain", false)
            && app.id().as_deref() == Some("nvim.desktop")
        {
            assert!(is_terminal_app(&app));
            let argv = build_app_argv(&app, Path::new("/tmp/prova.rs"));
            assert_eq!(argv, vec!["nvim", "/tmp/prova.rs"]);
        }
        let term = detect_terminal();
        assert!(term.is_some());
    }
}
