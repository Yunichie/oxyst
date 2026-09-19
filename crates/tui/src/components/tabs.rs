use oxyst_theme::Theme;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{documents::DocumentTabState, style::color};

#[derive(Debug)]
pub(crate) struct Tabs {
    theme: Theme,
    hits: Vec<(usize, Rect)>,
}

impl Tabs {
    pub(crate) fn new(theme: Theme) -> Self {
        Self {
            theme,
            hits: Vec::new(),
        }
    }

    pub(crate) fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    pub(crate) fn tab_at(&self, column: u16, row: u16) -> Option<usize> {
        self.hits.iter().find_map(|(index, area)| {
            (row == area.y && column >= area.x && column < area.right()).then_some(*index)
        })
    }

    pub(crate) fn draw(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        tabs: &[DocumentTabState],
        active: usize,
    ) {
        self.hits.clear();
        if area.width == 0 || area.height == 0 || tabs.is_empty() {
            return;
        }

        let labels = tabs
            .iter()
            .map(|tab| format!(" {}{} ", if tab.dirty { "● " } else { "" }, tab.label))
            .collect::<Vec<_>>();
        let start = visible_start(&labels, active, usize::from(area.width));
        let mut spans = Vec::new();
        let mut x = area.x;
        for (index, label) in labels.iter().enumerate().skip(start) {
            let remaining = usize::from(area.right().saturating_sub(x));
            if remaining == 0 {
                break;
            }
            let label = truncate(label, remaining);
            let width = UnicodeWidthStr::width(label.as_str());
            if width == 0 {
                break;
            }
            let width = u16::try_from(width)
                .unwrap_or(u16::MAX)
                .min(area.right() - x);
            let style = if index == active {
                Style::default()
                    .fg(color(self.theme.background))
                    .bg(color(self.theme.accent))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(color(self.theme.foreground))
                    .bg(color(self.theme.surface))
            };
            spans.push(Span::styled(label, style));
            self.hits.push((index, Rect::new(x, area.y, width, 1)));
            x = x.saturating_add(width);
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }
}

fn visible_start(labels: &[String], active: usize, width: usize) -> usize {
    let mut start = 0;
    while start < active
        && labels[start..=active]
            .iter()
            .map(|label| UnicodeWidthStr::width(label.as_str()))
            .sum::<usize>()
            > width
    {
        start += 1;
    }
    start
}

fn truncate(value: &str, width: usize) -> String {
    if UnicodeWidthStr::width(value) <= width {
        return value.to_owned();
    }
    if width == 1 {
        return "…".to_owned();
    }
    let mut output = String::new();
    let target = width - 1;
    let mut used = 0;
    for character in value.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if used + character_width > target {
            break;
        }
        output.push(character);
        used += character_width;
    }
    output.push('…');
    output
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use oxyst_theme::{ColorDepth, Theme, ThemeName};
    use ratatui::{Terminal, backend::TestBackend};

    use super::{Tabs, truncate, visible_start};
    use crate::documents::DocumentTabState;

    #[test]
    fn keeps_active_tab_visible_and_truncates_on_character_boundaries() {
        let labels = vec![
            " First ".to_owned(),
            " Second ".to_owned(),
            " 日本語 ".to_owned(),
        ];
        assert_eq!(visible_start(&labels, 2, 12), 2);
        assert_eq!(truncate(" 日本語 ", 6), " 日本…");
    }

    #[test]
    fn active_overflow_tab_is_drawn_and_clickable() -> Result<(), Infallible> {
        let theme = Theme::new(ThemeName::Dark, ColorDepth::Ansi16);
        let mut tabs = Tabs::new(theme);
        let states = [
            DocumentTabState {
                label: "First".to_owned(),
                dirty: false,
            },
            DocumentTabState {
                label: "Second".to_owned(),
                dirty: true,
            },
            DocumentTabState {
                label: "Third".to_owned(),
                dirty: false,
            },
        ];
        let mut terminal = Terminal::new(TestBackend::new(12, 1))?;
        terminal.draw(|frame| tabs.draw(frame, frame.area(), &states, 2))?;

        assert!(tabs.hits.iter().any(|(index, _)| *index == 2));
        let area = tabs
            .hits
            .iter()
            .find_map(|(index, area)| (*index == 2).then_some(*area))
            .unwrap_or_default();
        assert_eq!(tabs.tab_at(area.x, area.y), Some(2));
        Ok(())
    }
}
