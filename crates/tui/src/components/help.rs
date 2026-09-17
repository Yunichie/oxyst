use oxyst_config::CommandId;
use oxyst_theme::Theme;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

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
        let title = format!(
            " Help | scroll: {} / {} | close: {} ",
            keymap.display(CommandId::MoveUp),
            keymap.display(CommandId::MoveDown),
            keymap.display(CommandId::CloseOverlay)
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color(theme.accent)))
            .style(base(theme))
            .title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let bindings = std::iter::once(("Type text", "Printable characters / paste".to_owned()))
            .chain(CommandId::all().filter_map(|command| {
                let keys = keymap.display(command);
                (!keys.is_empty()).then_some((command.label(), keys))
            }))
            .chain([
                ("Preview to source", "Left click rendered text".to_owned()),
                ("Scroll preview", "Mouse wheel".to_owned()),
            ]);
        let mut lines = vec![Line::styled(
            "Keyboard",
            Style::default()
                .fg(color(theme.accent))
                .add_modifier(Modifier::BOLD),
        )];
        lines.extend(bindings.map(|(action, keys)| {
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

#[cfg(test)]
mod tests {
    use oxyst_config::Config;
    use oxyst_theme::{ColorDepth, Theme, ThemeName};
    use ratatui::{Terminal, backend::TestBackend};

    use super::{Help, Keymap};

    #[test]
    fn title_uses_configured_navigation_bindings() -> Result<(), Box<dyn std::error::Error>> {
        let mut config = Config::default();
        config
            .keys
            .insert("move_up".to_owned(), vec!["alt+k".to_owned()]);
        config
            .keys
            .insert("move_down".to_owned(), vec!["alt+j".to_owned()]);
        config
            .keys
            .insert("close_overlay".to_owned(), vec!["alt+x".to_owned()]);
        config
            .keys
            .insert("reload_fonts".to_owned(), vec!["alt+r".to_owned()]);
        let keymap = Keymap::new(&config).map_err(std::io::Error::other)?;
        let theme = Theme::new(ThemeName::Dark, ColorDepth::Ansi16);
        let help = Help::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 12))?;
        terminal.draw(|frame| help.draw(frame, frame.area(), &keymap, &theme))?;
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("scroll: alt+k / alt+j"));
        assert!(rendered.contains("close: alt+x"));
        assert!(rendered.contains("Reload fonts"));
        assert!(rendered.contains("alt+r"));
        Ok(())
    }
}
