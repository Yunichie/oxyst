use std::{
    fs,
    path::{Path, PathBuf},
};

use oxyst_theme::Theme;
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::Line,
    widgets::{Block, BorderType, Borders, Paragraph},
};

use crate::{action::Action, explorer::MAX_FILES, style::color};

use super::Component;

#[derive(Debug)]
pub(crate) struct FileExplorer {
    root: PathBuf,
    files: Vec<PathBuf>,
    selected: usize,
    scroll: usize,
    viewport_height: usize,
    visible: bool,
    loaded: bool,
    theme: Theme,
}

impl FileExplorer {
    pub(crate) fn new(root: PathBuf, theme: Theme) -> Self {
        Self {
            root,
            files: Vec::new(),
            selected: 0,
            scroll: 0,
            viewport_height: 0,
            visible: false,
            loaded: false,
            theme,
        }
    }

    pub(crate) fn set_root(&mut self, root: PathBuf) {
        self.root = root;
        self.files.clear();
        self.selected = 0;
        self.scroll = 0;
        self.loaded = false;
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

    pub(crate) fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub(crate) fn invalidate(&mut self) {
        self.loaded = false;
    }

    pub(crate) fn install_files(&mut self, mut files: Vec<PathBuf>) {
        let selected = self.selected_path();
        files.sort();
        files.dedup();
        files.truncate(MAX_FILES);
        self.files = files;
        self.loaded = true;
        self.restore_selection(selected);
    }

    pub(crate) fn apply_paths(&mut self, paths: &[PathBuf]) -> bool {
        if !self.loaded {
            return true;
        }
        let selected = self.selected_path();
        let mut needs_rescan = false;
        for path in paths {
            if !is_typst_path(path) || !path.starts_with(&self.root) {
                continue;
            }
            let include = fs::symlink_metadata(path)
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink());
            match (self.files.binary_search(path), include) {
                (Ok(_), true) | (Err(_), false) => {}
                (Ok(index), false) => {
                    needs_rescan |= self.files.len() == MAX_FILES;
                    self.files.remove(index);
                }
                (Err(index), true) if self.files.len() < MAX_FILES => {
                    self.files.insert(index, path.clone());
                }
                (Err(_), true) => needs_rescan = true,
            }
        }
        self.restore_selection(selected);
        needs_rescan
    }

    fn draw_explorer(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
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

    fn restore_selection(&mut self, selected: Option<PathBuf>) {
        self.selected = selected
            .and_then(|selected| self.files.iter().position(|path| path == &selected))
            .unwrap_or_else(|| self.selected.min(self.files.len().saturating_sub(1)));
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
            Action::Move(oxyst_document::Motion::Up) => self.select(-1),
            Action::Move(oxyst_document::Motion::Down) => self.select(1),
            Action::ToggleFileExplorer => self.toggle(),
            _ => {}
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        self.draw_explorer(frame, area, focused);
    }
}

fn is_typst_path(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "typ")
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use oxyst_theme::{ColorDepth, Theme, ThemeName};
    use ratatui::{Terminal, backend::TestBackend};

    use super::{Component, FileExplorer};

    #[test]
    fn scrolls_to_keep_the_selection_visible_and_refreshes_changes() -> Result<(), Box<dyn Error>> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root =
            std::env::temp_dir().join(format!("oxyst-explorer-{}-{unique}", std::process::id()));
        fs::create_dir_all(&root)?;
        for index in 0..6 {
            fs::write(root.join(format!("file-{index}.typ")), "text")?;
        }
        let theme = Theme::new(ThemeName::Dark, ColorDepth::Ansi16);
        let mut explorer = FileExplorer::new(root.clone(), theme);
        explorer.install_files(
            (0..6)
                .map(|index| root.join(format!("file-{index}.typ")))
                .collect(),
        );
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
        let new_file = root.join("file-new.typ");
        fs::write(&new_file, "text")?;
        assert!(!explorer.apply_paths(&[new_file]));
        terminal.draw(|frame| explorer.draw(frame, frame.area(), true))?;
        assert!(
            explorer
                .files
                .iter()
                .any(|path| path.ends_with("file-new.typ"))
        );
        assert_eq!(explorer.selected_path(), selected);

        fs::remove_file(root.join("file-new.typ"))?;
        assert!(!explorer.apply_paths(&[root.join("file-new.typ")]));
        assert!(
            !explorer
                .files
                .iter()
                .any(|path| path.ends_with("file-new.typ"))
        );

        drop(terminal);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn full_incremental_cache_requests_a_rescan_for_new_files() -> Result<(), Box<dyn Error>> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root =
            std::env::temp_dir().join(format!("oxyst-explorer-{}-{unique}", std::process::id()));
        fs::create_dir_all(&root)?;
        let mut explorer = FileExplorer::new(
            root.clone(),
            Theme::new(ThemeName::Dark, ColorDepth::Ansi16),
        );
        explorer.install_files(
            (0..crate::explorer::MAX_FILES)
                .map(|index| root.join(format!("file-{index:03}.typ")))
                .collect(),
        );
        let added = root.join("added.typ");
        fs::write(&added, "")?;

        assert!(explorer.apply_paths(&[added]));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn incremental_updates_ignore_symlinks_and_directories() -> Result<(), Box<dyn Error>> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root =
            std::env::temp_dir().join(format!("oxyst-explorer-{}-{unique}", std::process::id()));
        fs::create_dir_all(&root)?;
        let target = root.join("target.typ");
        let link = root.join("link.typ");
        let directory = root.join("directory.typ");
        fs::write(&target, "")?;
        fs::create_dir(&directory)?;
        let linked = {
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(&target, &link)
            }
            #[cfg(windows)]
            {
                std::os::windows::fs::symlink_file(&target, &link)
            }
        };
        let mut explorer = FileExplorer::new(
            root.clone(),
            Theme::new(ThemeName::Dark, ColorDepth::Ansi16),
        );
        explorer.install_files(Vec::new());

        assert!(!explorer.apply_paths(&[target.clone(), directory]));
        assert_eq!(explorer.files, vec![target]);
        if linked.is_ok() {
            assert!(!explorer.apply_paths(&[link]));
            assert_eq!(explorer.files.len(), 1);
        }

        fs::remove_dir_all(root)?;
        Ok(())
    }
}
