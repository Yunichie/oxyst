#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandContext {
    Normal,
    Overlay,
    Search,
    Help,
    Welcome,
    Confirmation,
}

#[derive(Clone, Copy, Debug)]
struct CommandSpec {
    id: CommandId,
    name: &'static str,
    label: &'static str,
    defaults: &'static [&'static str],
    contexts: &'static [CommandContext],
    palette: bool,
    allow_printable: bool,
}

const NORMAL: &[CommandContext] = &[CommandContext::Normal];
const NORMAL_WELCOME: &[CommandContext] = &[CommandContext::Normal, CommandContext::Welcome];
const NORMAL_OVERLAY_HELP_WELCOME: &[CommandContext] = &[
    CommandContext::Normal,
    CommandContext::Overlay,
    CommandContext::Help,
    CommandContext::Welcome,
];
const NORMAL_OVERLAY_SEARCH_WELCOME: &[CommandContext] = &[
    CommandContext::Normal,
    CommandContext::Overlay,
    CommandContext::Search,
    CommandContext::Welcome,
];
const OVERLAY_SEARCH_HELP_WELCOME: &[CommandContext] = &[
    CommandContext::Overlay,
    CommandContext::Search,
    CommandContext::Help,
    CommandContext::Welcome,
];
const NORMAL_OVERLAY_WELCOME: &[CommandContext] = &[
    CommandContext::Normal,
    CommandContext::Overlay,
    CommandContext::Welcome,
];
const SEARCH: &[CommandContext] = &[CommandContext::Search];
const CONFIRMATION: &[CommandContext] = &[CommandContext::Confirmation];
const WELCOME: &[CommandContext] = &[CommandContext::Welcome];

macro_rules! command {
    ($id:ident, $name:literal, $label:literal, $defaults:expr, $contexts:expr) => {
        CommandSpec {
            id: CommandId::$id,
            name: $name,
            label: $label,
            defaults: $defaults,
            contexts: $contexts,
            palette: false,
            allow_printable: false,
        }
    };
    ($id:ident, $name:literal, $label:literal, $defaults:expr, $contexts:expr, palette) => {
        CommandSpec {
            palette: true,
            ..command!($id, $name, $label, $defaults, $contexts)
        }
    };
    ($id:ident, $name:literal, $label:literal, $defaults:expr, $contexts:expr, printable) => {
        CommandSpec {
            allow_printable: true,
            ..command!($id, $name, $label, $defaults, $contexts)
        }
    };
}

macro_rules! define_commands {
    ($(command!($id:ident, $($spec:tt)*)),* $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
        #[repr(usize)]
        pub enum CommandId {
            $($id),*
        }

        const COMMANDS: &[CommandSpec] = &[
            $(command!($id, $($spec)*)),*
        ];
    };
}

define_commands! {
    command!(
        NewDocument,
        "new_document",
        "New document",
        &["ctrl+n"],
        NORMAL,
        palette
    ),
    command!(
        OpenFile,
        "open_file",
        "Open file",
        &[],
        NORMAL_WELCOME,
        palette
    ),
    command!(
        ExportPdf,
        "export_pdf",
        "Export as PDF",
        &[],
        NORMAL,
        palette
    ),
    command!(
        ExportPng,
        "export_png",
        "Export as PNG",
        &[],
        NORMAL,
        palette
    ),
    command!(
        ExportSvg,
        "export_svg",
        "Export as SVG",
        &[],
        NORMAL,
        palette
    ),
    command!(
        GoToLine,
        "go_to_line",
        "Go to line",
        &["ctrl+g"],
        NORMAL,
        palette
    ),
    command!(GoToPage, "go_to_page", "Go to page", &[], NORMAL, palette),
    command!(
        ToggleDiagnostics,
        "toggle_diagnostics",
        "Toggle diagnostics",
        &["ctrl+j"],
        NORMAL,
        palette
    ),
    command!(
        ToggleFileExplorer,
        "toggle_file_explorer",
        "Toggle file explorer",
        &["ctrl+b"],
        NORMAL,
        palette
    ),
    command!(
        UseDarkTheme,
        "use_dark_theme",
        "Use dark theme",
        &[],
        NORMAL,
        palette
    ),
    command!(
        UseLightTheme,
        "use_light_theme",
        "Use light theme",
        &[],
        NORMAL,
        palette
    ),
    command!(
        ReloadFonts,
        "reload_fonts",
        "Reload fonts",
        &[],
        NORMAL,
        palette
    ),
    command!(Quit, "quit", "Quit", &["ctrl+q"], NORMAL_WELCOME, palette),
    command!(CloseTab, "close_tab", "Close tab", &["ctrl+w"], NORMAL, palette),
    command!(
        NextTab,
        "next_tab",
        "Next tab",
        &["alt+right"],
        NORMAL,
        palette
    ),
    command!(
        PreviousTab,
        "previous_tab",
        "Previous tab",
        &["alt+left"],
        NORMAL,
        palette
    ),
    command!(Save, "save", "Save", &["ctrl+s"], NORMAL),
    command!(Recompile, "recompile", "Recompile", &["ctrl+r"], NORMAL),
    command!(Undo, "undo", "Undo", &["ctrl+z"], NORMAL),
    command!(Redo, "redo", "Redo", &["ctrl+y", "ctrl+shift+z"], NORMAL),
    command!(MoveLeft, "move_left", "Move left", &["left"], NORMAL),
    command!(MoveRight, "move_right", "Move right", &["right"], NORMAL),
    command!(
        MoveUp,
        "move_up",
        "Move up",
        &["up"],
        NORMAL_OVERLAY_HELP_WELCOME
    ),
    command!(
        MoveDown,
        "move_down",
        "Move down",
        &["down"],
        NORMAL_OVERLAY_HELP_WELCOME
    ),
    command!(
        SelectLeft,
        "select_left",
        "Select left",
        &["shift+left"],
        NORMAL
    ),
    command!(
        SelectRight,
        "select_right",
        "Select right",
        &["shift+right"],
        NORMAL
    ),
    command!(SelectUp, "select_up", "Select up", &["shift+up"], NORMAL),
    command!(
        SelectDown,
        "select_down",
        "Select down",
        &["shift+down"],
        NORMAL
    ),
    command!(
        WordLeft,
        "word_left",
        "Move one word left",
        &["ctrl+left"],
        NORMAL
    ),
    command!(
        WordRight,
        "word_right",
        "Move one word right",
        &["ctrl+right"],
        NORMAL
    ),
    command!(
        SelectWordLeft,
        "select_word_left",
        "Select one word left",
        &["ctrl+shift+left"],
        NORMAL
    ),
    command!(
        SelectWordRight,
        "select_word_right",
        "Select one word right",
        &["ctrl+shift+right"],
        NORMAL
    ),
    command!(
        LineStart,
        "line_start",
        "Move to line start",
        &["home"],
        NORMAL
    ),
    command!(LineEnd, "line_end", "Move to line end", &["end"], NORMAL),
    command!(
        SelectLineStart,
        "select_line_start",
        "Select to line start",
        &["shift+home"],
        NORMAL
    ),
    command!(
        SelectLineEnd,
        "select_line_end",
        "Select to line end",
        &["shift+end"],
        NORMAL
    ),
    command!(
        DocumentStart,
        "document_start",
        "Move to document start",
        &["ctrl+home"],
        NORMAL
    ),
    command!(
        DocumentEnd,
        "document_end",
        "Move to document end",
        &["ctrl+end"],
        NORMAL
    ),
    command!(
        SelectDocumentStart,
        "select_document_start",
        "Select to document start",
        &["ctrl+shift+home"],
        NORMAL
    ),
    command!(
        SelectDocumentEnd,
        "select_document_end",
        "Select to document end",
        &["ctrl+shift+end"],
        NORMAL
    ),
    command!(SelectAll, "select_all", "Select all", &["ctrl+a"], NORMAL),
    command!(Copy, "copy", "Copy", &["ctrl+c"], NORMAL),
    command!(Cut, "cut", "Cut", &["ctrl+x"], NORMAL),
    command!(Paste, "paste", "Paste", &["ctrl+v"], NORMAL),
    command!(Find, "find", "Find", &["ctrl+f"], NORMAL),
    command!(
        FindReplace,
        "find_replace",
        "Find and replace",
        &["ctrl+h"],
        NORMAL
    ),
    command!(FindNext, "find_next", "Find next", &["enter"], SEARCH),
    command!(
        FindPrevious,
        "find_previous",
        "Find previous",
        &["shift+enter"],
        SEARCH
    ),
    command!(
        SearchToggleField,
        "search_toggle_field",
        "Switch search field",
        &["tab"],
        SEARCH
    ),
    command!(
        ReplaceCurrent,
        "replace_current",
        "Replace current",
        &["ctrl+enter"],
        SEARCH
    ),
    command!(
        Backspace,
        "backspace",
        "Delete backward",
        &["backspace"],
        NORMAL_OVERLAY_SEARCH_WELCOME
    ),
    command!(Delete, "delete", "Delete forward", &["delete"], NORMAL),
    command!(
        Newline,
        "newline",
        "New line or choose",
        &["enter"],
        NORMAL_OVERLAY_WELCOME
    ),
    command!(
        SwitchFocus,
        "switch_focus",
        "Switch pane focus",
        &["tab", "f6"],
        NORMAL
    ),
    command!(
        Fullscreen,
        "fullscreen",
        "Full-screen focused pane",
        &["f2"],
        NORMAL
    ),
    command!(ZoomIn, "zoom_in", "Zoom preview in", &["ctrl+="], NORMAL),
    command!(ZoomOut, "zoom_out", "Zoom preview out", &["ctrl+-"], NORMAL),
    command!(
        PreviewPageUp,
        "preview_page_up",
        "Previous preview page",
        &["ctrl+pageup"],
        NORMAL
    ),
    command!(
        PreviewPageDown,
        "preview_page_down",
        "Next preview page",
        &["ctrl+pagedown"],
        NORMAL
    ),
    command!(
        CommandPalette,
        "command_palette",
        "Command palette",
        &["ctrl+shift+p", ":"],
        NORMAL_WELCOME,
        printable
    ),
    command!(
        NextDiagnostic,
        "next_diagnostic",
        "Next diagnostic",
        &["f8"],
        NORMAL
    ),
    command!(
        PreviousDiagnostic,
        "previous_diagnostic",
        "Previous diagnostic",
        &["shift+f8"],
        NORMAL
    ),
    command!(
        Help,
        "help",
        "Help",
        &["f1", "?"],
        NORMAL_WELCOME,
        printable
    ),
    command!(
        CloseOverlay,
        "close_overlay",
        "Close overlay",
        &["esc"],
        OVERLAY_SEARCH_HELP_WELCOME
    ),
    command!(
        Confirm,
        "confirm",
        "Confirm",
        &["y"],
        CONFIRMATION,
        printable
    ),
    command!(
        CancelConfirmation,
        "cancel_confirmation",
        "Cancel",
        &["n", "esc"],
        CONFIRMATION,
        printable
    ),
    command!(
        DiscardChanges,
        "discard_changes",
        "Discard changes",
        &["d"],
        CONFIRMATION,
        printable
    ),
    command!(
        WelcomeNew,
        "welcome_new",
        "Create a new document",
        &["n"],
        WELCOME,
        printable
    ),
    command!(
        WelcomeOpen,
        "welcome_open",
        "Open a document",
        &["o"],
        WELCOME,
        printable
    ),
}

impl CommandId {
    pub fn all() -> impl ExactSizeIterator<Item = Self> {
        COMMANDS.iter().map(|command| command.id)
    }

    pub fn from_name(name: &str) -> Option<Self> {
        COMMANDS
            .iter()
            .find(|command| command.name == name)
            .map(|command| command.id)
    }

    pub fn name(self) -> &'static str {
        self.spec().name
    }

    pub fn label(self) -> &'static str {
        self.spec().label
    }

    pub fn default_bindings(self) -> &'static [&'static str] {
        self.spec().defaults
    }

    pub fn supports(self, context: CommandContext) -> bool {
        self.spec().contexts.contains(&context)
    }

    pub fn show_in_palette(self) -> bool {
        self.spec().palette
    }

    pub fn allows_printable_binding(self) -> bool {
        self.spec().allow_printable
    }

    fn spec(self) -> &'static CommandSpec {
        &COMMANDS[self as usize]
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{COMMANDS, CommandId};

    #[test]
    fn every_command_has_one_unique_specification() {
        assert_eq!(COMMANDS.len(), CommandId::all().len());
        assert!(COMMANDS.iter().enumerate().all(|(index, command)| {
            command.id as usize == index && CommandId::from_name(command.name) == Some(command.id)
        }));
        assert_eq!(
            COMMANDS
                .iter()
                .map(|command| command.name)
                .collect::<BTreeSet<_>>()
                .len(),
            COMMANDS.len()
        );
    }
}
