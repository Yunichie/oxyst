use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use typst_tui_theme::Theme;

use crate::style::{base, color};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WelcomeChoice {
    NewDocument,
    OpenFile,
    OpenRecent,
}

#[derive(Debug, Default)]
pub(crate) struct Welcome {
    selected: usize,
}

impl Welcome {
    const CHOICES: &[(WelcomeChoice, &str)] = &[
        (WelcomeChoice::NewDocument, "New document"),
        (WelcomeChoice::OpenFile, "Open file"),
        (WelcomeChoice::OpenRecent, "Open recent (none)"),
    ];

    pub(crate) fn move_selection(&mut self, direction: isize) {
        self.selected = self
            .selected
            .saturating_add_signed(direction)
            .min(Self::CHOICES.len() - 1);
    }

    pub(crate) fn selected(&self) -> WelcomeChoice {
        Self::CHOICES[self.selected].0
    }

    pub(crate) fn draw(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        frame.render_widget(Block::default().style(base(theme)), area);
        let mut lines = vec![
            Line::raw("typst-tui").style(
                Style::default()
                    .fg(color(theme.accent))
                    .add_modifier(Modifier::BOLD),
            ),
            Line::raw(""),
        ];
        lines.extend(Self::CHOICES.iter().enumerate().map(|(index, (_, label))| {
            let marker = if index == self.selected { ">" } else { " " };
            let line = Line::from(vec![Span::raw(format!("{marker} {label}"))]);
            if index == self.selected {
                line.style(Style::default().bg(color(theme.selection)))
            } else if index == 2 {
                line.style(Style::default().fg(color(theme.muted)))
            } else {
                line
            }
        }));
        lines.extend([
            Line::raw(""),
            Line::raw("Enter to choose  |  ? for help")
                .style(Style::default().fg(color(theme.muted))),
        ]);
        frame.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Center)
                .style(base(theme)),
            Rect::new(
                area.x,
                area.y.saturating_add(area.height.saturating_sub(8) / 2),
                area.width,
                8.min(area.height),
            ),
        );
    }
}
