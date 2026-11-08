//! Adaptive GTK system theming, live CSS/config monitoring, and dynamic VTE
//! terminal color synchronization.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use vte4::prelude::*;

pub const STYLE_CSS: &str = r#"
/* Unified Top Bar */
.topbar {
    background-color: var(--headerbar-bg-color, var(--window-bg-color));
    color: var(--headerbar-fg-color, var(--window-fg-color));
    border-bottom: 1px solid var(--headerbar-border-color, var(--border-color, color-mix(in srgb, currentColor 15%, transparent)));
    min-height: 34px;
    padding: 0 2px;
}

.topbar:backdrop {
    background-color: var(--headerbar-backdrop-color, var(--window-bg-color));
}

.topbar-btn {
    min-width: 26px;
    min-height: 26px;
    padding: 0 4px;
    border-radius: 4px;
}

.session-tabbar,
.session-tabbar > revealer > box {
    background-color: transparent;
    background-image: none;
    box-shadow: none;
    border: none;
}

.session-tabbar tabbox {
    background-color: transparent;
    padding: 0;
    border: none;
}

.session-tabbar tab {
    background-color: transparent;
    color: color-mix(in srgb, var(--headerbar-fg-color, var(--window-fg-color, currentColor)) 70%, transparent);
    border-radius: 0;
    border-right: 1px solid var(--headerbar-border-color, var(--border-color, color-mix(in srgb, currentColor 15%, transparent)));
    min-height: 34px;
    padding: 0 10px;
    margin: 0;
    font-size: 12.5px;
    box-shadow: none;
}

.session-tabbar tab:hover {
    background-color: color-mix(in srgb, currentColor 7%, transparent);
    color: var(--headerbar-fg-color, var(--window-fg-color, inherit));
}

.session-tabbar tab:checked {
    background-color: var(--view-bg-color, var(--window-bg-color));
    color: var(--view-fg-color, var(--window-fg-color));
    box-shadow: inset 0 2px 0 0 var(--accent-bg-color, currentColor);
}

/* SFTP Explorer Sidebar */
.explorer-sidebar {
    background-color: var(--sidebar-bg-color, var(--window-bg-color));
    color: var(--sidebar-fg-color, var(--window-fg-color));
}

.explorer-sidebar:backdrop {
    background-color: var(--sidebar-backdrop-color, var(--window-bg-color));
}

.explorer-toolbar {
    padding: 4px 6px;
    border-bottom: 1px solid var(--sidebar-border-color, var(--border-color, color-mix(in srgb, currentColor 15%, transparent)));
    min-height: 30px;
}

.path-entry {
    min-height: 24px;
    padding: 0 6px;
    font-size: 12px;
    font-family: monospace;
}

.icon-btn {
    min-width: 22px;
    min-height: 22px;
    padding: 2px;
    border-radius: 4px;
}

.thin-progress,
.thin-progress trough {
    min-height: 2px;
    background-color: transparent;
    border-radius: 0;
}

.thin-progress progress {
    min-height: 2px;
    background-color: var(--accent-bg-color, currentColor);
    border-radius: 0;
}

.explorer-tree {
    background-color: transparent;
    color: inherit;
    font-size: 12.5px;
}

.explorer-tree > row {
    min-height: 22px;
    padding: 1px 6px;
    margin: 0;
    border-radius: 0;
}

.explorer-tree > row:hover {
    background-color: color-mix(in srgb, currentColor 7%, transparent);
}

.explorer-tree > row:selected {
    background-color: color-mix(in srgb, var(--accent-bg-color, currentColor) 24%, transparent);
    color: inherit;
}

.explorer-tree > row:selected:hover {
    background-color: color-mix(in srgb, var(--accent-bg-color, currentColor) 32%, transparent);
}

.explorer-tree-icon {
    opacity: 0.75;
}

.hidden-entry {
    opacity: 0.55;
}

paned > separator {
    background-color: var(--sidebar-border-color, var(--border-color, color-mix(in srgb, currentColor 15%, transparent)));
    min-width: 1px;
}

/* Terminal & Drop Overlay */
.terminal-box {
    background-color: var(--view-bg-color, var(--window-bg-color));
    color: var(--view-fg-color, var(--window-fg-color));
}

.session-terminal {
    color: var(--view-fg-color, var(--window-fg-color));
}

.drop-overlay {
    background-color: color-mix(in srgb, var(--window-bg-color) 78%, transparent);
}

.drop-card {
    background-color: var(--card-bg-color, var(--popover-bg-color));
    color: var(--card-fg-color, var(--popover-fg-color));
    border: 1px dashed var(--accent-color, currentColor);
    border-radius: 6px;
    padding: 20px 32px;
}

.drop-active {
    outline: 1px dashed var(--accent-color, currentColor);
    outline-offset: -2px;
}

/* 22px Status Bar */
.statusbar {
    background-color: var(--headerbar-bg-color, var(--window-bg-color));
    color: color-mix(in srgb, var(--headerbar-fg-color, var(--window-fg-color)) 72%, transparent);
    border-top: 1px solid var(--headerbar-border-color, var(--border-color, color-mix(in srgb, currentColor 15%, transparent)));
    padding: 0 10px;
    min-height: 22px;
    font-size: 11.5px;
}

.statusbar:backdrop {
    background-color: var(--headerbar-backdrop-color, var(--window-bg-color));
}

.status-dot {
    font-size: 9px;
}

.status-dot.ok {
    color: var(--success-color, #2ec27e);
}

.status-dot.wait {
    color: var(--warning-color, #e5a50a);
}

.status-dot.err {
    color: var(--error-color, #e01b24);
}

.status-btn {
    min-height: 18px;
    padding: 0 6px;
    border-radius: 3px;
    font-size: 11px;
    color: var(--accent-color, currentColor);
}

.sync-badge {
    color: var(--success-color, #2ec27e);
    font-family: monospace;
    font-size: 11px;
}

.muted-label {
    opacity: 0.6;
    font-size: 11.5px;
}

/* Inline Terminal SSH Host Autocompletion */
popover.suggest-popover > contents {
    padding: 2px 0;
}

.suggest-list {
    background-color: transparent;
    font-family: monospace;
    font-size: 12px;
}

.suggest-list > row {
    padding: 3px 10px;
    min-height: 22px;
    border-radius: 0;
}

.suggest-list > row:hover {
    background-color: color-mix(in srgb, currentColor 7%, transparent);
}

.suggest-list > row:selected {
    background-color: color-mix(in srgb, var(--accent-bg-color, currentColor) 24%, transparent);
    color: inherit;
}

/* Minimal "Apri con…" Quick-Pick Dialog */
.picker-dialog {
    padding: 8px;
}

.picker-input {
    min-height: 28px;
    padding: 0 8px;
    margin-bottom: 6px;
    font-size: 12.5px;
}

.picker-list {
    background-color: transparent;
}

.picker-list > row {
    border-radius: 4px;
    padding: 0;
}

.picker-list > row:hover {
    background-color: color-mix(in srgb, currentColor 7%, transparent);
}

.picker-list > row:selected {
    background-color: color-mix(in srgb, var(--accent-bg-color, currentColor) 24%, transparent);
    color: inherit;
}

.picker-row {
    padding: 5px 8px;
    font-size: 12.5px;
}
"#;

/// High-priority internal CSS variable probes so user selectors in `gtk.css`
/// cannot shadow `probe_on_terminal` when reading resolved theme variables.
const PROBE_CSS: &str = r#"
.riso-probe-view-bg,
.session-terminal.riso-probe-view-bg { color: var(--view-bg-color, rgba(1, 2, 3, 0.01)); }
.riso-probe-win-bg,
.session-terminal.riso-probe-win-bg { color: var(--window-bg-color, rgba(1, 2, 3, 0.01)); }
.riso-probe-view-fg,
.session-terminal.riso-probe-view-fg { color: var(--view-fg-color, rgba(1, 2, 3, 0.01)); }
.riso-probe-win-fg,
.session-terminal.riso-probe-win-fg { color: var(--window-fg-color, rgba(1, 2, 3, 0.01)); }
.riso-probe-accent-bg,
.session-terminal.riso-probe-accent-bg { color: var(--accent-bg-color, rgba(1, 2, 3, 0.01)); }
.riso-probe-red,
.session-terminal.riso-probe-red { color: var(--red-3, rgba(1, 2, 3, 0.01)); }
.riso-probe-green,
.session-terminal.riso-probe-green { color: var(--green-3, rgba(1, 2, 3, 0.01)); }
.riso-probe-yellow,
.session-terminal.riso-probe-yellow { color: var(--yellow-3, rgba(1, 2, 3, 0.01)); }
.riso-probe-blue,
.session-terminal.riso-probe-blue { color: var(--blue-3, rgba(1, 2, 3, 0.01)); }
.riso-probe-purple,
.session-terminal.riso-probe-purple { color: var(--purple-3, rgba(1, 2, 3, 0.01)); }
"#;

const ADWAITA_COLOR_NAMES: &[&str] = &[
    "window_bg_color",
    "window_fg_color",
    "view_bg_color",
    "view_fg_color",
    "headerbar_bg_color",
    "headerbar_fg_color",
    "headerbar_backdrop_color",
    "headerbar_border_color",
    "sidebar_bg_color",
    "sidebar_fg_color",
    "sidebar_backdrop_color",
    "sidebar_border_color",
    "secondary_sidebar_bg_color",
    "secondary_sidebar_fg_color",
    "card_bg_color",
    "card_fg_color",
    "dialog_bg_color",
    "dialog_fg_color",
    "popover_bg_color",
    "popover_fg_color",
    "accent_bg_color",
    "accent_fg_color",
    "accent_color",
    "success_color",
    "warning_color",
    "error_color",
    "destructive_color",
    "borders",
];

thread_local! {
    static MANAGER: RefCell<Option<Rc<ThemeManager>>> = const { RefCell::new(None) };
}

#[derive(Default, Clone, PartialEq, Eq)]
struct GSettingsSnapshot {
    color_scheme: String,
    gtk_theme: String,
    accent_color: String,
}

#[derive(Default, Clone)]
struct CssInspection {
    define_colors: HashMap<String, String>,
    custom_props: HashMap<String, String>,
}

#[derive(Clone)]
enum WatchFilter {
    AllThemeFiles,
    ExactNames(HashSet<String>),
}

struct ThemeManager {
    display: gdk::Display,
    gtk4_dir: PathBuf,
    gtk3_dir: PathBuf,
    dconf_dir: PathBuf,
    gtk_css_path: PathBuf,
    startup_inspection: CssInspection,

    configured_theme_provider: gtk::CssProvider,
    reset_provider: gtk::CssProvider,
    live_user_provider: gtk::CssProvider,
    bridge_provider: gtk::CssProvider,

    gsettings: Option<gio::Settings>,
    last_gs_snapshot: RefCell<GSettingsSnapshot>,
    active_theme_css: RefCell<Option<PathBuf>>,
    base_monitors: RefCell<Vec<gio::FileMonitor>>,
    dynamic_monitors: RefCell<Vec<gio::FileMonitor>>,
    debounce: RefCell<Option<glib::SourceId>>,
    terminal_refresh_queued: Cell<bool>,
    terminals: RefCell<Vec<glib::WeakRef<vte4::Terminal>>>,
}

/// Initializes adaptive GTK / libadwaita theming and starts real-time watchers
/// on user GTK CSS files (`~/.config/gtk-4.0/gtk.css`, `@import`s, chained
/// symlinks), `settings.ini`, `gsettings` / `dconf`, `adw::StyleManager`, and
/// `gtk::Settings`.
pub fn init(app: &adw::Application, display: &gdk::Display) {
    let configured_theme_provider = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(
        display,
        &configured_theme_provider,
        gtk::STYLE_PROVIDER_PRIORITY_THEME + 2,
    );

    let app_provider = gtk::CssProvider::new();
    app_provider.load_from_string(STYLE_CSS);
    gtk::style_context_add_provider_for_display(
        display,
        &app_provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // Registered at PRIORITY_USER (after GTK's internal startup provider) so
    // any `@define-color` or `:root { --* }` entries removed from `gtk.css` at
    // runtime can be cleanly reset without requiring a restart.
    let reset_provider = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(
        display,
        &reset_provider,
        gtk::STYLE_PROVIDER_PRIORITY_USER,
    );

    // Registered above PRIORITY_USER so runtime edits/replacements of
    // `~/.config/gtk-4.0/gtk.css` (and its `@import`s) take effect immediately.
    let live_user_provider = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(
        display,
        &live_user_provider,
        gtk::STYLE_PROVIDER_PRIORITY_USER + 1,
    );

    // Bridges `@define-color` and legacy `theme_*` variables onto Adwaita CSS
    // custom properties (`--*`) when not already explicitly set on `:root`.
    let bridge_provider = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(
        display,
        &bridge_provider,
        gtk::STYLE_PROVIDER_PRIORITY_USER + 2,
    );

    let probe_provider = gtk::CssProvider::new();
    probe_provider.load_from_string(PROBE_CSS);
    gtk::style_context_add_provider_for_display(
        display,
        &probe_provider,
        gtk::STYLE_PROVIDER_PRIORITY_USER + 100,
    );

    let cfg = glib::user_config_dir();
    let gtk4_dir = cfg.join("gtk-4.0");
    let gtk3_dir = cfg.join("gtk-3.0");
    let dconf_dir = cfg.join("dconf");
    let gtk_css_path = gtk4_dir.join("gtk.css");
    let startup_inspection = inspect_css_tree(&gtk_css_path);
    let gsettings = open_interface_gsettings();

    let manager = Rc::new(ThemeManager {
        display: display.clone(),
        gtk4_dir,
        gtk3_dir,
        dconf_dir,
        gtk_css_path,
        startup_inspection,
        configured_theme_provider,
        reset_provider,
        live_user_provider,
        bridge_provider,
        gsettings,
        last_gs_snapshot: RefCell::default(),
        active_theme_css: RefCell::default(),
        base_monitors: RefCell::default(),
        dynamic_monitors: RefCell::default(),
        debounce: RefCell::default(),
        terminal_refresh_queued: Cell::new(false),
        terminals: RefCell::default(),
    });

    // Connect watchers BEFORE reading GSettings keys so GLib subscribes to the
    // dconf backend immediately.
    manager.setup_watchers();
    manager.sync_color_scheme();
    manager.reload_all_css();
    manager.refresh_dynamic_monitors();

    MANAGER.with(|slot| {
        *slot.borrow_mut() = Some(manager);
    });

    app.connect_shutdown(|_| {
        MANAGER.with(|slot| {
            if let Some(mgr) = slot.borrow_mut().take() {
                if let Some(id) = mgr.debounce.borrow_mut().take() {
                    id.remove();
                }
                mgr.base_monitors.borrow_mut().clear();
                mgr.dynamic_monitors.borrow_mut().clear();
                mgr.terminals.borrow_mut().clear();
            }
        });
    });
}

/// Binds a `vte4::Terminal` to the live system GTK theme colors so that
/// background, foreground, cursor, selection highlight, and the 16-color ANSI
/// palette update in real time whenever the system GTK theme changes.
pub fn bind_terminal(term: &vte4::Terminal) {
    apply_terminal_colors(term);

    MANAGER.with(|slot| {
        if let Some(mgr) = slot.borrow().as_ref() {
            mgr.register_terminal(term);
        }
    });

    term.connect_realize(apply_terminal_colors);
    term.connect_map(apply_terminal_colors);
    term.connect_root_notify(|t| {
        if t.root().is_some() {
            apply_terminal_colors(t);
        }
    });
}

impl ThemeManager {
    fn register_terminal(&self, term: &vte4::Terminal) {
        let mut list = self.terminals.borrow_mut();
        list.retain(|w| w.upgrade().is_some());
        list.push(term.downgrade());
    }

    fn setup_watchers(self: &Rc<Self>) {
        // 1. Watch `org.gnome.desktop.interface` in dconf/gsettings FIRST and
        //    immediately read the watched keys to activate backend subscription.
        if let Some(gs) = &self.gsettings {
            let weak = Rc::downgrade(self);
            gs.connect_changed(None, move |_, key| {
                if matches!(key, "color-scheme" | "gtk-theme" | "accent-color")
                    && let Some(this) = weak.upgrade()
                {
                    *this.last_gs_snapshot.borrow_mut() = snapshot_gsettings(this.gsettings.as_ref());
                    this.schedule_reload();
                }
            });
            *self.last_gs_snapshot.borrow_mut() = snapshot_gsettings(Some(gs));
        }

        // 2. Watch `~/.config` (for creation or symlink swap of `gtk-4.0`,
        //    `gtk-3.0`, or `dconf`) plus the individual config directories.
        let mut base = Vec::new();
        let cfg_dir = glib::user_config_dir();
        if let Some(m) = watch_dir(&cfg_dir, {
            let weak = Rc::downgrade(self);
            move |file, other| {
                if (is_watched_config_subdir(file) || other.is_some_and(is_watched_config_subdir))
                    && let Some(this) = weak.upgrade()
                {
                    this.rebind_config_dir_monitors();
                    this.schedule_reload();
                }
            }
        }) {
            base.push(m);
        }
        *self.base_monitors.borrow_mut() = base;
        self.rebind_config_dir_monitors();

        // 3. Watch `adw::StyleManager` and `gtk::Settings` notifications.
        let style_manager = adw::StyleManager::default();

        let weak = Rc::downgrade(self);
        style_manager.connect_dark_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                this.reload_all_css();
                this.queue_terminal_refresh();
            }
        });

        let weak = Rc::downgrade(self);
        style_manager.connect_accent_color_rgba_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                this.reload_all_css();
                this.queue_terminal_refresh();
            }
        });

        let weak = Rc::downgrade(self);
        style_manager.connect_high_contrast_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                this.queue_terminal_refresh();
            }
        });

        let settings = gtk::Settings::for_display(&self.display);
        let weak = Rc::downgrade(self);
        settings.connect_gtk_theme_name_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                this.schedule_reload();
            }
        });

        let weak = Rc::downgrade(self);
        settings.connect_gtk_application_prefer_dark_theme_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                this.schedule_reload();
            }
        });
    }

    fn rebind_config_dir_monitors(self: &Rc<Self>) {
        let mut base = self.base_monitors.borrow_mut();
        base.truncate(1); // Keep root `~/.config` monitor at index 0

        for dir in [&self.gtk4_dir, &self.gtk3_dir] {
            let weak = Rc::downgrade(self);
            if let Some(m) = watch_dir(dir, move |file, other| {
                if (is_theme_dir_event_file(file) || other.is_some_and(is_theme_dir_event_file))
                    && let Some(this) = weak.upgrade()
                {
                    this.schedule_reload();
                }
            }) {
                base.push(m);
            }
        }

        if self.gsettings.is_some() {
            let weak = Rc::downgrade(self);
            if let Some(m) = watch_dir(&self.dconf_dir, move |file, other| {
                if (is_dconf_user_db(file) || other.is_some_and(is_dconf_user_db))
                    && let Some(this) = weak.upgrade()
                {
                    let now = snapshot_gsettings(this.gsettings.as_ref());
                    let changed = {
                        let mut prev = this.last_gs_snapshot.borrow_mut();
                        if *prev != now {
                            *prev = now;
                            true
                        } else {
                            false
                        }
                    };
                    if changed {
                        this.schedule_reload();
                    }
                }
            }) {
                base.push(m);
            }
        }
    }

    fn refresh_dynamic_monitors(self: &Rc<Self>) {
        let active_theme_css = self.active_theme_css.borrow().clone();
        let mut targets = collect_watch_targets(
            &self.gtk4_dir,
            &self.gtk_css_path,
            active_theme_css.as_deref(),
        );
        targets.remove(&self.gtk4_dir);
        targets.remove(&self.gtk3_dir);
        targets.remove(&self.dconf_dir);

        let mut monitors = Vec::new();
        for (dir, filter) in targets {
            let weak = Rc::downgrade(self);
            if let Some(m) = watch_dir(&dir, move |file, other| {
                let matched = match &filter {
                    WatchFilter::AllThemeFiles => {
                        is_theme_dir_event_file(file) || other.is_some_and(is_theme_dir_event_file)
                    }
                    WatchFilter::ExactNames(names) => {
                        matches_basename_set(file, names)
                            || other.is_some_and(|o| matches_basename_set(o, names))
                    }
                };
                if matched && let Some(this) = weak.upgrade() {
                    this.schedule_reload();
                }
            }) {
                monitors.push(m);
            }
        }
        *self.dynamic_monitors.borrow_mut() = monitors;
    }

    fn schedule_reload(self: &Rc<Self>) {
        if let Some(id) = self.debounce.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(80), move || {
            let Some(this) = weak.upgrade() else { return };
            this.debounce.borrow_mut().take();
            this.rebind_config_dir_monitors();
            this.sync_color_scheme();
            this.reload_all_css();
            this.refresh_dynamic_monitors();
            this.queue_terminal_refresh();
        });
        *self.debounce.borrow_mut() = Some(id);
    }

    fn sync_color_scheme(&self) {
        let style_manager = adw::StyleManager::default();
        let user_inspection = inspect_css_tree(&self.gtk_css_path);

        // 1. If `~/.config/gtk-4.0/gtk.css` explicitly defines a literal light or
        //    dark window/view background, align `StyleManager` with it so built-in
        //    libadwaita borders/overlays match the active `gtk.css` palette.
        if let Some(dark_css) = inspection_bg_prefers_dark(&user_inspection) {
            style_manager.set_color_scheme(if dark_css {
                adw::ColorScheme::ForceDark
            } else {
                adw::ColorScheme::ForceLight
            });
            return;
        }

        // 2. When a system settings portal is active, `ColorScheme::Default`
        //    follows portal light/dark changes automatically; only apply
        //    `PreferDark` if no explicit `color-scheme` key is set and a dark
        //    GTK theme/INI override is configured.
        let gs_explicit = gsettings_explicit_color_scheme(self.gsettings.as_ref());
        if style_manager.system_supports_color_schemes() {
            let theme_override_dark = if gs_explicit.is_some() {
                None
            } else if is_gnome_desktop() {
                gsettings_theme_prefers_dark(self.gsettings.as_ref()).or_else(env_theme_prefers_dark)
            } else {
                ini_prefers_dark(&self.gtk4_dir)
                    .or_else(|| gsettings_theme_prefers_dark(self.gsettings.as_ref()))
                    .or_else(|| ini_prefers_dark(&self.gtk3_dir))
                    .or_else(|| gtk_settings_prefers_dark(&self.display))
                    .or_else(env_theme_prefers_dark)
            };
            style_manager.set_color_scheme(if theme_override_dark == Some(true) {
                adw::ColorScheme::PreferDark
            } else {
                adw::ColorScheme::Default
            });
            return;
        }

        // 3. Fallback when no `org.freedesktop.appearance` portal is present.
        let prefer_dark = gs_explicit
            .or_else(|| ini_prefers_dark(&self.gtk4_dir))
            .or_else(|| gsettings_theme_prefers_dark(self.gsettings.as_ref()))
            .or_else(|| ini_prefers_dark(&self.gtk3_dir))
            .or_else(|| gtk_settings_prefers_dark(&self.display))
            .or_else(env_theme_prefers_dark)
            .unwrap_or(false);

        style_manager.set_color_scheme(if prefer_dark {
            adw::ColorScheme::PreferDark
        } else {
            adw::ColorScheme::Default
        });
    }

    fn reload_all_css(&self) {
        let style_manager = adw::StyleManager::default();
        let dark = style_manager.is_dark();
        let accent = style_manager.accent_color_rgba();

        // 1. Configured named GTK4 theme palette (`GTK_THEME`, `settings.ini`, `gsettings`)
        let theme_css_path = self
            .active_configured_theme_name()
            .and_then(|name| resolve_named_gtk4_theme_css(&name, dark));
        *self.active_theme_css.borrow_mut() = theme_css_path.clone();

        let configured_inspection = theme_css_path
            .as_deref()
            .map(inspect_css_tree)
            .unwrap_or_default();
        let mut configured_css = build_extracted_palette_css(&configured_inspection);

        if !style_manager.is_system_supports_accent_colors()
            && !configured_inspection
                .define_colors
                .contains_key("accent_bg_color")
            && !configured_inspection
                .custom_props
                .contains_key("--accent-bg-color")
            && let Some(hex) = gsettings_accent_hex(self.gsettings.as_ref())
        {
            configured_css.push_str(&format!(
                "@define-color accent_bg_color {hex};\n\
                 @define-color accent_color {hex};\n\
                 :root {{ --accent-bg-color: {hex}; --accent-color: {hex}; }}\n"
            ));
        }
        load_validated_css(&self.configured_theme_provider, &configured_css);

        // 2. Live `~/.config/gtk-4.0/gtk.css` reload
        if self.gtk_css_path.exists() {
            let target = fs::canonicalize(&self.gtk_css_path)
                .unwrap_or_else(|_| self.gtk_css_path.clone());
            self.live_user_provider.load_from_path(&target);
        } else {
            self.live_user_provider.load_from_string("");
        }

        // 3. Bridge `@define-color` & legacy `theme_*` onto Adwaita `:root` `--*`
        let user_inspection = inspect_css_tree(&self.gtk_css_path);
        let bridge_css = build_bridge_css(&user_inspection);
        load_validated_css(&self.bridge_provider, &bridge_css);

        // 4. Reset any `@define-color` or `:root { --* }` entries that existed at
        //    startup in GTK's internal `PRIORITY_USER` provider if now removed.
        let reset_css = build_reset_css(
            &self.startup_inspection,
            &user_inspection,
            &configured_inspection,
            dark,
            &accent,
        );
        load_validated_css(&self.reset_provider, &reset_css);
    }

    fn active_configured_theme_name(&self) -> Option<String> {
        if let Ok(env_theme) = std::env::var("GTK_THEME") {
            let trimmed = env_theme.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        if let Some(name) = ini_theme_name(&self.gtk4_dir) {
            return Some(name);
        }
        if let Some(name) = gsettings_theme_name(self.gsettings.as_ref()) {
            return Some(name);
        }
        if let Some(name) = ini_theme_name(&self.gtk3_dir) {
            return Some(name);
        }
        gtk_settings_theme_name(&self.display)
    }

    fn queue_terminal_refresh(self: &Rc<Self>) {
        // Immediate pass so visible terminals update without waiting a frame.
        self.refresh_all_terminals();

        if self.terminal_refresh_queued.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            let Some(this) = weak.upgrade() else { return };
            this.terminal_refresh_queued.set(false);
            this.refresh_all_terminals();

            let weak2 = Rc::downgrade(&this);
            glib::timeout_add_local_once(Duration::from_millis(60), move || {
                if let Some(this) = weak2.upgrade() {
                    this.refresh_all_terminals();
                }
            });
        });
    }

    fn refresh_all_terminals(&self) {
        let live: Vec<vte4::Terminal> = {
            let mut list = self.terminals.borrow_mut();
            let mut out = Vec::with_capacity(list.len());
            list.retain(|w| {
                if let Some(t) = w.upgrade() {
                    out.push(t);
                    true
                } else {
                    false
                }
            });
            out
        };
        for term in &live {
            apply_terminal_colors(term);
            term.queue_draw();
        }
    }
}

// ---------------------------------------------------------------------------
// File, Symlink Chain, GSettings & CSS Discovery Helpers
// ---------------------------------------------------------------------------

fn watch_dir(
    dir: &Path,
    on_event: impl Fn(&gio::File, Option<&gio::File>) + 'static,
) -> Option<gio::FileMonitor> {
    let gfile = gio::File::for_path(dir);
    let monitor = gfile
        .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, None::<&gio::Cancellable>)
        .ok()?;
    monitor.connect_changed(move |_, file, other, event| {
        use gio::FileMonitorEvent as E;
        if matches!(
            event,
            E::Changed
                | E::ChangesDoneHint
                | E::Created
                | E::Deleted
                | E::MovedIn
                | E::MovedOut
                | E::Renamed
                | E::AttributeChanged
        ) {
            on_event(file, other);
        }
    });
    Some(monitor)
}

fn is_watched_config_subdir(file: &gio::File) -> bool {
    file.basename()
        .and_then(|n| n.to_str().map(|s| matches!(s, "gtk-4.0" | "gtk-3.0" | "dconf")))
        .unwrap_or(false)
}

fn is_dconf_user_db(file: &gio::File) -> bool {
    file.basename()
        .and_then(|n| n.to_str().map(|s| s == "user"))
        .unwrap_or(false)
}

fn is_theme_dir_event_file(file: &gio::File) -> bool {
    let Some(name) = file.basename() else {
        return false;
    };
    let s = name.to_string_lossy();
    // Ignore editor swap/backup lock files; allow all CSS, INI, symlink, and asset updates.
    !(s.ends_with(".swp") || s.ends_with(".swo") || s.ends_with('~') || s.ends_with(".lock"))
}

fn matches_basename_set(file: &gio::File, names: &HashSet<String>) -> bool {
    file.basename()
        .and_then(|b| b.to_str().map(|s| names.contains(s)))
        .unwrap_or(false)
}

fn open_interface_gsettings() -> Option<gio::Settings> {
    let source = gio::SettingsSchemaSource::default()?;
    let _schema = source.lookup("org.gnome.desktop.interface", true)?;
    Some(gio::Settings::new("org.gnome.desktop.interface"))
}

fn snapshot_gsettings(gs: Option<&gio::Settings>) -> GSettingsSnapshot {
    let Some(gs) = gs else {
        return GSettingsSnapshot::default();
    };
    let Some(schema) = gs.settings_schema() else {
        return GSettingsSnapshot::default();
    };
    let read_str = |key: &str| -> String {
        if schema.has_key(key) {
            gs.string(key).to_string()
        } else {
            String::new()
        }
    };
    GSettingsSnapshot {
        color_scheme: read_str("color-scheme"),
        gtk_theme: read_str("gtk-theme"),
        accent_color: read_str("accent-color"),
    }
}

fn gsettings_explicit_color_scheme(gs: Option<&gio::Settings>) -> Option<bool> {
    let gs = gs?;
    let schema = gs.settings_schema()?;
    if !schema.has_key("color-scheme") {
        return None;
    }
    match gs.string("color-scheme").as_str() {
        "prefer-dark" => Some(true),
        "prefer-light" => Some(false),
        _ => None,
    }
}

fn gsettings_theme_name(gs: Option<&gio::Settings>) -> Option<String> {
    let gs = gs?;
    let schema = gs.settings_schema()?;
    if !schema.has_key("gtk-theme") {
        return None;
    }
    let val = gs.string("gtk-theme");
    let trimmed = val.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn gsettings_theme_prefers_dark(gs: Option<&gio::Settings>) -> Option<bool> {
    let name = gsettings_theme_name(gs)?;
    theme_name_prefers_dark(&name)
}

fn gsettings_accent_hex(gs: Option<&gio::Settings>) -> Option<&'static str> {
    let gs = gs?;
    let schema = gs.settings_schema()?;
    if !schema.has_key("accent-color") {
        return None;
    }
    match gs.string("accent-color").as_str() {
        "blue" => Some("#3584e4"),
        "teal" => Some("#2190a4"),
        "green" => Some("#3a944a"),
        "yellow" => Some("#c88800"),
        "orange" => Some("#ed5b00"),
        "red" => Some("#e62d42"),
        "pink" => Some("#d56199"),
        "purple" => Some("#9141ac"),
        "slate" => Some("#6f8396"),
        _ => None,
    }
}

fn ini_theme_name(dir: &Path) -> Option<String> {
    let ini = dir.join("settings.ini");
    let kf = glib::KeyFile::new();
    kf.load_from_file(&ini, glib::KeyFileFlags::NONE).ok()?;
    let val = kf.string("Settings", "gtk-theme-name").ok()?;
    let trimmed = val.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn ini_prefers_dark(dir: &Path) -> Option<bool> {
    let ini = dir.join("settings.ini");
    let kf = glib::KeyFile::new();
    kf.load_from_file(&ini, glib::KeyFileFlags::NONE).ok()?;
    if let Ok(prefer_dark) = kf.boolean("Settings", "gtk-application-prefer-dark-theme") {
        return Some(prefer_dark);
    }
    if let Ok(theme_name) = kf.string("Settings", "gtk-theme-name") {
        return theme_name_prefers_dark(theme_name.as_str());
    }
    None
}

fn gtk_settings_theme_name(display: &gdk::Display) -> Option<String> {
    let s = gtk::Settings::for_display(display);
    let name = s.gtk_theme_name()?;
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("Adwaita-empty") {
        return None;
    }
    Some(trimmed.to_string())
}

fn gtk_settings_prefers_dark(display: &gdk::Display) -> Option<bool> {
    let s = gtk::Settings::for_display(display);
    if s.is_gtk_application_prefer_dark_theme() {
        return Some(true);
    }
    let name = gtk_settings_theme_name(display)?;
    theme_name_prefers_dark(&name)
}

fn env_theme_prefers_dark() -> Option<bool> {
    let env_theme = std::env::var("GTK_THEME").ok()?;
    theme_name_prefers_dark(&env_theme)
}

fn is_gnome_desktop() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|v| v.to_ascii_uppercase().split(':').any(|p| p == "GNOME"))
        .unwrap_or(false)
}

fn theme_name_prefers_dark(name: &str) -> Option<bool> {
    let low = name.trim().to_ascii_lowercase();
    if low.is_empty() {
        return None;
    }
    if low.ends_with(":dark") || low.contains("-dark") || low.ends_with("_dark") {
        Some(true)
    } else if low.ends_with(":light") || low.contains("-light") || low.ends_with("_light") {
        Some(false)
    } else {
        None
    }
}

fn is_builtin_adwaita_theme(name: &str) -> bool {
    let base = name.split(':').next().unwrap_or(name).trim();
    matches!(
        base.to_ascii_lowercase().as_str(),
        "" | "adwaita"
            | "adwaita-dark"
            | "adwaita-light"
            | "adwaita-empty"
            | "default"
            | "highcontrast"
            | "highcontrastinverse"
    )
}

fn resolve_named_gtk4_theme_css(raw_name: &str, dark: bool) -> Option<PathBuf> {
    let raw = raw_name.trim();
    if is_builtin_adwaita_theme(raw) {
        return None;
    }
    let (name, variant) = match raw.split_once(':') {
        Some((n, v)) => (n.trim(), Some(v.trim())),
        None => (raw, None),
    };
    if is_builtin_adwaita_theme(name) {
        return None;
    }
    let want_dark = variant
        .and_then(|v| {
            if v.eq_ignore_ascii_case("dark") {
                Some(true)
            } else if v.eq_ignore_ascii_case("light") {
                Some(false)
            } else {
                None
            }
        })
        .unwrap_or(dark);

    let mut search_roots = vec![
        glib::user_data_dir().join("themes"),
        glib::home_dir().join(".themes"),
    ];
    for sys in glib::system_data_dirs() {
        search_roots.push(sys.join("themes"));
    }

    for root in &search_roots {
        if want_dark {
            let dark_css = root.join(name).join("gtk-4.0").join("gtk-dark.css");
            if dark_css.exists() {
                return Some(dark_css);
            }
            if !name.to_ascii_lowercase().contains("dark") {
                for suffix in ["-Dark", "-dark"] {
                    let alt = root
                        .join(format!("{name}{suffix}"))
                        .join("gtk-4.0")
                        .join("gtk.css");
                    if alt.exists() {
                        return Some(alt);
                    }
                }
            }
        }
        let css = root.join(name).join("gtk-4.0").join("gtk.css");
        if css.exists() {
            return Some(css);
        }
    }
    None
}

fn add_watch_target(
    targets: &mut HashMap<PathBuf, WatchFilter>,
    dir: PathBuf,
    filter: WatchFilter,
) {
    match targets.get_mut(&dir) {
        None => {
            targets.insert(dir, filter);
        }
        Some(existing) => match (existing, filter) {
            (WatchFilter::AllThemeFiles, _) => {}
            (slot @ WatchFilter::ExactNames(_), WatchFilter::AllThemeFiles) => {
                *slot = WatchFilter::AllThemeFiles;
            }
            (WatchFilter::ExactNames(set), WatchFilter::ExactNames(more)) => {
                set.extend(more);
            }
        },
    }
}

fn collect_watch_targets(
    gtk4_dir: &Path,
    gtk_css_path: &Path,
    active_theme_css: Option<&Path>,
) -> HashMap<PathBuf, WatchFilter> {
    let mut targets = HashMap::new();

    walk_symlink_chain(gtk4_dir, 0, &mut targets);
    walk_symlink_chain(gtk_css_path, 0, &mut targets);

    if let Ok(canon_dir) = fs::canonicalize(gtk4_dir) {
        add_watch_target(&mut targets, canon_dir, WatchFilter::AllThemeFiles);
    }
    if let Ok(entries) = fs::read_dir(gtk4_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            walk_symlink_chain(&p, 0, &mut targets);
            if let Ok(canon) = fs::canonicalize(&p)
                && let Some(parent) = canon.parent()
            {
                add_watch_target(
                    &mut targets,
                    parent.to_path_buf(),
                    WatchFilter::AllThemeFiles,
                );
            }
        }
    }

    let mut visited = HashSet::new();
    collect_import_watch_targets(gtk_css_path, 0, &mut visited, &mut targets);

    if let Some(theme_css) = active_theme_css {
        walk_symlink_chain(theme_css, 0, &mut targets);
        collect_import_watch_targets(theme_css, 0, &mut visited, &mut targets);
    }

    targets
}

/// Walks `path` component-by-component like POSIX path lookup so intermediate
/// directory symlinks (e.g. `~/.config/omarchy/current/theme/gtk.css`) register
/// exact-name watches on their parent directories (`~/.config/omarchy` watching
/// `"current"`).
fn walk_symlink_chain(
    path: &Path,
    hops: usize,
    targets: &mut HashMap<PathBuf, WatchFilter>,
) {
    if hops > 12 {
        return;
    }
    let mut cur = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => cur.push(p.as_os_str()),
            Component::RootDir => cur.push(Component::RootDir.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if let Ok(canon) = fs::canonicalize(&cur) {
                    cur = canon;
                }
                cur.pop();
            }
            Component::Normal(part) => {
                let parent = cur.clone();
                cur.push(part);
                if let Ok(meta) = fs::symlink_metadata(&cur)
                    && meta.file_type().is_symlink()
                {
                    if !parent.as_os_str().is_empty()
                        && let Some(name) = part.to_str()
                    {
                        let mut set = HashSet::new();
                        set.insert(name.to_string());
                        add_watch_target(targets, parent.clone(), WatchFilter::ExactNames(set));
                    }
                    if let Ok(link_target) = fs::read_link(&cur) {
                        let resolved = if link_target.is_absolute() {
                            link_target
                        } else {
                            parent.join(link_target)
                        };
                        walk_symlink_chain(&resolved, hops + 1, targets);
                        cur = resolved;
                    }
                }
            }
        }
    }
}

fn collect_import_watch_targets(
    css_file: &Path,
    depth: usize,
    visited: &mut HashSet<PathBuf>,
    targets: &mut HashMap<PathBuf, WatchFilter>,
) {
    if depth > 4 {
        return;
    }
    walk_symlink_chain(css_file, 0, targets);
    let real = fs::canonicalize(css_file).unwrap_or_else(|_| css_file.to_path_buf());
    if !visited.insert(real.clone()) {
        return;
    }
    if let Some(parent) = real.parent() {
        add_watch_target(targets, parent.to_path_buf(), WatchFilter::AllThemeFiles);
    }
    let Ok(text) = fs::read_to_string(&real).or_else(|_| fs::read_to_string(css_file)) else {
        return;
    };
    let base_dir = real
        .parent()
        .or_else(|| css_file.parent())
        .unwrap_or_else(|| Path::new("."));
    for imported in parse_css_imports(&text, base_dir) {
        if let Some(parent) = imported.parent() {
            add_watch_target(targets, parent.to_path_buf(), WatchFilter::AllThemeFiles);
        }
        collect_import_watch_targets(&imported, depth + 1, visited, targets);
    }
}

fn parse_css_imports(css: &str, base_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let stripped = strip_css_comments(css);
    for stmt in stripped.split(';') {
        let trimmed = stmt.trim();
        let Some(rest) = trimmed.strip_prefix("@import") else {
            continue;
        };
        let rest = rest.trim();
        let inner = if let Some(u) = rest.strip_prefix("url(") {
            u.split(')').next().unwrap_or("").trim()
        } else {
            rest
        };
        let unquoted = inner.trim_matches(|c| c == '"' || c == '\'').trim();
        if unquoted.is_empty() || unquoted.starts_with("resource://") {
            continue;
        }
        let path_str = unquoted.strip_prefix("file://").unwrap_or(unquoted);
        let p = Path::new(path_str);
        let resolved = if p.is_absolute() {
            p.to_path_buf()
        } else {
            base_dir.join(p)
        };
        out.push(resolved);
    }
    out
}

fn inspect_css_tree(css_file: &Path) -> CssInspection {
    let mut out = CssInspection::default();
    let mut visited = HashSet::new();
    inspect_css_recursive(css_file, 0, &mut visited, &mut out);
    out
}

fn inspect_css_recursive(
    css_file: &Path,
    depth: usize,
    visited: &mut HashSet<PathBuf>,
    out: &mut CssInspection,
) {
    if depth > 4 {
        return;
    }
    let real = fs::canonicalize(css_file).unwrap_or_else(|_| css_file.to_path_buf());
    if !visited.insert(real.clone()) {
        return;
    }
    let Ok(text) = fs::read_to_string(&real).or_else(|_| fs::read_to_string(css_file)) else {
        return;
    };
    let base_dir = real
        .parent()
        .or_else(|| css_file.parent())
        .unwrap_or_else(|| Path::new("."));

    // Process `@import`s first so later definitions in the importing file win.
    for imported in parse_css_imports(&text, base_dir) {
        inspect_css_recursive(&imported, depth + 1, visited, out);
    }

    let stripped = strip_css_comments(&text);
    for stmt in stripped.split(';') {
        let trimmed = stmt.trim();
        let tail = trimmed
            .rfind('}')
            .map(|idx| trimmed[idx + 1..].trim())
            .unwrap_or(trimmed);
        if let Some(rest) = tail.strip_prefix("@define-color") {
            let rest = rest.trim();
            let mut parts = rest.splitn(2, char::is_whitespace);
            if let (Some(ident), Some(val)) = (parts.next(), parts.next()) {
                let ident = ident.trim();
                let val = val.trim();
                if is_css_ident(ident) && is_safe_css_value(val) {
                    out.define_colors.insert(ident.to_string(), val.to_string());
                }
            }
        }
    }

    extract_root_custom_props(&stripped, &mut out.custom_props);
}

fn extract_root_custom_props(stripped_css: &str, props: &mut HashMap<String, String>) {
    let mut rest = stripped_css;
    while let Some(open) = rest.find('{') {
        let selector = rest[..open].trim();
        let selector_tail = selector
            .rfind(['}', ';'])
            .map(|idx| selector[idx + 1..].trim())
            .unwrap_or(selector);
        let after_open = &rest[open + 1..];
        let Some(close) = find_matching_brace(after_open) else {
            break;
        };
        let body = &after_open[..close];
        if selector_tail
            .split(',')
            .any(|s| matches!(s.trim(), ":root" | "*" | "window"))
        {
            for decl in body.split(';') {
                let decl = decl.trim();
                if let Some((k, v)) = decl.split_once(':') {
                    let k = k.trim();
                    let v = v.trim();
                    if k.starts_with("--") && is_css_ident(&k[2..]) && is_safe_css_value(v) {
                        props.insert(k.to_string(), v.to_string());
                    }
                }
            }
        }
        rest = &after_open[close + 1..];
    }
}

fn find_matching_brace(s: &str) -> Option<usize> {
    let mut depth = 1usize;
    for (idx, ch) in s.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_css_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn is_safe_css_value(s: &str) -> bool {
    !s.is_empty() && !s.contains(['{', '}', ';'])
}

fn inspection_bg_prefers_dark(ins: &CssInspection) -> Option<bool> {
    let candidates = [
        ins.custom_props.get("--view-bg-color"),
        ins.custom_props.get("--window-bg-color"),
        ins.define_colors.get("view_bg_color"),
        ins.define_colors.get("window_bg_color"),
        ins.define_colors.get("theme_base_color"),
        ins.define_colors.get("theme_bg_color"),
    ];
    for val in candidates.into_iter().flatten() {
        if let Some(rgba) = parse_literal_rgba(val, ins, 0)
            && rgba.alpha() > 0.2
        {
            return Some(luminance(&rgba) < 0.5);
        }
    }
    None
}

fn parse_literal_rgba(raw: &str, ins: &CssInspection, depth: usize) -> Option<gdk::RGBA> {
    if depth > 4 {
        return None;
    }
    let s = raw.trim().trim_end_matches("!important").trim();
    if let Some(ref_name) = s.strip_prefix('@') {
        let target = ins.define_colors.get(ref_name.trim())?;
        return parse_literal_rgba(target, ins, depth + 1);
    }
    if let Some(inner) = s.strip_prefix("var(").and_then(|r| r.strip_suffix(')')) {
        let var_name = inner.split(',').next()?.trim();
        let target = ins.custom_props.get(var_name)?;
        return parse_literal_rgba(target, ins, depth + 1);
    }
    gdk::RGBA::parse(s).ok()
}

/// Builds a pure palette stylesheet (`@define-color` + `:root` custom properties
/// + legacy `theme_*` bridges) without third-party widget selectors.
fn build_extracted_palette_css(ins: &CssInspection) -> String {
    if ins.define_colors.is_empty() && ins.custom_props.is_empty() {
        return String::new();
    }
    let mut css = String::new();
    for (name, val) in &ins.define_colors {
        css.push_str(&format!("@define-color {name} {val};\n"));
    }
    if !ins.custom_props.is_empty() {
        css.push_str(":root {\n");
        for (prop, val) in &ins.custom_props {
            css.push_str(&format!("  {prop}: {val};\n"));
        }
        css.push_str("}\n");
    }
    css.push_str(&build_bridge_css(ins));
    css
}

/// Bridges `@define-color` variables (including legacy GTK/KDE `theme_*` names)
/// onto Adwaita `:root { --*: ... }` custom properties when not already set.
fn build_bridge_css(ins: &CssInspection) -> String {
    if ins.define_colors.is_empty() {
        return String::new();
    }
    let mut extra_defines: Vec<(&str, &str)> = Vec::new();
    let legacy_map: &[(&str, &[&str])] = &[
        (
            "theme_bg_color",
            &["window_bg_color", "headerbar_bg_color", "sidebar_bg_color"],
        ),
        (
            "theme_fg_color",
            &["window_fg_color", "headerbar_fg_color", "sidebar_fg_color"],
        ),
        (
            "theme_base_color",
            &["view_bg_color", "card_bg_color", "popover_bg_color", "dialog_bg_color"],
        ),
        (
            "theme_text_color",
            &["view_fg_color", "card_fg_color", "popover_fg_color", "dialog_fg_color"],
        ),
        (
            "theme_selected_bg_color",
            &["accent_bg_color", "accent_color"],
        ),
        ("theme_selected_fg_color", &["accent_fg_color"]),
    ];

    let mut bridged_names: HashSet<&str> = HashSet::new();
    for (legacy, targets) in legacy_map {
        if ins.define_colors.contains_key(*legacy) {
            for target in *targets {
                let css_var = format!("--{}", target.replace('_', "-"));
                if !ins.define_colors.contains_key(*target)
                    && !ins.custom_props.contains_key(&css_var)
                {
                    extra_defines.push((target, legacy));
                    bridged_names.insert(target);
                }
            }
        }
    }

    let mut css = String::new();
    for (target, src) in &extra_defines {
        css.push_str(&format!("@define-color {target} @{src};\n"));
    }

    let mut root_lines = String::new();
    for &name in ADWAITA_COLOR_NAMES {
        let css_var = format!("--{}", name.replace('_', "-"));
        if !ins.custom_props.contains_key(&css_var)
            && (ins.define_colors.contains_key(name) || bridged_names.contains(name))
        {
            root_lines.push_str(&format!("  {css_var}: @{name};\n"));
        }
    }
    if !root_lines.is_empty() {
        css.push_str(":root {\n");
        css.push_str(&root_lines);
        css.push_str("}\n");
    }
    css
}

fn build_reset_css(
    startup: &CssInspection,
    current_user: &CssInspection,
    configured: &CssInspection,
    dark: bool,
    accent: &gdk::RGBA,
) -> String {
    if startup.define_colors.is_empty() && startup.custom_props.is_empty() {
        return String::new();
    }
    let mut css = String::new();

    for name in startup.define_colors.keys() {
        if !current_user.define_colors.contains_key(name) {
            let fallback = configured
                .define_colors
                .get(name)
                .cloned()
                .or_else(|| adwaita_default_color(name, dark, accent));
            if let Some(val) = fallback {
                css.push_str(&format!("@define-color {name} {val};\n"));
            }
        }
    }

    let mut root_resets = String::new();
    for prop in startup.custom_props.keys() {
        if !current_user.custom_props.contains_key(prop) {
            if let Some(val) = configured.custom_props.get(prop) {
                root_resets.push_str(&format!("  {prop}: {val};\n"));
            } else {
                let define_name = prop.trim_start_matches("--").replace('-', "_");
                if ADWAITA_COLOR_NAMES.contains(&define_name.as_str()) {
                    root_resets.push_str(&format!("  {prop}: @{define_name};\n"));
                } else if let Some(val) = adwaita_default_color(&define_name, dark, accent) {
                    root_resets.push_str(&format!("  {prop}: {val};\n"));
                }
            }
        }
    }
    if !root_resets.is_empty() {
        css.push_str(":root {\n");
        css.push_str(&root_resets);
        css.push_str("}\n");
    }

    css
}

/// Validates generated CSS before loading into `target` so malformed third-party
/// theme expressions never emit GTK stderr warnings.
fn load_validated_css(target: &gtk::CssProvider, css: &str) {
    if css.trim().is_empty() {
        target.load_from_string("");
        return;
    }
    let had_err = Rc::new(Cell::new(false));
    let test_provider = gtk::CssProvider::new();
    let flag = had_err.clone();
    test_provider.connect_parsing_error(move |_, _, _| {
        flag.set(true);
    });
    test_provider.load_from_string(css);
    if !had_err.get() {
        target.load_from_string(css);
        return;
    }

    // Filter line-by-line if a third-party `@define-color` used unsupported GTK3 syntax.
    let mut clean = String::new();
    for chunk in css.split_inclusive('\n') {
        if chunk.trim_start().starts_with("@define-color") {
            let line_err = Rc::new(Cell::new(false));
            let lp = gtk::CssProvider::new();
            let lf = line_err.clone();
            lp.connect_parsing_error(move |_, _, _| lf.set(true));
            lp.load_from_string(chunk);
            if !line_err.get() {
                clean.push_str(chunk);
            }
        } else {
            clean.push_str(chunk);
        }
    }
    let final_err = Rc::new(Cell::new(false));
    let fp = gtk::CssProvider::new();
    let ff = final_err.clone();
    fp.connect_parsing_error(move |_, _, _| ff.set(true));
    fp.load_from_string(&clean);
    if !final_err.get() {
        target.load_from_string(&clean);
    } else {
        target.load_from_string("");
    }
}

fn strip_css_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(nc) = chars.next() {
                if nc == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

fn adwaita_default_color(name: &str, dark: bool, accent: &gdk::RGBA) -> Option<String> {
    let bg = if dark {
        rgb_hex(0x1d1d20)
    } else {
        rgb_hex(0xffffff)
    };
    match name {
        "accent_bg_color" => return Some(rgba_to_css(accent)),
        "accent_color" => return Some(rgba_to_css(&ensure_contrast(accent, &bg, 3.5))),
        "accent_fg_color" => {
            return Some(if luminance(accent) > 0.55 {
                "rgba(0, 0, 0, 0.8)".to_string()
            } else {
                "#ffffff".to_string()
            });
        }
        _ => {}
    }
    let val = if dark {
        match name {
            "window_bg_color" => "#222226",
            "window_fg_color" => "#ffffff",
            "view_bg_color" => "#1d1d20",
            "view_fg_color" => "#ffffff",
            "headerbar_bg_color" => "#2e2e32",
            "headerbar_fg_color" => "#ffffff",
            "headerbar_backdrop_color" => "#222226",
            "headerbar_border_color" => "rgba(255, 255, 255, 0.12)",
            "sidebar_bg_color" => "#2e2e32",
            "sidebar_fg_color" => "#ffffff",
            "sidebar_backdrop_color" => "#28282c",
            "sidebar_border_color" => "rgba(255, 255, 255, 0.12)",
            "secondary_sidebar_bg_color" => "#28282c",
            "secondary_sidebar_fg_color" => "#ffffff",
            "card_bg_color" => "rgba(255, 255, 255, 0.08)",
            "card_fg_color" => "#ffffff",
            "dialog_bg_color" | "popover_bg_color" => "#36363a",
            "dialog_fg_color" | "popover_fg_color" => "#ffffff",
            "borders" | "border_color" => "rgba(255, 255, 255, 0.15)",
            "success_color" => "#8ff0a4",
            "warning_color" => "#f8e45c",
            "error_color" | "destructive_color" => "#ff7b63",
            _ => return None,
        }
    } else {
        match name {
            "window_bg_color" | "headerbar_backdrop_color" | "dialog_bg_color" => "#fafafb",
            "window_fg_color"
            | "view_fg_color"
            | "headerbar_fg_color"
            | "sidebar_fg_color"
            | "secondary_sidebar_fg_color"
            | "card_fg_color"
            | "dialog_fg_color"
            | "popover_fg_color" => "rgba(0, 0, 0, 0.8)",
            "view_bg_color" | "card_bg_color" | "popover_bg_color" => "#ffffff",
            "headerbar_bg_color" | "sidebar_bg_color" => "#ebebed",
            "headerbar_border_color" | "borders" | "border_color" => "rgba(0, 0, 0, 0.15)",
            "sidebar_backdrop_color" => "#f2f2f4",
            "sidebar_border_color" => "rgba(0, 0, 0, 0.12)",
            "secondary_sidebar_bg_color" => "#f3f3f5",
            "success_color" => "#26a269",
            "warning_color" => "#ae7b03",
            "error_color" | "destructive_color" => "#c01c28",
            _ => return None,
        }
    };
    Some(val.to_string())
}

fn rgba_to_css(c: &gdk::RGBA) -> String {
    let r = (c.red().clamp(0.0, 1.0) * 255.0).round() as u8;
    let g = (c.green().clamp(0.0, 1.0) * 255.0).round() as u8;
    let b = (c.blue().clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{r:02x}{g:02x}{b:02x}")
}

// ---------------------------------------------------------------------------
// VTE Terminal GTK Color Synchronization
// ---------------------------------------------------------------------------

fn apply_terminal_colors(term: &vte4::Terminal) {
    let resolved = resolve_gtk_colors(term);
    let palette = build_ansi_palette(&resolved);
    let palette_refs: Vec<&gdk::RGBA> = palette.iter().collect();

    term.set_colors(Some(&resolved.fg), Some(&resolved.bg), &palette_refs);
    term.set_color_cursor(Some(&resolved.fg));
    term.set_color_cursor_foreground(Some(&resolved.bg));

    let highlight_bg = mix(&resolved.bg, &resolved.accent, 0.30);
    term.set_color_highlight(Some(&highlight_bg));
    term.set_color_highlight_foreground(Some(&resolved.fg));
}

#[derive(Debug, Clone, Copy)]
struct ResolvedTheme {
    bg: gdk::RGBA,
    fg: gdk::RGBA,
    accent: gdk::RGBA,
    red: gdk::RGBA,
    green: gdk::RGBA,
    yellow: gdk::RGBA,
    blue: gdk::RGBA,
    purple: gdk::RGBA,
}

fn resolve_gtk_colors(term: &vte4::Terminal) -> ResolvedTheme {
    let style_manager = adw::StyleManager::default();
    let prefer_dark = style_manager.is_dark();
    let fallback_probe = term.root().is_none().then(|| gtk::Label::new(None));

    let probe_var = |cls: &str| -> Option<gdk::RGBA> {
        if term.root().is_some() {
            probe_on_terminal(term, cls)
        } else if let Some(lbl) = &fallback_probe {
            lbl.set_css_classes(&[cls]);
            let c = lbl.color();
            (c.alpha() > 0.05).then_some(c)
        } else {
            None
        }
    };

    let raw_bg = probe_var("riso-probe-view-bg")
        .or_else(|| probe_var("riso-probe-win-bg"))
        .or_else(|| {
            lookup_named_color(
                term,
                &[
                    "view_bg_color",
                    "theme_base_color",
                    "window_bg_color",
                    "theme_bg_color",
                ],
            )
        })
        .unwrap_or_else(|| {
            if prefer_dark {
                rgb_hex(0x1d1d20)
            } else {
                rgb_hex(0xffffff)
            }
        });

    let base_surface = if prefer_dark {
        rgb_hex(0x000000)
    } else {
        rgb_hex(0xffffff)
    };
    let bg = composite_over(&raw_bg, &base_surface);

    let raw_fg = probe_var("riso-probe-view-fg")
        .or_else(|| probe_var("riso-probe-win-fg"))
        .or_else(|| {
            lookup_named_color(
                term,
                &[
                    "view_fg_color",
                    "theme_text_color",
                    "window_fg_color",
                    "theme_fg_color",
                ],
            )
        })
        .or_else(|| {
            let c = term.color();
            (c.alpha() > 0.05 && contrast_ratio(&composite_over(&c, &bg), &bg) >= 2.0).then_some(c)
        })
        .unwrap_or_else(|| {
            if luminance(&bg) < 0.5 {
                rgb_hex(0xffffff)
            } else {
                rgb_hex(0x1e1e1e)
            }
        });
    let fg = composite_over(&raw_fg, &bg);

    let raw_accent = probe_var("riso-probe-accent-bg")
        .or_else(|| lookup_named_color(term, &["accent_bg_color", "theme_selected_bg_color"]))
        .unwrap_or_else(|| style_manager.accent_color_rgba());
    let accent = composite_over(&raw_accent, &bg);

    let pick_hue = |probe_cls: &str, named: &[&str], fallback: u32| -> gdk::RGBA {
        let c = probe_var(probe_cls)
            .or_else(|| lookup_named_color(term, named))
            .unwrap_or_else(|| rgb_hex(fallback));
        composite_over(&c, &bg)
    };

    ResolvedTheme {
        bg,
        fg,
        accent,
        red: pick_hue("riso-probe-red", &["red_3", "error_color"], 0xe01b24),
        green: pick_hue("riso-probe-green", &["green_3", "success_color"], 0x33d17a),
        yellow: pick_hue("riso-probe-yellow", &["yellow_3", "warning_color"], 0xf6d32d),
        blue: pick_hue("riso-probe-blue", &["blue_3"], 0x3584e4),
        purple: pick_hue("riso-probe-purple", &["purple_3"], 0x9141ac),
    }
}

fn probe_on_terminal(term: &vte4::Terminal, class_name: &str) -> Option<gdk::RGBA> {
    term.add_css_class(class_name);
    let c = term.color();
    term.remove_css_class(class_name);
    // Sentinel fallback `rgba(1, 2, 3, 0.01)` indicates the CSS variable is unset.
    (c.alpha() > 0.05).then_some(c)
}

#[allow(deprecated)]
fn lookup_named_color(widget: &impl IsA<gtk::Widget>, names: &[&str]) -> Option<gdk::RGBA> {
    let ctx = widget.style_context();
    for name in names {
        if let Some(c) = ctx.lookup_color(name)
            && c.alpha() > 0.05
        {
            return Some(c);
        }
    }
    None
}

fn build_ansi_palette(theme: &ResolvedTheme) -> [gdk::RGBA; 16] {
    let dark_bg = luminance(&theme.bg) < 0.5;
    let white = rgb_hex(0xffffff);
    let black = rgb_hex(0x000000);

    let cyan_seed = mix(&theme.green, &theme.blue, 0.52);

    let min_normal = if dark_bg { 4.2 } else { 4.5 };
    let red = ensure_contrast(&theme.red, &theme.bg, min_normal);
    let green = ensure_contrast(&theme.green, &theme.bg, min_normal);
    let yellow = ensure_contrast(&theme.yellow, &theme.bg, min_normal);
    let blue = ensure_contrast(&theme.blue, &theme.bg, min_normal);
    let magenta = ensure_contrast(&theme.purple, &theme.bg, min_normal);
    let cyan = ensure_contrast(&cyan_seed, &theme.bg, min_normal);

    let brighten_step = |c: &gdk::RGBA| -> gdk::RGBA {
        if dark_bg {
            ensure_contrast(&mix(c, &white, 0.25), &theme.bg, 5.5)
        } else {
            ensure_contrast(&mix(c, &black, 0.12), &theme.bg, 4.0)
        }
    };

    let (ansi_black, ansi_white, ansi_bright_black, ansi_bright_white) = if dark_bg {
        (
            mix(&theme.bg, &theme.fg, 0.08),
            mix(&theme.bg, &theme.fg, 0.82),
            ensure_contrast(&mix(&theme.bg, &theme.fg, 0.48), &theme.bg, 3.8),
            mix(&theme.fg, &white, 0.50),
        )
    } else {
        (
            mix(&theme.bg, &theme.fg, 0.92),
            mix(&theme.bg, &theme.fg, 0.20),
            ensure_contrast(&mix(&theme.bg, &theme.fg, 0.60), &theme.bg, 3.8),
            mix(&theme.bg, &theme.fg, 0.06),
        )
    };

    [
        ansi_black,
        red,
        green,
        yellow,
        blue,
        magenta,
        cyan,
        ansi_white,
        ansi_bright_black,
        brighten_step(&red),
        brighten_step(&green),
        brighten_step(&yellow),
        brighten_step(&blue),
        brighten_step(&magenta),
        brighten_step(&cyan),
        ansi_bright_white,
    ]
}

fn rgb_hex(hex: u32) -> gdk::RGBA {
    let r = ((hex >> 16) & 0xff) as f32 / 255.0;
    let g = ((hex >> 8) & 0xff) as f32 / 255.0;
    let b = (hex & 0xff) as f32 / 255.0;
    gdk::RGBA::new(r, g, b, 1.0)
}

fn composite_over(fg: &gdk::RGBA, bg: &gdk::RGBA) -> gdk::RGBA {
    let a = fg.alpha().clamp(0.0, 1.0);
    gdk::RGBA::new(
        fg.red() * a + bg.red() * (1.0 - a),
        fg.green() * a + bg.green() * (1.0 - a),
        fg.blue() * a + bg.blue() * (1.0 - a),
        1.0,
    )
}

fn mix(a: &gdk::RGBA, b: &gdk::RGBA, t: f32) -> gdk::RGBA {
    let t = t.clamp(0.0, 1.0);
    gdk::RGBA::new(
        a.red() * (1.0 - t) + b.red() * t,
        a.green() * (1.0 - t) + b.green() * t,
        a.blue() * (1.0 - t) + b.blue() * t,
        1.0,
    )
}

fn srgb_to_linear(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(c: &gdk::RGBA) -> f32 {
    0.2126 * srgb_to_linear(c.red())
        + 0.7152 * srgb_to_linear(c.green())
        + 0.0722 * srgb_to_linear(c.blue())
}

fn contrast_ratio(a: &gdk::RGBA, b: &gdk::RGBA) -> f32 {
    let la = luminance(a);
    let lb = luminance(b);
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

fn ensure_contrast(color: &gdk::RGBA, bg: &gdk::RGBA, min_ratio: f32) -> gdk::RGBA {
    if contrast_ratio(color, bg) >= min_ratio {
        return *color;
    }
    let pole = if luminance(bg) < 0.5 {
        rgb_hex(0xffffff)
    } else {
        rgb_hex(0x000000)
    };
    let mut lo = 0.0_f32;
    let mut hi = 1.0_f32;
    for _ in 0..16 {
        let mid = 0.5 * (lo + hi);
        let candidate = mix(color, &pole, mid);
        if contrast_ratio(&candidate, bg) >= min_ratio {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    mix(color, &pole, hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stylesheet_parses_without_errors_or_warnings() {
        if gtk::init().is_err() {
            return;
        }
        for css in [STYLE_CSS, PROBE_CSS] {
            let errors: Rc<RefCell<Vec<String>>> = Rc::default();
            let provider = gtk::CssProvider::new();
            let errs = errors.clone();
            provider.connect_parsing_error(move |_, section, err| {
                errs.borrow_mut().push(format!(
                    "line {}: {}",
                    section.start_location().lines() + 1,
                    err.message()
                ));
            });
            provider.load_from_string(css);
            assert!(
                errors.borrow().is_empty(),
                "CSS errors/warnings: {:?}",
                errors.borrow()
            );
        }
    }

    #[test]
    fn css_imports_bridge_reset_and_chained_symlinks_work() {
        let work = std::env::temp_dir().join(format!("riso-theme-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&work);
        let themes_dir = work.join("omarchy").join("themes").join("mocha");
        let gtk4_dir = work.join("gtk-4.0");
        fs::create_dir_all(&themes_dir).unwrap();
        fs::create_dir_all(&gtk4_dir).unwrap();

        let theme_colors = themes_dir.join("colors.css");
        fs::write(
            &theme_colors,
            "/* comment */\n\
             @define-color theme_base_color #181825;\n\
             @define-color theme_bg_color #1e1e2e;\n\
             @define-color theme_selected_bg_color #89b4fa;\n\
             :root { --headerbar-bg-color: #11111b; }\n",
        )
        .unwrap();

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&themes_dir, work.join("omarchy").join("current")).unwrap();
        }

        let gtk_css = gtk4_dir.join("gtk.css");
        let import_rel = if cfg!(unix) {
            "../omarchy/current/colors.css"
        } else {
            "../omarchy/themes/mocha/colors.css"
        };
        fs::write(
            &gtk_css,
            format!("@import url(\"{import_rel}\");\n/* @import 'ignored.css'; */\n"),
        )
        .unwrap();

        let ins = inspect_css_tree(&gtk_css);
        assert_eq!(
            ins.define_colors.get("theme_base_color").map(String::as_str),
            Some("#181825")
        );
        assert_eq!(
            ins.custom_props
                .get("--headerbar-bg-color")
                .map(String::as_str),
            Some("#11111b")
        );
        assert_eq!(inspection_bg_prefers_dark(&ins), Some(true));

        // Legacy `theme_*` bridges onto Adwaita variables without clobbering explicit `--headerbar-bg-color`.
        let bridge = build_bridge_css(&ins);
        assert!(bridge.contains("@define-color view_bg_color @theme_base_color;"));
        assert!(bridge.contains("--view-bg-color: @view_bg_color;"));
        assert!(bridge.contains("--accent-bg-color: @accent_bg_color;"));
        assert!(!bridge.contains("--headerbar-bg-color:"));

        // Deleting `gtk.css` produces reset rules for both `@define-color` and `:root { --* }`.
        let empty = CssInspection::default();
        let reset = build_reset_css(&ins, &empty, &empty, false, &rgb_hex(0x3584e4));
        assert!(reset.contains("--headerbar-bg-color: @headerbar_bg_color;"));

        #[cfg(unix)]
        {
            let targets = collect_watch_targets(&gtk4_dir, &gtk_css, None);
            let omarchy_parent = fs::canonicalize(work.join("omarchy")).unwrap_or(work.join("omarchy"));
            let has_current_watch = targets.iter().any(|(dir, filter)| {
                let d = fs::canonicalize(dir).unwrap_or_else(|_| dir.clone());
                d == omarchy_parent
                    && match filter {
                        WatchFilter::AllThemeFiles => true,
                        WatchFilter::ExactNames(s) => s.contains("current"),
                    }
            });
            assert!(has_current_watch, "expected intermediate symlink watch on omarchy/current");
        }

        let _ = fs::remove_dir_all(&work);
    }

    #[test]
    fn ansi_palette_maintains_contrast_on_light_and_dark_gtk_themes() {
        let light = ResolvedTheme {
            bg: rgb_hex(0xffffff),
            fg: composite_over(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.8), &rgb_hex(0xffffff)),
            accent: rgb_hex(0x3584e4),
            red: rgb_hex(0xe01b24),
            green: rgb_hex(0x33d17a),
            yellow: rgb_hex(0xf6d32d),
            blue: rgb_hex(0x3584e4),
            purple: rgb_hex(0x9141ac),
        };
        let light_pal = build_ansi_palette(&light);
        for idx in [1, 2, 3, 4, 5, 6, 8, 9, 10, 11, 12, 13, 14] {
            let ratio = contrast_ratio(&light_pal[idx], &light.bg);
            assert!(
                ratio >= 3.5,
                "light ANSI color {idx} contrast too low: {ratio}"
            );
        }

        let dark = ResolvedTheme {
            bg: rgb_hex(0x1d1d20),
            fg: rgb_hex(0xffffff),
            accent: rgb_hex(0x3584e4),
            red: rgb_hex(0xe01b24),
            green: rgb_hex(0x33d17a),
            yellow: rgb_hex(0xf6d32d),
            blue: rgb_hex(0x3584e4),
            purple: rgb_hex(0x9141ac),
        };
        let dark_pal = build_ansi_palette(&dark);
        for idx in [1, 2, 3, 4, 5, 6, 8, 9, 10, 11, 12, 13, 14] {
            let ratio = contrast_ratio(&dark_pal[idx], &dark.bg);
            assert!(
                ratio >= 3.5,
                "dark ANSI color {idx} contrast too low: {ratio}"
            );
        }
    }
}
