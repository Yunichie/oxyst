use typst_tui_document::Motion;

use crate::compile::CompileResult;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Pane {
    Editor,
    Preview,
}

pub(crate) enum Action {
    Insert(char),
    InsertText(String),
    Backspace,
    Delete,
    Move(Motion),
    Undo,
    Redo,
    Recompile,
    CompileFinished(CompileResult),
    Tick,
    SwitchFocus,
    ToggleDiagnostics,
    NavigateDiagnostic(isize),
    Click { column: u16, row: u16 },
    ScrollAt { column: u16, row: u16, lines: isize },
    ScrollPreviewPages(isize),
    Save,
    RequestQuit,
    Quit,
    CancelQuit,
}
