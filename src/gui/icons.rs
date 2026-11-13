//! Minimal 1px-stroke symbolic icons.

use std::fs;
use std::sync::Once;

use gtk::{gdk, glib};

static INIT: Once = Once::new();

const ICONS: &[(&str, &str)] = &[
    // Hamburger menu (3 thin horizontal 1px bars)
    (
        "riso-menu-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" d="M2.5 3.5h11v1h-11v-1zm0 4h11v1h-11v-1zm0 4h11v1h-11v-1z"/></svg>"##,
    ),
    // Right-dock sidebar split layout
    (
        "riso-sidebar-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" fill-rule="evenodd" d="M2 2.5l.5-.5h11l.5.5v11l-.5.5h-11l-.5-.5v-11zM3 3v10h6V3H3zm7 0v10h3V3h-3z"/></svg>"##,
    ),
    // Chevron left (<)
    (
        "riso-chevron-left-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" d="M10.15 3.15l.7.7L6.71 8l4.14 4.15-.7.7L5.29 8l4.86-4.85z"/></svg>"##,
    ),
    // Chevron right (>)
    (
        "riso-chevron-right-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" d="M5.85 3.15L10.71 8l-4.86 4.85-.7-.7L9.29 8 5.15 3.85l.7-.7z"/></svg>"##,
    ),
    // Thin +
    (
        "riso-plus-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" d="M7.5 2.5h1v5H13.5v1h-5v5h-1v-5h-5v-1h5v-5z"/></svg>"##,
    ),
    // Thin × close
    (
        "riso-close-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" d="M4.65 3.95L8 7.29l3.35-3.34.7.7L8.71 8l3.34 3.35-.7.7L8 8.71l-3.35 3.34-.7-.7L7.29 8 3.95 4.65l.7-.7z"/></svg>"##,
    ),
    // Vertical 3-dots menu
    (
        "riso-dots-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" d="M8 2.75a1.15 1.15 0 1 1 0 2.3 1.15 1.15 0 0 1 0-2.3zm0 4.1a1.15 1.15 0 1 1 0 2.3 1.15 1.15 0 0 1 0-2.3zm0 4.1a1.15 1.15 0 1 1 0 2.3 1.15 1.15 0 0 1 0-2.3z"/></svg>"##,
    ),
    // Terminal square icon
    (
        "riso-terminal-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" fill-rule="evenodd" d="M2 2.5l.5-.5h11l.5.5v11l-.5.5h-11l-.5-.5v-11zM3 3v10h10V3H3zm1.85 2.65l2.85 2.85-2.85 2.85-.7-.7L6.29 8.5 4.15 6.35l.7-.7zM8 10h3.5v1H8v-1z"/></svg>"##,
    ),
    // Closed folder outline
    (
        "riso-folder-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" fill-rule="evenodd" d="M1.5 2h4.29l1.5 1.5H14.5l.5.5v9.5l-.5.5h-13l-.5-.5v-11l.5-.5zM2 3v10h12V4.5H7.09l-1.5-1.5H2z"/></svg>"##,
    ),
    // Open folder outline
    (
        "riso-folder-open-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" fill-rule="evenodd" d="M1.5 2h4.29l1.5 1.5H13.5l.5.5v2h1l.47.66-1.5 6.5-.49.34H1.5l-.5-.5v-10.5l.5-.5zM2 3v7.38l1.06-4.01.48-.37H13V4.5H7.09l-1.5-1.5H2zm1.93 4l-1.31 5h10.47l1.27-5H3.93z"/></svg>"##,
    ),
    // File document outline
    (
        "riso-file-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" fill-rule="evenodd" d="M3.5 1.5h6.29L13 4.71V14l-.5.5h-9L3 14V2l.5-.5zM4 2.5v11h8V5.5H9.5L9 5V2.5H4zm6 .71V4.5h1.79L10 3.21z"/></svg>"##,
    ),
    // New file (+ document)
    (
        "riso-new-file-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" fill-rule="evenodd" d="M3.5 1.5h6.29L13 4.71V9h-1V5.5H9.5L9 5V2.5H4v11h4v1H3.5L3 14V2l.5-.5zm6.5 1.71V4.5h1.79L10 3.21zM12 10h1v2h2v1h-2v2h-1v-2h-2v-1h2v-2z"/></svg>"##,
    ),
    // New folder (+ folder)
    (
        "riso-new-folder-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" fill-rule="evenodd" d="M1.5 2h4.29l1.5 1.5H14.5l.5.5v5h-1V4.5H7.09l-1.5-1.5H2v10h6v1H1.5l-.5-.5v-11l.5-.5zM12 10h1v2h2v1h-2v2h-1v-2h-2v-1h2v-2z"/></svg>"##,
    ),
    // Up arrow
    (
        "riso-up-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" d="M8 2.79l4.35 4.36-.7.7L8.5 4.71V13.5h-1V4.71L4.35 7.85l-.7-.7L8 2.79z"/></svg>"##,
    ),
    // Refresh
    (
        "riso-refresh-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" d="M13.45 5.69A5.5 5.5 0 1 0 13.5 9h-1a4.5 4.5 0 1 1-.39-2.85L10 6.5v-1l3.08-.49.42.42.26 2.86-.99.09-.18-1.94-.14.25z"/></svg>"##,
    ),
    // Upload arrow
    (
        "riso-upload-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" d="M8 2.29l3.85 3.86-.7.7L8.5 4.21V11h-1V4.21L4.85 6.85l-.7-.7L8 2.29zM3 12h1v1h8v-1h1v1.5l-.5.5h-9l-.5-.5V12z"/></svg>"##,
    ),
];

pub fn register(display: &gdk::Display) {
    INIT.call_once(|| {
        let candidates = [
            glib::user_runtime_dir().join("riso").join("icons"),
            std::env::temp_dir().join(format!("riso-icons-{}", std::process::id())),
        ];
        for base in candidates {
            let dir = base.join("hicolor").join("scalable").join("actions");
            if fs::create_dir_all(&dir).is_ok() {
                if let Ok(existing) = fs::read_dir(&dir) {
                    for entry in existing.flatten() {
                        let path = entry.path();
                        if path.extension().and_then(|s| s.to_str()) == Some("svg") {
                            let _ = fs::remove_file(path);
                        }
                    }
                }
                let mut wrote_any = false;
                for (name, svg) in ICONS {
                    if fs::write(dir.join(format!("{name}.svg")), svg).is_ok() {
                        wrote_any = true;
                    }
                }
                if wrote_any {
                    gtk::IconTheme::for_display(display).add_search_path(&base);
                    break;
                }
            }
        }
    });
}
