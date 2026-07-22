use std::collections::BTreeMap;

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};
use typst_tui_compiler::{Diagnostic, Severity};

#[derive(Debug, Default)]
pub(crate) struct Diagnostics {
    items: Vec<Diagnostic>,
    line_severity: BTreeMap<usize, Severity>,
    selected: Option<usize>,
    scroll: usize,
    visible: bool,
}

impl Diagnostics {
    pub(crate) fn set_items(&mut self, items: Vec<Diagnostic>) {
        self.items = items;
        self.selected = self.selected.filter(|index| *index < self.items.len());
        self.scroll = 0;
        self.line_severity.clear();
        for diagnostic in &self.items {
            let Some(line) = diagnostic.line.filter(|_| diagnostic.is_main) else {
                continue;
            };
            let severity = self
                .line_severity
                .entry(line)
                .or_insert(diagnostic.severity);
            if diagnostic.severity == Severity::Error {
                *severity = Severity::Error;
            }
        }
    }

    pub(crate) fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.visible
    }

    pub(crate) fn drawer_height(&self, available: u16) -> u16 {
        available.min(8).min(available.div_ceil(3).max(3))
    }

    pub(crate) fn severity_at(&self, line: usize) -> Option<Severity> {
        self.line_severity.get(&line).copied()
    }

    pub(crate) fn errors(&self) -> usize {
        self.items
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .count()
    }

    pub(crate) fn warnings(&self) -> usize {
        self.items.len().saturating_sub(self.errors())
    }

    pub(crate) fn select(&mut self, direction: isize) -> Option<&Diagnostic> {
        if self.items.is_empty() {
            self.selected = None;
            return None;
        }

        self.selected = Some(match self.selected {
            None if direction < 0 => self.items.len() - 1,
            None => 0,
            Some(0) if direction < 0 => self.items.len() - 1,
            Some(index) if direction < 0 => index - 1,
            Some(index) => (index + 1) % self.items.len(),
        });
        self.selected.and_then(|index| self.items.get(index))
    }

    pub(crate) fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let title = format!(
            " Diagnostics | {} errors, {} warnings ",
            self.errors(),
            self.warnings()
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan))
            .title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        if self.items.is_empty() {
            frame.render_widget(Paragraph::new("No diagnostics"), inner);
            return;
        }

        self.keep_selection_visible(usize::from(inner.height));
        let end = (self.scroll + usize::from(inner.height)).min(self.items.len());
        let lines = self.items[self.scroll..end]
            .iter()
            .enumerate()
            .map(|(offset, diagnostic)| {
                let severity_color = match diagnostic.severity {
                    Severity::Error => Color::Red,
                    Severity::Warning => Color::Yellow,
                };
                let severity = match diagnostic.severity {
                    Severity::Error => "E ",
                    Severity::Warning => "W ",
                };
                let line = Line::from(vec![
                    Span::styled(severity, Style::default().fg(severity_color)),
                    Span::raw(format_diagnostic(diagnostic)),
                ]);
                if self.selected == Some(self.scroll + offset) {
                    line.style(Style::default().bg(Color::DarkGray))
                } else {
                    line
                }
            })
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn keep_selection_visible(&mut self, height: usize) {
        let Some(selected) = self.selected else {
            return;
        };
        if selected < self.scroll {
            self.scroll = selected;
        } else if selected >= self.scroll + height.max(1) {
            self.scroll = selected + 1 - height.max(1);
        }
    }
}

pub(crate) fn format_diagnostic(diagnostic: &Diagnostic) -> String {
    let message = diagnostic.message.replace(['\r', '\n'], " ");
    match (&diagnostic.path, diagnostic.line, diagnostic.column) {
        (Some(path), Some(line), Some(column)) => {
            format!("{path}:{}:{}: {message}", line + 1, column + 1)
        }
        _ => message,
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use ratatui::{Terminal, backend::TestBackend};
    use typst_tui_compiler::{Diagnostic, Severity};

    use super::Diagnostics;

    fn diagnostic(severity: Severity, line: usize, message: &str) -> Diagnostic {
        Diagnostic {
            severity,
            message: message.to_owned(),
            path: Some("main.typ".to_owned()),
            line: Some(line),
            column: Some(2),
            is_main: true,
        }
    }

    #[test]
    fn draws_and_navigates_diagnostics() -> Result<(), Infallible> {
        let mut diagnostics = Diagnostics::default();
        diagnostics.set_items(vec![
            diagnostic(Severity::Warning, 1, "first warning"),
            diagnostic(Severity::Error, 1, "blocking error"),
        ]);
        diagnostics.toggle();

        assert_eq!(diagnostics.severity_at(1), Some(Severity::Error));
        assert_eq!(
            diagnostics.select(1).map(|item| item.message.as_str()),
            Some("first warning")
        );
        assert_eq!(
            diagnostics.select(-1).map(|item| item.message.as_str()),
            Some("blocking error")
        );

        let backend = TestBackend::new(50, 6);
        let mut terminal = Terminal::new(backend)?;
        terminal.draw(|frame| diagnostics.draw(frame, frame.area()))?;

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("1 errors, 1 warnings"));
        assert!(rendered.contains("blocking error"));

        Ok(())
    }
}
