use typst_tui_document::Motion;

#[derive(Debug)]
pub(crate) enum Action {
    Insert(char),
    InsertText(String),
    Backspace,
    Delete,
    Move(Motion),
    Undo,
    Redo,
    Save,
    RequestQuit,
    Quit,
    CancelQuit,
}
