mod config;
mod connections;
mod editsync;
mod explorer;
mod icons;
mod remote;
mod session;
mod sftp;
mod ui;
mod window;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

const APP_ID: &str = "io.github.sshclient.App";

const STYLE_CSS: &str = r#"
window {
    background-color: #1f1f1f;
    color: #cccccc;
}

/* Unified VS Code Top Bar */
.vs-topbar {
    background-color: #181818;
    border-bottom: 1px solid #2b2b2b;
    min-height: 34px;
    padding: 0 2px;
}

.vs-topbar-btn {
    min-width: 26px;
    min-height: 26px;
    padding: 0 4px;
    border-radius: 4px;
    color: #9d9d9d;
}

.vs-topbar-btn:hover {
    background-color: rgba(255, 255, 255, 0.07);
    color: #e0e0e0;
}

.vs-tabbar {
    background-color: transparent;
}

.vs-tabbar tabbox {
    background-color: #181818;
    padding: 0;
    border: none;
}

.vs-tabbar tab {
    background-color: #181818;
    color: #8b8b8b;
    border-radius: 0;
    border-right: 1px solid #2b2b2b;
    min-height: 34px;
    padding: 0 10px;
    margin: 0;
    font-size: 12.5px;
    box-shadow: none;
}

.vs-tabbar tab:hover {
    background-color: #1f1f1f;
    color: #cccccc;
}

.vs-tabbar tab:checked {
    background-color: #1f1f1f;
    color: #ffffff;
    box-shadow: inset 0 1px 0 0 #0078d4;
}

/* SFTP Explorer Sidebar */
.vs-explorer {
    background-color: #181818;
    border-right: 1px solid #2b2b2b;
}

.vs-explorer-bar {
    padding: 4px 6px;
    border-bottom: 1px solid #2b2b2b;
    min-height: 30px;
}

.vs-path-entry {
    background-color: #252526;
    color: #cccccc;
    border: 1px solid #323232;
    border-radius: 3px;
    min-height: 24px;
    padding: 0 6px;
    font-size: 12px;
    font-family: monospace;
    box-shadow: none;
}

.vs-path-entry:focus-within {
    border-color: #0078d4;
    outline: none;
}

.vs-icon-btn {
    min-width: 22px;
    min-height: 22px;
    padding: 2px;
    border-radius: 3px;
    color: #9d9d9d;
}

.vs-icon-btn:hover {
    background-color: rgba(255, 255, 255, 0.08);
    color: #e0e0e0;
}

.vs-thin-progress,
.vs-thin-progress trough {
    min-height: 2px;
    background-color: transparent;
    border-radius: 0;
}

.vs-thin-progress progress {
    min-height: 2px;
    background-color: #0078d4;
    border-radius: 0;
}

.vs-tree {
    background-color: transparent;
    font-size: 12.5px;
    color: #cccccc;
}

.vs-tree > row {
    min-height: 22px;
    padding: 1px 6px;
    margin: 0;
    border-radius: 0;
}

.vs-tree > row:hover {
    background-color: #2a2d2e;
}

.vs-tree > row:selected {
    background-color: #04395e;
    color: #ffffff;
}

.vs-tree-icon {
    color: #9d9d9d;
}

.vs-hidden-entry {
    opacity: 0.55;
}

paned > separator {
    background-color: #2b2b2b;
    min-width: 1px;
}

/* Terminal & Drop Overlay */
.vs-terminal-box {
    background-color: #1f1f1f;
}

.vs-drop-overlay {
    background-color: rgba(24, 24, 24, 0.78);
}

.vs-drop-card {
    background-color: #252526;
    border: 1px dashed #0078d4;
    border-radius: 4px;
    padding: 20px 32px;
}

.vs-drop-active {
    outline: 1px dashed #0078d4;
    outline-offset: -2px;
}

/* 22px VS Code Status Bar */
.vs-statusbar {
    background-color: #181818;
    border-top: 1px solid #2b2b2b;
    padding: 0 10px;
    min-height: 22px;
    font-size: 11.5px;
    color: #8b8b8b;
}

.vs-dot {
    font-size: 9px;
}

.vs-dot.ok {
    color: #3fb950;
}

.vs-dot.wait {
    color: #d29922;
}

.vs-dot.err {
    color: #f85149;
}

.vs-status-btn {
    min-height: 18px;
    padding: 0 6px;
    border-radius: 2px;
    font-size: 11px;
    color: #4daafc;
}

.vs-status-btn:hover {
    background-color: rgba(255, 255, 255, 0.07);
}

.vs-sync-badge {
    color: #3fb950;
    font-family: monospace;
    font-size: 11px;
}

/* VS Code Command-Palette Launcher */
.vs-launcher-bg {
    background-color: #1f1f1f;
}

.vs-palette {
    background-color: #252526;
    border: 1px solid #3c3c3c;
    border-radius: 4px;
}

.vs-palette-input-row {
    padding: 6px 8px;
}

.vs-palette-entry {
    background-color: #313131;
    color: #cccccc;
    border: 1px solid #3c3c3c;
    border-radius: 3px;
    min-height: 28px;
    padding: 0 8px;
    font-family: monospace;
    font-size: 13px;
    box-shadow: none;
}

.vs-palette-entry:focus-within {
    border-color: #0078d4;
    outline: none;
}

.vs-palette-list {
    background-color: transparent;
    border-top: 1px solid #323232;
}

.vs-palette-list > row {
    padding: 0;
    margin: 0;
    border-radius: 0;
}

.vs-palette-list > row:hover {
    background-color: #2a2d2e;
}

.vs-palette-list > row:selected {
    background-color: #04395e;
    color: #ffffff;
}

.vs-palette-row {
    padding: 5px 10px;
    min-height: 26px;
    font-size: 12.5px;
}

.vs-row-btn {
    min-width: 20px;
    min-height: 20px;
    padding: 1px;
    border-radius: 3px;
    color: #858585;
}

.vs-row-btn:hover {
    background-color: rgba(255, 255, 255, 0.1);
    color: #e0e0e0;
}

.vs-muted {
    color: #858585;
    font-size: 11.5px;
}

/* Minimal Dialogs ("Salva connessione" / "Apri con…") */
.vs-form-dialog {
    padding: 16px;
    background-color: #252526;
}

.vs-field {
    background-color: #313131;
    color: #cccccc;
    border: 1px solid #3c3c3c;
    border-radius: 3px;
    min-height: 30px;
    padding: 0 8px;
    font-size: 12.5px;
    box-shadow: none;
}

.vs-field:focus-within {
    border-color: #0078d4;
    outline: none;
}

.vs-btn {
    background-color: #313131;
    color: #cccccc;
    border: 1px solid #3c3c3c;
    border-radius: 3px;
    min-height: 28px;
    padding: 0 12px;
    font-size: 12.5px;
    box-shadow: none;
}

.vs-btn:hover {
    background-color: #3c3c3c;
}

.vs-btn-primary {
    background-color: #0078d4;
    color: #ffffff;
    border: none;
    border-radius: 3px;
    min-height: 28px;
    padding: 0 14px;
    font-size: 12.5px;
    box-shadow: none;
}

.vs-btn-primary:hover {
    background-color: #1a86d9;
}

.vs-btn-primary:disabled {
    opacity: 0.45;
}

.vs-picker-dialog {
    background-color: #252526;
    padding: 6px;
}

.vs-input {
    background-color: #313131;
    color: #cccccc;
    border: 1px solid #3c3c3c;
    border-radius: 3px;
    min-height: 28px;
    padding: 0 8px;
    margin-bottom: 4px;
    font-size: 12.5px;
    box-shadow: none;
}

.vs-input:focus-within {
    border-color: #0078d4;
    outline: none;
}

.vs-list {
    background-color: transparent;
}

.vs-list > row {
    border-radius: 3px;
    padding: 0;
}

.vs-list > row:hover {
    background-color: #2a2d2e;
}

.vs-list > row:selected {
    background-color: #04395e;
    color: #ffffff;
}

.vs-row {
    padding: 5px 8px;
    font-size: 12.5px;
}

popover > contents {
    background-color: #252526;
    border: 1px solid #3c3c3c;
    border-radius: 4px;
    padding: 4px;
}
"#;

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE | gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    app.connect_startup(|app| {
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);

        if let Some(display) = gdk::Display::default() {
            icons::register(&display);
            let provider = gtk::CssProvider::new();
            provider.load_from_string(STYLE_CSS);
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        let quit = gio::SimpleAction::new("quit", None);
        quit.connect_activate(glib::clone!(
            #[weak]
            app,
            move |_, _| {
                for w in app.windows() {
                    w.close();
                }
            }
        ));
        app.add_action(&quit);

        app.set_accels_for_action("app.quit", &["<Control><Shift>q"]);
        app.set_accels_for_action("win.new-tab", &["<Control><Shift>t"]);
        app.set_accels_for_action("win.toggle-sidebar", &["<Control><Shift>b"]);
        app.set_accels_for_action("win.close-tab", &["<Control><Shift>w"]);
        app.set_accels_for_action("win.next-tab", &["<Control>Tab"]);
        app.set_accels_for_action("win.prev-tab", &["<Control><Shift>Tab"]);
    });

    app.connect_command_line(|app, cmdline| {
        let args: Vec<String> = cmdline
            .arguments()
            .into_iter()
            .skip(1)
            .filter_map(|os| os.into_string().ok())
            .filter(|s| !s.starts_with('-'))
            .collect();
        let win = window::AppWindow::new(app, &args);
        win.window.present();
        glib::ExitCode::SUCCESS.into()
    });

    app.connect_activate(|app| {
        let win = window::AppWindow::new(app, &[]);
        win.window.present();
    });

    app.run()
}
