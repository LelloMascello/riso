//! TUI: modal overlay dialogs (prompt, confirm, open-with, drop choice, help).

use std::path::PathBuf;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use super::drop::parse_dropped_paths;
use super::event::OpenWith;
use super::line_input::LineInput;
use super::theme;
use crate::files::Entry;

#[derive(Clone)]
pub enum PromptKind {
    Rename(Entry),
    NewFile,
    NewDir,
    GoToPath,
    Upload { dest: String },
    Download { entries: Vec<Entry> },
    OpenWithCmd(Entry),
}

#[derive(Clone)]
pub enum ConfirmKind {
    Delete(Vec<Entry>),
    OverwriteUpload { files: Vec<PathBuf>, dest: String },
    CloseActiveTab(usize),
}

pub enum Dialog {
    Prompt {
        title: String,
        hint: String,
        input: LineInput,
        kind: PromptKind,
    },
    Confirm {
        title: String,
        body: String,
        accept: String,
        destructive: bool,
        kind: ConfirmKind,
    },
    DropChoice {
        raw: String,
        paths: Vec<PathBuf>,
        dest: String,
    },
    OpenWithPick {
        entry: Entry,
        selected: usize,
    },
    Help,
}

pub enum DialogResult {
    None,
    Close,
    PromptSubmit { kind: PromptKind, value: String },
    ConfirmYes(ConfirmKind),
    DropUpload { paths: Vec<PathBuf>, dest: String },
    DropPasteTerminal(String),
    OpenWithChosen { entry: Entry, how: OpenWith },
}

impl Dialog {
    pub fn prompt(title: impl Into<String>, hint: impl Into<String>, initial: &str, kind: PromptKind) -> Self {
        Dialog::Prompt {
            title: title.into(),
            hint: hint.into(),
            input: LineInput::with_text(initial),
            kind,
        }
    }

    pub fn confirm(
        title: impl Into<String>,
        body: impl Into<String>,
        accept: impl Into<String>,
        destructive: bool,
        kind: ConfirmKind,
    ) -> Self {
        Dialog::Confirm {
            title: title.into(),
            body: body.into(),
            accept: accept.into(),
            destructive,
            kind,
        }
    }

    pub fn handle_paste(&mut self, text: &str) {
        let Dialog::Prompt { input, kind, .. } = self else { return };
        if matches!(kind, PromptKind::Upload { .. } | PromptKind::Download { .. })
            && let Some(dropped) = parse_dropped_paths(text)
        {
            let formatted: Vec<String> = dropped
                .paths
                .iter()
                .map(|p| {
                    let s = p.to_string_lossy();
                    if s.contains(' ') {
                        crate::remote::shell_quote(&s)
                    } else {
                        s.into_owned()
                    }
                })
                .collect();
            if matches!(kind, PromptKind::Download { .. }) {
                if let Some(first) = dropped.paths.first() {
                    let dir = if first.is_dir() {
                        first.as_path()
                    } else {
                        first.parent().unwrap_or(first.as_path())
                    };
                    input.set_text(&dir.to_string_lossy());
                }
            } else {
                let joined = formatted.join(" ");
                if input.is_empty() {
                    input.set_text(&joined);
                } else {
                    input.insert_str(" ");
                    input.insert_str(&joined);
                }
            }
            return;
        }
        input.insert_str(text.lines().next().unwrap_or_default());
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match self {
            Dialog::Prompt { input, kind, .. } => match key.code {
                KeyCode::Esc => DialogResult::Close,
                KeyCode::Enter => {
                    let value = input.text().trim().to_string();
                    if value.is_empty() {
                        DialogResult::Close
                    } else {
                        DialogResult::PromptSubmit {
                            kind: kind.clone(),
                            value,
                        }
                    }
                }
                _ => {
                    input.handle_key(key);
                    DialogResult::None
                }
            },
            Dialog::Confirm { kind, .. } => match key.code {
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => DialogResult::Close,
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Char('s') | KeyCode::Char('S') => {
                    DialogResult::ConfirmYes(kind.clone())
                }
                _ => DialogResult::None,
            },
            Dialog::DropChoice { raw, paths, dest } => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => DialogResult::Close,
                KeyCode::Enter | KeyCode::Char('u') | KeyCode::Char('U') => DialogResult::DropUpload {
                    paths: paths.clone(),
                    dest: dest.clone(),
                },
                KeyCode::Char('v') | KeyCode::Char('V') | KeyCode::Char('p') => {
                    DialogResult::DropPasteTerminal(raw.clone())
                }
                _ => DialogResult::None,
            },
            Dialog::OpenWithPick { entry, selected } => match key.code {
                KeyCode::Esc => DialogResult::Close,
                KeyCode::Up | KeyCode::Char('k') => {
                    *selected = selected.saturating_sub(1);
                    DialogResult::None
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    *selected = (*selected + 1).min(2);
                    DialogResult::None
                }
                KeyCode::Enter => match *selected {
                    0 => DialogResult::OpenWithChosen {
                        entry: entry.clone(),
                        how: OpenWith::Editor,
                    },
                    1 => DialogResult::OpenWithChosen {
                        entry: entry.clone(),
                        how: OpenWith::Default,
                    },
                    _ => DialogResult::PromptSubmit {
                        kind: PromptKind::OpenWithCmd(entry.clone()),
                        value: String::new(),
                    },
                },
                _ => DialogResult::None,
            },
            Dialog::Help => match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::F(1) => {
                    DialogResult::Close
                }
                _ => DialogResult::None,
            },
        }
    }

    pub fn render(&self, f: &mut Frame, area: Rect) -> Option<(u16, u16)> {
        match self {
            Dialog::Prompt { title, hint, input, .. } => {
                let w = 64u16.min(area.width.saturating_sub(4)).max(24);
                let h = if hint.is_empty() { 5 } else { 6 };
                let rect = centered(area, w, h);
                f.render_widget(Clear, rect);
                let block = Block::new()
                    .borders(Borders::ALL)
                    .border_style(theme::border(true))
                    .title(Span::styled(format!(" {title} "), theme::title(true)));
                let inner = block.inner(rect);
                f.render_widget(block, rect);

                let rows = if hint.is_empty() {
                    Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(inner)
                } else {
                    Layout::vertical([
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(1),
                    ])
                    .split(inner)
                };

                let input_row = rows[0];
                let txt = input.text();
                let max_w = input_row.width as usize;
                let cur_cols = input.cursor_cols();
                let scroll_cols = cur_cols.saturating_sub(max_w.saturating_sub(1));
                let visible = slice_from_col(&txt, scroll_cols, max_w);
                f.render_widget(Paragraph::new(visible), input_row);

                if !hint.is_empty() {
                    f.render_widget(Paragraph::new(Span::styled(hint.clone(), theme::dim())), rows[1]);
                }
                let footer = rows[rows.len() - 1];
                f.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled("Invio", theme::key()),
                        Span::styled(" conferma  ", theme::dim()),
                        Span::styled("Esc", theme::key()),
                        Span::styled(" annulla", theme::dim()),
                    ])),
                    footer,
                );
                let cx = input_row.x + (cur_cols - scroll_cols) as u16;
                Some((cx.min(input_row.right().saturating_sub(1)), input_row.y))
            }
            Dialog::Confirm { title, body, accept, destructive, .. } => {
                let w = 56u16.min(area.width.saturating_sub(4)).max(24);
                let h = if body.is_empty() { 5 } else { 7 };
                let rect = centered(area, w, h);
                f.render_widget(Clear, rect);
                let bstyle = if *destructive { theme::error() } else { theme::border(true) };
                let block = Block::new()
                    .borders(Borders::ALL)
                    .border_style(bstyle)
                    .title(Span::styled(format!(" {title} "), theme::title(true)));
                let inner = block.inner(rect);
                f.render_widget(block, rect);

                let mut lines = Vec::new();
                if !body.is_empty() {
                    lines.push(Line::from(Span::styled(body.clone(), theme::dim())));
                    lines.push(Line::default());
                }
                lines.push(Line::from(vec![
                    Span::styled("y/Invio", if *destructive { theme::error() } else { theme::key() }),
                    Span::raw(format!(" {accept}   ")),
                    Span::styled("n/Esc", theme::key()),
                    Span::styled(" Annulla", theme::dim()),
                ]));
                f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
                None
            }
            Dialog::DropChoice { paths, dest, .. } => {
                let w = 62u16.min(area.width.saturating_sub(4)).max(30);
                let h = 7;
                let rect = centered(area, w, h);
                f.render_widget(Clear, rect);
                let block = Block::new()
                    .borders(Borders::ALL)
                    .border_style(theme::border(true))
                    .title(Span::styled(" File locali rilevati ", theme::title(true)));
                let inner = block.inner(rect);
                f.render_widget(block, rect);

                let summary = if paths.len() == 1 {
                    paths[0].file_name().unwrap_or_default().to_string_lossy().into_owned()
                } else {
                    format!("{} elementi", paths.len())
                };
                let lines = vec![
                    Line::from(format!("Rilevato: {summary}")),
                    Line::from(Span::styled(format!("Destinazione SFTP: {dest}"), theme::dim())),
                    Line::default(),
                    Line::from(vec![
                        Span::styled("u/Invio", theme::key()),
                        Span::raw(" Carica via SFTP   "),
                        Span::styled("v", theme::key()),
                        Span::raw(" Incolla nel terminale   "),
                        Span::styled("Esc", theme::key()),
                        Span::styled(" Annulla", theme::dim()),
                    ]),
                ];
                f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
                None
            }
            Dialog::OpenWithPick { entry, selected } => {
                let w = 48u16.min(area.width.saturating_sub(4)).max(24);
                let h = 7;
                let rect = centered(area, w, h);
                f.render_widget(Clear, rect);
                let block = Block::new()
                    .borders(Borders::ALL)
                    .border_style(theme::border(true))
                    .title(Span::styled(format!(" Apri {} con… ", entry.name), theme::title(true)));
                let inner = block.inner(rect);
                f.render_widget(block, rect);

                let opts = [
                    "Editor terminale ($EDITOR)",
                    "Applicazione predefinita (xdg-open)",
                    "Comando personalizzato…",
                ];
                let mut lines: Vec<Line> = opts
                    .iter()
                    .enumerate()
                    .map(|(i, label)| {
                        let s = if i == *selected { theme::selection(true) } else { theme::base() };
                        Line::from(Span::styled(format!(" {label} "), s))
                    })
                    .collect();
                lines.push(Line::default());
                lines.push(Line::from(Span::styled("j/k scegli  Invio apri  Esc annulla", theme::dim())));
                f.render_widget(Paragraph::new(lines), inner);
                None
            }
            Dialog::Help => {
                let w = 74u16.min(area.width.saturating_sub(2)).max(36);
                let h = 25u16.min(area.height.saturating_sub(2)).max(12);
                let rect = centered(area, w, h);
                f.render_widget(Clear, rect);
                let block = Block::new()
                    .borders(Borders::ALL)
                    .border_style(theme::border(true))
                    .title(Span::styled(" Scorciatoie riso-tui ", theme::title(true)));
                let inner = block.inner(rect);
                f.render_widget(block, rect);

                let row = |k: &'static str, d: &'static str| {
                    Line::from(vec![
                        Span::styled(format!("  {k:<16}"), theme::key()),
                        Span::raw(d),
                    ])
                };
                let lines = vec![
                    Line::from(Span::styled("Generale", theme::accent())),
                    row("Alt+n / Alt+w", "Nuova scheda / Chiudi scheda"),
                    row("Alt+[ / Alt+]", "Scheda precedente / successiva (o Alt+1..9)"),
                    row("Alt+s / Tab*", "Cambia focus Terminale ↔ Explorer SFTP"),
                    row("Alt+b", "Mostra / Nascondi barra laterale SFTP"),
                    row("Alt+r", "Ridisegna schermo"),
                    row("Alt+q", "Esci"),
                    Line::default(),
                    Line::from(Span::styled("Explorer SFTP (stile Yazi)", theme::accent())),
                    row("j/k, ↑/↓, Ctrl+u/d", "Naviga riga per riga o mezza pagina"),
                    row("l/Invio / h/Bksp", "Entra/apri ($EDITOR) / Cartella superiore"),
                    row("gg / G / gh / g/", "Cima / Fondo / Home (~) / Root (/)"),
                    row("Spazio / v / a", "Seleziona / Selezione visuale / Tutto"),
                    row("/ / , / .", "Filtra / Ordina (Nome, Dim, Mod) / Nascosti"),
                    row("y / x / p / Y", "Copia / Taglia / Incolla / Svuota appunti"),
                    row("n / N / r / d", "Nuovo file / Nuova cartella / Rinomina / Elimina"),
                    row("o / O / u / D", "Apri app / Apri con… / Carica / Scarica"),
                    row("c / f / P", "cd terminale qui / Segui terminale / Vai a…"),
                    row("Mouse", "1 click apre cartella · 2 click apre file · Ctrl selez."),
                    Line::default(),
                    Line::from(Span::styled("Drag & Drop", theme::accent())),
                    Line::from("  • Trascina file locali sulla finestra per caricarli via SFTP"),
                    Line::from("  • Trascina voci SFTP su cartella (sposta), terminale (percorso) o [↓]"),
                ];
                f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
                None
            }
        }
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

fn slice_from_col(s: &str, skip_cols: usize, max_cols: usize) -> String {
    let mut col = 0;
    let mut out = String::new();
    for c in s.chars() {
        let cw = crate::line_edit::char_cols(c);
        if col >= skip_cols {
            if out.chars().map(crate::line_edit::char_cols).sum::<usize>() + cw > max_cols {
                break;
            }
            out.push(c);
        }
        col += cw;
    }
    out
}
