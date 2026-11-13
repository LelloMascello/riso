//! TUI: embedded terminal emulator (vt100) for the ssh session — rendering,
//! replies to terminal queries, and encoding of keys/mouse for the PTY.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

use super::theme::vt_color;

const SCROLLBACK: usize = 10_000;

/// Side effects requested by the remote program while parsing its output.
#[derive(Default)]
pub struct TermCallbacks {
    /// Bytes to send back to the remote program (query replies).
    pub replies: Vec<u8>,
    /// Window title set with OSC 0/2.
    pub title: Option<String>,
    /// Working directory announced with OSC 7.
    pub cwd: Option<String>,
    /// OSC 52 clipboard payloads (base64) to forward to the host terminal.
    pub clipboard: Vec<Vec<u8>>,
    pub bell: bool,
}

impl vt100::Callbacks for TermCallbacks {
    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.bell = true;
    }

    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = Some(String::from_utf8_lossy(title).into_owned());
    }

    fn copy_to_clipboard(&mut self, _: &mut vt100::Screen, _ty: &[u8], data: &[u8]) {
        self.clipboard.push(data.to_vec());
    }

    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let first = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        match (i1, c) {
            // Primary device attributes: VT220-ish with ANSI colour.
            (None, 'c') if first == 0 => self.replies.extend_from_slice(b"\x1b[?62;22c"),
            // Secondary device attributes.
            (Some(b'>'), 'c') if first == 0 => self.replies.extend_from_slice(b"\x1b[>1;10;0c"),
            (None, 'n') if first == 5 => self.replies.extend_from_slice(b"\x1b[0n"),
            (None | Some(b'?'), 'n') if first == 6 => {
                let (row, col) = screen.cursor_position();
                let q = if i1.is_some() { "?" } else { "" };
                self.replies
                    .extend_from_slice(format!("\x1b[{q}{};{}R", row + 1, col + 1).as_bytes());
            }
            _ => {}
        }
    }

    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        if params.first() == Some(&&b"7"[..])
            && let Some(url) = params.get(1)
        {
            self.cwd = parse_osc7(&String::from_utf8_lossy(url));
        }
    }
}

/// `file://host/some%20path` → `/some path`.
fn parse_osc7(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://").or_else(|| url.strip_prefix("kitty-shell-cwd://"))?;
    let path = &rest[rest.find('/')?..];
    Some(percent_decode(path))
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub struct Term {
    pub parser: vt100::Parser<TermCallbacks>,
}

impl Term {
    pub fn new(rows: u16, cols: u16) -> Self {
        Term {
            parser: vt100::Parser::new_with_callbacks(rows.max(1), cols.max(1), SCROLLBACK, TermCallbacks::default()),
        }
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows.max(1), cols.max(1));
    }

    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    pub fn callbacks(&mut self) -> &mut TermCallbacks {
        self.parser.callbacks_mut()
    }

    pub fn scrollback(&self) -> usize {
        self.screen().scrollback()
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let cur = self.scrollback() as isize;
        let next = (cur + delta).max(0) as usize;
        self.parser.screen_mut().set_scrollback(next);
    }

    pub fn reset_scroll(&mut self) {
        if self.scrollback() != 0 {
            self.parser.screen_mut().set_scrollback(0);
        }
    }

    /// Draws the visible screen; returns the cursor position if shown.
    pub fn render(&self, area: Rect, buf: &mut Buffer) -> Option<(u16, u16)> {
        let screen = self.screen();
        let (rows, cols) = screen.size();
        for r in 0..rows.min(area.height) {
            for c in 0..cols.min(area.width) {
                let (x, y) = (area.x + c, area.y + r);
                let Some(cell) = screen.cell(r, c) else { continue };
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut style = Style::new().fg(vt_color(cell.fgcolor())).bg(vt_color(cell.bgcolor()));
                let mut m = Modifier::empty();
                if cell.bold() {
                    m |= Modifier::BOLD;
                }
                if cell.dim() {
                    m |= Modifier::DIM;
                }
                if cell.italic() {
                    m |= Modifier::ITALIC;
                }
                if cell.underline() {
                    m |= Modifier::UNDERLINED;
                }
                if cell.inverse() {
                    m |= Modifier::REVERSED;
                }
                style = style.add_modifier(m);
                let out = &mut buf[(x, y)];
                if cell.has_contents() {
                    out.set_symbol(cell.contents());
                } else {
                    out.set_symbol(" ");
                }
                out.set_style(style);
                if cell.is_wide() && c + 1 < cols.min(area.width) {
                    buf[(x + 1, y)].reset();
                }
            }
        }
        if screen.hide_cursor() || screen.scrollback() > 0 {
            return None;
        }
        let (r, c) = screen.cursor_position();
        (r < area.height && c < area.width).then_some((area.x + c, area.y + r))
    }

    /// Wraps pasted text for the remote program.
    pub fn paste_bytes(&self, text: &str) -> Vec<u8> {
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.screen().bracketed_paste() {
            let mut v = b"\x1b[200~".to_vec();
            v.extend_from_slice(text.replace("\x1b[201~", "").as_bytes());
            v.extend_from_slice(b"\x1b[201~");
            v
        } else {
            text.into_bytes()
        }
    }

    /// Whether the remote program wants mouse events.
    pub fn wants_mouse(&self) -> bool {
        self.screen().mouse_protocol_mode() != MouseProtocolMode::None
    }

    /// Encodes a mouse event at cell (`col`, `row`) of the screen, if the
    /// current mouse mode reports it.
    pub fn mouse_bytes(&self, ev: &MouseEvent, col: u16, row: u16) -> Option<Vec<u8>> {
        let screen = self.screen();
        let mode = screen.mouse_protocol_mode();
        let (base, release, motion) = match ev.kind {
            MouseEventKind::Down(b) => (button_code(b), false, false),
            MouseEventKind::Up(b) => (button_code(b), true, false),
            MouseEventKind::Drag(b) => (button_code(b) + 32, false, true),
            MouseEventKind::Moved => (3 + 32, false, true),
            MouseEventKind::ScrollUp => (64, false, false),
            MouseEventKind::ScrollDown => (65, false, false),
            MouseEventKind::ScrollLeft => (66, false, false),
            MouseEventKind::ScrollRight => (67, false, false),
        };
        let allowed = match mode {
            MouseProtocolMode::None => false,
            MouseProtocolMode::Press => !release && !motion,
            MouseProtocolMode::PressRelease => !motion,
            MouseProtocolMode::ButtonMotion => !matches!(ev.kind, MouseEventKind::Moved),
            MouseProtocolMode::AnyMotion => true,
        };
        if !allowed {
            return None;
        }
        let mut code = base;
        if ev.modifiers.contains(KeyModifiers::SHIFT) {
            code += 4;
        }
        if ev.modifiers.contains(KeyModifiers::ALT) {
            code += 8;
        }
        if ev.modifiers.contains(KeyModifiers::CONTROL) {
            code += 16;
        }
        let (x, y) = (col as u32 + 1, row as u32 + 1);
        Some(match screen.mouse_protocol_encoding() {
            MouseProtocolEncoding::Sgr => {
                format!("\x1b[<{code};{x};{y}{}", if release { 'm' } else { 'M' }).into_bytes()
            }
            _ => {
                let code = if release { 3 + (code & !3) } else { code };
                let enc = |v: u32| (32 + v).min(255) as u8;
                vec![0x1b, b'[', b'M', enc(code), enc(x), enc(y)]
            }
        })
    }
}

fn button_code(b: MouseButton) -> u32 {
    match b {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

/// Encodes a key press the way xterm does.
pub fn key_bytes(key: &KeyEvent, app_cursor: bool) -> Option<Vec<u8>> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    // xterm modifier parameter: 1 + shift(1) + alt(2) + ctrl(4)
    let m = 1 + shift as u8 + 2 * alt as u8 + 4 * ctrl as u8;

    let csi_letter = |letter: char| -> Vec<u8> {
        if m > 1 {
            format!("\x1b[1;{m}{letter}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{letter}").into_bytes()
        } else {
            format!("\x1b[{letter}").into_bytes()
        }
    };
    let csi_tilde = |n: u8| -> Vec<u8> {
        if m > 1 { format!("\x1b[{n};{m}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() }
    };

    let mut out = match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                let b = match c.to_ascii_lowercase() {
                    c @ 'a'..='z' => (c as u8) - b'a' + 1,
                    '@' | ' ' | '2' => 0,
                    '[' | '3' => 0x1b,
                    '\\' | '4' => 0x1c,
                    ']' | '5' => 0x1d,
                    '^' | '6' => 0x1e,
                    '_' | '-' | '7' => 0x1f,
                    '8' | '?' => 0x7f,
                    _ => return None,
                };
                vec![b]
            } else {
                let mut s = [0u8; 4];
                c.encode_utf8(&mut s).as_bytes().to_vec()
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => {
            if ctrl { vec![0x08] } else { vec![0x7f] }
        }
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => csi_letter('A'),
        KeyCode::Down => csi_letter('B'),
        KeyCode::Right => csi_letter('C'),
        KeyCode::Left => csi_letter('D'),
        KeyCode::Home => csi_letter('H'),
        KeyCode::End => csi_letter('F'),
        KeyCode::Insert => csi_tilde(2),
        KeyCode::Delete => csi_tilde(3),
        KeyCode::PageUp => csi_tilde(5),
        KeyCode::PageDown => csi_tilde(6),
        KeyCode::F(n) => match n {
            1..=4 => {
                let l = ['P', 'Q', 'R', 'S'][n as usize - 1];
                if m > 1 { format!("\x1b[1;{m}{l}").into_bytes() } else { format!("\x1bO{l}").into_bytes() }
            }
            5 => csi_tilde(15),
            6 => csi_tilde(17),
            7 => csi_tilde(18),
            8 => csi_tilde(19),
            9 => csi_tilde(20),
            10 => csi_tilde(21),
            11 => csi_tilde(23),
            12 => csi_tilde(24),
            _ => return None,
        },
        _ => return None,
    };
    // Alt as ESC prefix for keys without an xterm modifier parameter.
    let has_param = matches!(
        key.code,
        KeyCode::Up
            | KeyCode::Down
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::Insert
            | KeyCode::Delete
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::F(_)
    );
    if alt && !has_param {
        out.insert(0, 0x1b);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn encodes_keys() {
        assert_eq!(key_bytes(&k(KeyCode::Char('c'), KeyModifiers::CONTROL), false), Some(vec![3]));
        assert_eq!(key_bytes(&k(KeyCode::Char('b'), KeyModifiers::ALT), false), Some(b"\x1bb".to_vec()));
        assert_eq!(key_bytes(&k(KeyCode::Up, KeyModifiers::NONE), true), Some(b"\x1bOA".to_vec()));
        assert_eq!(key_bytes(&k(KeyCode::Left, KeyModifiers::CONTROL), false), Some(b"\x1b[1;5D".to_vec()));
        assert_eq!(key_bytes(&k(KeyCode::Delete, KeyModifiers::NONE), false), Some(b"\x1b[3~".to_vec()));
    }

    #[test]
    fn osc7_and_queries() {
        assert_eq!(parse_osc7("file://arch/home/leo/a%20b").as_deref(), Some("/home/leo/a b"));
        let mut t = Term::new(5, 20);
        t.process(b"\x1b]7;file://h/tmp\x07ab\x1b[6n\x1b[c");
        assert_eq!(t.callbacks().cwd.as_deref(), Some("/tmp"));
        assert_eq!(t.callbacks().replies, b"\x1b[1;3R\x1b[?62;22c".to_vec());
    }
}
