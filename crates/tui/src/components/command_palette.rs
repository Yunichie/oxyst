use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use typst_tui_render::ExportFormat;
use typst_tui_theme::Theme;
use unicode_width::UnicodeWidthStr;

use crate::style::{base, color};

use super::modal_area;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Command {
    OpenFile,
    Export(ExportFormat),
    GoToLine,
    GoToPage,
    ToggleDiagnostics,
    ToggleFileExplorer,
    UseDarkTheme,
    UseLightTheme,
    ReloadFonts,
    Quit,
}

impl Command {
    const ALL: &[Self] = &[
        Self::OpenFile,
        Self::Export(ExportFormat::Pdf),
        Self::Export(ExportFormat::Png),
        Self::Export(ExportFormat::Svg),
        Self::GoToLine,
        Self::GoToPage,
        Self::ToggleDiagnostics,
        Self::ToggleFileExplorer,
        Self::UseDarkTheme,
        Self::UseLightTheme,
        Self::ReloadFonts,
        Self::Quit,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::OpenFile => "Open file",
            Self::Export(ExportFormat::Pdf) => "Export as PDF",
            Self::Export(ExportFormat::Png) => "Export as PNG",
            Self::Export(ExportFormat::Svg) => "Export as SVG",
            Self::GoToLine => "Go to line",
            Self::GoToPage => "Go to page",
            Self::ToggleDiagnostics => "Toggle diagnostics",
            Self::ToggleFileExplorer => "Toggle file explorer",
            Self::UseDarkTheme => "Use dark theme",
            Self::UseLightTheme => "Use light theme",
            Self::ReloadFonts => "Reload fonts",
            Self::Quit => "Quit",
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct CommandPalette {
    query: String,
    selected: usize,
}

impl CommandPalette {
    pub(crate) fn input(&mut self, character: char) {
        self.query.push(character);
        self.selected = 0;
    }

    pub(crate) fn input_text(&mut self, text: &str) {
        self.query.push_str(text);
        self.selected = 0;
    }

    pub(crate) fn backspace(&mut self) {
        self.query.pop();
        self.selected = 0;
    }

    pub(crate) fn move_selection(&mut self, direction: isize) {
        let count = self.matches().len();
        if count == 0 {
            self.selected = 0;
        } else {
            self.selected = self
                .selected
                .saturating_add_signed(direction)
                .min(count - 1);
        }
    }

    pub(crate) fn selected(&self) -> Option<Command> {
        self.matches().get(self.selected).copied()
    }

    pub(crate) fn draw(&self, frame: &mut Frame, theme: &Theme) {
        let area = modal_area(frame.area(), 60, 16);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color(theme.accent)))
            .style(base(theme))
            .title(" Command palette ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let [query_area, results_area] =
            Layout::vertical([Constraint::Length(2), Constraint::Fill(1)]).areas(inner);
        let available = usize::from(results_area.height);
        let lines = self
            .matches()
            .into_iter()
            .take(available)
            .enumerate()
            .map(|(index, command)| {
                let line = Line::raw(format!("  {}", command.label()));
                if index == self.selected {
                    line.style(
                        Style::default()
                            .bg(color(theme.selection))
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    line
                }
            })
            .collect::<Vec<_>>();
        let cursor = u16::try_from(UnicodeWidthStr::width(self.query.as_str())).unwrap_or(u16::MAX);
        let visible_width = inner.width.saturating_sub(2).max(1);
        let scroll = cursor.saturating_sub(visible_width.saturating_sub(1));
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("> ", Style::default().fg(color(theme.accent))),
                Span::raw(&self.query),
            ]))
            .scroll((0, scroll)),
            query_area,
        );
        frame.render_widget(Paragraph::new(lines), results_area);
        frame.set_cursor_position((
            inner
                .x
                .saturating_add(2)
                .saturating_add(cursor.saturating_sub(scroll))
                .min(inner.right().saturating_sub(1)),
            inner.y,
        ));
    }

    fn matches(&self) -> Vec<Command> {
        Command::ALL
            .iter()
            .copied()
            .filter(|command| fuzzy_match(command.label(), &self.query))
            .collect()
    }
}

fn fuzzy_match(candidate: &str, query: &str) -> bool {
    let mut candidate = candidate.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .flat_map(char::to_lowercase)
        .all(|needle| candidate.by_ref().any(|character| character == needle))
}

#[cfg(test)]
mod tests {
    use super::{Command, CommandPalette, fuzzy_match};

    #[test]
    fn fuzzy_search_matches_subsequences() {
        assert!(fuzzy_match("Export as PDF", "epdf"));
        assert!(!fuzzy_match("Export as SVG", "pdf"));

        let mut palette = CommandPalette::default();
        palette.input_text("png");
        assert_eq!(
            palette.selected(),
            Some(Command::Export(typst_tui_render::ExportFormat::Png))
        );
    }
}
