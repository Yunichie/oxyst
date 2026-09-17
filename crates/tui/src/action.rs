use oxyst_config::CommandId;
use oxyst_document::Motion;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Pane {
    Explorer,
    Editor,
    Preview,
}

pub(crate) enum Action {
    Command(CommandId),
    Insert(char),
    InsertText(String),
    Backspace,
    Delete,
    Move(Motion),
    Select(Motion),
    SelectAll,
    Copy,
    Cut,
    PasteClipboard,
    Undo,
    Redo,
    MouseDown { column: u16, row: u16 },
    MouseDrag { column: u16, row: u16 },
    ScrollAt { column: u16, row: u16, lines: isize },
    ZoomPreview(isize),
    OverlayInput(char),
    OverlayInputText(String),
    OverlayBackspace,
    OverlayMove(isize),
    OverlaySubmit,
    RequestQuit,
    Quit,
    CancelQuit,
}
