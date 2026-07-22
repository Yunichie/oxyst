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

use crate::style::color;

const MAX_FILES: usize = 512;
const MAX_DEPTH: usize = 8;

#[derive(Debug)]
pub(crate) struct FileExplorer {
    root: PathBuf,
    files: Vec<PathBuf>,
    selected: usize,
    visible: bool,
}

impl FileExplorer {
    pub(crate) fn new(root: PathBuf) -> Self {
        let mut explorer = Self {
            root,
            files: Vec::new(),
            selected: 0,
            visible: false,
        };
        explorer.refresh();
        explorer
    }

    pub(crate) fn set_root(&mut self, root: PathBuf) {
        self.root = root;
        self.refresh();
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
    }

    pub(crate) fn selected_path(&self) -> Option<PathBuf> {
        self.files.get(self.selected).cloned()
    }

    pub(crate) fn draw(&self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
        let border = if focused { theme.accent } else { theme.border };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color(border)))
            .title(" Files ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let lines = self
            .files
            .iter()
            .enumerate()
            .take(usize::from(inner.height))
            .map(|(index, path)| {
                let label = path
                    .strip_prefix(&self.root)
                    .unwrap_or(path)
                    .to_string_lossy();
                let line = Line::raw(format!(" {}", label));
                if index == self.selected {
                    line.style(Style::default().bg(color(theme.selection)))
                } else {
                    line
                }
            })
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn refresh(&mut self) {
        self.files.clear();
        scan(&self.root, 0, &mut self.files);
        self.files.sort();
        self.selected = self.selected.min(self.files.len().saturating_sub(1));
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
