use std::path::PathBuf;

use oxyst_theme::Theme;
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::Line,
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use super::{Component, modal_area};
use crate::{
    action::Action,
    style::{base, color},
};

#[derive(Debug)]
pub(crate) struct RecentPicker {
    entries: Vec<PathBuf>,
    selected: usize,
    theme: Theme,
}

impl RecentPicker {
    pub(crate) fn new(entries: Vec<PathBuf>, theme: Theme) -> Self {
        Self {
            entries,
            selected: 0,
            theme,
        }
    }

    pub(crate) fn selected(&self) -> Option<PathBuf> {
        self.entries.get(self.selected).cloned()
    }

    fn move_selection(&mut self, direction: isize) {
        if self.entries.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self
                .selected
                .saturating_add_signed(direction)
                .min(self.entries.len() - 1);
        }
    }
}

impl Component for RecentPicker {
    fn update(&mut self, action: Action) {
        if let Action::OverlayMove(direction) = action {
            self.move_selection(direction);
        }
    }

    fn draw(&mut self, frame: &mut Frame, _area: Rect, _focused: bool) {
        let height = u16::try_from(self.entries.len())
            .unwrap_or(u16::MAX)
            .saturating_add(2)
            .min(14);
        let area = modal_area(frame.area(), 72, height.max(3));
        frame.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color(self.theme.accent)))
            .style(base(&self.theme))
            .title(" Recent files ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let lines = self
            .entries
            .iter()
            .enumerate()
            .skip(
                self.selected
                    .saturating_sub(usize::from(inner.height).saturating_sub(1)),
            )
            .take(usize::from(inner.height))
            .map(|(index, path)| {
                let line = Line::raw(format!(" {}", path.display()));
                if index == self.selected {
                    line.style(Style::default().bg(color(self.theme.selection)))
                } else {
                    line
                }
            })
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

#[cfg(test)]
mod tests {
    use std::{convert::Infallible, path::PathBuf};

    use oxyst_theme::{ColorDepth, Theme, ThemeName};
    use ratatui::{Terminal, backend::TestBackend};

    use super::{Action, Component, RecentPicker};

    #[test]
    fn navigates_and_draws_recent_paths() -> Result<(), Infallible> {
        let theme = Theme::new(ThemeName::Dark, ColorDepth::Ansi16);
        let second = PathBuf::from("project/second.typ");
        let mut picker = RecentPicker::new(
            vec![PathBuf::from("project/first.typ"), second.clone()],
            theme,
        );
        picker.update(Action::OverlayMove(1));
        assert_eq!(picker.selected(), Some(second));

        let mut terminal = Terminal::new(TestBackend::new(80, 8))?;
        terminal.draw(|frame| picker.draw(frame, frame.area(), true))?;
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Recent files"));
        assert!(rendered.contains("second.typ"));
        Ok(())
    }
}
