use std::path::{Path, PathBuf};

use oxyst_theme::Theme;

use crate::components::{Diagnostics, Editor, PreviewViewState};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct DocumentId(u64);

impl DocumentId {
    pub(crate) fn value(self) -> u64 {
        self.0
    }
}

#[derive(Debug)]
struct DocumentTab {
    id: DocumentId,
    path: Option<PathBuf>,
    display_name: String,
    editor: Option<Editor>,
    diagnostics: Option<Diagnostics>,
    preview: PreviewViewState,
}

#[derive(Debug, Default)]
pub(crate) struct Documents {
    tabs: Vec<DocumentTab>,
    active: Option<usize>,
    next_id: u64,
    untitled_count: usize,
}

impl Documents {
    pub(crate) fn active_id(&self) -> Option<DocumentId> {
        self.active.map(|index| self.tabs[index].id)
    }

    pub(crate) fn active_index(&self) -> Option<usize> {
        self.active
    }

    pub(crate) fn id_at(&self, index: usize) -> Option<DocumentId> {
        self.tabs.get(index).map(|tab| tab.id)
    }

    pub(crate) fn active_path(&self) -> Option<&Path> {
        self.active
            .and_then(|index| self.tabs[index].path.as_deref())
    }

    pub(crate) fn active_display_name(&self) -> Option<&str> {
        self.active
            .map(|index| self.tabs[index].display_name.as_str())
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub(crate) fn index_of(&self, id: DocumentId) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    pub(crate) fn find_path(&self, path: &Path) -> Option<DocumentId> {
        self.tabs
            .iter()
            .find(|tab| tab.path.as_deref() == Some(path))
            .map(|tab| tab.id)
    }

    pub(crate) fn path(&self, id: DocumentId) -> Option<&Path> {
        self.index_of(id)
            .and_then(|index| self.tabs[index].path.as_deref())
    }

    pub(crate) fn display_name(&self, id: DocumentId) -> Option<&str> {
        self.index_of(id)
            .map(|index| self.tabs[index].display_name.as_str())
    }

    pub(crate) fn ids(&self) -> impl Iterator<Item = DocumentId> + '_ {
        self.tabs.iter().map(|tab| tab.id)
    }

    pub(crate) fn tab_states(&self, active_dirty: bool) -> Vec<DocumentTabState> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| DocumentTabState {
                label: tab.display_name.clone(),
                dirty: if Some(index) == self.active {
                    active_dirty
                } else {
                    tab.editor.as_ref().is_some_and(Editor::is_dirty)
                },
            })
            .collect()
    }

    pub(crate) fn insert(
        &mut self,
        path: Option<PathBuf>,
        editor: Editor,
        diagnostics: Diagnostics,
        preview: PreviewViewState,
        active_editor: &mut Editor,
        active_diagnostics: &mut Diagnostics,
    ) -> DocumentId {
        self.park_active(active_editor, active_diagnostics, preview);
        self.next_id = self.next_id.saturating_add(1);
        let id = DocumentId(self.next_id);
        let display_name = path.as_deref().map_or_else(
            || {
                self.untitled_count = self.untitled_count.saturating_add(1);
                match self.untitled_count {
                    1 => "Untitled".to_owned(),
                    number => format!("Untitled {number}"),
                }
            },
            display_name,
        );
        self.tabs.push(DocumentTab {
            id,
            path,
            display_name,
            editor: None,
            diagnostics: None,
            preview: PreviewViewState::default(),
        });
        self.active = Some(self.tabs.len() - 1);
        *active_editor = editor;
        *active_diagnostics = diagnostics;
        id
    }

    pub(crate) fn activate(
        &mut self,
        id: DocumentId,
        active_editor: &mut Editor,
        active_diagnostics: &mut Diagnostics,
        preview: PreviewViewState,
    ) -> Option<PreviewViewState> {
        let target = self.index_of(id)?;
        if Some(target) == self.active {
            return None;
        }
        let editor = self.tabs[target].editor.take()?;
        let Some(diagnostics) = self.tabs[target].diagnostics.take() else {
            self.tabs[target].editor = Some(editor);
            return None;
        };
        let target_preview = self.tabs[target].preview;
        self.park_active(active_editor, active_diagnostics, preview);
        *active_editor = editor;
        *active_diagnostics = diagnostics;
        self.active = Some(target);
        Some(target_preview)
    }

    pub(crate) fn relative_id(&self, offset: isize) -> Option<DocumentId> {
        let active = self.active?;
        if self.tabs.len() < 2 {
            return None;
        }
        let len = self.tabs.len() as isize;
        let target = (active as isize + offset).rem_euclid(len) as usize;
        Some(self.tabs[target].id)
    }

    pub(crate) fn remove_active(
        &mut self,
        active_editor: &mut Editor,
        active_diagnostics: &mut Diagnostics,
    ) -> Option<(DocumentId, PreviewViewState)> {
        let active = self.active?;
        if self.tabs.len() == 1 {
            let removed = self.tabs.remove(active);
            self.active = None;
            return Some((removed.id, PreviewViewState::default()));
        }
        let target = if active + 1 < self.tabs.len() {
            active + 1
        } else {
            active - 1
        };
        let editor = self.tabs[target].editor.take()?;
        let Some(diagnostics) = self.tabs[target].diagnostics.take() else {
            self.tabs[target].editor = Some(editor);
            return None;
        };
        let preview = self.tabs[target].preview;
        let removed = self.tabs.remove(active);
        let target = if target > active { target - 1 } else { target };
        *active_editor = editor;
        *active_diagnostics = diagnostics;
        self.active = Some(target);
        Some((removed.id, preview))
    }

    pub(crate) fn rename(&mut self, id: DocumentId, path: PathBuf) {
        if let Some(index) = self.index_of(id) {
            self.tabs[index].display_name = display_name(&path);
            self.tabs[index].path = Some(path);
        }
    }

    pub(crate) fn is_dirty(&self, id: DocumentId, active_editor: &Editor) -> bool {
        let Some(index) = self.index_of(id) else {
            return false;
        };
        if Some(index) == self.active {
            active_editor.is_dirty()
        } else {
            self.tabs[index]
                .editor
                .as_ref()
                .is_some_and(Editor::is_dirty)
        }
    }

    pub(crate) fn editor(&self, id: DocumentId) -> Option<&Editor> {
        let index = self.index_of(id)?;
        (Some(index) != self.active)
            .then(|| self.tabs[index].editor.as_ref())
            .flatten()
    }

    pub(crate) fn editor_mut(&mut self, id: DocumentId) -> Option<&mut Editor> {
        let index = self.index_of(id)?;
        (Some(index) != self.active)
            .then(|| self.tabs[index].editor.as_mut())
            .flatten()
    }

    pub(crate) fn set_theme(&mut self, theme: Theme) {
        for tab in &mut self.tabs {
            if let Some(editor) = &mut tab.editor {
                editor.set_theme(theme);
            }
            if let Some(diagnostics) = &mut tab.diagnostics {
                diagnostics.set_theme(theme);
            }
        }
    }

    fn park_active(
        &mut self,
        active_editor: &mut Editor,
        active_diagnostics: &mut Diagnostics,
        preview: PreviewViewState,
    ) {
        let Some(active) = self.active else {
            return;
        };
        let tab = &mut self.tabs[active];
        tab.editor = Some(std::mem::take(active_editor));
        tab.diagnostics = Some(std::mem::take(active_diagnostics));
        tab.preview = preview;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DocumentTabState {
    pub(crate) label: String,
    pub(crate) dirty: bool,
}

fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use oxyst_theme::{ColorDepth, Theme, ThemeName};

    use super::{Diagnostics, Documents, Editor, PreviewViewState};
    use crate::action::EditorAction;

    fn theme() -> Theme {
        Theme::new(ThemeName::Dark, ColorDepth::Ansi16)
    }

    fn insert(
        documents: &mut Documents,
        editor: &mut Editor,
        diagnostics: &mut Diagnostics,
        text: &str,
    ) -> super::DocumentId {
        documents.insert(
            None,
            Editor::new(text, theme(), false),
            Diagnostics::new(theme()),
            PreviewViewState::default(),
            editor,
            diagnostics,
        )
    }

    #[test]
    fn switching_preserves_independent_editor_and_dirty_state() {
        let mut documents = Documents::default();
        let mut editor = Editor::default();
        let mut diagnostics = Diagnostics::default();
        let first = insert(&mut documents, &mut editor, &mut diagnostics, "one");
        editor.update(EditorAction::Insert('!'));
        let second = insert(&mut documents, &mut editor, &mut diagnostics, "two");

        assert_eq!(documents.active_id(), Some(second));
        assert!(documents.tab_states(editor.is_dirty())[0].dirty);
        assert!(
            documents
                .activate(
                    first,
                    &mut editor,
                    &mut diagnostics,
                    PreviewViewState::default()
                )
                .is_some()
        );
        assert_eq!(editor.text(), "!one");
        assert!(editor.is_dirty());
    }

    #[test]
    fn closing_selects_the_right_tab_then_the_left_tab() {
        let mut documents = Documents::default();
        let mut editor = Editor::default();
        let mut diagnostics = Diagnostics::default();
        let first = insert(&mut documents, &mut editor, &mut diagnostics, "one");
        let second = insert(&mut documents, &mut editor, &mut diagnostics, "two");
        let third = insert(&mut documents, &mut editor, &mut diagnostics, "three");
        assert!(
            documents
                .activate(
                    second,
                    &mut editor,
                    &mut diagnostics,
                    PreviewViewState::default()
                )
                .is_some()
        );

        documents.remove_active(&mut editor, &mut diagnostics);
        assert_eq!(documents.active_id(), Some(third));
        documents.remove_active(&mut editor, &mut diagnostics);
        assert_eq!(documents.active_id(), Some(first));
    }
}
