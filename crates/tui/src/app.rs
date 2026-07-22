use std::{fs, path::PathBuf};

use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use typst_tui_document::Document;

use crate::{action::Action, components::Editor, event, input};

pub(crate) struct App {
    document: Document,
    editor: Editor,
    path: Option<PathBuf>,
    display_name: String,
    quit_confirmation: bool,
    should_quit: bool,
    status: Option<String>,
}

impl App {
    pub(crate) fn new(path: Option<PathBuf>, text: &str) -> Self {
        let display_name = path
            .as_deref()
            .and_then(|path| path.file_name())
            .map_or_else(
                || "Untitled".to_owned(),
                |name| name.to_string_lossy().into(),
            );

        Self {
            document: Document::new(text),
            editor: Editor::default(),
            path,
            display_name,
            quit_confirmation: false,
            should_quit: false,
            status: None,
        }
    }

    pub(crate) fn run(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        while !self.should_quit {
            terminal.draw(|frame| self.draw(frame))?;
            if let Some(action) = input::resolve(event::read()?, self.quit_confirmation) {
                self.update(action);
            }
        }

        Ok(())
    }

    fn update(&mut self, action: Action) {
        match action {
            Action::Save => self.save(),
            Action::RequestQuit if self.document.is_dirty() => {
                self.quit_confirmation = true;
                self.status = None;
            }
            Action::RequestQuit | Action::Quit => self.should_quit = true,
            Action::CancelQuit => self.quit_confirmation = false,
            _ => {
                self.editor.update(&action, &mut self.document);
                self.status = None;
            }
        }
    }

    fn save(&mut self) {
        let Some(path) = self.path.as_deref() else {
            self.status = Some("Cannot save an untitled buffer".to_owned());
            return;
        };

        match fs::write(path, self.document.text()) {
            Ok(()) => {
                self.document.mark_saved();
                self.status = Some("Saved".to_owned());
            }
            Err(error) => self.status = Some(format!("Save failed: {error}")),
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        let [header_area, editor_area, status_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        let dirty_marker = if self.document.is_dirty() { " *" } else { "" };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" typst-tui ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!("{}{}", self.display_name, dirty_marker)),
            ])),
            header_area,
        );
        self.editor.draw(frame, editor_area, &self.document, true);

        let cursor = self.document.cursor_position();
        let (status, foreground) = if self.quit_confirmation {
            (
                "Unsaved changes. Press Y to quit, N or Esc to cancel".to_owned(),
                Color::Yellow,
            )
        } else if let Some(status) = &self.status {
            (status.clone(), Color::White)
        } else {
            (
                format!("Ln {}, Col {}", cursor.line + 1, cursor.column + 1),
                Color::Gray,
            )
        };
        frame.render_widget(
            Paragraph::new(format!(" {status}"))
                .style(Style::default().fg(foreground).bg(Color::DarkGray)),
            status_area,
        );
    }
}
