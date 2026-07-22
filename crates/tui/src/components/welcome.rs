use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use typst_tui_theme::Theme;

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
    theme: Theme,
}

impl Welcome {
    const CHOICES: &[(WelcomeChoice, &str)] = &[
        (WelcomeChoice::NewDocument, "New document"),
        (WelcomeChoice::OpenFile, "Open file"),
        (WelcomeChoice::OpenRecent, "Open recent (none)"),
    ];

    pub(crate) fn new(theme: Theme) -> Self {
        Self { selected: 0, theme }
    }

    pub(crate) fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
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
            Line::raw("typst-tui").style(
                Style::default()
                    .fg(color(self.theme.accent))
                    .add_modifier(Modifier::BOLD),
            ),
            Line::raw(""),
        ];
        lines.extend(Self::CHOICES.iter().enumerate().map(|(index, (_, label))| {
            let marker = if index == self.selected { ">" } else { " " };
            let line = Line::from(vec![Span::raw(format!("{marker} {label}"))]);
            if index == self.selected {
                line.style(Style::default().bg(color(self.theme.selection)))
            } else if index == 2 {
                line.style(Style::default().fg(color(self.theme.muted)))
            } else {
                line
            }
        }));
        lines.extend([
            Line::raw(""),
            Line::raw("Enter to choose  |  ? for help")
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
        Self::new(Theme::new(
            typst_tui_theme::ThemeName::Dark,
            typst_tui_theme::ColorDepth::Ansi16,
        ))
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
