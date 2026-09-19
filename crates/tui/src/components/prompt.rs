use oxyst_render::ExportFormat;
use oxyst_theme::Theme;
use ratatui::{
    Frame,
    style::Style,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::documents::DocumentId;
use crate::style::{base, color};

use super::modal_area;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PromptKind {
    OpenFile,
    SaveAs(DocumentId),
    Export(DocumentId, ExportFormat),
    GoToLine(DocumentId),
    GoToPage(DocumentId),
}

impl PromptKind {
    const fn title(self) -> &'static str {
        match self {
            Self::OpenFile => " Open file ",
            Self::SaveAs(_) => " Save as ",
            Self::Export(_, ExportFormat::Pdf) => " Export PDF ",
            Self::Export(_, ExportFormat::Png) => " Export PNG ",
            Self::Export(_, ExportFormat::Svg) => " Export SVG ",
            Self::GoToLine(_) => " Go to line ",
            Self::GoToPage(_) => " Go to page ",
        }
    }
}

#[derive(Debug)]
pub(crate) struct Prompt {
    kind: PromptKind,
    value: String,
}

impl Prompt {
    pub(crate) fn new(kind: PromptKind, value: impl Into<String>) -> Self {
        Self {
            kind,
            value: value.into(),
        }
    }

    pub(crate) fn kind(&self) -> PromptKind {
        self.kind
    }

    pub(crate) fn value(&self) -> &str {
        &self.value
    }

    pub(crate) fn input(&mut self, character: char) {
        self.value.push(character);
    }

    pub(crate) fn input_text(&mut self, text: &str) {
        self.value.push_str(text);
    }

    pub(crate) fn backspace(&mut self) {
        self.value.pop();
    }

    pub(crate) fn draw(&self, frame: &mut Frame, theme: &Theme) {
        let area = modal_area(frame.area(), 72, 5);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color(theme.accent)))
            .style(base(theme))
            .title(self.kind.title());
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let cursor = u16::try_from(UnicodeWidthStr::width(self.value.as_str())).unwrap_or(u16::MAX);
        let visible_width = inner.width.saturating_sub(2).max(1);
        let scroll = cursor.saturating_sub(visible_width.saturating_sub(1));
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("> ", Style::default().fg(color(theme.accent))),
                Span::raw(&self.value),
            ]))
            .scroll((0, scroll)),
            inner,
        );
        frame.set_cursor_position((
            inner
                .x
                .saturating_add(2)
                .saturating_add(cursor.saturating_sub(scroll))
                .min(inner.right().saturating_sub(1)),
            inner.y,
        ));
    }
}
