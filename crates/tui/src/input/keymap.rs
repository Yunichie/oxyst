use std::collections::BTreeMap;

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use oxyst_config::Config;
use oxyst_document::Motion;

use crate::{action::Action, event::Event};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InputMode {
    Normal,
    Overlay,
    Help,
    Search,
    Welcome,
    Confirmation,
    QuitConfirmation,
}

#[derive(Clone, Debug)]
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
                        let chord = KeyChord::parse(binding)
                            .map_err(|error| format!("invalid `{action}` binding: {error}"))?;
                        if !allows_printable_binding(action) && chord.is_printable() {
                            return Err(format!(
                                "invalid `{action}` binding `{binding}`: unmodified printable keys \
                                 are reserved for text input"
                            ));
                        }
                        Ok(chord)
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

pub(crate) fn resolve(
    event: Event,
    mode: InputMode,
    keymap: &Keymap,
    accepts_text: bool,
) -> Option<Action> {
    match event {
        Event::CompileFinished(result) => Some(Action::CompileFinished(result)),
        Event::WordCountFinished(result) => Some(Action::WordCountFinished(result)),
        Event::PreviewPagesFinished(result) => Some(Action::PreviewPagesFinished(result)),
        Event::ExportFinished(result) => Some(Action::ExportFinished(result)),
        Event::Resize => Some(Action::Resize),
        Event::ProjectFilesChanged => Some(Action::ProjectFilesChanged),
        Event::FileWatchFailed(error) => Some(Action::FileWatchFailed(error)),
        Event::Tick => Some(Action::Tick),
        Event::Paste(text) if matches!(mode, InputMode::Normal) && accepts_text => {
            Some(Action::InsertText(text))
        }
        Event::Paste(text) if matches!(mode, InputMode::Overlay | InputMode::Search) => {
            Some(Action::OverlayInputText(text))
        }
        Event::Mouse(mouse) if matches!(mode, InputMode::Normal) => resolve_mouse(mouse),
        Event::Key(key) if is_press(key) => {
            resolve_key_with_context(key, mode, keymap, accepts_text)
        }
        Event::Paste(_) | Event::Key(_) | Event::Mouse(_) | Event::Ignored => None,
    }
}

#[cfg(test)]
fn resolve_key(key: KeyEvent, mode: InputMode, keymap: &Keymap) -> Option<Action> {
    resolve_key_with_context(key, mode, keymap, true)
}

fn resolve_key_with_context(
    key: KeyEvent,
    mode: InputMode,
    keymap: &Keymap,
    accepts_text: bool,
) -> Option<Action> {
    match mode {
        InputMode::QuitConfirmation => {
            if keymap.matches("confirm", key) {
                Some(Action::Quit)
            } else if keymap.matches("cancel_confirmation", key) {
                Some(Action::CancelQuit)
            } else {
                None
            }
        }
        InputMode::Confirmation => {
            if keymap.matches("confirm", key) {
                Some(Action::OverlaySubmit)
            } else if keymap.matches("cancel_confirmation", key) {
                Some(Action::CloseOverlay)
            } else {
                None
            }
        }
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
        InputMode::Search => resolve_search_key(key, keymap),
        InputMode::Overlay => resolve_overlay_key(key, keymap),
        InputMode::Welcome => {
            if keymap.matches("help", key) {
                Some(Action::OpenHelp)
            } else if keymap.matches("command_palette", key) {
                Some(Action::OpenCommandPalette)
            } else if keymap.matches("quit", key) {
                Some(Action::RequestQuit)
            } else if keymap.matches("welcome_new", key) {
                Some(Action::NewDocument)
            } else if keymap.matches("welcome_open", key) {
                Some(Action::OpenFile)
            } else {
                resolve_overlay_key(key, keymap)
            }
        }
        InputMode::Normal => resolve_normal_key(key, keymap, accepts_text),
    }
}

fn resolve_search_key(key: KeyEvent, keymap: &Keymap) -> Option<Action> {
    if let Some(character) = printable_character(key) {
        Some(Action::OverlayInput(character))
    } else if keymap.matches("close_overlay", key) {
        Some(Action::CloseOverlay)
    } else if keymap.matches("find_previous", key) {
        Some(Action::SearchNext(true))
    } else if keymap.matches("replace_current", key) {
        Some(Action::ReplaceCurrent)
    } else if keymap.matches("find_next", key) {
        Some(Action::SearchNext(false))
    } else if keymap.matches("search_toggle_field", key) {
        Some(Action::SearchToggleField)
    } else if keymap.matches("backspace", key) {
        Some(Action::OverlayBackspace)
    } else {
        None
    }
}

fn resolve_overlay_key(key: KeyEvent, keymap: &Keymap) -> Option<Action> {
    if let Some(character) = printable_character(key) {
        Some(Action::OverlayInput(character))
    } else if keymap.matches("close_overlay", key) {
        Some(Action::CloseOverlay)
    } else if keymap.matches("move_up", key) {
        Some(Action::OverlayMove(-1))
    } else if keymap.matches("move_down", key) {
        Some(Action::OverlayMove(1))
    } else if keymap.matches("newline", key) {
        Some(Action::OverlaySubmit)
    } else if keymap.matches("backspace", key) {
        Some(Action::OverlayBackspace)
    } else {
        None
    }
}

fn resolve_normal_key(key: KeyEvent, keymap: &Keymap, accepts_text: bool) -> Option<Action> {
    let binding = |name| keymap.matches(name, key);
    if accepts_text && let Some(character) = printable_character(key) {
        Some(Action::Insert(character))
    } else if binding("quit") {
        Some(Action::RequestQuit)
    } else if binding("select_all") {
        Some(Action::SelectAll)
    } else if binding("copy") {
        Some(Action::Copy)
    } else if binding("cut") {
        Some(Action::Cut)
    } else if binding("paste") {
        Some(Action::PasteClipboard)
    } else if binding("find_replace") {
        Some(Action::OpenReplace)
    } else if binding("find") {
        Some(Action::OpenFind)
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
    } else if binding("select_word_left") {
        Some(Action::Select(Motion::WordLeft))
    } else if binding("select_word_right") {
        Some(Action::Select(Motion::WordRight))
    } else if binding("select_left") {
        Some(Action::Select(Motion::Left))
    } else if binding("select_right") {
        Some(Action::Select(Motion::Right))
    } else if binding("select_up") {
        Some(Action::Select(Motion::Up))
    } else if binding("select_down") {
        Some(Action::Select(Motion::Down))
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
    } else if binding("select_document_start") {
        Some(Action::Select(Motion::DocumentStart))
    } else if binding("select_document_end") {
        Some(Action::Select(Motion::DocumentEnd))
    } else if binding("select_line_start") {
        Some(Action::Select(Motion::LineStart))
    } else if binding("select_line_end") {
        Some(Action::Select(Motion::LineEnd))
    } else if binding("backspace") {
        Some(Action::Backspace)
    } else if binding("delete") {
        Some(Action::Delete)
    } else if binding("newline") {
        Some(Action::Insert('\n'))
    } else {
        None
    }
}

fn printable_character(key: KeyEvent) -> Option<char> {
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
    {
        return None;
    }
    match key.code {
        KeyCode::Char(character) => Some(character),
        _ => None,
    }
}

fn allows_printable_binding(action: &str) -> bool {
    matches!(
        action,
        "command_palette"
            | "help"
            | "confirm"
            | "cancel_confirmation"
            | "welcome_new"
            | "welcome_open"
    )
}

fn resolve_mouse(mouse: MouseEvent) -> Option<Action> {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => Some(Action::MouseDown {
            column: mouse.column,
            row: mouse.row,
        }),
        MouseEventKind::Drag(MouseButton::Left) => Some(Action::MouseDrag {
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

    fn is_printable(self) -> bool {
        matches!(self.code, KeyCode::Char(_))
            && !self
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
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
    use oxyst_config::Config;

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

    #[test]
    fn terminal_paste_key_stream_cannot_open_commands() -> Result<(), String> {
        let keymap = Keymap::new(&Config::default())?;
        for character in ['#', 'l', 'e', 't', ' ', 'x', ':', ' ', '?'] {
            let modifiers = if matches!(character, ':' | '?') {
                KeyModifiers::SHIFT
            } else {
                KeyModifiers::NONE
            };
            let action = resolve_key(
                KeyEvent::new(KeyCode::Char(character), modifiers),
                InputMode::Normal,
                &keymap,
            );
            assert!(matches!(action, Some(Action::Insert(actual)) if actual == character));
        }
        Ok(())
    }

    #[test]
    fn printable_characters_remain_text_in_overlay_inputs() -> Result<(), String> {
        let keymap = Keymap::new(&Config::default())?;
        for mode in [InputMode::Search, InputMode::Overlay] {
            for character in [':', '?'] {
                let action = resolve_key(
                    KeyEvent::new(KeyCode::Char(character), KeyModifiers::SHIFT),
                    mode,
                    &keymap,
                );
                assert!(matches!(
                    action,
                    Some(Action::OverlayInput(actual)) if actual == character
                ));
            }
        }
        Ok(())
    }

    #[test]
    fn printable_bindings_are_limited_to_context_aware_commands() {
        let mut config = Config::default();
        config.keys.insert("save".to_owned(), vec![":".to_owned()]);

        let result = Keymap::new(&config);

        assert!(matches!(
            result,
            Err(error) if error.contains("unmodified printable keys are reserved for text input")
        ));
    }

    #[test]
    fn printable_shortcuts_trigger_only_outside_text_contexts() -> Result<(), String> {
        let keymap = Keymap::new(&Config::default())?;
        let key = |character| {
            super::Event::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::SHIFT))
        };

        assert!(matches!(
            super::resolve(key(':'), InputMode::Normal, &keymap, true),
            Some(Action::Insert(':'))
        ));
        assert!(matches!(
            super::resolve(key('?'), InputMode::Normal, &keymap, true),
            Some(Action::Insert('?'))
        ));
        assert!(matches!(
            super::resolve(key(':'), InputMode::Normal, &keymap, false),
            Some(Action::OpenCommandPalette)
        ));
        assert!(matches!(
            super::resolve(key('?'), InputMode::Normal, &keymap, false),
            Some(Action::OpenHelp)
        ));
        assert!(matches!(
            super::resolve(key(':'), InputMode::Welcome, &keymap, false),
            Some(Action::OpenCommandPalette)
        ));
        assert!(matches!(
            super::resolve(key('?'), InputMode::Welcome, &keymap, false),
            Some(Action::OpenHelp)
        ));
        Ok(())
    }

    #[test]
    fn printable_contextual_shortcuts_are_remappable() -> Result<(), String> {
        let mut config = Config::default();
        config
            .keys
            .insert("command_palette".to_owned(), vec![";".to_owned()]);
        config.keys.insert("help".to_owned(), vec!["!".to_owned()]);
        let keymap = Keymap::new(&config)?;

        assert!(matches!(
            super::resolve(
                super::Event::Key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::NONE)),
                InputMode::Normal,
                &keymap,
                false
            ),
            Some(Action::OpenCommandPalette)
        ));
        assert!(matches!(
            super::resolve(
                super::Event::Key(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::SHIFT)),
                InputMode::Normal,
                &keymap,
                false
            ),
            Some(Action::OpenHelp)
        ));
        Ok(())
    }

    #[test]
    fn paste_is_delivered_as_one_text_action() -> Result<(), String> {
        let keymap = Keymap::new(&Config::default())?;
        let text = ":\n?界";
        let normal = super::resolve(
            super::Event::Paste(text.to_owned()),
            InputMode::Normal,
            &keymap,
            true,
        );
        let overlay = super::resolve(
            super::Event::Paste(text.to_owned()),
            InputMode::Overlay,
            &keymap,
            true,
        );

        assert!(matches!(
            normal,
            Some(Action::InsertText(pasted)) if pasted == text
        ));
        assert!(matches!(
            overlay,
            Some(Action::OverlayInputText(pasted)) if pasted == text
        ));
        Ok(())
    }

    #[test]
    fn default_selection_and_clipboard_bindings_resolve() -> Result<(), String> {
        let keymap = Keymap::new(&Config::default())?;
        let selection = resolve_key(
            KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT),
            InputMode::Normal,
            &keymap,
        );
        let copy = resolve_key(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            InputMode::Normal,
            &keymap,
        );
        let paste = resolve_key(
            KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
            InputMode::Normal,
            &keymap,
        );

        assert!(matches!(selection, Some(Action::Select(_))));
        assert!(matches!(copy, Some(Action::Copy)));
        assert!(matches!(paste, Some(Action::PasteClipboard)));
        Ok(())
    }

    #[test]
    fn search_bindings_are_scoped_to_the_search_overlay() -> Result<(), String> {
        let keymap = Keymap::new(&Config::default())?;
        let replace = resolve_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL),
            InputMode::Search,
            &keymap,
        );
        let previous = resolve_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
            InputMode::Search,
            &keymap,
        );

        assert!(matches!(replace, Some(Action::ReplaceCurrent)));
        assert!(matches!(previous, Some(Action::SearchNext(true))));
        Ok(())
    }

    #[test]
    fn internal_layout_and_file_events_bypass_input_modes() -> Result<(), String> {
        let keymap = Keymap::new(&Config::default())?;
        assert!(matches!(
            super::resolve(super::Event::Resize, InputMode::Help, &keymap, true),
            Some(Action::Resize)
        ));
        assert!(matches!(
            super::resolve(
                super::Event::ProjectFilesChanged,
                InputMode::Confirmation,
                &keymap,
                true
            ),
            Some(Action::ProjectFilesChanged)
        ));
        Ok(())
    }

    #[test]
    fn contextual_bindings_are_configurable() -> Result<(), String> {
        let mut config = Config::default();
        config
            .keys
            .insert("confirm".to_owned(), vec!["alt+y".to_owned()]);
        config
            .keys
            .insert("cancel_confirmation".to_owned(), vec!["alt+n".to_owned()]);
        config
            .keys
            .insert("welcome_new".to_owned(), vec!["alt+n".to_owned()]);
        config
            .keys
            .insert("welcome_open".to_owned(), vec!["alt+o".to_owned()]);
        let keymap = Keymap::new(&config)?;

        assert!(matches!(
            resolve_key(
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::ALT),
                InputMode::QuitConfirmation,
                &keymap
            ),
            Some(Action::Quit)
        ));
        assert!(matches!(
            resolve_key(
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT),
                InputMode::Confirmation,
                &keymap
            ),
            Some(Action::CloseOverlay)
        ));
        assert!(matches!(
            resolve_key(
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT),
                InputMode::Welcome,
                &keymap
            ),
            Some(Action::NewDocument)
        ));
        assert!(matches!(
            resolve_key(
                KeyEvent::new(KeyCode::Char('o'), KeyModifiers::ALT),
                InputMode::Welcome,
                &keymap
            ),
            Some(Action::OpenFile)
        ));
        assert!(
            resolve_key(
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
                InputMode::Confirmation,
                &keymap
            )
            .is_none()
        );
        Ok(())
    }
}
