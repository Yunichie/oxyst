use typst_tui_document::Motion;

use crate::compile::CompileResult;
use crate::export::ExportResult;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Pane {
    Explorer,
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
    ExportFinished(ExportResult),
    Tick,
    SwitchFocus,
    ToggleDiagnostics,
    NavigateDiagnostic(isize),
    Click { column: u16, row: u16 },
    ScrollAt { column: u16, row: u16, lines: isize },
    ScrollPreviewPages(isize),
    ZoomPreview(isize),
    ToggleFullscreen,
    ToggleFileExplorer,
    OpenCommandPalette,
    OpenHelp,
    OpenGoToLine,
    CloseOverlay,
    OverlayInput(char),
    OverlayInputText(String),
    OverlayBackspace,
    OverlayMove(isize),
    OverlaySubmit,
    Save,
    RequestQuit,
    Quit,
    CancelQuit,
}
