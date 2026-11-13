//! TUI: styles built exclusively from the terminal's own ANSI palette.
//!
//! No RGB value is ever emitted by `riso-tui`'s chrome and there are no
//! hardcoded theme presets: every style uses `Color::Reset` or the 16 ANSI
//! colours from the user's terminal (the same palette shown by `fastfetch`).
//! Changing the terminal's colour scheme updates the whole UI live.

use ratatui::style::{Color, Modifier, Style};

pub fn base() -> Style {
    Style::new().fg(Color::Reset).bg(Color::Reset)
}

pub fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}

pub fn accent() -> Style {
    Style::new().fg(Color::Blue)
}

pub fn divider(focused: bool) -> Style {
    if focused { accent() } else { dim() }
}

pub fn border(focused: bool) -> Style {
    divider(focused)
}

pub fn title(focused: bool) -> Style {
    let s = Style::new().add_modifier(Modifier::BOLD);
    if focused { s.fg(Color::Blue) } else { s }
}

/// Highlighted row. Reverse video adapts to any terminal palette.
pub fn selection(focused: bool) -> Style {
    if focused {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new().add_modifier(Modifier::UNDERLINED)
    }
}

pub fn directory() -> Style {
    Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD)
}

pub fn symlink() -> Style {
    Style::new().fg(Color::Cyan)
}

pub fn executable() -> Style {
    Style::new().fg(Color::Green)
}

pub fn hidden() -> Style {
    Style::new().fg(Color::DarkGray)
}

pub fn archive() -> Style {
    Style::new().fg(Color::Red)
}

pub fn media() -> Style {
    Style::new().fg(Color::Magenta)
}

pub fn code() -> Style {
    Style::new().fg(Color::Cyan)
}

pub fn config() -> Style {
    Style::new().fg(Color::Yellow)
}

pub fn marked() -> Style {
    Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
}

pub fn cut() -> Style {
    Style::new().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)
}

pub fn ok() -> Style {
    Style::new().fg(Color::Green)
}

pub fn wait() -> Style {
    Style::new().fg(Color::Yellow)
}

pub fn error() -> Style {
    Style::new().fg(Color::Red)
}

pub fn tab_active() -> Style {
    Style::new().fg(Color::Reset).add_modifier(Modifier::BOLD)
}

pub fn tab_inactive() -> Style {
    Style::new().fg(Color::DarkGray)
}

pub fn tab_attention() -> Style {
    Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
}

pub fn key() -> Style {
    Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD)
}

pub fn toast() -> Style {
    Style::new().add_modifier(Modifier::REVERSED)
}

pub fn gauge() -> Style {
    Style::new().fg(Color::Blue)
}

pub fn drop_target() -> Style {
    Style::new().fg(Color::Blue).add_modifier(Modifier::REVERSED | Modifier::BOLD)
}

/// Maps a cell colour of the embedded remote terminal. Indices 0–15 become
/// the host terminal's ANSI colours (so remote output follows terminal
/// palette changes too); the 256-colour cube and truecolor are passed through.
pub fn vt_color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => match i {
            0 => Color::Black,
            1 => Color::Red,
            2 => Color::Green,
            3 => Color::Yellow,
            4 => Color::Blue,
            5 => Color::Magenta,
            6 => Color::Cyan,
            7 => Color::Gray,
            8 => Color::DarkGray,
            9 => Color::LightRed,
            10 => Color::LightGreen,
            11 => Color::LightYellow,
            12 => Color::LightBlue,
            13 => Color::LightMagenta,
            14 => Color::LightCyan,
            15 => Color::White,
            n => Color::Indexed(n),
        },
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}
