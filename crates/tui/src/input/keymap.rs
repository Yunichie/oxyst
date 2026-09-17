use std::collections::BTreeMap;

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use oxyst_config::{CommandContext, CommandId, Config};

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
    bindings: BTreeMap<CommandId, Vec<KeyChord>>,
    labels: BTreeMap<CommandId, Vec<String>>,
}

impl Keymap {
    pub(crate) fn new(config: &Config) -> Result<Self, String> {
        let bindings = config
            .keys
            .iter()
            .map(|(action, bindings)| {
                let command = CommandId::from_name(action)
                    .ok_or_else(|| format!("unknown keybinding command `{action}`"))?;
                let chords = bindings
                    .iter()
                    .map(|binding| {
                        let chord = KeyChord::parse(binding)
                            .map_err(|error| format!("invalid `{action}` binding: {error}"))?;
                        if !command.allows_printable_binding() && chord.is_printable() {
                            return Err(format!(
                                "invalid `{action}` binding `{binding}`: unmodified printable keys \
                                 are reserved for text input"
                            ));
                        }
                        Ok(chord)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((command, chords))
            })
            .collect::<Result<_, String>>()?;
        let labels = config
            .keys
            .iter()
            .filter_map(|(name, bindings)| {
                CommandId::from_name(name).map(|command| (command, bindings.clone()))
            })
            .collect();
        Ok(Self { bindings, labels })
    }

    pub(crate) fn display(&self, command: CommandId) -> String {
        self.labels
            .get(&command)
            .map(|bindings| bindings.join(" / "))
            .unwrap_or_default()
    }

    fn matches(&self, command: CommandId, context: CommandContext, key: KeyEvent) -> bool {
        if !command.supports(context) {
            return false;
        }
        self.bindings
            .get(&command)
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
        Event::Paste(_)
        | Event::Key(_)
        | Event::Mouse(_)
        | Event::Resize
        | Event::ProjectFilesChanged
        | Event::FileWatchFailed(_)
        | Event::ExplorerScanFinished(_)
        | Event::ExportFinished(_)
        | Event::Tick
        | Event::Ignored => None,
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
        InputMode::QuitConfirmation => resolve_command(
            key,
            keymap,
            CommandContext::QuitConfirmation,
            &[CommandId::Confirm, CommandId::CancelConfirmation],
        ),
        InputMode::Confirmation => resolve_command(
            key,
            keymap,
            CommandContext::Confirmation,
            &[CommandId::Confirm, CommandId::CancelConfirmation],
        ),
        InputMode::Help => resolve_command(
            key,
            keymap,
            CommandContext::Help,
            &[
                CommandId::CloseOverlay,
                CommandId::MoveUp,
                CommandId::MoveDown,
            ],
        ),
        InputMode::Search => resolve_search_key(key, keymap),
        InputMode::Overlay => resolve_overlay_key(key, keymap, CommandContext::Overlay),
        InputMode::Welcome => resolve_command(
            key,
            keymap,
            CommandContext::Welcome,
            &[
                CommandId::Help,
                CommandId::CommandPalette,
                CommandId::Quit,
                CommandId::OpenFile,
                CommandId::WelcomeNew,
                CommandId::WelcomeOpen,
            ],
        )
        .or_else(|| resolve_overlay_key(key, keymap, CommandContext::Welcome)),
        InputMode::Normal => resolve_normal_key(key, keymap, accepts_text),
    }
}

fn resolve_search_key(key: KeyEvent, keymap: &Keymap) -> Option<Action> {
    if let Some(character) = printable_character(key) {
        Some(Action::OverlayInput(character))
    } else {
        resolve_command(
            key,
            keymap,
            CommandContext::Search,
            &[
                CommandId::CloseOverlay,
                CommandId::FindPrevious,
                CommandId::ReplaceCurrent,
                CommandId::FindNext,
                CommandId::SearchToggleField,
                CommandId::Backspace,
            ],
        )
    }
}

fn resolve_overlay_key(key: KeyEvent, keymap: &Keymap, context: CommandContext) -> Option<Action> {
    if let Some(character) = printable_character(key) {
        Some(Action::OverlayInput(character))
    } else {
        resolve_command(
            key,
            keymap,
            context,
            &[
                CommandId::CloseOverlay,
                CommandId::MoveUp,
                CommandId::MoveDown,
                CommandId::Newline,
                CommandId::Backspace,
            ],
        )
    }
}

fn resolve_normal_key(key: KeyEvent, keymap: &Keymap, accepts_text: bool) -> Option<Action> {
    if accepts_text && let Some(character) = printable_character(key) {
        Some(Action::Insert(character))
    } else {
        resolve_command(
            key,
            keymap,
            CommandContext::Normal,
            &[
                CommandId::Quit,
                CommandId::SelectAll,
                CommandId::Copy,
                CommandId::Cut,
                CommandId::Paste,
                CommandId::FindReplace,
                CommandId::Find,
                CommandId::Recompile,
                CommandId::Save,
                CommandId::ToggleDiagnostics,
                CommandId::ToggleFileExplorer,
                CommandId::CommandPalette,
                CommandId::Help,
                CommandId::OpenFile,
                CommandId::ExportPdf,
                CommandId::ExportPng,
                CommandId::ExportSvg,
                CommandId::GoToLine,
                CommandId::GoToPage,
                CommandId::UseDarkTheme,
                CommandId::UseLightTheme,
                CommandId::ReloadFonts,
                CommandId::Fullscreen,
                CommandId::ZoomIn,
                CommandId::ZoomOut,
                CommandId::Redo,
                CommandId::Undo,
                CommandId::WordLeft,
                CommandId::WordRight,
                CommandId::MoveLeft,
                CommandId::MoveRight,
                CommandId::MoveUp,
                CommandId::MoveDown,
                CommandId::SelectWordLeft,
                CommandId::SelectWordRight,
                CommandId::SelectLeft,
                CommandId::SelectRight,
                CommandId::SelectUp,
                CommandId::SelectDown,
                CommandId::PreviewPageUp,
                CommandId::PreviewPageDown,
                CommandId::SwitchFocus,
                CommandId::PreviousDiagnostic,
                CommandId::NextDiagnostic,
                CommandId::DocumentStart,
                CommandId::DocumentEnd,
                CommandId::LineStart,
                CommandId::LineEnd,
                CommandId::SelectDocumentStart,
                CommandId::SelectDocumentEnd,
                CommandId::SelectLineStart,
                CommandId::SelectLineEnd,
                CommandId::Backspace,
                CommandId::Delete,
                CommandId::Newline,
            ],
        )
    }
}

fn resolve_command(
    key: KeyEvent,
    keymap: &Keymap,
    context: CommandContext,
    commands: &[CommandId],
) -> Option<Action> {
    commands
        .iter()
        .copied()
        .find(|command| keymap.matches(*command, context, key))
        .map(Action::Command)
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
    use oxyst_config::{CommandId, Config};

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
            Some(Action::Command(CommandId::Save))
        ));
        assert!(!matches!(
            resolve_key(
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
                InputMode::Normal,
                &keymap
            ),
            Some(Action::Command(CommandId::Save))
        ));
        Ok(())
    }

    #[test]
    fn palette_only_commands_can_be_bound() -> Result<(), String> {
        let mut config = Config::default();
        config
            .keys
            .insert("reload_fonts".to_owned(), vec!["alt+r".to_owned()]);
        let keymap = Keymap::new(&config)?;

        assert!(matches!(
            resolve_key(
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT),
                InputMode::Normal,
                &keymap
            ),
            Some(Action::Command(CommandId::ReloadFonts))
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
            Some(Action::Command(CommandId::CommandPalette))
        ));
        assert!(matches!(
            super::resolve(key('?'), InputMode::Normal, &keymap, false),
            Some(Action::Command(CommandId::Help))
        ));
        assert!(matches!(
            super::resolve(key(':'), InputMode::Welcome, &keymap, false),
            Some(Action::Command(CommandId::CommandPalette))
        ));
        assert!(matches!(
            super::resolve(key('?'), InputMode::Welcome, &keymap, false),
            Some(Action::Command(CommandId::Help))
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
            Some(Action::Command(CommandId::CommandPalette))
        ));
        assert!(matches!(
            super::resolve(
                super::Event::Key(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::SHIFT)),
                InputMode::Normal,
                &keymap,
                false
            ),
            Some(Action::Command(CommandId::Help))
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

        assert!(matches!(
            selection,
            Some(Action::Command(CommandId::SelectRight))
        ));
        assert!(matches!(copy, Some(Action::Command(CommandId::Copy))));
        assert!(matches!(paste, Some(Action::Command(CommandId::Paste))));
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

        assert!(matches!(
            replace,
            Some(Action::Command(CommandId::ReplaceCurrent))
        ));
        assert!(matches!(
            previous,
            Some(Action::Command(CommandId::FindPrevious))
        ));
        Ok(())
    }

    #[test]
    fn internal_events_do_not_enter_input_resolution() -> Result<(), String> {
        let keymap = Keymap::new(&Config::default())?;
        assert!(super::resolve(super::Event::Resize, InputMode::Help, &keymap, true).is_none());
        assert!(
            super::resolve(
                super::Event::ProjectFilesChanged,
                InputMode::Confirmation,
                &keymap,
                true
            )
            .is_none()
        );
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
            Some(Action::Command(CommandId::Confirm))
        ));
        assert!(matches!(
            resolve_key(
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT),
                InputMode::Confirmation,
                &keymap
            ),
            Some(Action::Command(CommandId::CancelConfirmation))
        ));
        assert!(matches!(
            resolve_key(
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT),
                InputMode::Welcome,
                &keymap
            ),
            Some(Action::Command(CommandId::WelcomeNew))
        ));
        assert!(matches!(
            resolve_key(
                KeyEvent::new(KeyCode::Char('o'), KeyModifiers::ALT),
                InputMode::Welcome,
                &keymap
            ),
            Some(Action::Command(CommandId::WelcomeOpen))
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
