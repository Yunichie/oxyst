use std::path::PathBuf;

use oxyst_config::CommandId;
use oxyst_render::ExportFormat;
use oxyst_theme::Theme;
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use super::{
    CommandPalette, Component, Help, Prompt, PromptKind, RecentPicker, Search, SearchMode,
    modal_area,
};
use crate::{
    action::Action,
    input::{InputMode, Keymap},
    style::{base, color},
};

#[derive(Debug)]
pub(crate) enum ConfirmIntent {
    Open(PathBuf),
    SaveAs(PathBuf),
    Export(ExportFormat, PathBuf),
}

#[derive(Debug)]
struct Confirmation {
    message: String,
    intent: ConfirmIntent,
}

#[derive(Debug, Default)]
enum Overlay {
    #[default]
    None,
    Palette(CommandPalette),
    Prompt(Prompt),
    Search(Search),
    Help(Help),
    Recent(RecentPicker),
    Confirm(Confirmation),
}

pub(crate) enum OverlaySubmission {
    Command(CommandId),
    Prompt(Prompt),
    Confirm(ConfirmIntent),
    Recent(PathBuf),
}

pub(crate) struct OverlayHost {
    current: Overlay,
    keymap: Keymap,
    theme: Theme,
}

impl OverlayHost {
    pub(crate) fn new(keymap: Keymap, theme: Theme) -> Self {
        Self {
            current: Overlay::None,
            keymap,
            theme,
        }
    }

    pub(crate) fn input_mode(&self) -> Option<InputMode> {
        match self.current {
            Overlay::None => None,
            Overlay::Palette(_) | Overlay::Prompt(_) => Some(InputMode::Overlay),
            Overlay::Search(_) => Some(InputMode::Search),
            Overlay::Help(_) => Some(InputMode::Help),
            Overlay::Recent(_) => Some(InputMode::Overlay),
            Overlay::Confirm(_) => Some(InputMode::Confirmation),
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        !matches!(self.current, Overlay::None)
    }

    pub(crate) fn close(&mut self) {
        self.current = Overlay::None;
    }

    pub(crate) fn open_palette(&mut self) {
        self.current = Overlay::Palette(CommandPalette::default());
    }

    pub(crate) fn open_help(&mut self) {
        self.current = Overlay::Help(Help::default());
    }

    pub(crate) fn open_prompt(&mut self, kind: PromptKind, value: impl Into<String>) {
        self.current = Overlay::Prompt(Prompt::new(kind, value));
    }

    pub(crate) fn open_search(&mut self, mode: SearchMode, query: impl Into<String>) {
        self.current = Overlay::Search(Search::new(mode, query));
    }

    pub(crate) fn open_recent(&mut self, entries: Vec<PathBuf>) {
        self.current = Overlay::Recent(RecentPicker::new(entries, self.theme));
    }

    pub(crate) fn confirm(&mut self, message: String, intent: ConfirmIntent) {
        self.current = Overlay::Confirm(Confirmation { message, intent });
    }

    pub(crate) fn search_query(&self) -> Option<&str> {
        match &self.current {
            Overlay::Search(search) => Some(search.query()),
            _ => None,
        }
    }

    pub(crate) fn search_replace(&self) -> Option<(&str, &str, bool)> {
        match &self.current {
            Overlay::Search(search) => {
                Some((search.query(), search.replacement(), search.can_replace()))
            }
            _ => None,
        }
    }

    pub(crate) fn search_query_is_active(&self) -> bool {
        matches!(&self.current, Overlay::Search(search) if search.query_is_active())
    }

    pub(crate) fn toggle_search_field(&mut self) {
        if let Overlay::Search(search) = &mut self.current {
            search.toggle_field();
        }
    }

    pub(crate) fn submit(&mut self) -> Option<OverlaySubmission> {
        match std::mem::take(&mut self.current) {
            Overlay::Palette(palette) => palette.selected().map(OverlaySubmission::Command),
            Overlay::Prompt(prompt) => Some(OverlaySubmission::Prompt(prompt)),
            Overlay::Confirm(confirmation) => Some(OverlaySubmission::Confirm(confirmation.intent)),
            Overlay::Recent(recent) => recent.selected().map(OverlaySubmission::Recent),
            Overlay::Search(_) | Overlay::Help(_) | Overlay::None => None,
        }
    }

    pub(crate) fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    pub(crate) fn update(&mut self, action: Action) {
        match (&mut self.current, action) {
            (Overlay::Palette(palette), Action::OverlayInput(character)) => {
                palette.input(character);
            }
            (Overlay::Palette(palette), Action::OverlayInputText(text)) => {
                palette.input_text(&text);
            }
            (Overlay::Palette(palette), Action::OverlayBackspace) => palette.backspace(),
            (Overlay::Palette(palette), Action::OverlayMove(direction)) => {
                palette.move_selection(direction);
            }
            (Overlay::Prompt(prompt), Action::OverlayInput(character)) => {
                prompt.input(character);
            }
            (Overlay::Prompt(prompt), Action::OverlayInputText(text)) => {
                prompt.input_text(&text);
            }
            (Overlay::Prompt(prompt), Action::OverlayBackspace) => prompt.backspace(),
            (Overlay::Search(search), Action::OverlayInput(character)) => {
                search.input(character);
            }
            (Overlay::Search(search), Action::OverlayInputText(text)) => {
                search.input_text(&text);
            }
            (Overlay::Search(search), Action::OverlayBackspace) => search.backspace(),
            (Overlay::Help(help), Action::OverlayMove(direction)) => help.scroll(direction),
            (Overlay::Recent(recent), action) => recent.update(action),
            _ => {}
        }
    }
}

impl Component for OverlayHost {
    fn draw(&mut self, frame: &mut Frame, _area: Rect, _focused: bool) {
        match &mut self.current {
            Overlay::None => {}
            Overlay::Palette(palette) => palette.draw(frame, &self.theme),
            Overlay::Prompt(prompt) => prompt.draw(frame, &self.theme),
            Overlay::Search(search) => search.draw(frame, &self.theme),
            Overlay::Help(help) => help.draw(frame, frame.area(), &self.keymap, &self.theme),
            Overlay::Recent(recent) => recent.draw(frame, frame.area(), true),
            Overlay::Confirm(confirmation) => {
                let area = modal_area(frame.area(), 72, 5);
                frame.render_widget(Clear, area);
                frame.render_widget(
                    Paragraph::new(format!(
                        "{}\n{} confirm | {} cancel",
                        confirmation.message,
                        self.keymap.display(CommandId::Confirm),
                        self.keymap.display(CommandId::CancelConfirmation)
                    ))
                    .centered()
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(color(self.theme.warning)))
                            .style(base(&self.theme))
                            .title(" Confirm "),
                    ),
                    area,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use oxyst_config::Config;
    use oxyst_theme::{ColorDepth, Theme, ThemeName};

    use super::{Keymap, OverlayHost, OverlaySubmission};
    use crate::action::Action;

    #[test]
    fn routes_recent_picker_input_to_a_submission() -> Result<(), String> {
        let theme = Theme::new(ThemeName::Dark, ColorDepth::Ansi16);
        let mut overlay = OverlayHost::new(Keymap::new(&Config::default())?, theme);
        let second = PathBuf::from("second.typ");
        overlay.open_recent(vec![PathBuf::from("first.typ"), second.clone()]);

        overlay.update(Action::OverlayMove(1));
        assert!(matches!(
            overlay.submit(),
            Some(OverlaySubmission::Recent(path)) if path == second
        ));
        Ok(())
    }
}
