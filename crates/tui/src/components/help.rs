use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};
use typst_tui_theme::Theme;

use crate::{
    input::Keymap,
    style::{base, color},
};

#[derive(Debug, Default)]
pub(crate) struct Help {
    scroll: u16,
}

impl Help {
    pub(crate) fn scroll(&mut self, direction: isize) {
        self.scroll = self.scroll.saturating_add_signed(
            i16::try_from(direction).unwrap_or(if direction < 0 { i16::MIN } else { i16::MAX }),
        );
    }

    pub(crate) fn draw(&self, frame: &mut Frame, area: Rect, keymap: &Keymap, theme: &Theme) {
        frame.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color(theme.accent)))
            .style(base(theme))
            .title(" Help | Up/Down to scroll | Esc to close ");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let bindings = [
            ("Type text", "Printable characters / paste".to_owned()),
            (
                "Move cursor",
                four(keymap, "move_left", "move_right", "move_up", "move_down"),
            ),
            (
                "Select",
                four(
                    keymap,
                    "select_left",
                    "select_right",
                    "select_up",
                    "select_down",
                ),
            ),
            ("Move by word", pair(keymap, "word_left", "word_right")),
            (
                "Select by word",
                pair(keymap, "select_word_left", "select_word_right"),
            ),
            ("Line start / end", pair(keymap, "line_start", "line_end")),
            (
                "Select to line start / end",
                pair(keymap, "select_line_start", "select_line_end"),
            ),
            (
                "Document start / end",
                pair(keymap, "document_start", "document_end"),
            ),
            (
                "Select to document start / end",
                pair(keymap, "select_document_start", "select_document_end"),
            ),
            ("Select all", keymap.display("select_all")),
            (
                "Copy / cut / paste",
                format!(
                    "{} / {} / {}",
                    keymap.display("copy"),
                    keymap.display("cut"),
                    keymap.display("paste")
                ),
            ),
            (
                "Find / find and replace",
                pair(keymap, "find", "find_replace"),
            ),
            (
                "Find next / previous",
                pair(keymap, "find_next", "find_previous"),
            ),
            ("Switch find field", keymap.display("search_toggle_field")),
            ("Replace current", keymap.display("replace_current")),
            (
                "Delete backward / forward",
                pair(keymap, "backspace", "delete"),
            ),
            ("New line", keymap.display("newline")),
            ("Undo / redo", pair(keymap, "undo", "redo")),
            ("Save", keymap.display("save")),
            ("Recompile", keymap.display("recompile")),
            ("Switch pane focus", keymap.display("switch_focus")),
            ("Full-screen focused pane", keymap.display("fullscreen")),
            ("Zoom preview", pair(keymap, "zoom_in", "zoom_out")),
            (
                "Previous / next preview page",
                pair(keymap, "preview_page_up", "preview_page_down"),
            ),
            ("Command palette", keymap.display("command_palette")),
            ("Diagnostics drawer", keymap.display("toggle_diagnostics")),
            (
                "Previous / next diagnostic",
                pair(keymap, "previous_diagnostic", "next_diagnostic"),
            ),
            ("File explorer", keymap.display("toggle_file_explorer")),
            ("Go to line", keymap.display("go_to_line")),
            ("Help", keymap.display("help")),
            ("Quit", keymap.display("quit")),
            ("Choose / confirm", "Enter / Y".to_owned()),
            ("Cancel", keymap.display("close_overlay")),
            ("Preview to source", "Left click rendered text".to_owned()),
            ("Scroll preview", "Mouse wheel".to_owned()),
        ];
        let mut lines = vec![Line::styled(
            "Keyboard",
            Style::default()
                .fg(color(theme.accent))
                .add_modifier(Modifier::BOLD),
        )];
        lines.extend(bindings.into_iter().map(|(action, keys)| {
            Line::from(vec![
                Span::styled(
                    format!("{action:<30}"),
                    Style::default().fg(color(theme.foreground)),
                ),
                Span::styled(keys, Style::default().fg(color(theme.info))),
            ])
        }));
        frame.render_widget(
            Paragraph::new(lines)
                .scroll((self.scroll, 0))
                .wrap(Wrap { trim: false }),
            inner,
        );
    }
}

fn pair(keymap: &Keymap, first: &str, second: &str) -> String {
    format!("{} / {}", keymap.display(first), keymap.display(second))
}

fn four(keymap: &Keymap, first: &str, second: &str, third: &str, fourth: &str) -> String {
    format!(
        "{} / {} / {} / {}",
        keymap.display(first),
        keymap.display(second),
        keymap.display(third),
        keymap.display(fourth)
    )
}
