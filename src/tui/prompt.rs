//! TUI: `[user@host ~]$ ssh ` prompt state rendered directly inside the
//! session's embedded VT100 terminal (no banners, no suggestion popups).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::line_input::LineInput;
use super::term::Term;
use crate::config::{HostStore, local_prompt_prefix};
use crate::line_edit::{chars_cols, rows_above_origin, str_cols};

pub enum PromptAction {
    None,
    Submit(String),
}

pub struct Prompt {
    pub prefix: String,
    pub input: LineInput,
    history_pos: Option<usize>,
    draft: String,
}

impl Prompt {
    pub fn new() -> Self {
        Prompt {
            prefix: local_prompt_prefix(),
            input: LineInput::default(),
            history_pos: None,
            draft: String::new(),
        }
    }

    pub fn write_initial(&self, term: &mut Term) {
        term.process(self.prefix.as_bytes());
    }

    pub fn reset_into(&mut self, term: &mut Term) {
        let (_, col) = term.screen().cursor_position();
        if col > 0 {
            term.process(b"\r\n");
        }
        self.input = LineInput::default();
        self.history_pos = None;
        self.draft.clear();
        term.process(self.prefix.as_bytes());
    }

    pub fn insert_str(&mut self, s: &str, term: &mut Term) {
        let line = s.lines().next().unwrap_or_default();
        if line.is_empty() {
            return;
        }
        self.mutate_in(term, |input| input.insert_str(line));
    }

    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        store: &HostStore,
        term: &mut Term,
    ) -> PromptAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Enter => {
                let text = self.input.text();
                if text.trim().is_empty() {
                    term.process(b"\r\n");
                    self.input = LineInput::default();
                    self.history_pos = None;
                    self.draft.clear();
                    term.process(self.prefix.as_bytes());
                    return PromptAction::None;
                }
                term.process(b"\r\n");
                return PromptAction::Submit(text);
            }
            KeyCode::Char('c') if ctrl => {
                term.process(b"^C\r\n");
                self.input = LineInput::default();
                self.history_pos = None;
                self.draft.clear();
                term.process(self.prefix.as_bytes());
                return PromptAction::None;
            }
            KeyCode::Char('l') if ctrl => {
                let line = format!("\x1b[2J\x1b[H{}{}", self.prefix, self.input.text());
                term.process(line.as_bytes());
                self.reposition_cursor(term);
                return PromptAction::None;
            }
            KeyCode::Up | KeyCode::Char('p') if (key.code == KeyCode::Up || ctrl) => {
                self.step_history(true, store, term);
                return PromptAction::None;
            }
            KeyCode::Down | KeyCode::Char('n') if (key.code == KeyCode::Down || ctrl) => {
                self.step_history(false, store, term);
                return PromptAction::None;
            }
            _ => {}
        }

        self.mutate_in(term, |input| {
            if input.handle_key(key) {
                // Any direct edit leaves history browsing.
            }
        });
        if !matches!(key.code, KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End) {
            self.history_pos = None;
        }
        PromptAction::None
    }

    fn step_history(&mut self, older: bool, store: &HostStore, term: &mut Term) {
        let hist = store.history();
        if hist.is_empty() {
            return;
        }
        let next = match (self.history_pos, older) {
            (None, true) => {
                self.draft = self.input.text();
                Some(0)
            }
            (None, false) => None,
            (Some(i), true) => Some((i + 1).min(hist.len() - 1)),
            (Some(0), false) => None,
            (Some(i), false) => Some(i - 1),
        };
        self.history_pos = next;
        let text = match next {
            Some(i) => hist[i].clone(),
            None => std::mem::take(&mut self.draft),
        };
        self.mutate_in(term, |input| input.set_text(&text));
    }

    fn mutate_in(&mut self, term: &mut Term, f: impl FnOnce(&mut LineInput)) {
        let old_vis_cols = str_cols(&self.prefix) + self.input.cursor_cols();
        f(&mut self.input);
        let full_text = self.input.text();
        let cols = term.screen().size().1.max(1) as usize;
        let cur_col = term.screen().cursor_position().1 as usize;
        let up = rows_above_origin(old_vis_cols, cur_col, cols);

        let mut seq = String::with_capacity(self.prefix.len() + full_text.len() + 16);
        if up > 0 {
            seq.push_str(&format!("\x1b[{up}A"));
        }
        seq.push_str("\r\x1b[J");
        seq.push_str(&self.prefix);
        seq.push_str(&full_text);
        term.process(seq.as_bytes());
        self.reposition_cursor(term);
    }

    fn reposition_cursor(&self, term: &mut Term) {
        if self.input.cursor >= self.input.chars.len() {
            return;
        }
        let cols = term.screen().size().1.max(1) as usize;
        let full_vis_cols = str_cols(&self.prefix) + chars_cols(&self.input.chars);
        let end_col = term.screen().cursor_position().1 as usize;
        let end_up = rows_above_origin(full_vis_cols, end_col, cols);
        let cursor_text: String = self.input.chars[..self.input.cursor].iter().collect();

        let mut pos_seq = String::with_capacity(self.prefix.len() + cursor_text.len() + 16);
        if end_up > 0 {
            pos_seq.push_str(&format!("\x1b[{end_up}A"));
        }
        pos_seq.push('\r');
        pos_seq.push_str(&self.prefix);
        pos_seq.push_str(&cursor_text);
        term.process(pos_seq.as_bytes());
    }
}
