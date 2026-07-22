use std::collections::BTreeMap;

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use typst_tui_config::Config;
use typst_tui_document::Motion;

use crate::{action::Action, event::Event};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InputMode {
    Normal,
    Overlay,
    Help,
    Welcome,
    Confirmation,
    QuitConfirmation,
}

#[derive(Debug)]
pub(crate) struct Keymap {
    bindings: BTreeMap<String, Vec<KeyChord>>,
    labels: BTreeMap<String, Vec<String>>,
}

impl Keymap {
    pub(crate) fn new(config: &Config) -> Result<Self, String> {
        let bindings = config
            .keys
            .iter()
            .map(|(action, bindings)| {
                let chords = bindings
                    .iter()
                    .map(|binding| {
                        KeyChord::parse(binding)
                            .map_err(|error| format!("invalid `{action}` binding: {error}"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((action.clone(), chords))
            })
            .collect::<Result<_, String>>()?;
        Ok(Self {
            bindings,
            labels: config.keys.clone(),
        })
    }

    pub(crate) fn display(&self, action: &str) -> String {
        self.labels
            .get(action)
            .map(|bindings| bindings.join(" / "))
            .unwrap_or_default()
    }

    fn matches(&self, action: &str, key: KeyEvent) -> bool {
        self.bindings
            .get(action)
            .is_some_and(|bindings| bindings.iter().any(|binding| binding.matches(key)))
    }
}

pub(crate) fn resolve(event: Event, mode: InputMode, keymap: &Keymap) -> Option<Action> {
    match event {
        Event::CompileFinished(result) => Some(Action::CompileFinished(result)),
        Event::ExportFinished(result) => Some(Action::ExportFinished(result)),
        Event::Tick => Some(Action::Tick),
        Event::Paste(text) if matches!(mode, InputMode::Normal) => Some(Action::InsertText(text)),
        Event::Paste(text) if matches!(mode, InputMode::Overlay) => {
            Some(Action::OverlayInputText(text))
        }
        Event::Mouse(mouse) if matches!(mode, InputMode::Normal) => resolve_mouse(mouse),
        Event::Key(key) if is_press(key) => resolve_key(key, mode, keymap),
        Event::Paste(_) | Event::Key(_) | Event::Mouse(_) | Event::Ignored => None,
    }
}

fn resolve_key(key: KeyEvent, mode: InputMode, keymap: &Keymap) -> Option<Action> {
    match mode {
        InputMode::QuitConfirmation => match key.code {
            KeyCode::Char('y' | 'Y') => Some(Action::Quit),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(Action::CancelQuit),
            _ => None,
        },
        InputMode::Confirmation => match key.code {
            KeyCode::Char('y' | 'Y') => Some(Action::OverlaySubmit),
            KeyCode::Char('n' | 'N') => Some(Action::CloseOverlay),
            _ if keymap.matches("close_overlay", key) => Some(Action::CloseOverlay),
            _ => None,
        },
        InputMode::Help => {
            if keymap.matches("close_overlay", key) {
                Some(Action::CloseOverlay)
            } else if keymap.matches("move_up", key) {
                Some(Action::OverlayMove(-1))
            } else if keymap.matches("move_down", key) {
                Some(Action::OverlayMove(1))
            } else {
                None
            }
        }
        InputMode::Overlay => resolve_overlay_key(key, keymap),
        InputMode::Welcome => {
            if keymap.matches("help", key) {
                Some(Action::OpenHelp)
            } else if keymap.matches("quit", key) {
                Some(Action::RequestQuit)
            } else {
                resolve_overlay_key(key, keymap)
            }
        }
        InputMode::Normal => resolve_normal_key(key, keymap),
    }
}

fn resolve_overlay_key(key: KeyEvent, keymap: &Keymap) -> Option<Action> {
    if keymap.matches("close_overlay", key) {
        Some(Action::CloseOverlay)
    } else if keymap.matches("move_up", key) {
        Some(Action::OverlayMove(-1))
    } else if keymap.matches("move_down", key) {
        Some(Action::OverlayMove(1))
    } else if keymap.matches("newline", key) {
        Some(Action::OverlaySubmit)
    } else if keymap.matches("backspace", key) {
        Some(Action::OverlayBackspace)
    } else if let KeyCode::Char(character) = key.code
        && !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
    {
        Some(Action::OverlayInput(character))
    } else {
        None
    }
}

fn resolve_normal_key(key: KeyEvent, keymap: &Keymap) -> Option<Action> {
    let binding = |name| keymap.matches(name, key);
    if binding("quit") {
        Some(Action::RequestQuit)
    } else if binding("recompile") {
        Some(Action::Recompile)
    } else if binding("save") {
        Some(Action::Save)
    } else if binding("toggle_diagnostics") {
        Some(Action::ToggleDiagnostics)
    } else if binding("toggle_file_explorer") {
        Some(Action::ToggleFileExplorer)
    } else if binding("command_palette") {
        Some(Action::OpenCommandPalette)
    } else if binding("help") {
        Some(Action::OpenHelp)
    } else if binding("go_to_line") {
        Some(Action::OpenGoToLine)
    } else if binding("fullscreen") {
        Some(Action::ToggleFullscreen)
    } else if binding("zoom_in") {
        Some(Action::ZoomPreview(1))
    } else if binding("zoom_out") {
        Some(Action::ZoomPreview(-1))
    } else if binding("redo") {
        Some(Action::Redo)
    } else if binding("undo") {
        Some(Action::Undo)
    } else if binding("word_left") {
        Some(Action::Move(Motion::WordLeft))
    } else if binding("word_right") {
        Some(Action::Move(Motion::WordRight))
    } else if binding("move_left") {
        Some(Action::Move(Motion::Left))
    } else if binding("move_right") {
        Some(Action::Move(Motion::Right))
    } else if binding("move_up") {
        Some(Action::Move(Motion::Up))
    } else if binding("move_down") {
        Some(Action::Move(Motion::Down))
    } else if binding("preview_page_up") {
        Some(Action::ScrollPreviewPages(-1))
    } else if binding("preview_page_down") {
        Some(Action::ScrollPreviewPages(1))
    } else if binding("switch_focus") {
        Some(Action::SwitchFocus)
    } else if binding("previous_diagnostic") {
        Some(Action::NavigateDiagnostic(-1))
    } else if binding("next_diagnostic") {
        Some(Action::NavigateDiagnostic(1))
    } else if binding("document_start") {
        Some(Action::Move(Motion::DocumentStart))
    } else if binding("document_end") {
        Some(Action::Move(Motion::DocumentEnd))
    } else if binding("line_start") {
        Some(Action::Move(Motion::LineStart))
    } else if binding("line_end") {
        Some(Action::Move(Motion::LineEnd))
    } else if binding("backspace") {
        Some(Action::Backspace)
    } else if binding("delete") {
        Some(Action::Delete)
    } else if binding("newline") {
        Some(Action::Insert('\n'))
    } else if let KeyCode::Char(character) = key.code
        && !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
    {
        Some(Action::Insert(character))
    } else {
        None
    }
}

fn resolve_mouse(mouse: MouseEvent) -> Option<Action> {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => Some(Action::Click {
            column: mouse.column,
            row: mouse.row,
        }),
        MouseEventKind::ScrollUp => Some(Action::ScrollAt {
            column: mouse.column,
            row: mouse.row,
            lines: -3,
        }),
        MouseEventKind::ScrollDown => Some(Action::ScrollAt {
            column: mouse.column,
            row: mouse.row,
            lines: 3,
        }),
        _ => None,
    }
}

fn is_press(key: KeyEvent) -> bool {
    matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct KeyChord {
    code: KeyCode,
    modifiers: KeyModifiers,
}

impl KeyChord {
    fn parse(source: &str) -> Result<Self, String> {
        let parts = source.split('+').collect::<Vec<_>>();
        let Some(key) = parts.last() else {
            return Err("binding is empty".to_owned());
        };
        if key.is_empty() {
            return Err(format!("`{source}` has no key"));
        }
        let mut modifiers = KeyModifiers::NONE;
        for modifier in &parts[..parts.len().saturating_sub(1)] {
            let flag = match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => KeyModifiers::CONTROL,
                "shift" => KeyModifiers::SHIFT,
                "alt" => KeyModifiers::ALT,
                "super" => KeyModifiers::SUPER,
                _ => return Err(format!("`{modifier}` is not a modifier")),
            };
            modifiers.insert(flag);
        }
        let code = parse_code(key).ok_or_else(|| format!("`{key}` is not a supported key"))?;
        Ok(Self { code, modifiers })
    }

    fn matches(self, event: KeyEvent) -> bool {
        let mut actual_modifiers = event.modifiers.intersection(
            KeyModifiers::CONTROL | KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::SUPER,
        );
        if matches!(self.code, KeyCode::Char(character) if !character.is_ascii_alphabetic())
            && !self.modifiers.contains(KeyModifiers::SHIFT)
        {
            actual_modifiers.remove(KeyModifiers::SHIFT);
        }
        actual_modifiers == self.modifiers && codes_match(self.code, event.code)
    }
}

fn parse_code(source: &str) -> Option<KeyCode> {
    let normalized = source.to_ascii_lowercase();
    let named = match normalized.as_str() {
        "left" => Some(KeyCode::Left),
        "right" => Some(KeyCode::Right),
        "up" => Some(KeyCode::Up),
        "down" => Some(KeyCode::Down),
        "home" => Some(KeyCode::Home),
        "end" => Some(KeyCode::End),
        "pageup" => Some(KeyCode::PageUp),
        "pagedown" => Some(KeyCode::PageDown),
        "backspace" => Some(KeyCode::Backspace),
        "delete" => Some(KeyCode::Delete),
        "enter" => Some(KeyCode::Enter),
        "tab" => Some(KeyCode::Tab),
        "esc" | "escape" => Some(KeyCode::Esc),
        "space" => Some(KeyCode::Char(' ')),
        _ => normalized
            .strip_prefix('f')
            .and_then(|number| number.parse::<u8>().ok())
            .filter(|number| (1..=24).contains(number))
            .map(KeyCode::F),
    };
    named.or_else(|| {
        let mut characters = source.chars();
        let character = characters.next()?;
        characters
            .next()
            .is_none()
            .then_some(KeyCode::Char(character))
    })
}

fn codes_match(expected: KeyCode, actual: KeyCode) -> bool {
    match (expected, actual) {
        (KeyCode::Char(expected), KeyCode::Char(actual)) => expected.eq_ignore_ascii_case(&actual),
        _ => expected == actual,
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use typst_tui_config::Config;

    use super::{InputMode, KeyChord, Keymap, resolve_key};
    use crate::action::Action;

    #[test]
    fn configured_bindings_override_defaults() -> Result<(), String> {
        let mut config = Config::default();
        config
            .keys
            .insert("save".to_owned(), vec!["alt+w".to_owned()]);
        let keymap = Keymap::new(&config)?;

        assert!(matches!(
            resolve_key(
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT),
                InputMode::Normal,
                &keymap
            ),
            Some(Action::Save)
        ));
        assert!(!matches!(
            resolve_key(
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
                InputMode::Normal,
                &keymap
            ),
            Some(Action::Save)
        ));
        Ok(())
    }

    #[test]
    fn shifted_punctuation_matches_literal_binding() -> Result<(), String> {
        let chord = KeyChord::parse(":")?;
        assert!(chord.matches(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::SHIFT)));
        Ok(())
    }

    #[test]
    fn typing_remains_non_modal() -> Result<(), String> {
        let keymap = Keymap::new(&Config::default())?;
        let action = resolve_key(
            KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT),
            InputMode::Normal,
            &keymap,
        );
        assert!(matches!(action, Some(Action::Insert('A'))));
        Ok(())
    }
}
