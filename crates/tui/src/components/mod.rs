mod command_palette;
mod diagnostics;
mod editor;
mod file_explorer;
mod help;
mod preview;
mod prompt;
mod search;
mod welcome;

pub(crate) use command_palette::{Command, CommandPalette};
pub(crate) use diagnostics::{Diagnostics, format_diagnostic};
pub(crate) use editor::Editor;
pub(crate) use file_explorer::FileExplorer;
pub(crate) use help::Help;
pub(crate) use preview::Preview;
pub(crate) use prompt::{Prompt, PromptKind};
pub(crate) use search::{Search, SearchMode};
pub(crate) use welcome::{Welcome, WelcomeChoice};

pub(crate) fn modal_area(
    area: ratatui::layout::Rect,
    width: u16,
    height: u16,
) -> ratatui::layout::Rect {
    let width = width.min(area.width.saturating_sub(2)).max(1);
    let height = height.min(area.height.saturating_sub(2)).max(1);
    ratatui::layout::Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}
