//! RISO — Remote Interface for SSH Operations.
//!
//! One crate, two frontends:
//!
//! * `riso-gui` — GTK4/libadwaita application (cargo feature `gui`), code in [`gui`];
//! * `riso-tui` — terminal application (cargo feature `tui`), code in [`tui`].
//!
//! Modules at the crate root are shared by both frontends and must not
//! depend on any UI toolkit.

// The TUI does not use every shared helper yet: keep a GUI-only build strict,
// silence unused-code warnings otherwise.
#![cfg_attr(not(feature = "gui"), allow(dead_code))]

#[cfg(not(any(feature = "gui", feature = "tui")))]
compile_error!("enable at least one frontend feature: `gui` and/or `tui`");

// --- Shared (frontend-agnostic) ---------------------------------------------
mod config;
mod files;
mod line_edit;
mod paths;
mod remote;
mod sftp;

// --- Frontends ----------------------------------------------------------------
#[cfg(feature = "gui")]
mod gui;
#[cfg(feature = "tui")]
mod tui;

use std::process::ExitCode;

/// Installed executable name of the graphical frontend.
pub const GUI_BIN: &str = "riso-gui";
/// Installed executable name of the terminal frontend.
pub const TUI_BIN: &str = "riso-tui";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Frontend {
    #[cfg(feature = "gui")]
    Gui,
    #[cfg(feature = "tui")]
    Tui,
}

/// Picks the frontend to run.
///
/// With a single frontend compiled in, that one is always used. When both are
/// compiled in, `--gui` / `--tui` win, then the executable name
/// (`riso-tui` → TUI), and the GUI is the default.
fn select_frontend(argv0: &str, args: &mut Vec<String>) -> Frontend {
    let mut forced = None;
    args.retain(|a| match a.as_str() {
        #[cfg(feature = "gui")]
        "--gui" => {
            forced = Some(Frontend::Gui);
            false
        }
        #[cfg(feature = "tui")]
        "--tui" => {
            forced = Some(Frontend::Tui);
            false
        }
        _ => true,
    });
    if let Some(f) = forced {
        return f;
    }

    #[cfg(all(feature = "gui", feature = "tui"))]
    {
        let exe = std::path::Path::new(argv0)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if exe == TUI_BIN { Frontend::Tui } else { Frontend::Gui }
    }
    #[cfg(not(all(feature = "gui", feature = "tui")))]
    {
        let _ = argv0;
        #[cfg(feature = "gui")]
        return Frontend::Gui;
        #[cfg(feature = "tui")]
        return Frontend::Tui;
    }
}

fn main() -> ExitCode {
    let mut argv = std::env::args();
    let argv0 = argv.next().unwrap_or_default();
    let mut args: Vec<String> = argv.collect();

    match select_frontend(&argv0, &mut args) {
        #[cfg(feature = "gui")]
        Frontend::Gui => gui::run(args).into(),
        #[cfg(feature = "tui")]
        Frontend::Tui => tui::run(args),
    }
}
