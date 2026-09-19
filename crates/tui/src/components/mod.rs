mod command_palette;
mod diagnostics;
mod editor;
mod file_explorer;
mod header;
mod help;
mod overlay;
mod preview;
mod prompt;
mod recent_picker;
mod search;
mod status_bar;
mod tabs;
mod welcome;

use ratatui::{Frame, layout::Rect};

pub(crate) use command_palette::CommandPalette;
pub(crate) use diagnostics::{Diagnostics, format_diagnostic};
pub(crate) use editor::Editor;
pub(crate) use file_explorer::FileExplorer;
pub(crate) use header::{Header, HeaderState};
pub(crate) use help::Help;
pub(crate) use overlay::{ConfirmIntent, OverlayHost, OverlaySubmission};
pub(crate) use preview::{Preview, PreviewViewState};
pub(crate) use prompt::{Prompt, PromptKind};
pub(crate) use recent_picker::RecentPicker;
pub(crate) use search::{Search, SearchMode};
pub(crate) use status_bar::{StatusBar, StatusBarState};
pub(crate) use tabs::Tabs;
pub(crate) use welcome::{Welcome, WelcomeChoice};

pub(crate) trait Component {
    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool);
}

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
