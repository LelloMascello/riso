//! Minimal 1px-stroke symbolic icons (VS Code Codicon geometry).

use std::fs;
use std::sync::Once;

use gtk::{gdk, glib};

static INIT: Once = Once::new();

const ICONS: &[(&str, &str)] = &[
    // Sidebar split layout
    (
        "vs-sidebar-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" fill-rule="evenodd" d="M2 2.5l.5-.5h11l.5.5v11l-.5.5h-11l-.5-.5v-11zM3 3v10h3V3H3zm4 0v10h6V3H7z"/></svg>"##,
    ),
    // Thin +
    (
        "vs-plus-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" d="M7.5 2h1v5.5H14v1H8.5V14h-1V8.5H2v-1h5.5V2z"/></svg>"##,
    ),
    // Closed folder outline
    (
        "vs-folder-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" fill-rule="evenodd" d="M1.5 2h4.29l1.5 1.5H14.5l.5.5v9.5l-.5.5h-13l-.5-.5v-11l.5-.5zM2 3v10h12V4.5H7.09l-1.5-1.5H2z"/></svg>"##,
    ),
    // Open folder outline
    (
        "vs-folder-open-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" fill-rule="evenodd" d="M1.5 2h4.29l1.5 1.5H13.5l.5.5v2h1l.47.66-1.5 6.5-.49.34H1.5l-.5-.5v-10.5l.5-.5zM2 3v7.38l1.06-4.01.48-.37H13V4.5H7.09l-1.5-1.5H2zm1.93 4l-1.31 5h10.47l1.27-5H3.93z"/></svg>"##,
    ),
    // File document outline
    (
        "vs-file-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" fill-rule="evenodd" d="M3.5 1.5h6.29L13 4.71V14l-.5.5h-9L3 14V2l.5-.5zM4 2.5v11h8V5.5H9.5L9 5V2.5H4zm6 .71V4.5h1.79L10 3.21z"/></svg>"##,
    ),
    // New file (+ document)
    (
        "vs-new-file-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" fill-rule="evenodd" d="M3.5 1.5h6.29L13 4.71V9h-1V5.5H9.5L9 5V2.5H4v11h4v1H3.5L3 14V2l.5-.5zm6.5 1.71V4.5h1.79L10 3.21zM12 10h1v2h2v1h-2v2h-1v-2h-2v-1h2v-2z"/></svg>"##,
    ),
    // New folder (+ folder)
    (
        "vs-new-folder-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" fill-rule="evenodd" d="M1.5 2h4.29l1.5 1.5H14.5l.5.5v5h-1V4.5H7.09l-1.5-1.5H2v10h6v1H1.5l-.5-.5v-11l.5-.5zM12 10h1v2h2v1h-2v2h-1v-2h-2v-1h2v-2z"/></svg>"##,
    ),
    // Up arrow
    (
        "vs-up-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" d="M8 2.79l4.35 4.36-.7.7L8.5 4.71V13.5h-1V4.71L4.35 7.85l-.7-.7L8 2.79z"/></svg>"##,
    ),
    // Refresh
    (
        "vs-refresh-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" d="M13.45 5.69A5.5 5.5 0 1 0 13.5 9h-1a4.5 4.5 0 1 1-.39-2.85L10 6.5v-1l3.08-.49.42.42.26 2.86-.99.09-.18-1.94-.14.25z"/></svg>"##,
    ),
    // Edit pencil
    (
        "vs-edit-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" fill-rule="evenodd" d="M11.85 2.15a.5.5 0 0 1 .71 0l1.29 1.29a.5.5 0 0 1 0 .71L5.21 12.79l-.35.15-2.5.56-.58-.58.56-2.5.15-.35 8.64-8.64-.28.72zM3.67 10.68l-.36 1.61 1.61-.36 7.86-7.86-1.25-1.25-7.86 7.86z"/></svg>"##,
    ),
    // Trash
    (
        "vs-trash-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" fill-rule="evenodd" d="M6 2h4l.5.5V4H14v1h-1v8.5l-.5.5h-9l-.5-.5V5H2V4h3.5V2.5L6 2zm.5 1v1h3V3h-3zM4 5v8h8V5H4zm2 1.5h1v5H6v-5zm3 0h1v5H9v-5z"/></svg>"##,
    ),
    // Upload arrow
    (
        "vs-upload-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" d="M8 2.29l3.85 3.86-.7.7L8.5 4.21V11h-1V4.21L4.85 6.85l-.7-.7L8 2.29zM3 12h1v1h8v-1h1v1.5l-.5.5h-9l-.5-.5V12z"/></svg>"##,
    ),
    // Right arrow
    (
        "vs-arrow-right-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" d="M13.21 8l-4.36 4.35-.7-.7 3.14-3.15H2.5v-1h8.79L8.15 4.35l.7-.7L13.21 8z"/></svg>"##,
    ),
    // Remote host
    (
        "vs-server-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" fill-rule="evenodd" d="M2.5 3h11l.5.5v3l-.5.5h-11l-.5-.5v-3l.5-.5zM3 4v2h10V4H3zm-.5 5h11l.5.5v3l-.5.5h-11l-.5-.5v-3l.5-.5zM3 10v2h10v-2H3zm8-5.5h1v1h-1v-1zm0 6h1v1h-1v-1z"/></svg>"##,
    ),
];

pub fn register(display: &gdk::Display) {
    INIT.call_once(|| {
        let base = glib::user_runtime_dir().join("riso").join("icons");
        let dir = base.join("hicolor").join("scalable").join("actions");
        if fs::create_dir_all(&dir).is_ok() {
            for (name, svg) in ICONS {
                let _ = fs::write(dir.join(format!("{name}.svg")), svg);
            }
            gtk::IconTheme::for_display(display).add_search_path(&base);
        }
    });
}
