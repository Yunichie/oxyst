use oxyst_theme::Theme;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};

use crate::{
    action::Action,
    style::{base, color},
};

use super::Component;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WelcomeChoice {
    NewDocument,
    OpenFile,
    OpenRecent,
}

#[derive(Debug)]
pub(crate) struct Welcome {
    selected: usize,
    recent_count: usize,
    choose_binding: String,
    help_binding: String,
    theme: Theme,
}

impl Welcome {
    const CHOICES: &[(WelcomeChoice, &str)] = &[
        (WelcomeChoice::NewDocument, "New document"),
        (WelcomeChoice::OpenFile, "Open file"),
        (WelcomeChoice::OpenRecent, "Open recent"),
    ];

    pub(crate) fn new(theme: Theme, choose_binding: String, help_binding: String) -> Self {
        Self {
            selected: 0,
            recent_count: 0,
            choose_binding,
            help_binding,
            theme,
        }
    }

    pub(crate) fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    pub(crate) fn set_recent_count(&mut self, count: usize) {
        self.recent_count = count;
    }

    pub(crate) fn move_selection(&mut self, direction: isize) {
        self.selected = self
            .selected
            .saturating_add_signed(direction)
            .min(Self::CHOICES.len() - 1);
    }

    pub(crate) fn selected(&self) -> WelcomeChoice {
        Self::CHOICES[self.selected].0
    }

    fn draw_welcome(&self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Block::default().style(base(&self.theme)), area);
        let mut lines = vec![
            Line::raw("oxyst").style(
                Style::default()
                    .fg(color(self.theme.accent))
                    .add_modifier(Modifier::BOLD),
            ),
            Line::raw(""),
        ];
        lines.extend(Self::CHOICES.iter().enumerate().map(|(index, (_, label))| {
            let marker = if index == self.selected { ">" } else { " " };
            let label = if index == 2 {
                format!("{label} ({})", self.recent_count)
            } else {
                (*label).to_owned()
            };
            let line = Line::from(vec![Span::raw(format!("{marker} {label}"))]);
            if index == self.selected {
                line.style(Style::default().bg(color(self.theme.selection)))
            } else if index == 2 && self.recent_count == 0 {
                line.style(Style::default().fg(color(self.theme.muted)))
            } else {
                line
            }
        }));
        lines.extend([
            Line::raw(""),
            Line::raw(format!(
                "{} choose  |  {} help",
                self.choose_binding, self.help_binding
            ))
            .style(Style::default().fg(color(self.theme.muted))),
        ]);
        frame.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Center)
                .style(base(&self.theme)),
            Rect::new(
                area.x,
                area.y.saturating_add(area.height.saturating_sub(8) / 2),
                area.width,
                8.min(area.height),
            ),
        );
    }
}

impl Default for Welcome {
    fn default() -> Self {
        Self::new(
            Theme::new(
                oxyst_theme::ThemeName::Dark,
                oxyst_theme::ColorDepth::Ansi16,
            ),
            "enter".to_owned(),
            "?".to_owned(),
        )
    }
}

impl Component for Welcome {
    fn update(&mut self, action: Action) {
        if let Action::OverlayMove(direction) = action {
            self.move_selection(direction);
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, _focused: bool) {
        self.draw_welcome(frame, area);
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use oxyst_theme::{ColorDepth, Theme, ThemeName};
    use ratatui::{Terminal, backend::TestBackend};

    use super::{Action, Component, Welcome, WelcomeChoice};

    #[test]
    fn draws_and_selects_available_recent_files() -> Result<(), Infallible> {
        let theme = Theme::new(ThemeName::Dark, ColorDepth::Ansi16);
        let mut welcome = Welcome::new(theme, "ctrl+enter".to_owned(), "alt+h".to_owned());
        welcome.set_recent_count(2);
        welcome.update(Action::OverlayMove(2));
        assert_eq!(welcome.selected(), WelcomeChoice::OpenRecent);

        let mut terminal = Terminal::new(TestBackend::new(50, 12))?;
        terminal.draw(|frame| welcome.draw(frame, frame.area(), true))?;
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Open recent (2)"));
        assert!(rendered.contains("ctrl+enter choose"));
        assert!(rendered.contains("alt+h help"));
        Ok(())
    }
}
