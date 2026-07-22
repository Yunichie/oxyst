use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use typst_tui_document::Motion;

use crate::{action::Action, event::Event};

pub(crate) fn resolve(event: Event, confirming_quit: bool) -> Option<Action> {
    match event {
        Event::Paste(text) if !confirming_quit => Some(Action::InsertText(text)),
        Event::CompileFinished(result) => Some(Action::CompileFinished(result)),
        Event::Tick => Some(Action::Tick),
        Event::Key(key) if is_press(key) => resolve_key(key, confirming_quit),
        Event::Paste(_) | Event::Key(_) | Event::Ignored => None,
    }
}

fn resolve_key(key: KeyEvent, confirming_quit: bool) -> Option<Action> {
    if confirming_quit {
        return match key.code {
            KeyCode::Char('y' | 'Y') => Some(Action::Quit),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(Action::CancelQuit),
            _ => None,
        };
    }

    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    match key.code {
        KeyCode::Char('q' | 'Q') if control => Some(Action::RequestQuit),
        KeyCode::Char('r' | 'R') if control => Some(Action::Recompile),
        KeyCode::Char('s' | 'S') if control => Some(Action::Save),
        KeyCode::Char('z' | 'Z') if control && shift => Some(Action::Redo),
        KeyCode::Char('z' | 'Z') if control => Some(Action::Undo),
        KeyCode::Char('y' | 'Y') if control => Some(Action::Redo),
        KeyCode::Left if control => Some(Action::Move(Motion::WordLeft)),
        KeyCode::Right if control => Some(Action::Move(Motion::WordRight)),
        KeyCode::Left => Some(Action::Move(Motion::Left)),
        KeyCode::Right => Some(Action::Move(Motion::Right)),
        KeyCode::Up => Some(Action::Move(Motion::Up)),
        KeyCode::Down => Some(Action::Move(Motion::Down)),
        KeyCode::PageUp if control => Some(Action::ScrollPreviewPages(-1)),
        KeyCode::PageDown if control => Some(Action::ScrollPreviewPages(1)),
        KeyCode::Tab | KeyCode::F(6) => Some(Action::SwitchFocus),
        KeyCode::Home if control => Some(Action::Move(Motion::DocumentStart)),
        KeyCode::End if control => Some(Action::Move(Motion::DocumentEnd)),
        KeyCode::Home => Some(Action::Move(Motion::LineStart)),
        KeyCode::End => Some(Action::Move(Motion::LineEnd)),
        KeyCode::Backspace => Some(Action::Backspace),
        KeyCode::Delete => Some(Action::Delete),
        KeyCode::Enter => Some(Action::Insert('\n')),
        KeyCode::Char(character)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER) =>
        {
            Some(Action::Insert(character))
        }
        _ => None,
    }
}

fn is_press(key: KeyEvent) -> bool {
    matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::resolve_key;
    use crate::action::Action;

    #[test]
    fn typing_is_non_modal() {
        let action = resolve_key(
            KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT),
            false,
        );

        assert!(matches!(action, Some(Action::Insert('A'))));
    }

    #[test]
    fn quit_confirmation_consumes_text_input() {
        let action = resolve_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE), true);

        assert!(matches!(action, Some(Action::CancelQuit)));
    }
}
