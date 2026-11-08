//! Adaptive GTK system theming and dynamic VTE terminal color synchronization.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
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

/* Internal CSS variable probes for VTE terminal synchronization */
.riso-probe-view-bg { color: var(--view-bg-color, rgba(1, 2, 3, 0.01)); }
.riso-probe-win-bg { color: var(--window-bg-color, rgba(1, 2, 3, 0.01)); }
.riso-probe-view-fg { color: var(--view-fg-color, rgba(1, 2, 3, 0.01)); }
.riso-probe-win-fg { color: var(--window-fg-color, rgba(1, 2, 3, 0.01)); }
.riso-probe-accent-bg { color: var(--accent-bg-color, rgba(1, 2, 3, 0.01)); }
.riso-probe-red { color: var(--red-3, rgba(1, 2, 3, 0.01)); }
.riso-probe-green { color: var(--green-3, rgba(1, 2, 3, 0.01)); }
.riso-probe-yellow { color: var(--yellow-3, rgba(1, 2, 3, 0.01)); }
.riso-probe-blue { color: var(--blue-3, rgba(1, 2, 3, 0.01)); }
.riso-probe-purple { color: var(--purple-3, rgba(1, 2, 3, 0.01)); }
"#;

/// Initializes adaptive GTK / libadwaita color scheme handling and registers
/// the application stylesheet on `display`.
pub fn init(display: &gdk::Display) {
    let style_manager = adw::StyleManager::default();
    if style_manager.system_supports_color_schemes() {
        style_manager.set_color_scheme(adw::ColorScheme::Default);
    } else if system_prefers_dark_fallback() {
        style_manager.set_color_scheme(adw::ColorScheme::PreferDark);
    } else {
        style_manager.set_color_scheme(adw::ColorScheme::Default);
    }

    let provider = gtk::CssProvider::new();
    provider.load_from_string(STYLE_CSS);
    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

/// Fallback dark-theme detection for window managers without an active
/// `org.freedesktop.appearance` settings portal.
fn system_prefers_dark_fallback() -> bool {
    if let Ok(env_theme) = std::env::var("GTK_THEME") {
        let low = env_theme.to_ascii_lowercase();
        if low.ends_with(":dark") || low.contains("-dark") {
            return true;
        }
        if low.ends_with(":light") || low.contains("-light") {
            return false;
        }
    }

    let cfg = glib::user_config_dir();
    for subdir in ["gtk-4.0", "gtk-3.0"] {
        let ini = cfg.join(subdir).join("settings.ini");
        let kf = glib::KeyFile::new();
        if kf.load_from_file(&ini, glib::KeyFileFlags::NONE).is_ok() {
            if let Ok(prefer_dark) = kf.boolean("Settings", "gtk-application-prefer-dark-theme") {
                return prefer_dark;
            }
            if let Ok(theme_name) = kf.string("Settings", "gtk-theme-name")
                && theme_name.to_ascii_lowercase().contains("dark")
            {
                return true;
            }
        }
    }
    false
}

/// Binds a `vte4::Terminal` to the live system GTK theme colors so that
/// background, foreground, cursor, selection highlight, and the 16-color ANSI
/// palette adapt automatically to light/dark switches, system accent changes,
/// and custom GTK themes.
pub fn bind_terminal(term: &vte4::Terminal) {
    apply_terminal_colors(term);

    let scheduled = Rc::new(Cell::new(false));
    let schedule_refresh = {
        let weak_term = term.downgrade();
        let scheduled = scheduled.clone();
        move || {
            if scheduled.replace(true) {
                return;
            }
            let weak_term = weak_term.clone();
            let scheduled = scheduled.clone();
            glib::idle_add_local_once(move || {
                scheduled.set(false);
                if let Some(t) = weak_term.upgrade() {
                    apply_terminal_colors(&t);
                }
            });
        }
    };

    term.connect_realize(apply_terminal_colors);

    let style_manager = adw::StyleManager::default();
    let mut handler_ids = Vec::new();

    let cb = schedule_refresh.clone();
    let sm_dark = style_manager.connect_dark_notify(move |_| cb());
    handler_ids.push((style_manager.clone().upcast::<glib::Object>(), sm_dark));

    let cb = schedule_refresh.clone();
    let sm_accent = style_manager.connect_accent_color_rgba_notify(move |_| cb());
    handler_ids.push((style_manager.clone().upcast::<glib::Object>(), sm_accent));

    let cb = schedule_refresh.clone();
    let sm_hc = style_manager.connect_high_contrast_notify(move |_| cb());
    handler_ids.push((style_manager.upcast::<glib::Object>(), sm_hc));

    let settings = gtk::Settings::for_display(&term.display());
    let cb = schedule_refresh.clone();
    let s_theme = settings.connect_gtk_theme_name_notify(move |_| cb());
    handler_ids.push((settings.clone().upcast::<glib::Object>(), s_theme));

    let cb = schedule_refresh;
    let s_dark = settings.connect_gtk_application_prefer_dark_theme_notify(move |_| cb());
    handler_ids.push((settings.upcast::<glib::Object>(), s_dark));

    let handler_ids = RefCell::new(handler_ids);
    term.connect_destroy(move |_| {
        for (obj, id) in handler_ids.borrow_mut().drain(..) {
            obj.disconnect(id);
        }
    });
}

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
    let probe = gtk::Label::new(None);

    let raw_bg = probe_css_color(&probe, "riso-probe-view-bg")
        .or_else(|| probe_css_color(&probe, "riso-probe-win-bg"))
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

    let raw_fg = probe_css_color(&probe, "riso-probe-view-fg")
        .or_else(|| probe_css_color(&probe, "riso-probe-win-fg"))
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

    let raw_accent = probe_css_color(&probe, "riso-probe-accent-bg")
        .or_else(|| lookup_named_color(term, &["accent_bg_color", "theme_selected_bg_color"]))
        .unwrap_or_else(|| style_manager.accent_color_rgba());
    let accent = composite_over(&raw_accent, &bg);

    let pick_hue = |probe_cls: &str, named: &[&str], fallback: u32| -> gdk::RGBA {
        let c = probe_css_color(&probe, probe_cls)
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

fn probe_css_color(probe: &gtk::Label, class_name: &str) -> Option<gdk::RGBA> {
    probe.set_css_classes(&[class_name]);
    let c = probe.color();
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
        provider.load_from_string(STYLE_CSS);
        assert!(
            errors.borrow().is_empty(),
            "CSS errors/warnings: {:?}",
            errors.borrow()
        );
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
