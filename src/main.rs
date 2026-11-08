mod config;
mod editsync;
mod explorer;
mod icons;
mod remote;
mod session;
mod sftp;
mod theme;
mod ui;
mod window;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

const APP_ID: &str = "io.github.sshclient.App";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE | gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    app.connect_startup(|app| {
        if let Some(display) = gdk::Display::default() {
            icons::register(&display);
            theme::init(&display);
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
        glib::ExitCode::SUCCESS
    });

    app.connect_activate(|app| {
        let win = window::AppWindow::new(app, &[]);
        win.window.present();
    });

    app.run()
}
