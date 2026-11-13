//! TUI: single-line text editor used by the ssh prompt and by dialogs.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::line_edit::{chars_cols, next_word_boundary, prev_word_boundary};

#[derive(Default, Clone)]
pub struct LineInput {
    pub chars: Vec<char>,
    pub cursor: usize,
}

impl LineInput {
    pub fn with_text(text: &str) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let cursor = chars.len();
        LineInput { chars, cursor }
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn set_text(&mut self, text: &str) {
        *self = Self::with_text(text);
    }

    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars().filter(|c| !c.is_control()) {
            self.chars.insert(self.cursor, c);
            self.cursor += 1;
        }
    }

    /// Display columns before the cursor.
    pub fn cursor_cols(&self) -> usize {
        chars_cols(&self.chars[..self.cursor])
    }

    /// Selects `[start, end)` replacing it with `text`, cursor at the end.
    pub fn replace_range(&mut self, start: usize, end: usize, text: &str) {
        let end = end.min(self.chars.len());
        let start = start.min(end);
        self.chars.splice(start..end, text.chars());
        self.cursor = start + text.chars().count();
    }

    /// Applies an editing key. Returns `true` if the key was consumed.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let len = self.chars.len();
        match key.code {
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = len,
            KeyCode::Char('b') if ctrl => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Char('f') if ctrl => self.cursor = (self.cursor + 1).min(len),
            KeyCode::Char('u') if ctrl => {
                self.chars.drain(..self.cursor);
                self.cursor = 0;
            }
            KeyCode::Char('k') if ctrl => self.chars.truncate(self.cursor),
            KeyCode::Char('w') if ctrl => self.delete_prev_word(),
            KeyCode::Char('h') if ctrl => self.backspace(),
            KeyCode::Char('d') if ctrl => self.delete(),
            KeyCode::Char('b') if alt => self.cursor = prev_word_boundary(&self.chars, self.cursor),
            KeyCode::Char('f') if alt => self.cursor = next_word_boundary(&self.chars, self.cursor),
            KeyCode::Char(c) if !ctrl && !alt => {
                self.chars.insert(self.cursor, c);
                self.cursor += 1;
            }
            KeyCode::Backspace if ctrl || alt => self.delete_prev_word(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left if ctrl || alt => {
                self.cursor = prev_word_boundary(&self.chars, self.cursor)
            }
            KeyCode::Right if ctrl || alt => {
                self.cursor = next_word_boundary(&self.chars, self.cursor)
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(len),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = len,
            _ => return false,
        }
        true
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
        }
    }

    fn delete(&mut self) {
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }

    fn delete_prev_word(&mut self) {
        let start = prev_word_boundary(&self.chars, self.cursor);
        self.chars.drain(start..self.cursor);
        self.cursor = start;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn editing_keys() {
        let mut l = LineInput::with_text("-p 22 leo@host");
        l.handle_key(key(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(l.text(), "-p 22 ");
        l.handle_key(key(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(l.text(), "-p 22 x");
        l.handle_key(key(KeyCode::Home, KeyModifiers::NONE));
        l.handle_key(key(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(l.text(), "p 22 x");
        l.replace_range(5, 6, "host");
        assert_eq!((l.text().as_str(), l.cursor), ("p 22 host", 9));
    }
}
