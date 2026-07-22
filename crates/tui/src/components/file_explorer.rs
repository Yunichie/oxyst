use std::{
    fs,
    path::{Path, PathBuf},
};

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::Line,
    widgets::{Block, BorderType, Borders, Paragraph},
};
use typst_tui_theme::Theme;

use crate::{action::Action, style::color};

use super::Component;

const MAX_FILES: usize = 512;
const MAX_DEPTH: usize = 8;

#[derive(Debug)]
pub(crate) struct FileExplorer {
    root: PathBuf,
    files: Vec<PathBuf>,
    selected: usize,
    scroll: usize,
    viewport_height: usize,
    visible: bool,
    dirty: bool,
    theme: Theme,
}

impl FileExplorer {
    pub(crate) fn new(root: PathBuf, theme: Theme) -> Self {
        let mut explorer = Self {
            root,
            files: Vec::new(),
            selected: 0,
            scroll: 0,
            viewport_height: 0,
            visible: false,
            dirty: false,
            theme,
        };
        explorer.refresh();
        explorer
    }

    pub(crate) fn set_root(&mut self, root: PathBuf) {
        self.root = root;
        self.selected = 0;
        self.scroll = 0;
        self.refresh();
    }

    pub(crate) fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    pub(crate) fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.visible
    }

    pub(crate) fn select(&mut self, direction: isize) {
        if self.files.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self
                .selected
                .saturating_add_signed(direction)
                .min(self.files.len() - 1);
        }
        self.keep_selection_visible();
    }

    pub(crate) fn selected_path(&self) -> Option<PathBuf> {
        self.files.get(self.selected).cloned()
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn draw_explorer(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        if self.dirty {
            self.refresh();
        }
        let border = if focused {
            self.theme.accent
        } else {
            self.theme.border
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color(border)))
            .title(" Files ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.viewport_height = usize::from(inner.height);
        self.keep_selection_visible();
        let lines = self
            .files
            .iter()
            .enumerate()
            .skip(self.scroll)
            .take(usize::from(inner.height))
            .map(|(index, path)| {
                let label = path
                    .strip_prefix(&self.root)
                    .unwrap_or(path)
                    .to_string_lossy();
                let line = Line::raw(format!(" {}", label));
                if index == self.selected {
                    line.style(Style::default().bg(color(self.theme.selection)))
                } else {
                    line
                }
            })
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn refresh(&mut self) {
        let selected = self.selected_path();
        self.files.clear();
        scan(&self.root, 0, &mut self.files);
        self.files.sort();
        self.selected = selected
            .and_then(|selected| self.files.iter().position(|path| path == &selected))
            .unwrap_or_else(|| self.selected.min(self.files.len().saturating_sub(1)));
        self.dirty = false;
        self.keep_selection_visible();
    }

    fn keep_selection_visible(&mut self) {
        if self.files.is_empty() {
            self.selected = 0;
            self.scroll = 0;
            return;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.viewport_height > 0
            && self.selected >= self.scroll.saturating_add(self.viewport_height)
        {
            self.scroll = self.selected + 1 - self.viewport_height;
        }
        let max_scroll = self.files.len().saturating_sub(self.viewport_height.max(1));
        self.scroll = self.scroll.min(max_scroll);
    }
}

impl Component for FileExplorer {
    fn update(&mut self, action: Action) {
        match action {
            Action::Move(typst_tui_document::Motion::Up) => self.select(-1),
            Action::Move(typst_tui_document::Motion::Down) => self.select(1),
            Action::ToggleFileExplorer => self.toggle(),
            _ => {}
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        self.draw_explorer(frame, area, focused);
    }
}

fn scan(directory: &Path, depth: usize, files: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH || files.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        if files.len() >= MAX_FILES {
            return;
        }
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if !entry.file_name().to_string_lossy().starts_with('.')
                && entry.file_name() != "target"
            {
                scan(&path, depth + 1, files);
            }
        } else if path.extension().is_some_and(|extension| extension == "typ") {
            files.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use ratatui::{Terminal, backend::TestBackend};
    use typst_tui_theme::{ColorDepth, Theme, ThemeName};

    use super::{Component, FileExplorer};

    #[test]
    fn scrolls_to_keep_the_selection_visible_and_refreshes_changes() -> Result<(), Box<dyn Error>> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = std::env::temp_dir().join(format!(
            "typst-tui-explorer-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root)?;
        for index in 0..6 {
            fs::write(root.join(format!("file-{index}.typ")), "text")?;
        }
        let theme = Theme::new(ThemeName::Dark, ColorDepth::Ansi16);
        let mut explorer = FileExplorer::new(root.clone(), theme);
        let mut terminal = Terminal::new(TestBackend::new(24, 4))?;
        terminal.draw(|frame| explorer.draw(frame, frame.area(), true))?;

        explorer.select(3);
        terminal.draw(|frame| explorer.draw(frame, frame.area(), true))?;
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("file-3.typ"));
        assert!(!rendered.contains("file-0.typ"));

        let selected = explorer.selected_path();
        fs::write(root.join("file-new.typ"), "text")?;
        explorer.mark_dirty();
        terminal.draw(|frame| explorer.draw(frame, frame.area(), true))?;
        assert!(
            explorer
                .files
                .iter()
                .any(|path| path.ends_with("file-new.typ"))
        );
        assert_eq!(explorer.selected_path(), selected);

        drop(terminal);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
